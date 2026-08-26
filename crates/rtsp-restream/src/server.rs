//! The async I/O layer (issue #12 item X3).
//!
//! A `TcpListener::accept()` loop, one isolated task per inbound RTSP
//! connection, driving X1's [`RtspSession`] state machine over the real
//! socket and X2's packetizer for the negotiated stream's
//! [`StreamProvider`]-supplied frames -- the role go2rtc's own
//! `pkg/rtsp/server.go` + `conn.go` play
//! (`RESEARCH-rtsp-restream-server.md` F-11).
//!
//! Each accepted connection gets its own top-level [`tokio::spawn`]ed task
//! (`handle_connection`). Once that connection's `PLAY` succeeds, it spawns
//! a second, nested task (`spawn_play_task`) that owns frame delivery:
//! subscribing to the negotiated stream, packetizing each [`Frame`] via
//! [`crate::packetize`], and writing the resulting RTP payloads to the
//! connection's own interleaved channel until `TEARDOWN` or connection
//! close ends it.
//!
//! # Panic isolation, and why this layer has no restart-on-panic concept
//!
//! A panic anywhere inside one connection's task tree -- its own read loop,
//! or its nested play task -- is caught by Tokio's per-task panic model at
//! that `tokio::spawn` boundary and cannot propagate to the accept loop or
//! to any other connection's task, unconditionally, regardless of anything
//! this module does. That is INV-5(a) (see this crate's own issue #12 plan
//! entry), and it needs no supervision code to hold: every connection, and
//! every play task inside it, already gets its own `tokio::spawn`.
//!
//! What this layer deliberately does NOT do is restart a connection's play
//! task after it panics. Unlike a long-lived, process-owned resource (e.g.
//! issue #18's own per-camera client task, which reconnects with backoff
//! because many independent downstream subscribers depend on that one
//! upstream connection staying alive), a play task here exists solely to
//! serve one specific inbound client's own chosen `PLAY` -- it is not a
//! resource this crate owns independently of that client's connection. If
//! it ends, whether by panic, by the stream itself ending, or by a bad
//! frame that will never packetize, the correct and RFC-consistent response
//! is exactly what go2rtc's own connection handling already does on any
//! session-ending error (`RESEARCH-rtsp-restream-server.md` F-11): end that
//! session. The client's own RTSP stack -- including this very project's own
//! issue #18 RTSP client crate, which implements exactly this reconnect-
//! with-backoff behavior on its side of an equivalent connection -- is
//! already expected to reconnect if it wants to keep watching. Inventing a
//! same-socket, resubscribe-and-resume recovery here would depart from that
//! prior art for no requirement driving it, and would introduce real
//! correctness questions (RTP timestamp/sequence continuity across a
//! resubscribe, `RtspSession`'s own state ambiguity mid-`Playing`) that Do
//! step 5 (isolation only, not restart) never asks this item to answer.

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

/// A bound RTSP restream server: accepts connections and serves them
/// against a caller-supplied [`StreamProvider`].
#[derive(Debug)]
pub struct RtspServer {
    listener: TcpListener,
}

impl RtspServer {
    /// Binds a new server to `addr`. The bind address/port is a caller-
    /// supplied constructor parameter -- never hardcoded -- matching D-3's
    /// "configuration point" convention already established for this
    /// project's `MoQ` relay exposure.
    ///
    /// # Errors
    ///
    /// Returns any error `TcpListener::bind` itself returns (e.g. the
    /// address is already in use).
    pub async fn bind(addr: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self { listener })
    }

    /// The address this server actually bound to -- useful when `addr`
    /// above named port 0 and the caller (e.g. a test) needs to learn which
    /// port the OS assigned.
    ///
    /// # Errors
    ///
    /// Returns any error the underlying socket's `local_addr` query itself
    /// returns.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Runs the accept loop, dispatching each inbound connection to its own
    /// isolated task (Do step 2/5) against `provider`. Never returns: a
    /// transient per-connection `accept()` error is logged and the loop
    /// continues, since it says nothing about the listening socket itself
    /// (matching this crate's own "one bad thing never ends unrelated work"
    /// convention, see the module doc's panic-isolation discussion).
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

/// Drives one accepted connection's [`RtspSession`] until the connection
/// closes, a message fails to parse, or the client sends something this
/// crate does not expect from a client (a `Response`). Isolated from every
/// other connection and from the accept loop purely by being its own
/// [`tokio::spawn`]ed task (see the module doc) -- nothing below needs to
/// catch its own panics for that isolation to hold.
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
            // A client may send RTCP receiver reports on the interleaved
            // channel's odd (RTCP) number; this crate does not implement
            // RTCP (D-7's narrowed scope), so they are silently discarded
            // rather than treated as a protocol error.
            Message::Data(_) => continue,
            // A server never expects a `Response` from its own client;
            // treat it the same as a malformed message and end the session.
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
                // A second PLAY on an already-playing session (e.g. a
                // client resuming after a seek this crate does not
                // otherwise distinguish): stop the old delivery task
                // before the new one takes over, rather than running two
                // writers against the same interleaved channel.
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

/// Wraps `payload` in an RTSP `$`-prefixed interleaved frame: channel byte
/// plus a big-endian 16-bit length, per RFC 2326 §10.12 -- the same framing
/// issue #18's own mock-camera test double emits in the opposite (server-
/// fabricating-frames-for-a-client-to-parse) direction.
///
/// # Panics
///
/// Panics if `payload` is longer than [`u16::MAX`] bytes. Every payload this
/// module emits is one RTP packet, already bounded well under that by
/// [`crate::packetize::MAX_SINGLE_NAL_SIZE`] plus a small fixed header.
fn interleave(channel: u8, payload: &[u8]) -> Vec<u8> {
    let len = u16::try_from(payload.len()).expect("one RTP packet fits in a u16 length");
    let mut framed = Vec::with_capacity(4 + payload.len());
    framed.push(b'$');
    framed.push(channel);
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(payload);
    framed
}

/// Allocates SSRCs unique within this process -- distinct per play task so
/// concurrent sessions against the same or different streams never share
/// one, even though nothing in this crate's own narrowed scope (no RTCP)
/// would actually break if they did.
static NEXT_SSRC: AtomicU32 = AtomicU32::new(1);

fn allocate_ssrc() -> u32 {
    NEXT_SSRC.fetch_add(1, Ordering::Relaxed)
}

/// Either video packetizer this crate implements, or the AAC one, selected
/// from the set-up track's own codec -- the mirror image of issue #18's own
/// client-task `Depacketizer` enum. Normalizes
/// [`AacPacketizer::packetize`]'s single-packet return into the same
/// `Vec<Vec<u8>>` shape [`H264Packetizer`]/[`H265Packetizer`] already use,
/// so the play task below has one write loop regardless of codec.
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

/// Spawns the dedicated write task (Do step 3) that streams one `PLAY`ed
/// track's frames, from `provider.subscribe(&stream_name)`, as RTP over
/// `rtp_channel` until the stream ends, a write fails (the connection
/// closed), or this task is aborted (by `TEARDOWN` or the connection's own
/// read loop ending -- see `handle_connection`).
///
/// Its own panic (e.g. injected by a test double's [`FrameReceiver::recv`],
/// or a future bug) is isolated the same way `handle_connection`'s panic
/// isolation from the accept loop is: this is its own [`tokio::spawn`], and
/// its `JoinHandle` is deliberately discarded (only its [`AbortHandle`] is
/// kept) so a panic here has no observer to propagate through at all.
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
            // `FrameReceiver::recv` is a synchronous, possibly-blocking
            // call by contract (see `crate::provider`'s own doc); running
            // it directly here would block this task's executor thread, so
            // it runs on the blocking-task pool instead, exactly as that
            // trait's own doc prescribes. The receiver is moved in and
            // handed back out each call since `Box<dyn FrameReceiver>` is
            // not `Clone` and `recv` takes `&mut self`.
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
                // One frame that fails to packetize is not this session's
                // failure to handle -- matching issue #18's own client-task
                // precedent of logging and dropping a single bad unit
                // rather than ending the whole session over it.
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
