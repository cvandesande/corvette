//! One accepted TCP connection's RTSP session state machine.

use super::MockCameraConfig;
use super::digest::{self, Credentials};
use super::rtp;
use super::rtsp_message::{self, Request};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc};
use tokio::time::MissedTickBehavior;

const RTP_CHANNEL: u8 = 0;
const SSRC: u32 = 0x1234_5678;

pub(super) async fn handle(
    stream: TcpStream,
    config: Arc<MockCameraConfig>,
    close_signal: Arc<Notify>,
    local_addr: SocketAddr,
) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let (tx, mut rx) = mpsc::channel(8);

    // A dedicated reader task keeps `mpsc::Receiver::recv` (cancel-safe) as
    // the only thing `select!`ed against the socket's read side below --
    // selecting directly on a partially read request would drop already
    // consumed bytes on cancellation and corrupt the stream.
    let reader_task = tokio::spawn(async move {
        loop {
            match rtsp_message::read_request(&mut reader).await {
                Ok(Some(request)) => {
                    if tx.send(request).await.is_err() {
                        return;
                    }
                }
                _ => return,
            }
        }
    });

    drive_handshake(&mut rx, &mut write_half, &config, &close_signal, local_addr).await;
    reader_task.abort();
}

/// Handles `OPTIONS`/`DESCRIBE`/`SETUP` (and any auth challenges) until
/// `PLAY` starts the frame stream, or the connection ends.
async fn drive_handshake(
    rx: &mut mpsc::Receiver<Request>,
    writer: &mut (impl AsyncWrite + Unpin),
    config: &MockCameraConfig,
    close_signal: &Notify,
    local_addr: SocketAddr,
) {
    let mut last_nonce: Option<String> = None;
    let mut session_id: Option<String> = None;

    loop {
        let request = tokio::select! {
            () = close_signal.notified() => return,
            request = rx.recv() => match request {
                Some(request) => request,
                None => return,
            },
        };

        if !authenticated(&request, config, &mut last_nonce) {
            let nonce = last_nonce
                .as_deref()
                .expect("authenticated() always assigns a fresh nonce on failure");
            if send(writer, &rtsp_message::unauthorized(&request, nonce))
                .await
                .is_err()
            {
                return;
            }
            continue;
        }

        match request.method.as_str() {
            "OPTIONS" => {
                if send(writer, &rtsp_message::ok(&request, session_id.as_deref()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            "DESCRIBE" => {
                let content_base = format!("rtsp://{local_addr}/stream/");
                if send(writer, &rtsp_message::describe(&request, &content_base))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            "SETUP" => {
                let id = digest::fresh_token();
                if send(
                    writer,
                    &rtsp_message::setup(&request, &id, config.declared_timeout.as_secs()),
                )
                .await
                .is_err()
                {
                    return;
                }
                session_id = Some(id);
            }
            "PLAY" => {
                let Some(id) = session_id.clone() else {
                    let _ = send(writer, &rtsp_message::bad_request(&request)).await;
                    return;
                };
                if send(writer, &rtsp_message::play_ok(&request, &id))
                    .await
                    .is_err()
                {
                    return;
                }
                stream_frames(rx, writer, config, close_signal, &id).await;
                return;
            }
            "TEARDOWN" => {
                let _ = send(writer, &rtsp_message::ok(&request, session_id.as_deref())).await;
                return;
            }
            _ => {
                if send(writer, &rtsp_message::not_implemented(&request))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}

/// After `PLAY`: pushes synthetic RTP frames on a steady interval while
/// answering any keep-alive (`GET_PARAMETER`/`OPTIONS`) that arrives, until
/// no keep-alive has arrived within the declared timeout -- at which point
/// frames silently stop, matching the real cameras' observed failure mode
/// exactly: no TCP close, no RTSP error, socket left open.
async fn stream_frames(
    rx: &mut mpsc::Receiver<Request>,
    writer: &mut (impl AsyncWrite + Unpin),
    config: &MockCameraConfig,
    close_signal: &Notify,
    session_id: &str,
) {
    let mut last_keepalive = Instant::now();
    let mut ticker = tokio::time::interval(config.frame_interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut sequence_number: u16 = 0;
    let mut frame_index: u64 = 0;
    let mut stalled = false;
    let mut pending_keepalive_reply: Option<Vec<u8>> = None;

    loop {
        if !stalled && last_keepalive.elapsed() >= config.declared_timeout {
            stalled = true;
        }

        tokio::select! {
            () = close_signal.notified() => return,
            _ = ticker.tick(), if !stalled => {
                if let Some(reply) = pending_keepalive_reply.take()
                    && writer.write_all(&reply).await.is_err()
                {
                    return;
                }
                sequence_number = sequence_number.wrapping_add(1);
                frame_index += 1;
                let timestamp = u32::from(sequence_number).wrapping_mul(3000);
                let packet = rtp::fabricate_h264_packet(sequence_number, timestamp, SSRC, frame_index);
                let framed = rtp::interleave(RTP_CHANNEL, &packet);
                if writer.write_all(&framed).await.is_err() {
                    return;
                }
            }
            request = rx.recv() => {
                let Some(request) = request else { return };
                last_keepalive = Instant::now();
                let is_keepalive = matches!(request.method.as_str(), "GET_PARAMETER" | "OPTIONS");
                let response = rtsp_message::ok(&request, Some(session_id)).to_bytes();
                if config.delay_keepalive_reply_to_next_frame && is_keepalive && !stalled {
                    pending_keepalive_reply = Some(response);
                } else if writer.write_all(&response).await.is_err() {
                    return;
                }
            }
        }
    }
}

fn authenticated(
    request: &Request,
    config: &MockCameraConfig,
    last_nonce: &mut Option<String>,
) -> bool {
    let credentials: Option<Credentials> = request
        .header("Authorization")
        .and_then(digest::parse_authorization);
    let ok = credentials.is_some_and(|credentials| {
        digest::verify(
            &credentials,
            last_nonce.as_deref(),
            &request.method,
            &request.uri,
            &config.username,
            &config.password,
        )
    });
    if !ok {
        *last_nonce = Some(digest::fresh_token());
    }
    ok
}

async fn send(
    writer: &mut (impl AsyncWrite + Unpin),
    response: &rtsp_message::Response,
) -> std::io::Result<()> {
    writer.write_all(&response.to_bytes()).await
}
