//! Issue #12 item G3's own Verify step: "a synthetic Annex-B sequence
//! produces a valid `m3u8` playlist and a sequence of CMAF segments that
//! `ffprobe` accepts without error" -- a real, independently authored
//! CMAF/HLS parser, not a hand-rolled one this crate's own code could
//! coincidentally agree with itself about.
//!
//! Drives the same real `hls` worker/server code
//! `tests/g3_integration.rs` does, over real HTTP, then writes the fetched
//! playlist and segment bytes to real files on disk (`ffprobe`'s HLS demuxer
//! resolves a playlist's own relative segment URIs against the directory the
//! `.m3u8` file itself lives in, so segments must be real files, not bytes
//! held only in memory) and shells out to the real `ffprobe` binary against
//! them.
//!
//! `ffprobe` ships as part of `ffmpeg`, added to this repository's own
//! `flake.nix` devShell specifically for this check (see that file's own
//! comment) -- this test fails loudly, not silently, if it is missing,
//! naming `nix develop` as the fix, matching this project's existing
//! precedent for a required external tool
//! (`scripts/run_g2_fmp4_ws_browser_check.sh`'s own `command -v` checks).

use bytes::Bytes;
use corvette_media_bridge::hls::{HlsServer, MultiCameraHlsStore, run_hls_segment};
use corvette_rtsp_client::depacketize::{Codec, Frame as ClientFrame};
use std::io::Read as _;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;

const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const CAMERA_NAME: &str = "cam-ffprobe";

// Deliberately NOT the shorter SPS/PPS/IDR bytes `g1_integration.rs`'s,
// `g2_integration.rs`'s, and `g3_integration.rs`'s own `cam-synth` all share
// (reused from `moq-mux`'s own `h264::import` unit tests): those are real and
// valid enough for a container-level parser to accept without error, which
// is all this crate's own Rust unit tests and the HTTP-transport integration
// tests ever needed, but `ffprobe` -- unlike a hand-rolled or MSE-level
// structural check -- actually attempts to decode a frame while probing
// stream parameters, and rejected that shorter fixture ("non-existing PPS
// referenced", "sps_id out of range") for exactly the reason
// `examples/g2_browser_fixture.rs`'s own doc already names: it lacks real
// encoded macroblock data. This test instead reuses that same file's real,
// genuinely decodable H.264 SPS/PPS/IDR triple (a real encoder's own output,
// see that file's own doc for full provenance) byte-for-byte.
const SPS: &[u8] = &[
    0x67, 0x42, 0xc0, 0x1f, 0xdc, 0x42, 0x6c, 0x04, 0x40, 0x00, 0x00, 0x03, 0x00, 0x40, 0x00, 0x00,
    0x0f, 0x03, 0xc6, 0x0c, 0xe0,
];
const PPS: &[u8] = &[0x68, 0xce, 0x0f, 0x2c, 0x80];
const IDR: &[u8] = &[
    0x65, 0x88, 0x84, 0x04, 0xbc, 0x46, 0x28, 0x00, 0x0c, 0xc4, 0xc7, 0x00, 0x01, 0x7d, 0x68, 0xe0,
    0x00, 0x22, 0x43, 0x27, 0x27, 0x27, 0x5d, 0x75, 0xd7, 0x5d, 0x75, 0xd7, 0x5d, 0x75, 0xd7, 0x80,
];

fn annex_b(nal: &[u8]) -> Bytes {
    let mut payload = Vec::with_capacity(nal.len() + 4);
    payload.extend_from_slice(&[0, 0, 0, 1]);
    payload.extend_from_slice(nal);
    Bytes::from(payload)
}

/// Same shape as `g3_integration.rs`'s own `feed_synthetic_h264` -- see that
/// file's own doc for why every access unit here is an IDR.
async fn feed(sender: broadcast::Sender<ClientFrame>) {
    let mut timestamp: u32 = 0;
    loop {
        for nal in [SPS, PPS, IDR] {
            let frame = ClientFrame {
                codec: Codec::H264,
                timestamp,
                payload: annex_b(nal),
            };
            if sender.send(frame).is_err() {
                return;
            }
        }
        timestamp = timestamp.wrapping_add(3000);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Same minimal HTTP/1.1 GET helper as `g3_integration.rs`'s own `http_get`
/// -- see that file's own top-level doc for why this is hand-rolled rather
/// than a new test-only HTTP client dependency.
async fn http_get(addr: SocketAddr, path: &str) -> Vec<u8> {
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
    assert_eq!(status, 200, "GET {path} failed: {header_text}");
    response[split + 4..].to_vec()
}

fn segment_names(playlist: &str) -> Vec<String> {
    playlist
        .lines()
        .filter(|line| line.starts_with("segment-"))
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn ffprobe_accepts_this_crates_own_playlist_and_segments() {
    let mut store = MultiCameraHlsStore::default();
    store.register(CAMERA_NAME);
    let store = Arc::new(store);

    let (sender, receiver) = broadcast::channel::<ClientFrame>(64);
    let feeder = tokio::spawn(feed(sender.clone()));
    let worker = tokio::spawn(run_hls_segment(
        CAMERA_NAME.to_string(),
        receiver,
        Arc::clone(&store),
    ));

    let server = HlsServer::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("binds the HLS server");
    let addr = server.local_addr().unwrap();
    tokio::spawn(server.serve(Arc::clone(&store)));

    // Poll until the playlist names at least two segments -- see
    // `g3_integration.rs`'s own identical wait for why this takes roughly
    // 2x the 2-second target segment duration in wall time.
    let playlist_path = format!("/{CAMERA_NAME}/playlist.m3u8");
    let playlist = tokio::time::timeout(WAIT_TIMEOUT, async {
        loop {
            let body = http_get(addr, &playlist_path).await;
            let text = String::from_utf8(body).expect("a playlist is UTF-8 text");
            if segment_names(&text).len() >= 2 {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("a playlist naming at least two segments resolves before the test timeout");

    let init_bytes = http_get(addr, &format!("/{CAMERA_NAME}/init.mp4")).await;
    let mut segment_bytes = Vec::new();
    for name in segment_names(&playlist) {
        let bytes = http_get(addr, &format!("/{CAMERA_NAME}/{name}")).await;
        segment_bytes.push((name, bytes));
    }

    drop(worker);
    feeder.abort();

    // Write everything to real files in one directory: ffprobe's HLS demuxer
    // resolves the playlist's own relative segment URIs (and EXT-X-MAP's
    // "init.mp4") against the directory the .m3u8 file itself lives in.
    //
    // `hls::CameraHls::playlist_text` never writes `#EXT-X-ENDLIST` (this is
    // a genuinely live playlist -- see `hls`'s own module doc), but ffmpeg's
    // own HLS demuxer takes the absence of that tag to mean the stream is
    // still being appended to, and will sit waiting for a playlist reload
    // that never comes against a static on-disk copy like this one,
    // regardless of how many real segments are already present. Appending it
    // only to this on-disk copy tells ffprobe the copy itself is complete --
    // it does not change what this crate's own server ever actually serves a
    // real player -- so this test can assert against a real, finite parse
    // rather than hanging on a fixture that, from a VOD parser's point of
    // view, looks unfinished forever.
    let dir = std::env::temp_dir().join(format!("g3-ffprobe-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creates the fixture directory");
    let vod_playlist = format!("{playlist}#EXT-X-ENDLIST\n");
    std::fs::write(dir.join("playlist.m3u8"), &vod_playlist).expect("writes the playlist");
    std::fs::write(dir.join("init.mp4"), &init_bytes).expect("writes the init segment");
    for (name, bytes) in &segment_bytes {
        std::fs::write(dir.join(name), bytes).expect("writes a segment");
    }

    let playlist_file = dir.join("playlist.m3u8");
    let output = run_ffprobe_with_timeout(&playlist_file, &dir);

    std::fs::remove_dir_all(&dir).ok();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "ffprobe rejected this crate's own playlist/segments (exit {:?}):\n{stderr}",
        output.status.code()
    );
    assert!(
        stderr.trim().is_empty(),
        "ffprobe reported an error despite exiting successfully: {stderr}"
    );

    // csv output is one line per stream, comma-joined in `-show_entries`'s
    // own declared field order; checked as two independent substrings rather
    // than one exact joined string so this assertion does not depend on
    // memorizing that exact field order.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("video") && stdout.contains("h264"),
        "ffprobe must report a real h264 video stream: {stdout}"
    );
}

/// Runs `ffprobe` against `playlist_file`, waiting at most 30 seconds rather
/// than trusting `Child::wait`'s own unbounded block -- see this file's own
/// top-level doc and the `#EXT-X-ENDLIST` comment at this function's own call
/// site for why a hang here is possible in principle and must fail loudly,
/// not silently block the whole test suite, if it ever recurs.
///
/// `fixture_dir` is only used to clean up on an early failure path (spawn
/// failure, or the timeout itself); the caller still removes it on the
/// normal path.
fn run_ffprobe_with_timeout(
    playlist_file: &std::path::Path,
    fixture_dir: &std::path::Path,
) -> std::process::Output {
    let mut child = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error", // only real errors on stderr, not routine info
            "-show_entries",
            "stream=codec_type,codec_name",
            "-of",
            "csv=p=0",
        ])
        .arg(playlist_file)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| {
            std::fs::remove_dir_all(fixture_dir).ok();
            panic!(
                "ffprobe: failed to run ({err}) -- this test requires ffprobe on PATH; \
                 run under 'nix develop' (flake.nix's devShell adds it via pkgs.ffmpeg \
                 specifically for this check)"
            )
        });

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().expect("polling ffprobe's own exit status") {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            child.stdout.take().unwrap().read_to_end(&mut stdout).ok();
            child.stderr.take().unwrap().read_to_end(&mut stderr).ok();
            return std::process::Output {
                status,
                stdout,
                stderr,
            };
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            std::fs::remove_dir_all(fixture_dir).ok();
            panic!(
                "ffprobe did not exit within 30s -- this normally means the on-disk \
                 playlist copy still looks unfinished to ffmpeg's HLS demuxer (missing \
                 or malformed #EXT-X-ENDLIST)"
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
