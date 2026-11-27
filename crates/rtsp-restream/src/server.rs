//! The RTSP async I/O layer: an accept loop, one isolated task per connection, and a nested task delivering interleaved RTP per `PLAY`.
//!
//! Play tasks are never restarted; when one ends the session closes and the client's own RTSP stack is expected to reconnect.

mod message_stream;

use crate::packetize::{AacPacketizer, H264Packetizer, H265Packetizer, PacketizeError};
use crate::provider::{Frame, StreamProvider, TrackInfo};
use crate::sdp::payload_type_for_track;
use crate::session::RtspSession;
use message_stream::MessageStream;
use rtsp_types::{Message, Method, Response, StatusCode};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::AbortHandle;

#[derive(Debug)]
pub struct RtspServer {
    listener: TcpListener,
}

impl RtspServer {
    /// # Errors
    /// Whatever `TcpListener::bind` returns.
    pub async fn bind(addr: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self { listener })
    }

    /// # Errors
    /// Whatever the bound socket's `local_addr` query returns.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub async fn serve(self, provider: Arc<dyn StreamProvider>) -> ! {
        loop {
            match self.listener.accept().await {
                Ok((stream, _peer_addr)) => {
                    let provider = Arc::clone(&provider);
                    tokio::spawn(async move {
                        handle_connection(stream, provider).await;
                    });
                }
                Err(err) => {
                    log_event("accept", &err);
                }
            }
        }
    }
}

fn log_event(context: &str, err: &dyn std::fmt::Display) {
    eprintln!("rtsp-restream: {context}: {err}");
}

async fn handle_connection(stream: TcpStream, provider: Arc<dyn StreamProvider>) {
    let (read_half, write_half) = stream.into_split();
    let mut messages = MessageStream::new(read_half);
    let write_half = Arc::new(AsyncMutex::new(write_half));
    let mut session = RtspSession::new();
    let mut play_task: Option<AbortHandle> = None;

    loop {
        let Ok(message) = messages.read_message().await else {
            break;
        };

        let request = match message {
            Message::Request(request) => request,
            Message::Data(_) => continue,
            Message::Response(_) => break,
        };

        let method = request.method();
        let response = session.handle_request(provider.as_ref(), &request);

        if write_response(&write_half, &response).await.is_err() {
            break;
        }

        if method == Method::Teardown {
            if let Some(abort) = play_task.take() {
                abort.abort();
            }
        } else if method == Method::Play
            && response.status() == StatusCode::Ok
            && let Some(setup) = session.setup_track()
        {
            let track = setup.stream.tracks[setup.track_index].clone();
            let new_task = spawn_play_task(
                Arc::clone(&provider),
                Arc::clone(&write_half),
                setup.stream_name.to_string(),
                track,
                setup.track_index,
                setup.rtp_channel,
            );
            if let Some(previous) = play_task.replace(new_task) {
                previous.abort();
            }
        }
    }

    if let Some(abort) = play_task {
        abort.abort();
    }
}

async fn write_response(
    write_half: &Arc<AsyncMutex<OwnedWriteHalf>>,
    response: &Response<Vec<u8>>,
) -> io::Result<()> {
    let mut buffer = Vec::new();
    response.write(&mut buffer).map_err(io::Error::other)?;
    let mut writer = write_half.lock().await;
    writer.write_all(&buffer).await
}

fn interleave(channel: u8, payload: &[u8]) -> Vec<u8> {
    let len = u16::try_from(payload.len()).expect("one RTP packet fits in a u16 length");
    let mut framed = Vec::with_capacity(4 + payload.len());
    framed.push(b'$');
    framed.push(channel);
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(payload);
    framed
}

static NEXT_SSRC: AtomicU32 = AtomicU32::new(1);

fn allocate_ssrc() -> u32 {
    NEXT_SSRC.fetch_add(1, Ordering::Relaxed)
}

enum Packetizer {
    H264(H264Packetizer),
    H265(H265Packetizer),
    Aac(AacPacketizer),
}

impl Packetizer {
    const fn new(track: &TrackInfo, payload_type: u8, ssrc: u32) -> Self {
        match track {
            TrackInfo::H264 { .. } => Self::H264(H264Packetizer::new(payload_type, ssrc)),
            TrackInfo::H265 { .. } => Self::H265(H265Packetizer::new(payload_type, ssrc)),
            TrackInfo::Aac { sample_rate, .. } => {
                Self::Aac(AacPacketizer::new(payload_type, ssrc, *sample_rate))
            }
        }
    }

    fn packetize(&mut self, frame: &Frame) -> Result<Vec<Vec<u8>>, PacketizeError> {
        match self {
            Self::H264(packetizer) => packetizer.packetize(frame),
            Self::H265(packetizer) => packetizer.packetize(frame),
            Self::Aac(packetizer) => packetizer.packetize(frame).map(|packet| vec![packet]),
        }
    }
}

fn spawn_play_task(
    provider: Arc<dyn StreamProvider>,
    write_half: Arc<AsyncMutex<OwnedWriteHalf>>,
    stream_name: String,
    track: TrackInfo,
    track_index: usize,
    rtp_channel: u8,
) -> AbortHandle {
    let join_handle = tokio::spawn(async move {
        let Some(mut receiver) = provider.subscribe(&stream_name) else {
            return;
        };
        let payload_type = payload_type_for_track(track_index);
        let ssrc = allocate_ssrc();
        let mut packetizer = Packetizer::new(&track, payload_type, ssrc);

        loop {
            let (frame, returned_receiver) = tokio::task::spawn_blocking(move || {
                let frame = receiver.recv();
                (frame, receiver)
            })
            .await
            .expect("frame-receive task panicked");
            receiver = returned_receiver;

            let Some(frame) = frame else {
                return;
            };
            if frame.track_index != track_index {
                continue;
            }

            let packets = match packetizer.packetize(&frame) {
                Ok(packets) => packets,
                Err(err) => {
                    log_event("packetize_error", &err);
                    continue;
                }
            };

            let mut writer = write_half.lock().await;
            for packet in &packets {
                let framed = interleave(rtp_channel, packet);
                if writer.write_all(&framed).await.is_err() {
                    return;
                }
            }
        }
    });
    join_handle.abort_handle()
}
