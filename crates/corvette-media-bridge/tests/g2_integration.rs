// cam-mock's frames carry no real SPS/PPS, so only cam-synth can resolve an init segment.

use bytes::Bytes;
use corvette_media_bridge::supervise::spawn_supervised_with;
use corvette_media_bridge::ws_repackager::{
    Fmp4WsServer, MultiCameraFmp4Store, run_fmp4_repackage,
};
use corvette_rtsp_client::client::{CameraConfig, Client};
use corvette_rtsp_client::depacketize::{Codec, Frame as ClientFrame};
use corvette_rtsp_client::mock_camera::{MockCamera, MockCameraConfig};
use corvette_rtsp_client::session::Credentials;
use futures_util::StreamExt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

// Real H.264 SPS/PPS/IDR bytes reused from existing fixtures, not invented.
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

/// Repeats an SPS/PPS/IDR cycle at 3000 RTP ticks per frame (90 kHz clock) until all receivers go away.
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

fn read_box_type(bytes: &[u8], offset: usize) -> [u8; 4] {
    let mut box_type = [0u8; 4];
    box_type.copy_from_slice(&bytes[offset + 4..offset + 8]);
    box_type
}

fn box_len(bytes: &[u8], offset: usize) -> usize {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize
}

#[tokio::test]
async fn g2_end_to_end_against_a_mock_camera_and_a_synthetic_decodable_one() {
    const FRAGMENT_COUNT: usize = 5;

    let mut store = MultiCameraFmp4Store::default();
    store.register("cam-mock");
    store.register("cam-synth");
    let store = Arc::new(store);

    let mock_camera = MockCamera::spawn(MockCameraConfig::new())
        .await
        .expect("binds the mock camera");
    let mock_client = Arc::new(Client::new(mock_camera_config(mock_camera.addr())));
    let mock_repackage = spawn_supervised_with("cam-mock/fmp4-repackage".to_string(), {
        let client = Arc::clone(&mock_client);
        let store = Arc::clone(&store);
        move || {
            run_fmp4_repackage(
                "cam-mock".to_string(),
                client.subscribe(),
                Arc::clone(&store),
            )
        }
    });

    let (synth_sender, _first_receiver) = broadcast::channel::<ClientFrame>(64);
    let synth_feeder = tokio::spawn(feed_synthetic_h264(synth_sender.clone()));
    let synth_repackage = spawn_supervised_with("cam-synth/fmp4-repackage".to_string(), {
        let sender = synth_sender.clone();
        let store = Arc::clone(&store);
        move || {
            run_fmp4_repackage(
                "cam-synth".to_string(),
                sender.subscribe(),
                Arc::clone(&store),
            )
        }
    });

    let server = Fmp4WsServer::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("binds the whole-process fMP4-WS server");
    let addr = server.local_addr().unwrap();
    tokio::spawn(server.serve(Arc::clone(&store)));

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !mock_repackage.is_finished(),
        "cam-mock's fMP4-repackaging task must not panic on undecodable frames"
    );

    let (mut ws, _response) = tokio_tungstenite::connect_async(format!("ws://{addr}/cam-synth"))
        .await
        .expect("connects to cam-synth's own WebSocket endpoint");

    let init_segment = next_binary_message(&mut ws).await;
    let ftyp_len = box_len(&init_segment, 0);
    assert_eq!(read_box_type(&init_segment, 0), *b"ftyp");
    assert_eq!(read_box_type(&init_segment, ftyp_len), *b"moov");
    assert!(
        init_segment.windows(SPS.len()).any(|w| w == SPS),
        "the init segment must carry the real SPS bytes"
    );

    for _ in 0..FRAGMENT_COUNT {
        let fragment = next_binary_message(&mut ws).await;
        assert_eq!(
            read_box_type(&fragment, 0),
            *b"moof",
            "every message after the init segment must start with a moof box"
        );
    }

    drop(mock_repackage);
    drop(mock_client);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let fragment_after_kill = next_binary_message(&mut ws).await;
    assert_eq!(
        read_box_type(&fragment_after_kill, 0),
        *b"moof",
        "cam-synth's WebSocket output must survive cam-mock's Client being killed"
    );

    drop(synth_repackage);
    synth_feeder.abort();
}

async fn next_binary_message(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Vec<u8> {
    tokio::time::timeout(WAIT_TIMEOUT, async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Binary(bytes))) => return bytes.to_vec(),
                Some(Ok(_)) => {}
                Some(Err(err)) => panic!("WebSocket read failed: {err}"),
                None => panic!("WebSocket closed before a binary message arrived"),
            }
        }
    })
    .await
    .expect("a binary message arrives before the test timeout")
}
