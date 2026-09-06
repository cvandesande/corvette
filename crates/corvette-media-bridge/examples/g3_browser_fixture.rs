//! Test-support fixture for issue #12 item G3's own browser-based `Verify`
//! step: hosts this crate's real `hls` code for one synthetic camera, so a
//! headless-browser test (`crates/corvette-media-bridge/tests/browser/`) can
//! drive a real `hls.js` player -- and a real decoder -- against this item's
//! own actual playlist/segment output.
//!
//! Not shipped: a Cargo example, never built or copied by `P1`'s Dockerfile
//! (which only builds the `corvette-media-bridge` binary target). Prints
//! `LISTENING <port>` on its own stdout once bound, then runs until killed --
//! `scripts/run_g3_hls_browser_check.sh` is what launches and tears this
//! down around a `playwright test` run.
//!
//! # Panics
//!
//! Panics if the HTTP listener fails to bind.

use bytes::Bytes;
use corvette_media_bridge::hls::{HlsServer, MultiCameraHlsStore, run_hls_segment};
use corvette_media_bridge::supervise::spawn_supervised_with;
use corvette_rtsp_client::depacketize::{Codec, Frame as ClientFrame};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

const CAMERA_NAME: &str = "browser-check";

// The exact real, genuinely decodable H.264 SPS/PPS/IDR triple
// `examples/g2_browser_fixture.rs` uses -- see that file's own doc for full
// provenance (a real encoder's own output, not hand-picked bytes). Reused
// byte-for-byte rather than invented: this item's own browser check needs
// the identical property G2's does (a real decoder must produce a picture),
// and there is no reason for the two fixtures to disagree about what
// "genuinely decodable" means.
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

/// Feeds `sender` a repeating SPS/PPS/IDR cycle at roughly 30fps -- identical
/// in shape and rationale to `examples/g2_browser_fixture.rs`'s own `feed`
/// (see that file's own doc for why every access unit is the same real IDR,
/// and why the parameter sets are repeated every cycle rather than sent
/// once).
async fn feed(sender: broadcast::Sender<ClientFrame>) {
    let mut timestamp: u32 = 0;
    let mut send = |nal: &[u8]| {
        let _ = sender.send(ClientFrame {
            codec: Codec::H264,
            timestamp,
            payload: annex_b(nal),
        });
        timestamp = timestamp.wrapping_add(3000);
    };
    loop {
        send(SPS);
        send(PPS);
        for _ in 0..30 {
            send(IDR);
            tokio::time::sleep(Duration::from_millis(33)).await;
        }
    }
}

#[tokio::main]
async fn main() {
    let mut store = MultiCameraHlsStore::default();
    store.register(CAMERA_NAME);
    let store = Arc::new(store);

    let (sender, _first_receiver) = broadcast::channel::<ClientFrame>(64);
    tokio::spawn(feed(sender.clone()));
    let _segment = spawn_supervised_with(format!("{CAMERA_NAME}/hls-segment"), {
        let sender = sender.clone();
        let store = Arc::clone(&store);
        move || {
            run_hls_segment(
                CAMERA_NAME.to_string(),
                sender.subscribe(),
                Arc::clone(&store),
            )
        }
    });

    let server = HlsServer::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("binds the HLS listener");
    let port = server
        .local_addr()
        .expect("a bound listener has a local address")
        .port();

    // The one line a driving script needs to read: this fixture's own bound
    // port, so it can pass an `http://` playlist URL to the Playwright spec
    // without both sides needing to agree on a fixed port in advance.
    println!("LISTENING {port}");

    server.serve(store).await;
}
