//! Test-support fixture for issue #12 item G2's own browser-based `Verify`
//! step: hosts this crate's real `fmp4`/`ws_repackager` code for one
//! synthetic camera, so a headless-browser test
//! (`crates/corvette-media-bridge/tests/browser/`) can drive a real
//! `SourceBuffer` -- and a real decoder -- against this item's own actual
//! output.
//!
//! Not shipped: a Cargo example, never built or copied by `P1`'s Dockerfile
//! (which only builds the `corvette-media-bridge` binary target). Prints
//! `LISTENING <port>` on its own stdout once bound, then runs until killed --
//! `scripts/run_g2_fmp4_ws_browser_check.sh` is what launches and tears this
//! down around a `playwright test` run.
//!
//! # Panics
//!
//! Panics if the WebSocket listener fails to bind.

use bytes::Bytes;
use corvette_media_bridge::supervise::spawn_supervised_with;
use corvette_media_bridge::ws_repackager::{
    Fmp4WsServer, MultiCameraFmp4Store, run_fmp4_repackage,
};
use corvette_rtsp_client::depacketize::{Codec, Frame as ClientFrame};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

const CAMERA_NAME: &str = "browser-check";

// A real, genuinely decodable single-frame H.264 baseline elementary stream
// (a 64x64 solid-blue picture), encoded with a real encoder
// (`ffmpeg -f lavfi -i "color=c=blue:s=64x64:d=1:r=30" -c:v libx264 -profile:v
// baseline -level 3.1 -pix_fmt yuv420p -x264-params keyint=1 -f h264`) and
// its SPS/PPS/IDR NAL units extracted directly from the encoder's own
// output -- deliberately NOT the shorter SPS/PPS/IDR bytes
// `g1_integration.rs`/`tests/g2_integration.rs`/`fmp4::tests` all share
// (reused from `moq-mux`'s own `h264::import` unit tests). Those bytes are
// real and valid enough for a container-level parser (`moq-mux`'s own
// `Import`, this crate's own box builders, a real MSE `SourceBuffer`'s
// stream parser) to accept without error, which is all G1's own tests and
// this item's own Rust unit tests ever needed -- but a genuinely complete,
// decodable IDR slice needs real encoded macroblock data a real encoder
// produces, not a hand-picked few bytes. This item's own browser check goes
// one step further than every other test in this codebase by requiring an
// actual decoder to produce a picture (`HAVE_CURRENT_DATA`), which is why
// only this one fixture needs its own, larger, truly decodable bytes.
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

/// Feeds `sender` a repeating SPS/PPS/IDR cycle at roughly 30fps.
///
/// Every access unit is the same real IDR keyframe, not a mix of keyframes
/// and fabricated non-keyframe payloads: unlike `IDR` (real, valid bytes
/// reused from `moq-mux`'s own test fixture -- see this file's own top-level
/// doc), a fabricated non-IDR slice would carry no genuine P-frame
/// prediction data, and this fixture's whole point is a real, independently
/// decodable stream a real browser can actually play (`fmp4::tests`, an
/// unrelated Rust-only structural test, does use a fabricated non-IDR NAL,
/// but only to assert this crate's own NAL-type classification -- it never
/// asks a decoder to produce a picture from it). Repeating an IDR every
/// frame is bandwidth-wasteful for a real encoder but fully standard-
/// conformant and trivially decodable.
///
/// Repeats the SPS/PPS pair on every cycle rather than sending it once --
/// matching `g1_integration.rs`'s and `tests/g2_integration.rs`'s own
/// `feed_synthetic_h264` precedent exactly, for the same reason: this task
/// and `run_fmp4_repackage`'s own `client_frames.subscribe()` race at
/// startup (a `broadcast` channel never replays history to a receiver that
/// subscribes after a send), so a one-shot parameter-set announcement can
/// lose the race and leave every future viewer waiting forever. A repeating
/// announcement self-heals within one cycle regardless of who wins that
/// race.
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
    let mut store = MultiCameraFmp4Store::default();
    store.register(CAMERA_NAME);
    let store = Arc::new(store);

    let (sender, _first_receiver) = broadcast::channel::<ClientFrame>(64);
    tokio::spawn(feed(sender.clone()));
    let _repackage = spawn_supervised_with(format!("{CAMERA_NAME}/fmp4-repackage"), {
        let sender = sender.clone();
        let store = Arc::clone(&store);
        move || {
            run_fmp4_repackage(
                CAMERA_NAME.to_string(),
                sender.subscribe(),
                Arc::clone(&store),
            )
        }
    });

    let server = Fmp4WsServer::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("binds the fMP4-WS listener");
    let port = server
        .local_addr()
        .expect("a bound listener has a local address")
        .port();

    // The one line a driving script needs to read: this fixture's own bound
    // port, so it can pass a `ws://` URL to the Playwright spec without both
    // sides needing to agree on a fixed port in advance.
    println!("LISTENING {port}");

    server.serve(store).await;
}
