//! Integration test for issue #12 item G1: a real, locally-run `moq-relay`
//! (pinned per R1) plus issue #18's own mock camera server
//! (`corvette_rtsp_client::mock_camera`, reused directly) standing in for
//! one camera, exercising this crate's own `restream_provider`/`moq_publish`
//! modules end to end.
//!
//! **Why two cameras, only one backed by a real RTSP dial.** Issue #18's own
//! `mock_camera` fabricates RTP payloads containing no real SPS/PPS and
//! declares no `sprop-parameter-sets` in its SDP (confirmed by direct source
//! read of `corvette-rtsp-client/src/mock_camera/{rtp.rs,rtsp_message.rs}`):
//! it was built to test issue #18's own depacketizer's NAL/FU-A framing, not
//! to produce decodable video. `cam-mock` below still dials it through a
//! real `corvette_rtsp_client::client::Client`, proving the real RTSP
//! handshake/reconnect/task-wiring path holds and that this crate's own
//! frame-error handling (a keyframe with no SPS) degrades gracefully rather
//! than panicking -- but its `DESCRIBE`/MoQ-catalog can never resolve, so it
//! cannot exercise the catalog-resolution or content-correctness assertions.
//! `cam-synth` supplies a second, independent
//! `corvette_rtsp_client::depacketize::Frame` broadcast, fed directly (not
//! via any RTSP dial) with a real, valid H.264 SPS/PPS/IDR triple -- the
//! same fixture bytes `moq-mux`'s own `h264::import` unit tests
//! (`avc3_self_initializes_from_first_keyframe`) and this project's own R1
//! evidence use. This is not a second RTSP mock server: no RTSP protocol
//! code is duplicated here, only this crate's own public
//! `restream_provider`/`moq_publish` worker functions are driven directly,
//! exactly the way `crate::start` already wires a real `Client`'s
//! subscriptions to them. This mirrors this same plan's own X2 precedent
//! (a small fixture written for a test only, since the shipped depacketizer
//! it would otherwise reuse "is not wired into any shipped consumer") and
//! R1's own precedent (a synthetic Annex-B SPS/PPS/IDR sequence to prove the
//! codec-level `Split`/`Import` API).
//!
//! Ignored by default (`cargo test --workspace` skips it): it needs a real
//! `moq-relay` binary, which is not a workspace-managed build artifact. Run
//! it with:
//!
//! ```text
//! MOQ_RELAY_BIN=/path/to/moq-relay cargo test -p corvette-media-bridge \
//!     --test g1_integration -- --ignored --nocapture
//! ```

use corvette_media_bridge::config::MoqConfig;
use corvette_media_bridge::restream_provider::MultiCameraProvider;
use corvette_media_bridge::supervise::spawn_supervised_with;
use corvette_media_bridge::{moq_publish, restream_provider};
use corvette_rtsp_client::client::{CameraConfig, Client};
use corvette_rtsp_client::depacketize::{Codec, Frame as ClientFrame};
use corvette_rtsp_client::mock_camera::{MockCamera, MockCameraConfig};
use corvette_rtsp_client::session::Credentials;
use rtsp_restream::RtspServer;
use rtsp_types::headers::{CSEQ, TRANSPORT};
use rtsp_types::{Message, Method, ParseError, Request, Response, StatusCode, Url, Version};
use std::net::{Ipv4Addr, SocketAddr};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;

const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

// A real, valid H.264 SPS/PPS/IDR triple -- the exact fixture bytes
// moq-mux's own `codec::h264::import::tests::avc3_self_initializes_from_first_keyframe`
// uses (read directly from the pinned commit's source), reused here rather
// than invented, since the whole point is a byte-real, decodable keyframe.
const SPS: &[u8] = &[
    0x67, 0x42, 0xc0, 0x1f, 0xda, 0x01, 0x40, 0x16, 0xe9, 0xb8, 0x08, 0x08, 0x0a, 0x00, 0x00, 0x07,
    0xd0, 0x00, 0x01, 0xd4, 0xc0, 0x80,
];
const PPS: &[u8] = &[0x68, 0xce, 0x3c, 0x80];
const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x21];

fn annex_b(nal: &[u8]) -> bytes::Bytes {
    let mut payload = Vec::with_capacity(nal.len() + 4);
    payload.extend_from_slice(&[0, 0, 0, 1]);
    payload.extend_from_slice(nal);
    bytes::Bytes::from(payload)
}

/// Feeds `sender` a repeating SPS/PPS/IDR cycle at roughly 30fps (90 kHz RTP
/// clock, 3000 ticks/frame) until the receiver side goes away.
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
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Spawns a real `moq-relay` subprocess (pinned per R1), bound to an
/// ephemeral loopback port with an ephemeral self-signed certificate and
/// public (unauthenticated) access -- the exact CLI shape R1's own evidence
/// confirmed works.
struct RelayProcess {
    child: Child,
    port: u16,
}

impl RelayProcess {
    fn spawn() -> Self {
        let bin = std::env::var("MOQ_RELAY_BIN")
            .expect("set MOQ_RELAY_BIN to a moq-relay binary built at the pinned commit (see this test's own doc)");
        let port = pick_ephemeral_port();
        let child = Command::new(bin)
            .args([
                "--server-bind",
                &format!("127.0.0.1:{port}"),
                "--tls-generate",
                "localhost",
                "--auth-public",
                "/",
                "--log-level",
                "info",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawns the moq-relay subprocess");
        Self { child, port }
    }

    fn relay_url(&self, path: &str) -> url::Url {
        // A literal IP, not the hostname `--tls-generate` minted the cert
        // for: `web_transport_quinn::Client::connect` takes only the FIRST
        // address `lookup_host` resolves for a domain, with no IPv4/IPv6
        // Happy-Eyeballs racing (unlike moq-native's own dial, which does
        // race both). A literal IP skips DNS entirely, avoiding a
        // resolver-order-dependent connect timeout if "localhost" resolves
        // to `::1` first while the relay only bound `127.0.0.1`. TLS
        // hostname verification is irrelevant here since this test always
        // sets `tls_disable_verify` (see `MoqConfig`'s own doc).
        format!("https://127.0.0.1:{}/{path}", self.port)
            .parse()
            .unwrap()
    }
}

impl Drop for RelayProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn pick_ephemeral_port() -> u16 {
    std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("binds a throwaway TCP socket to learn a free port")
        .local_addr()
        .unwrap()
        .port()
}

// ---------------------------------------------------------------------
// A hand-rolled RTSP/RTP client, built directly against `rtsp-types` --
// not against `corvette-rtsp-client`, matching X3's own test convention
// (using a CLIENT crate to test a SERVER would risk a self-referential
// oracle).
// ---------------------------------------------------------------------

struct TestClient {
    stream: TcpStream,
    buffer: Vec<u8>,
    cseq: u32,
}

impl TestClient {
    async fn connect(addr: SocketAddr) -> Self {
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

    async fn read_response(&mut self) -> Response<Vec<u8>> {
        tokio::time::timeout(WAIT_TIMEOUT, async {
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
        })
        .await
        .expect("a Response arrives before the test timeout")
    }

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

    /// Retries `DESCRIBE` until it succeeds or the test timeout elapses --
    /// `cam-synth`'s own `StreamInfo` resolves asynchronously, the moment
    /// its feed task has observed a full SPS+PPS pair.
    async fn describe_until_ready(&mut self, stream_name: &str) -> Response<Vec<u8>> {
        tokio::time::timeout(WAIT_TIMEOUT, async {
            loop {
                let response = self
                    .send_request(
                        Method::Describe,
                        &format!("rtsp://127.0.0.1/{stream_name}"),
                        None,
                    )
                    .await;
                if response.status() == StatusCode::Ok {
                    return response;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("DESCRIBE succeeded before the test timeout")
    }

    async fn play(&mut self, stream_name: &str) {
        let describe = self.describe_until_ready(stream_name).await;
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
            .send_request(
                Method::Play,
                &format!("rtsp://127.0.0.1/{stream_name}"),
                None,
            )
            .await;
        assert_eq!(play.status(), StatusCode::Ok, "PLAY must succeed");
    }
}

fn rtp_payload(packet: &[u8]) -> &[u8] {
    &packet[12..]
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

#[tokio::test]
#[ignore = "needs a real moq-relay binary; set MOQ_RELAY_BIN and pass --ignored (see this file's own doc)"]
async fn g1_end_to_end_against_a_real_relay_and_mock_camera() {
    // ---- Wiring: two cameras, one whole-process RtspServer, a real relay ----
    let relay = RelayProcess::spawn();
    // Give the relay a moment to bind before anything dials it.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let moq_config = MoqConfig {
        relay_url: relay.relay_url("anon"),
        tls_disable_verify: true,
    };

    let mut provider = MultiCameraProvider::default();
    provider.register("cam-mock");
    provider.register("cam-synth");
    let provider = Arc::new(provider);

    // cam-mock: a real corvette_rtsp_client::Client dialing issue #18's own
    // mock camera server, unmodified.
    let mock_camera = MockCamera::spawn(MockCameraConfig::new())
        .await
        .expect("binds the mock camera");
    let mock_client = Arc::new(Client::new(mock_camera_config(mock_camera.addr())));
    let mock_restream_feed = spawn_supervised_with("cam-mock/restream-feed".to_string(), {
        let client = Arc::clone(&mock_client);
        let provider = Arc::clone(&provider);
        move || {
            restream_provider::run_restream_feed(
                "cam-mock".to_string(),
                client.subscribe(),
                Arc::clone(&provider),
            )
        }
    });
    let mock_moq_publish = spawn_supervised_with("cam-mock/moq-publish".to_string(), {
        let client = Arc::clone(&mock_client);
        let moq_config = moq_config.clone();
        move || {
            moq_publish::run_publish_loop(
                "cam-mock".to_string(),
                moq_config.clone(),
                client.subscribe(),
            )
        }
    });

    // cam-synth: a real, valid H.264 SPS/PPS/IDR triple, fed directly onto a
    // broadcast channel standing in for what `Client::subscribe()` returns
    // (see this file's own top-level doc for why).
    let (synth_sender, _first_receiver) = broadcast::channel::<ClientFrame>(64);
    let synth_feeder = tokio::spawn(feed_synthetic_h264(synth_sender.clone()));
    let synth_restream_feed = spawn_supervised_with("cam-synth/restream-feed".to_string(), {
        let sender = synth_sender.clone();
        let provider = Arc::clone(&provider);
        move || {
            restream_provider::run_restream_feed(
                "cam-synth".to_string(),
                sender.subscribe(),
                Arc::clone(&provider),
            )
        }
    });
    let synth_moq_publish = spawn_supervised_with("cam-synth/moq-publish".to_string(), {
        let sender = synth_sender.clone();
        let moq_config = moq_config.clone();
        move || {
            moq_publish::run_publish_loop(
                "cam-synth".to_string(),
                moq_config.clone(),
                sender.subscribe(),
            )
        }
    });

    let server = RtspServer::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("binds the whole-process RTSP server");
    let rtsp_addr = server.local_addr().unwrap();
    tokio::spawn(server.serve(provider as Arc<dyn rtsp_restream::StreamProvider>));

    // ---- Verify (b): a hand-rolled RTSP client DESCRIBEs/SETUPs/PLAYs
    // cam-synth and receives the same underlying video, byte-for-byte. ----
    let mut rtsp_client = TestClient::connect(rtsp_addr).await;
    verify_b_rtsp_client_receives_real_video(&mut rtsp_client).await;

    // ---- Verify (a): a moq_net subscriber sees cam-synth's catalog and
    // receives real frames. ----
    let (broadcast, _sub_session_handle) =
        verify_a_moq_subscriber_receives_catalog_and_frames(&moq_config).await;

    // ---- Verify (c): killing cam-mock's Client/publish tasks does not
    // affect cam-synth's outputs on either path. ----
    mock_restream_feed_drop_and_assert_isolated(
        mock_restream_feed,
        mock_moq_publish,
        mock_client,
        &rtsp_client,
        &broadcast,
    )
    .await;

    // Cleanup: nothing further to assert; dropping these ends the tasks.
    drop(synth_restream_feed);
    drop(synth_moq_publish);
    synth_feeder.abort();
}

async fn dial_relay_as(config: &MoqConfig) -> web_transport_quinn::Session {
    let builder = web_transport_quinn::ClientBuilder::new();
    let client = if config.tls_disable_verify {
        builder
            .dangerous()
            .with_no_certificate_verification()
            .expect("builds a no-verify TLS client")
    } else {
        builder
            .with_system_roots()
            .expect("builds a system-roots TLS client")
    };
    let mut request = web_transport_quinn::proto::ConnectRequest::new(config.relay_url.clone());
    for alpn in moq_net::Versions::all().alpns() {
        request = request.with_protocol(alpn.to_string());
    }
    client
        .connect(request)
        .await
        .expect("dials the real local moq-relay")
}

/// Verify (b): a hand-rolled RTSP client `DESCRIBE`/`SETUP`/`PLAY`s
/// `cam-synth` and receives the same underlying video, byte-for-byte.
async fn verify_b_rtsp_client_receives_real_video(rtsp_client: &mut TestClient) {
    rtsp_client.play("cam-synth").await;

    let sps_packet = rtsp_client.read_data_frame().await;
    let pps_packet = rtsp_client.read_data_frame().await;
    let idr_packet = rtsp_client.read_data_frame().await;
    assert_eq!(
        rtp_payload(&sps_packet),
        SPS,
        "RTSP client's first NAL must be the real SPS, byte-for-byte"
    );
    assert_eq!(
        rtp_payload(&pps_packet),
        PPS,
        "RTSP client's second NAL must be the real PPS, byte-for-byte"
    );
    assert_eq!(
        rtp_payload(&idr_packet),
        IDR,
        "RTSP client's third NAL must be the real IDR slice, byte-for-byte"
    );
    println!(
        "verify(b): RTSP DESCRIBE/SETUP/PLAY against cam-synth received the real SPS/PPS/IDR byte-for-byte"
    );
}

/// Verify (a): a `moq_net` subscriber sees `cam-synth`'s catalog and
/// receives real frames. Returns the broadcast consumer so the caller (the
/// verify(c) step) can keep reading from it after `cam-mock` is killed.
async fn verify_a_moq_subscriber_receives_catalog_and_frames(
    moq_config: &MoqConfig,
) -> (moq_net::broadcast::Consumer, moq_net::Session) {
    let sub_origin = moq_net::Origin::random().produce();
    let sub_session = dial_relay_as(moq_config).await;
    let moq_sub_client = moq_net::Client::new()
        .with_versions(moq_net::Versions::all())
        .with_subscriber(sub_origin.clone());
    // The returned `Session` handle must outlive this function: per
    // `moq_net`'s own doc, "the transport still closes when the last
    // Session clone drops" -- dropping it here (as this function's own
    // temporary earlier did, a real regression this fix corrects) would
    // close the subscriber's own QUIC connection the moment this function
    // returns, well before the caller is done reading from `broadcast`.
    let (sub_session_handle, sub_driver) = moq_sub_client
        .connect(sub_session)
        .await
        .expect("MoQ handshake as subscriber");
    tokio::spawn(sub_driver);

    let consumer = sub_origin.consume();
    let broadcast = tokio::time::timeout(WAIT_TIMEOUT, consumer.announced_broadcast("cam-synth"))
        .await
        .expect("cam-synth announced before the test timeout")
        .expect("cam-synth's broadcast is announced");

    let catalog_track = broadcast
        .track("catalog.json")
        .expect("catalog track exists");
    let mut catalog_subscriber = catalog_track
        .subscribe(None)
        .await
        .expect("subscribes to the catalog track");
    let catalog_frame = tokio::time::timeout(WAIT_TIMEOUT, catalog_subscriber.read_frame())
        .await
        .expect("catalog frame arrives before the test timeout")
        .expect("catalog read succeeds")
        .expect("a catalog frame is produced");
    assert!(
        !catalog_frame.payload.is_empty(),
        "cam-synth's catalog must be non-empty once resolved"
    );
    println!(
        "verify(a): moq_net subscriber received a {}-byte cam-synth catalog frame",
        catalog_frame.payload.len()
    );

    let video_track = broadcast.track("video").expect("video track exists");
    let mut video_subscriber = video_track
        .subscribe(None)
        .await
        .expect("subscribes to the video track");
    let video_frame = tokio::time::timeout(WAIT_TIMEOUT, video_subscriber.read_frame())
        .await
        .expect("video frame arrives before the test timeout")
        .expect("video read succeeds")
        .expect("a video frame is produced");
    assert!(
        !video_frame.payload.is_empty(),
        "cam-synth's video track must carry real frame bytes"
    );
    println!(
        "verify(a): moq_net subscriber received a {}-byte cam-synth video frame",
        video_frame.payload.len()
    );

    (broadcast, sub_session_handle)
}

/// Kills `cam-mock`'s real `Client` and both of its supervised tasks, then
/// asserts `cam-synth`'s own RTSP output (a fresh `PLAY`) and `MoQ` output (a
/// fresh frame read on the already-open subscription) both continue
/// completely unaffected.
async fn mock_restream_feed_drop_and_assert_isolated(
    mock_restream_feed: corvette_media_bridge::supervise::Supervised,
    mock_moq_publish: corvette_media_bridge::supervise::Supervised,
    mock_client: Arc<Client>,
    rtsp_client: &TestClient,
    broadcast: &moq_net::broadcast::Consumer,
) {
    drop(mock_restream_feed);
    drop(mock_moq_publish);
    drop(mock_client);
    // Task abort() only requests cancellation; give the runtime a moment to
    // actually drop the aborted tasks' state (including their own Arc<Client>
    // clones) before asserting the kill took effect.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // cam-synth's RTSP output: a brand-new connection can still PLAY it.
    let mut second_rtsp_client = TestClient::connect(rtsp_client.stream.peer_addr().unwrap()).await;
    second_rtsp_client.play("cam-synth").await;
    let packet = second_rtsp_client.read_data_frame().await;
    assert!(
        !packet.is_empty(),
        "cam-synth's RTSP output must survive cam-mock's Client being killed"
    );

    // cam-synth's MoQ output: the already-open subscription keeps producing.
    let video_track = broadcast.track("video").expect("video track still exists");
    let mut video_subscriber = video_track
        .subscribe(None)
        .await
        .expect("still subscribes to the video track");
    let frame = tokio::time::timeout(WAIT_TIMEOUT, video_subscriber.read_frame())
        .await
        .expect("a further video frame arrives before the test timeout")
        .expect("video read succeeds")
        .expect("a video frame is produced");
    assert!(
        !frame.payload.is_empty(),
        "cam-synth's MoQ output must survive cam-mock's Client being killed"
    );
    println!(
        "verify(c): cam-synth's RTSP and MoQ outputs both survived cam-mock's Client/tasks being killed"
    );
}
