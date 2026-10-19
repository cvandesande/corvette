//! End-to-end test of this crate's own real `hls` packager, served over HTTP.

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

// Reused from `moq-mux`'s `avc3_self_initializes_from_first_keyframe`.
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

/// Repeating SPS/PPS/IDR cycles, 3000 RTP ticks apart; every access unit is
/// an IDR, so the 2-second stream-time target accrues in about 60 cycles.
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

fn segment_paths(camera_name: &str, playlist: &str) -> Vec<String> {
    playlist
        .lines()
        .filter(|line| line.starts_with("segment-"))
        .map(|line| format!("/{camera_name}/{line}"))
        .collect()
}

#[tokio::test]
async fn g3_end_to_end_against_a_mock_camera_and_a_synthetic_decodable_one() {
    let mut store = MultiCameraHlsStore::default();
    store.register("cam-mock");
    store.register("cam-synth");
    let store = Arc::new(store);

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

    // A registered camera's playlist always exists: a valid, empty live
    // document (200, no EXT-X-MAP, no segments), never a 404.
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
