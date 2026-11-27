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

// Real ffmpeg encoder output: the shorter SPS/PPS/IDR bytes other tests share parse but never decode.
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

/// Repeats an SPS/PPS/IDR cycle: a one-shot parameter-set send can lose the startup subscribe race and starve every viewer.
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

    // The driving script reads this line to get the bound port for its ws:// URL.
    println!("LISTENING {port}");

    server.serve(store).await;
}
