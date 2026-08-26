//! Integration tests for issue #12 item X3's async I/O layer
//! (`rtsp_restream::RtspServer`): a real loopback `TcpListener` plus a fake
//! `StreamProvider`, exercised by hand-rolled RTSP clients built directly
//! against `rtsp-types` -- not against `corvette-rtsp-client`. Using a
//! CLIENT crate to test a SERVER would risk a self-referential oracle, the
//! same risk issue #18's own M1 item flagged and avoided (see this item's
//! own plan entry).

use rtsp_restream::{Frame, FrameReceiver, RtspServer, StreamInfo, StreamProvider, TrackInfo};
use rtsp_types::headers::{CSEQ, TRANSPORT};
use rtsp_types::{Message, Method, ParseError, Request, Response, StatusCode, Url, Version};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;

const WAIT_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------
// A fake `StreamProvider`: named streams backed by a broadcast channel the
// test itself pushes `Frame`s into. Two independent `subscribe` calls for
// the same name get two independent receivers of the same underlying
// frames -- the fan-out contract `StreamProvider::subscribe` documents
// (G1's own future job to implement for real against a live camera; this
// fixture only has to fulfill the same contract for this test, same as
// `session::tests::FakeProvider` fulfills it trivially with an exhausted
// receiver for X1's own narrower needs).
// ---------------------------------------------------------------------

struct FakeProvider {
    streams: Mutex<HashMap<String, StreamEntry>>,
}

enum StreamEntry {
    Live {
        info: StreamInfo,
        sender: broadcast::Sender<Frame>,
    },
    /// A stream whose subscriber's `recv` panics immediately, for the
    /// panic-isolation test. `fired` flips to `true` right before the
    /// panic, so the test can wait for it deterministically instead of a
    /// fixed sleep.
    Panicking {
        info: StreamInfo,
        fired: Arc<AtomicBool>,
    },
}

impl FakeProvider {
    fn new() -> Self {
        Self {
            streams: Mutex::new(HashMap::new()),
        }
    }

    /// Registers a live H.264 stream and returns the sender the test uses
    /// to push frames into it.
    fn add_live_h264_stream(&self, name: &str) -> broadcast::Sender<Frame> {
        let (sender, _receiver) = broadcast::channel(16);
        let entry = StreamEntry::Live {
            info: h264_stream_info(name),
            sender: sender.clone(),
        };
        self.streams
            .lock()
            .expect("test-only mutex is never poisoned")
            .insert(name.to_string(), entry);
        sender
    }

    /// Registers a stream whose subscriber panics on its first `recv`, and
    /// returns the flag that flips to `true` right before that panic.
    fn add_panicking_h264_stream(&self, name: &str) -> Arc<AtomicBool> {
        let fired = Arc::new(AtomicBool::new(false));
        let entry = StreamEntry::Panicking {
            info: h264_stream_info(name),
            fired: Arc::clone(&fired),
        };
        self.streams
            .lock()
            .expect("test-only mutex is never poisoned")
            .insert(name.to_string(), entry);
        fired
    }
}

impl StreamProvider for FakeProvider {
    fn describe(&self, name: &str) -> Option<StreamInfo> {
        let streams = self.streams.lock().expect("test-only mutex is never poisoned");
        match streams.get(name)? {
            StreamEntry::Live { info, .. } | StreamEntry::Panicking { info, .. } => {
                Some(info.clone())
            }
        }
    }

    fn subscribe(&self, name: &str) -> Option<Box<dyn FrameReceiver>> {
        let streams = self.streams.lock().expect("test-only mutex is never poisoned");
        match streams.get(name)? {
            StreamEntry::Live { sender, .. } => Some(Box::new(BroadcastFrameReceiver {
                receiver: sender.subscribe(),
            })),
            StreamEntry::Panicking { fired, .. } => Some(Box::new(PanickingFrameReceiver {
                fired: Arc::clone(fired),
            })),
        }
    }
}

fn h264_stream_info(name: &str) -> StreamInfo {
    StreamInfo {
        name: name.to_string(),
        tracks: vec![TrackInfo::H264 {
            sps: vec![0x67, 0x42, 0x00, 0x1f],
            pps: vec![0x68, 0xce],
        }],
    }
}

struct BroadcastFrameReceiver {
    receiver: broadcast::Receiver<Frame>,
}

impl FrameReceiver for BroadcastFrameReceiver {
    fn recv(&mut self) -> Option<Frame> {
        // `blocking_recv` is safe here specifically because
        // `rtsp_restream`'s own play task only ever calls `FrameReceiver::
        // recv` from inside `tokio::task::spawn_blocking`, matching
        // `crate::provider::FrameReceiver`'s own documented contract.
        self.receiver.blocking_recv().ok()
    }
}

struct PanickingFrameReceiver {
    fired: Arc<AtomicBool>,
}

impl FrameReceiver for PanickingFrameReceiver {
    fn recv(&mut self) -> Option<Frame> {
        self.fired.store(true, Ordering::SeqCst);
        panic!("deliberately injected panic for issue #12 X3's panic-isolation test");
    }
}

fn h264_frame(marker_byte: u8) -> Frame {
    Frame {
        track_index: 0,
        timestamp: Duration::ZERO,
        // A single-start-code-prefixed NAL unit small enough to stay a
        // one-packet, non-fragmented RTP payload: `packetize::H264Packetizer`
        // strips the start code and emits `[0x65, marker_byte]` verbatim as
        // the sole RTP packet's payload.
        payload: vec![0, 0, 0, 1, 0x65, marker_byte],
    }
}

// ---------------------------------------------------------------------
// A hand-rolled RTSP/RTP client, built directly against `rtsp-types`.
// ---------------------------------------------------------------------

struct TestClient {
    stream: TcpStream,
    buffer: Vec<u8>,
    cseq: u32,
}

impl TestClient {
    async fn connect(addr: std::net::SocketAddr) -> Self {
        let stream = TcpStream::connect(addr)
            .await
            .expect("connects to the loopback server");
        Self {
            stream,
            buffer: Vec::new(),
            cseq: 0,
        }
    }

    async fn send_request(
        &mut self,
        method: Method,
        uri: &str,
        transport: Option<&str>,
    ) -> Response<Vec<u8>> {
        self.cseq += 1;
        let mut builder = Request::builder(method, Version::V1_0)
            .request_uri(Url::parse(uri).expect("valid test URI"))
            .header(CSEQ, self.cseq.to_string());
        if let Some(transport) = transport {
            builder = builder.header(TRANSPORT, transport);
        }
        let request = builder.build(Vec::new());

        let mut bytes = Vec::new();
        request.write(&mut bytes).expect("serializes a request");
        self.stream
            .write_all(&bytes)
            .await
            .expect("writes the request to the server");

        self.read_response().await
    }

    /// Reads the next `Response`, discarding any `Data` frame that happens
    /// to arrive first (a server that streams before its own `PLAY`
    /// response, as issue #18's own mock camera deliberately reproduces on
    /// the client side, is not this crate's own behavior, but being
    /// lenient here costs nothing and matches `RtspSession`'s own
    /// documented "responses and frames share one stream" model).
    async fn read_response(&mut self) -> Response<Vec<u8>> {
        loop {
            match Message::<Vec<u8>>::parse(&self.buffer) {
                Ok((Message::Response(response), consumed)) => {
                    self.buffer.drain(0..consumed);
                    return response;
                }
                Ok((_, consumed)) => {
                    self.buffer.drain(0..consumed);
                }
                Err(ParseError::Incomplete(_)) => self.fill_buffer().await,
                Err(ParseError::Error) => panic!("malformed message from the server"),
            }
        }
    }

    /// Reads the next interleaved `Data` frame's raw body, discarding any
    /// control response found first.
    async fn read_data_frame(&mut self) -> Vec<u8> {
        tokio::time::timeout(WAIT_TIMEOUT, async {
            loop {
                match Message::<Vec<u8>>::parse(&self.buffer) {
                    Ok((Message::Data(data), consumed)) => {
                        self.buffer.drain(0..consumed);
                        return data.into_body();
                    }
                    Ok((_, consumed)) => {
                        self.buffer.drain(0..consumed);
                    }
                    Err(ParseError::Incomplete(_)) => self.fill_buffer().await,
                    Err(ParseError::Error) => panic!("malformed message from the server"),
                }
            }
        })
        .await
        .expect("a Data frame arrives before the test timeout")
    }

    async fn fill_buffer(&mut self) {
        let mut chunk = [0_u8; 4096];
        let read = self
            .stream
            .read(&mut chunk)
            .await
            .expect("reads from the server");
        assert!(read > 0, "server closed the connection unexpectedly");
        self.buffer.extend_from_slice(&chunk[..read]);
    }

    /// Runs `DESCRIBE`/`SETUP`/`PLAY` against `stream_name`'s sole track
    /// (`trackID=0`), asserting every step succeeds.
    async fn play(&mut self, stream_name: &str) {
        let describe = self
            .send_request(Method::Describe, &format!("rtsp://127.0.0.1/{stream_name}"), None)
            .await;
        assert_eq!(describe.status(), StatusCode::Ok, "DESCRIBE must succeed");

        let setup = self
            .send_request(
                Method::Setup,
                &format!("rtsp://127.0.0.1/{stream_name}/trackID=0"),
                Some("RTP/AVP/TCP;unicast;interleaved=0-1"),
            )
            .await;
        assert_eq!(setup.status(), StatusCode::Ok, "SETUP must succeed");

        let play = self
            .send_request(Method::Play, &format!("rtsp://127.0.0.1/{stream_name}"), None)
            .await;
        assert_eq!(play.status(), StatusCode::Ok, "PLAY must succeed");
    }
}

// ---------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------

async fn start_server(provider: FakeProvider) -> std::net::SocketAddr {
    let server = RtspServer::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("binds to an ephemeral loopback port");
    let addr = server.local_addr().expect("bound server has a local address");
    tokio::spawn(server.serve(Arc::new(provider)));
    addr
}

/// Waits until `condition` is true, polling instead of sleeping a fixed
/// duration -- deterministic completion the moment the awaited state is
/// reached, rather than a guess at how long it might take.
async fn wait_until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(WAIT_TIMEOUT, async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("condition became true before the test timeout");
}

fn rtp_payload(packet: &[u8]) -> &[u8] {
    &packet[12..]
}

// ---------------------------------------------------------------------
// Verify: two independent clients PLAY the same stream and both receive
// the same frames independently.
// ---------------------------------------------------------------------

#[tokio::test]
async fn two_independent_clients_play_the_same_stream_and_both_receive_the_same_frames() {
    let provider = FakeProvider::new();
    let sender = provider.add_live_h264_stream("cam1");
    let addr = start_server(provider).await;

    let mut client_a = TestClient::connect(addr).await;
    client_a.play("cam1").await;
    let mut client_b = TestClient::connect(addr).await;
    client_b.play("cam1").await;

    // Both play tasks must have actually subscribed before frames are
    // pushed -- a broadcast channel only delivers to receivers that
    // already exist at send time, so this is a correctness wait, not an
    // optimization.
    wait_until(|| sender.receiver_count() >= 2).await;

    for marker in 0_u8..3 {
        sender
            .send(h264_frame(marker))
            .expect("at least two receivers are subscribed");
    }

    for marker in 0_u8..3 {
        let packet_a = client_a.read_data_frame().await;
        let packet_b = client_b.read_data_frame().await;
        assert_eq!(
            rtp_payload(&packet_a),
            &[0x65, marker],
            "client A's frame {marker} payload"
        );
        assert_eq!(
            rtp_payload(&packet_b),
            &[0x65, marker],
            "client B's frame {marker} payload"
        );
    }
}

// ---------------------------------------------------------------------
// Verify: one session's frame-delivery task panicking neither kills the
// other session nor the listener (INV-5(a); unconditional under Tokio's
// per-task panic model, verified directly here rather than by mutation --
// see this item's own plan entry).
// ---------------------------------------------------------------------

#[tokio::test]
async fn one_sessions_panic_does_not_affect_another_session_or_the_listener() {
    let provider = FakeProvider::new();
    let panic_fired = provider.add_panicking_h264_stream("bad-cam");
    let healthy_sender = provider.add_live_h264_stream("good-cam");
    let addr = start_server(provider).await;

    // Client A: PLAYs the panicking stream. Its play task's very first
    // `recv()` call panics.
    let mut client_a = TestClient::connect(addr).await;
    client_a.play("bad-cam").await;
    wait_until(|| panic_fired.load(Ordering::SeqCst)).await;

    // Client B: PLAYs a healthy, independent stream concurrently and must
    // keep receiving frames, completely unaffected by A's panic.
    let mut client_b = TestClient::connect(addr).await;
    client_b.play("good-cam").await;
    wait_until(|| healthy_sender.receiver_count() >= 1).await;

    healthy_sender
        .send(h264_frame(0xAA))
        .expect("client B is subscribed");
    let packet_b = client_b.read_data_frame().await;
    assert_eq!(rtp_payload(&packet_b), &[0x65, 0xAA]);

    // The listener itself must still accept and correctly serve a brand
    // new connection.
    let mut client_c = TestClient::connect(addr).await;
    let response = client_c
        .send_request(Method::Options, "rtsp://127.0.0.1/anything", None)
        .await;
    assert_eq!(
        response.status(),
        StatusCode::Ok,
        "the listener must still accept and serve new connections after another session panicked"
    );
}
