//! Raw-TCP smoke tests for `mock_camera`, hand-rolling the RTSP/1.0
//! handshake independently of any client code (there is none yet in this
//! crate) so this test cannot merely prove two halves of one implementation
//! agree with each other.

use corvette_rtsp_client::mock_camera::{MockCamera, MockCameraConfig};
use md5::{Digest as _, Md5};
use std::fmt::Write as _;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

const USERNAME: &str = "admin";
const PASSWORD: &str = "test-password-not-real";
const REALM: &str = "mock-camera";
const STREAM_URI: &str = "rtsp://127.0.0.1/stream/";
const TRACK_URI: &str = "rtsp://127.0.0.1/stream/track1";

struct RawResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl RawResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

enum Interleaved {
    Rtp { channel: u8, payload: Vec<u8> },
    Response(RawResponse),
}

/// A bare RTSP/1.0 client, hand-rolled for these tests only.
struct RawClient {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    cseq: u32,
}

impl RawClient {
    async fn connect(addr: SocketAddr) -> Self {
        let stream = TcpStream::connect(addr)
            .await
            .expect("mock camera accepts loopback connections");
        let (read_half, write_half) = stream.into_split();
        Self {
            reader: BufReader::new(read_half),
            writer: write_half,
            cseq: 0,
        }
    }

    async fn write_request(&mut self, method: &str, uri: &str, extra_headers: &[(&str, String)]) {
        self.cseq += 1;
        let mut text = format!("{method} {uri} RTSP/1.0\r\nCSeq: {}\r\n", self.cseq);
        for (name, value) in extra_headers {
            let _: std::fmt::Result = write!(text, "{name}: {value}\r\n");
        }
        text.push_str("\r\n");
        self.writer
            .write_all(text.as_bytes())
            .await
            .expect("write request");
    }

    async fn request(
        &mut self,
        method: &str,
        uri: &str,
        extra_headers: &[(&str, String)],
    ) -> RawResponse {
        self.write_request(method, uri, extra_headers).await;
        self.read_response().await
    }

    async fn authenticated_request(&mut self, method: &str, uri: &str, nonce: &str) -> RawResponse {
        let authorization = digest_authorization(method, uri, nonce);
        self.request(method, uri, &[("Authorization", authorization)])
            .await
    }

    async fn read_response(&mut self) -> RawResponse {
        let mut status_line = String::new();
        self.reader
            .read_line(&mut status_line)
            .await
            .expect("read status line");
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .expect("status line carries a status code")
            .parse()
            .expect("status code is numeric");

        let mut headers = Vec::new();
        loop {
            let mut line = String::new();
            self.reader
                .read_line(&mut line)
                .await
                .expect("read header line");
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                break;
            }
            let (name, value) = line.split_once(':').expect("well-formed header line");
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }

        let content_length: usize = headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("Content-Length"))
            .and_then(|(_, value)| value.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0_u8; content_length];
        if content_length > 0 {
            self.reader.read_exact(&mut body).await.expect("read body");
        }

        RawResponse {
            status,
            headers,
            body,
        }
    }

    /// Reads one interleaved message off the wire: a `$`-prefixed binary
    /// RTP/RTCP frame, or a plain-text RTSP response -- distinguished by the
    /// leading byte, exactly the ambiguity a real client's demuxer resolves.
    async fn read_interleaved(&mut self) -> Interleaved {
        let peek = self.reader.fill_buf().await.expect("peek the next byte");
        if peek.first() == Some(&b'$') {
            let mut header = [0_u8; 4];
            self.reader
                .read_exact(&mut header)
                .await
                .expect("read interleaved frame header");
            let channel = header[1];
            let len = usize::from(u16::from_be_bytes([header[2], header[3]]));
            let mut payload = vec![0_u8; len];
            self.reader
                .read_exact(&mut payload)
                .await
                .expect("read interleaved frame payload");
            Interleaved::Rtp { channel, payload }
        } else {
            Interleaved::Response(self.read_response().await)
        }
    }
}

fn md5_hex(input: &str) -> String {
    Md5::digest(input)
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _: std::fmt::Result = write!(hex, "{byte:02x}");
            hex
        })
}

fn digest_authorization(method: &str, uri: &str, nonce: &str) -> String {
    let ha1 = md5_hex(&format!("{USERNAME}:{REALM}:{PASSWORD}"));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    let response = md5_hex(&format!("{ha1}:{nonce}:{ha2}"));
    format!(
        r#"Digest username="{USERNAME}", realm="{REALM}", nonce="{nonce}", uri="{uri}", response="{response}""#
    )
}

fn extract_nonce(response: &RawResponse) -> String {
    let header = response
        .header("WWW-Authenticate")
        .expect("401 response carries a WWW-Authenticate challenge");
    let start = header
        .find("nonce=\"")
        .expect("challenge carries a nonce field")
        + "nonce=\"".len();
    let end = header[start..].find('"').expect("nonce value is quoted");
    header[start..start + end].to_string()
}

async fn setup_and_play(client: &mut RawClient, config_timeout_secs: Option<u64>) -> String {
    let challenge = client.request("SETUP", TRACK_URI, &[]).await;
    assert_eq!(challenge.status, 401);
    let nonce = extract_nonce(&challenge);

    let setup = client
        .authenticated_request("SETUP", TRACK_URI, &nonce)
        .await;
    assert_eq!(setup.status, 200);
    let session_header = setup
        .header("Session")
        .expect("SETUP response carries a Session header")
        .to_string();
    if let Some(expected) = config_timeout_secs {
        assert!(
            session_header.contains(&format!("timeout={expected}")),
            "declared timeout must be echoed verbatim: {session_header}"
        );
    }

    let play = client
        .authenticated_request("PLAY", STREAM_URI, &nonce)
        .await;
    assert_eq!(play.status, 200);

    nonce
}

#[tokio::test]
async fn digest_auth_rejects_unauthenticated_and_basic_with_a_fresh_nonce_each_time() {
    let camera = MockCamera::spawn(MockCameraConfig::new())
        .await
        .expect("mock camera binds loopback");
    let mut client = RawClient::connect(camera.addr()).await;

    let unauthenticated = client.request("OPTIONS", STREAM_URI, &[]).await;
    assert_eq!(unauthenticated.status, 401);
    let nonce1 = extract_nonce(&unauthenticated);

    let basic_attempt = ("Authorization", "Basic YWRtaW46dGVzdA==".to_string());
    let basic = client
        .request("OPTIONS", STREAM_URI, std::slice::from_ref(&basic_attempt))
        .await;
    assert_eq!(basic.status, 401, "Basic auth is never accepted");
    let nonce2 = extract_nonce(&basic);
    assert_ne!(
        nonce1, nonce2,
        "every failed challenge issues a brand new nonce, never a cached one"
    );

    let stale = client
        .authenticated_request("OPTIONS", STREAM_URI, &nonce1)
        .await;
    assert_eq!(
        stale.status, 401,
        "a superseded nonce is never accepted, even if it was valid earlier"
    );
    let nonce3 = extract_nonce(&stale);

    let authenticated = client
        .authenticated_request("OPTIONS", STREAM_URI, &nonce3)
        .await;
    assert_eq!(
        authenticated.status, 200,
        "the most recently issued nonce is accepted"
    );
}

#[tokio::test]
async fn describe_sdp_puts_session_control_before_the_track_control_inside_m_video() {
    let camera = MockCamera::spawn(MockCameraConfig::new())
        .await
        .expect("mock camera binds loopback");
    let mut client = RawClient::connect(camera.addr()).await;

    let nonce = extract_nonce(&client.request("DESCRIBE", STREAM_URI, &[]).await);
    let described = client
        .authenticated_request("DESCRIBE", STREAM_URI, &nonce)
        .await;
    assert_eq!(described.status, 200);
    let sdp = String::from_utf8(described.body).expect("SDP body is UTF-8 text");

    let session_control = sdp
        .find("a=control:*")
        .expect("session-level control line present");
    let m_video = sdp.find("m=video").expect("video section present");
    let track_control = sdp
        .find("a=control:track1")
        .expect("track-level control line present");
    assert!(
        session_control < m_video,
        "session-level a=control must precede m=video"
    );
    assert!(
        track_control > m_video,
        "track-level a=control must sit inside the m=video section"
    );
}

#[tokio::test]
async fn setup_echoes_declared_timeout_and_play_streams_interleaved_rtp_frames() {
    let camera =
        MockCamera::spawn(MockCameraConfig::new().declared_timeout(Duration::from_secs(30)))
            .await
            .expect("mock camera binds loopback");
    let mut client = RawClient::connect(camera.addr()).await;
    setup_and_play(&mut client, Some(30)).await;

    for _ in 0..3 {
        match client.read_interleaved().await {
            Interleaved::Rtp { channel, payload } => {
                assert_eq!(
                    channel, 0,
                    "RTP frames are sent on the SETUP-negotiated interleaved channel 0"
                );
                assert!(!payload.is_empty());
            }
            Interleaved::Response(_) => panic!("expected an RTP frame, got a text response"),
        }
    }
}

#[tokio::test]
async fn missing_keepalive_silently_stops_frames_without_closing_the_socket() {
    let camera = MockCamera::spawn(
        MockCameraConfig::new()
            .declared_timeout(Duration::from_millis(150))
            .frame_interval(Duration::from_millis(20)),
    )
    .await
    .expect("mock camera binds loopback");
    let mut client = RawClient::connect(camera.addr()).await;
    let nonce = setup_and_play(&mut client, None).await;

    let deadline = tokio::time::Instant::now() + Duration::from_millis(400);
    while tokio::time::Instant::now() < deadline {
        let _ = tokio::time::timeout(Duration::from_millis(50), client.read_interleaved()).await;
    }

    let after_deadline =
        tokio::time::timeout(Duration::from_millis(200), client.read_interleaved()).await;
    assert!(
        after_deadline.is_err(),
        "frames must stop silently once the declared timeout has elapsed with no keep-alive"
    );

    let after_stall = client
        .authenticated_request("GET_PARAMETER", STREAM_URI, &nonce)
        .await;
    assert_eq!(
        after_stall.status, 200,
        "the socket stays open after the silent stall, unlike a real close"
    );
}

#[tokio::test]
async fn keepalive_reply_is_demuxed_correctly_between_rtp_frames() {
    let camera = MockCamera::spawn(
        MockCameraConfig::new()
            .declared_timeout(Duration::from_secs(30))
            .frame_interval(Duration::from_millis(30))
            .delay_keepalive_reply_to_next_frame(true),
    )
    .await
    .expect("mock camera binds loopback");
    let mut client = RawClient::connect(camera.addr()).await;
    let nonce = setup_and_play(&mut client, None).await;

    assert!(matches!(
        client.read_interleaved().await,
        Interleaved::Rtp { .. }
    ));

    let authorization = digest_authorization("GET_PARAMETER", STREAM_URI, &nonce);
    client
        .write_request(
            "GET_PARAMETER",
            STREAM_URI,
            &[("Authorization", authorization)],
        )
        .await;

    let mut saw_rtp_after_request = false;
    let mut saw_keepalive_response = false;
    for _ in 0..8 {
        match client.read_interleaved().await {
            Interleaved::Rtp { .. } => saw_rtp_after_request = true,
            Interleaved::Response(response) => {
                assert_eq!(response.status, 200);
                saw_keepalive_response = true;
            }
        }
        if saw_rtp_after_request && saw_keepalive_response {
            break;
        }
    }
    assert!(
        saw_rtp_after_request && saw_keepalive_response,
        "both an RTP frame and the keep-alive's own text response must be observed, neither corrupting the other"
    );
}

#[tokio::test]
async fn disconnect_closes_the_socket_immediately_unlike_the_silent_stall() {
    let camera = MockCamera::spawn(MockCameraConfig::new())
        .await
        .expect("mock camera binds loopback");
    let mut client = RawClient::connect(camera.addr()).await;
    setup_and_play(&mut client, None).await;

    assert!(matches!(
        client.read_interleaved().await,
        Interleaved::Rtp { .. }
    ));

    camera.disconnect();

    // Frames already in flight when disconnect() fires may still land in the
    // client's read buffer; drain them and require the socket to reach a
    // real, clean EOF -- unlike the silent stall, which never does.
    let reached_eof = tokio::time::timeout(Duration::from_secs(2), async {
        let mut probe = [0_u8; 256];
        loop {
            match client.reader.read(&mut probe).await {
                Ok(0) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .expect("disconnect() closes the socket within the timeout");
    assert!(
        reached_eof,
        "the socket must reach a clean EOF after disconnect(), a real close unlike the silent stall"
    );
}
