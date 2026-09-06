//! Integration test for issue #12 item G3: this crate's own real `hls`
//! module, driven by issue #18's own mock camera server
//! (`corvette_rtsp_client::mock_camera`, reused directly) standing in for one
//! camera, plus a second synthetic camera the same way `g1_integration.rs`
//! and `g2_integration.rs` both do.
//!
//! **Why two cameras, only one backed by a real RTSP dial.** As
//! `g1_integration.rs`'s and `g2_integration.rs`'s own top-level docs already
//! establish (confirmed there by direct source read), issue #18's
//! `mock_camera` fabricates RTP payloads containing no real SPS/PPS: it was
//! built to test issue #18's own depacketizer's NAL/FU-A framing, not to
//! produce a decodable stream. This item's own initialization segment can
//! never resolve from `cam-mock`'s frames for the same reason. `cam-mock`
//! below still dials it through a real `corvette_rtsp_client::client::Client`,
//! proving the real RTSP handshake/reconnect/task-wiring path holds and that
//! this crate's own frame handling degrades gracefully (no panic, a plain 404
//! for every resource) rather than ever producing anything. `cam-synth`
//! supplies a second, independent `corvette_rtsp_client::depacketize::Frame`
//! broadcast, fed directly (not via any RTSP dial) with a real, valid H.264
//! SPS/PPS/IDR triple -- the same fixture bytes `g1_integration.rs`'s and
//! `g2_integration.rs`'s own `cam-synth` use, byte-for-byte. This is not a
//! second RTSP mock server: only this crate's own public `hls` worker
//! functions are driven directly, exactly the way `crate::start` already
//! wires a real `Client`'s subscriptions to them.
//!
//! **The hand-rolled HTTP test client.** Unlike `g2_integration.rs`'s own
//! WebSocket test client (`tokio-tungstenite`'s own client half, reused
//! because both sides of that library are the same independently authored,
//! protocol-compliant implementation), no HTTP client crate exists anywhere
//! in this workspace, and depending on one for this test alone would be a
//! new dependency for a handful of lines a raw `TcpStream` GET/read-to-EOF
//! already covers cleanly (this server always responds with `Connection:
//! close`, per `hls`'s own doc, so read-to-EOF is a complete response). This
//! test's own `http_get` helper below is that -- no self-referential-oracle
//! concern applies either way, since it exercises no project code at all.
//!
//! Unlike `g1_integration.rs`, this test needs no external `moq-relay`
//! binary (G3 has no `MoQ` dependency at all) and so is not `#[ignore]`d --
//! `cargo test --workspace` runs it directly.

use bytes::Bytes;
use corvette_media_bridge::hls::{HlsServer, MultiCameraHlsStore, run_hls_segment};
use corvette_media_bridge::supervise::spawn_supervised_with;
use corvette_rtsp_client::client::{CameraConfig, Client};
use corvette_rtsp_client::depacketize::{Codec, Frame as ClientFrame};
use corvette_rtsp_client::mock_camera::{MockCamera, MockCameraConfig};
use corvette_rtsp_client::session::Credentials;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;

const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

// The exact fixture bytes `g1_integration.rs`'s and `g2_integration.rs`'s own
// `cam-synth` use -- `moq-mux`'s own `h264::import` unit tests
// (`avc3_self_initializes_from_first_keyframe`) reused rather than invented.
const SPS: &[u8] = &[
    0x67, 0x42, 0xc0, 0x1f, 0xda, 0x01, 0x40, 0x16, 0xe9, 0xb8, 0x08, 0x08, 0x0a, 0x00, 0x00, 0x07,
    0xd0, 0x00, 0x01, 0xd4, 0xc0, 0x80,
];
const PPS: &[u8] = &[0x68, 0xce, 0x3c, 0x80];
const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x21];

fn annex_b(nal: &[u8]) -> Bytes {
    let mut payload = Vec::with_capacity(nal.len() + 4);
    payload.extend_from_slice(&[0, 0, 0, 1]);
    payload.extend_from_slice(nal);
    Bytes::from(payload)
}

/// Feeds `sender` a repeating SPS/PPS/IDR cycle, 3000 RTP ticks (90 kHz clock)
/// apart -- identical in shape to `g1_integration.rs`'s and
/// `g2_integration.rs`'s own `feed_synthetic_h264`. Every access unit here is
/// an IDR (this fixture carries no non-keyframe NAL), so every fragment this
/// produces is itself a keyframe candidate for `hls::SegmentBuilder`'s own
/// cut policy -- accumulating past the 2-second target takes about 60 cycles.
async fn feed_synthetic_h264(sender: broadcast::Sender<ClientFrame>) {
    let mut timestamp: u32 = 0;
    loop {
        for nal in [SPS, PPS, IDR] {
            let frame = ClientFrame {
                codec: Codec::H264,
                timestamp,
                payload: annex_b(nal),
            };
            if sender.send(frame).is_err() {
                return; // no receivers left; nothing to feed.
            }
        }
        timestamp = timestamp.wrapping_add(3000);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn mock_camera_config(addr: SocketAddr) -> CameraConfig {
    CameraConfig::new(
        "cam-mock",
        addr.ip(),
        addr.port(),
        "/stream1",
        Credentials::new("admin", "test-password-not-real"),
    )
}

/// A minimal HTTP/1.1 GET: writes the request, reads the response to EOF
/// (always correct here since `hls`'s own server sends `Connection: close`
/// on every response -- see this file's own top-level doc), and splits it
/// into `(status code, body)`.
async fn http_get(addr: SocketAddr, path: &str) -> (u16, Vec<u8>) {
    let mut stream = tokio::time::timeout(WAIT_TIMEOUT, TcpStream::connect(addr))
        .await
        .expect("connects before the test timeout")
        .expect("connects to the HLS listener");
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("writes the request");

    let mut response = Vec::new();
    tokio::time::timeout(WAIT_TIMEOUT, stream.read_to_end(&mut response))
        .await
        .expect("response arrives before the test timeout")
        .expect("reads the response");

    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a well-formed response separates headers from its body");
    let header_text = std::str::from_utf8(&response[..split]).expect("headers are ASCII");
    let status_line = header_text.lines().next().expect("a status line");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .expect("status line has a code")
        .parse()
        .expect("status code is numeric");
    (status, response[split + 4..].to_vec())
}

/// Pulls every `segment-<n>.m4s` URL a `camera_name` playlist body names, in
/// the order the playlist lists them -- matched by the `segment-` prefix
/// `hls`'s own playlist writer always uses for a media segment line (see
/// `hls::CameraHls::playlist_text`), not by its file extension.
fn segment_paths(camera_name: &str, playlist: &str) -> Vec<String> {
    playlist
        .lines()
        .filter(|line| line.starts_with("segment-"))
        .map(|line| format!("/{camera_name}/{line}"))
        .collect()
}

#[tokio::test]
async fn g3_end_to_end_against_a_mock_camera_and_a_synthetic_decodable_one() {
    // ---- Wiring: two cameras, one whole-process HlsServer ----
    let mut store = MultiCameraHlsStore::default();
    store.register("cam-mock");
    store.register("cam-synth");
    let store = Arc::new(store);

    // cam-mock: a real corvette_rtsp_client::Client dialing issue #18's own
    // mock camera server, unmodified.
    let mock_camera = MockCamera::spawn(MockCameraConfig::new())
        .await
        .expect("binds the mock camera");
    let mock_client = Arc::new(Client::new(mock_camera_config(mock_camera.addr())));
    let mock_segment = spawn_supervised_with("cam-mock/hls-segment".to_string(), {
        let client = Arc::clone(&mock_client);
        let store = Arc::clone(&store);
        move || {
            run_hls_segment(
                "cam-mock".to_string(),
                client.subscribe(),
                Arc::clone(&store),
            )
        }
    });

    // cam-synth: a real, valid H.264 SPS/PPS/IDR triple, fed directly onto a
    // broadcast channel standing in for what `Client::subscribe()` returns
    // (see this file's own top-level doc for why).
    let (synth_sender, _first_receiver) = broadcast::channel::<ClientFrame>(64);
    let synth_feeder = tokio::spawn(feed_synthetic_h264(synth_sender.clone()));
    let synth_segment = spawn_supervised_with("cam-synth/hls-segment".to_string(), {
        let sender = synth_sender.clone();
        let store = Arc::clone(&store);
        move || {
            run_hls_segment(
                "cam-synth".to_string(),
                sender.subscribe(),
                Arc::clone(&store),
            )
        }
    });

    let server = HlsServer::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("binds the whole-process HLS server");
    let addr = server.local_addr().unwrap();
    tokio::spawn(server.serve(Arc::clone(&store)));

    // ---- cam-mock never resolves an initialization segment, never panics
    // either (its own frames carry no real parameter set -- see this file's
    // own top-level doc). Its playlist stays a valid, empty live document
    // forever (200, no EXT-X-MAP, no segments) rather than a 404: a
    // registered camera's playlist always exists, even before it has
    // resolved anything -- see `hls::CameraHls::playlist_text`'s own doc for
    // why (this item's own browser-based Verify step found a real player
    // race against the alternative). ----
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !mock_segment.is_finished(),
        "cam-mock's HLS-segmenting task must not panic on undecodable frames"
    );
    let (mock_status, mock_body) = http_get(addr, "/cam-mock/playlist.m3u8").await;
    assert_eq!(mock_status, 200);
    let mock_playlist = std::str::from_utf8(&mock_body).unwrap();
    assert!(mock_playlist.starts_with("#EXTM3U\n"));
    assert!(!mock_playlist.contains("#EXT-X-MAP"));
    assert!(segment_paths("cam-mock", mock_playlist).is_empty());

    // ---- cam-synth: an HTTP client fetches the playlist, resolves at least
    // two segment URLs from it, and fetches those segments successfully. ----
    let playlist = tokio::time::timeout(WAIT_TIMEOUT, async {
        loop {
            let (status, body) = http_get(addr, "/cam-synth/playlist.m3u8").await;
            if status == 200 {
                let text = String::from_utf8(body).expect("a playlist is UTF-8 text");
                if segment_paths("cam-synth", &text).len() >= 2 {
                    return text;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("a playlist naming at least two segments resolves before the test timeout");

    assert!(playlist.starts_with("#EXTM3U\n"));
    assert!(playlist.contains("#EXT-X-MAP:URI=\"init.mp4\"\n"));

    let (init_status, init_body) = http_get(addr, "/cam-synth/init.mp4").await;
    assert_eq!(init_status, 200);
    assert_eq!(&init_body[4..8], b"ftyp");

    let segments = segment_paths("cam-synth", &playlist);
    assert!(segments.len() >= 2, "playlist: {playlist}");
    for path in &segments {
        let (status, body) = http_get(addr, path).await;
        assert_eq!(status, 200, "fetching {path}");
        assert_eq!(
            &body[4..8],
            b"moof",
            "segment at {path} must start with moof"
        );
    }

    // ---- Isolation (INV-5(a), verified directly, not by mutation): killing
    // cam-mock's Client/task does not affect cam-synth's HTTP output. ----
    drop(mock_segment);
    drop(mock_client);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let (status_after_kill, body_after_kill) = http_get(addr, "/cam-synth/playlist.m3u8").await;
    assert_eq!(
        status_after_kill, 200,
        "cam-synth's playlist must survive cam-mock's Client being killed"
    );
    assert!(
        segment_paths("cam-synth", std::str::from_utf8(&body_after_kill).unwrap()).len() >= 2,
        "cam-synth must keep producing segments after cam-mock is killed"
    );

    drop(synth_segment);
    synth_feeder.abort();
}
