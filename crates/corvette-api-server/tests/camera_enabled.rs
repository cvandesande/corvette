//! The public camera on/off route through the public door: the admin gate, the
//! forwarded `Remote-User` and byte-for-byte passthrough against a fake detection
//! service on a Unix socket, and the camera list flag taken from that service's
//! status answer.

use corvette_api::CameraList;
use corvette_api_server::config::{ConfigSource, DEFAULT_CACHE_TTL};
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use serde_json::Value;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio::task::JoinHandle;

const FRONT: &str = "/corvette/api/v1/cameras/front/enabled";
const USER: &str = "Remote-User: door-front";
const ADMIN: &str = "Remote-Role: admin";
const UNAVAILABLE: &str = "the detection service could not be reached over its local socket";
const FRIGATE_CONFIG: &str = include_str!("fixtures/frigate-config.json");

type SeenRequest = (String, String, Vec<u8>);
type Answered = (String, Vec<(String, String)>, Vec<u8>);

fn http_answer(status: &str, content_type: &str, extras: &[&str], body: &[u8]) -> Vec<u8> {
    let mut raw =
        format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\n");
    for extra in extras {
        let _ = write!(raw, "{extra}\r\n");
    }
    let _ = write!(
        raw,
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    [raw.into_bytes(), body.to_vec()].concat()
}

/// Reads one request: its request line, head block and body bytes.
async fn read_request(stream: &mut UnixStream) -> SeenRequest {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let end = loop {
        if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break end;
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => {
                let text = String::from_utf8_lossy(&buffer).into_owned();
                return (String::new(), text, Vec::new());
            }
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    };
    let text = String::from_utf8_lossy(&buffer[..end]).into_owned();
    let mut body = buffer[end + 4..].to_vec();
    let declared = text
        .to_lowercase()
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while body.len() < declared {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => body.extend_from_slice(&chunk[..read]),
        }
    }
    (
        text.lines().next().unwrap_or_default().to_owned(),
        text,
        body,
    )
}

struct Backend {
    socket: PathBuf,
    answer: Arc<tokio::sync::Mutex<Option<Vec<u8>>>>,
    delay: Arc<Mutex<Duration>>,
    requests: Arc<Mutex<Vec<SeenRequest>>>,
    connections: Arc<AtomicUsize>,
    handle: JoinHandle<()>,
}

impl Backend {
    fn start(dir: &Path) -> Self {
        let socket = dir.join("detection.sock");
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind fake detection service");
        let answer: Arc<tokio::sync::Mutex<Option<Vec<u8>>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let delay: Arc<Mutex<Duration>> = Arc::new(Mutex::new(Duration::ZERO));
        let (requests, connections) = (
            Arc::new(Mutex::new(Vec::new())),
            Arc::new(AtomicUsize::new(0)),
        );
        let (answer_loop, delay_loop, requests_loop, connections_loop) = (
            Arc::clone(&answer),
            Arc::clone(&delay),
            Arc::clone(&requests),
            Arc::clone(&connections),
        );
        let handle = tokio::spawn(async move {
            while let Ok((mut stream, _peer)) = listener.accept().await {
                connections_loop.fetch_add(1, Ordering::SeqCst);
                let seen = read_request(&mut stream).await;
                requests_loop.lock().expect("requests").push(seen);
                let wait = *delay_loop.lock().expect("delay");
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
                if let Some(bytes) = &*answer_loop.lock().await {
                    let _ = stream.write_all(bytes).await;
                }
            }
        });
        Self {
            socket,
            answer,
            delay,
            requests,
            connections,
            handle,
        }
    }

    async fn set(&self, answer: Vec<u8>) {
        *self.answer.lock().await = Some(answer);
    }

    fn set_delay(&self, wait: Duration) {
        *self.delay.lock().expect("delay") = wait;
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    fn seen(&self) -> Vec<SeenRequest> {
        self.requests.lock().expect("requests").clone()
    }
}

struct Env {
    socket: PathBuf,
    dir: PathBuf,
    server: JoinHandle<()>,
    backend: Option<Backend>,
}

impl Env {
    fn build(
        tag: &str,
        with_backend: bool,
        socket_override: Option<PathBuf>,
        config_addr: &str,
    ) -> Self {
        Self::build_timed(
            tag,
            with_backend,
            socket_override,
            config_addr,
            CONNECTION_TIMEOUT,
        )
    }

    fn build_timed(
        tag: &str,
        with_backend: bool,
        socket_override: Option<PathBuf>,
        config_addr: &str,
        connection_timeout: Duration,
    ) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "corvette-camera-enabled-{}-{tag}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create test directory");
        let backend = with_backend.then(|| Backend::start(&dir));
        let socket = socket_override.or_else(|| backend.as_ref().map(|b| b.socket.clone()));
        let source = Arc::new(ConfigSource::new(config_addr.to_owned(), DEFAULT_CACHE_TTL));
        let mut server = ApiServer::bind(
            &dir.join("api.sock"),
            connection_timeout,
            source,
            &dir.join("frigate.db"),
        )
        .expect("bind");
        if let Some(path) = socket {
            server = server.with_detection_socket(path);
        }
        let api_socket = server.socket_path().to_path_buf();
        let handle = tokio::spawn(server.serve_until(std::future::pending::<()>()));
        Self {
            socket: api_socket,
            dir,
            server: handle,
            backend,
        }
    }

    fn finish(self) {
        self.server.abort();
        if let Some(backend) = self.backend {
            backend.handle.abort();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Answers every configuration request with the fixture config; its address is
/// bound before the loop starts so callers never race the port choice.
async fn serve_config() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind config fixture");
    let addr = listener.local_addr().expect("fixture addr").to_string();
    let handle = tokio::spawn(async move {
        loop {
            let (mut stream, _peer) = listener.accept().await.expect("accept config");
            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 512];
            while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => buffer.extend_from_slice(&chunk[..read]),
                }
            }
            let body = FRIGATE_CONFIG.as_bytes();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(body).await;
        }
    });
    (addr, handle)
}

async fn send(
    env: &Env,
    method: &str,
    path: &str,
    headers: &[&str],
    body: Option<&str>,
) -> Answered {
    let mut out = format!("{method} {path} HTTP/1.1\r\n");
    for header in headers {
        let _ = write!(out, "{header}\r\n");
    }
    match body {
        Some(body) => {
            let _ = write!(out, "Content-Length: {}\r\n\r\n{body}", body.len());
        }
        None => out.push_str("\r\n"),
    }
    let mut stream = UnixStream::connect(&env.socket).await.expect("connect");
    stream.write_all(out.as_bytes()).await.expect("write");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("read to end");
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("header end");
    let head = String::from_utf8_lossy(&raw[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or_default().to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_lowercase(), value.trim().to_owned()))
        .collect();
    (status, headers, raw[end + 4..].to_vec())
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(name_, _value)| name_ == name)
        .map(|(_name, value)| value.as_str())
}

fn status_code(status: &str) -> u16 {
    status
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .parse()
        .unwrap_or(0)
}

#[tokio::test]
async fn admin_toggle_relays_user_path_and_body_and_passes_the_answer_through() {
    let env = Env::build("relay", true, None, "127.0.0.1:1");
    let answer = br#"{"camera":"front","enabled":false}"#;
    env.backend
        .as_ref()
        .expect("backend")
        .set(http_answer("200 OK", "application/json", &[], answer))
        .await;
    let (status, _headers, body) = send(
        &env,
        "POST",
        FRONT,
        &[USER, ADMIN],
        Some(r#"{"enabled":false}"#),
    )
    .await;
    assert_eq!(status, "HTTP/1.0 200 OK");
    assert_eq!(body, answer);
    let (request_line, head, sent_body) = env
        .backend
        .as_ref()
        .expect("backend")
        .seen()
        .into_iter()
        .next()
        .expect("one relayed request");
    assert_eq!(
        request_line,
        "POST /corvette/api/v1/cameras/front/enabled HTTP/1.1"
    );
    assert!(
        head.to_lowercase().contains("remote-user: door-front"),
        "the service sees the caller's user"
    );
    assert_eq!(sent_body, br#"{"enabled":false}"#);
    env.finish();
}

#[tokio::test]
async fn a_toggle_without_the_admin_role_is_forbidden_before_the_backend() {
    let env = Env::build("role", true, None, "127.0.0.1:1");
    let (status, _headers, body) = send(
        &env,
        "POST",
        FRONT,
        &[USER, "Remote-Role: viewer"],
        Some(r#"{"enabled":false}"#),
    )
    .await;
    assert_eq!(status_code(&status), 403);
    let value: Value = serde_json::from_slice(&body).expect("problem body");
    assert_eq!(value.get("code").and_then(Value::as_str), Some("forbidden"));
    assert_eq!(env.backend.as_ref().expect("backend").connections(), 0);
    env.finish();
}

#[tokio::test]
async fn a_toggle_without_remote_user_is_unauthorized_before_the_backend() {
    let env = Env::build("user", true, None, "127.0.0.1:1");
    let (status, _headers, _body) =
        send(&env, "POST", FRONT, &[ADMIN], Some(r#"{"enabled":false}"#)).await;
    assert_eq!(status_code(&status), 401);
    assert_eq!(env.backend.as_ref().expect("backend").connections(), 0);
    env.finish();
}

#[tokio::test]
async fn a_body_the_service_refuses_passes_through_byte_for_byte() {
    let env = Env::build("body", true, None, "127.0.0.1:1");
    let refusal =
        br#"{"type":"about:blank","status":400,"detail":"body is not one boolean enabled"}"#;
    env.backend
        .as_ref()
        .expect("backend")
        .set(http_answer(
            "400 Bad Request",
            "application/problem+json",
            &[],
            refusal,
        ))
        .await;
    let (status, _headers, body) =
        send(&env, "POST", FRONT, &[USER, ADMIN], Some("not json at all")).await;
    assert_eq!(status_code(&status), 400);
    assert_eq!(body, refusal);
    env.finish();
}

#[tokio::test]
async fn an_unknown_camera_keeps_the_services_answer() {
    let env = Env::build("unknown", true, None, "127.0.0.1:1");
    let missing = br#"{"reason":"unknown camera"}"#;
    env.backend
        .as_ref()
        .expect("backend")
        .set(http_answer(
            "404 Not Found",
            "application/json",
            &[],
            missing,
        ))
        .await;
    let (status, _headers, body) = send(
        &env,
        "POST",
        "/corvette/api/v1/cameras/nonesuch/enabled",
        &[USER, ADMIN],
        Some(r#"{"enabled":true}"#),
    )
    .await;
    assert_eq!(status_code(&status), 404);
    assert_eq!(body, missing);
    env.finish();
}

#[tokio::test]
async fn a_get_on_the_toggle_path_carries_the_services_method_answer() {
    let env = Env::build("method", true, None, "127.0.0.1:1");
    env.backend
        .as_ref()
        .expect("backend")
        .set(http_answer(
            "405 Method Not Allowed",
            "application/json",
            &["Allow: POST"],
            b"{\"reason\":\"POST only\"}",
        ))
        .await;
    let (status, headers, _body) = send(&env, "GET", FRONT, &[USER, ADMIN], None).await;
    assert_eq!(status_code(&status), 405);
    assert_eq!(header(&headers, "allow"), Some("POST"));
    env.finish();
}

#[tokio::test]
async fn an_unreachable_socket_answers_unavailable() {
    let dir = std::env::temp_dir().join(format!(
        "corvette-camera-enabled-gone-{}",
        std::process::id()
    ));
    let env = Env::build(
        "gone",
        false,
        Some(dir.join("absent-detection.sock")),
        "127.0.0.1:1",
    );
    let (status, _headers, body) = send(
        &env,
        "POST",
        FRONT,
        &[USER, ADMIN],
        Some(r#"{"enabled":false}"#),
    )
    .await;
    assert_eq!(status_code(&status), 503);
    assert!(
        String::from_utf8_lossy(&body).contains(UNAVAILABLE),
        "the contract's unavailable answer"
    );
    env.finish();
}

#[tokio::test]
async fn an_unset_socket_answers_unavailable() {
    let env = Env::build("unset", false, None, "127.0.0.1:1");
    let (status, _headers, body) = send(
        &env,
        "POST",
        FRONT,
        &[USER, ADMIN],
        Some(r#"{"enabled":false}"#),
    )
    .await;
    assert_eq!(status_code(&status), 503);
    assert!(String::from_utf8_lossy(&body).contains(UNAVAILABLE));
    env.finish();
}

fn status_body(answer: &str) -> Vec<u8> {
    http_answer("200 OK", "application/json", &[], answer.as_bytes())
}

/// front and back are live entries, living is absent: the list then shows
/// back off (status), front on (status) and living off (configuration value).
const STATUS_LIST: &str =
    r#"{"cameras":[{"camera":"front","state":"live"},{"camera":"back","state":"off"}]}"#;

#[tokio::test]
async fn the_camera_list_shows_the_flags_the_status_answers() {
    let (addr, config) = serve_config().await;
    let env = Env::build("flags", true, None, &addr);
    env.backend
        .as_ref()
        .expect("backend")
        .set(status_body(STATUS_LIST))
        .await;
    let (status, _headers, body) = send(
        &env,
        "GET",
        "/corvette/api/v1/cameras",
        &[USER, ADMIN],
        None,
    )
    .await;
    assert_eq!(status, "HTTP/1.0 200 OK");
    let list: CameraList = serde_json::from_slice(&body).expect("camera list");
    let flags: std::collections::BTreeMap<&str, bool> = list
        .cameras
        .iter()
        .map(|entry| (entry.name.as_str(), entry.enabled))
        .collect();
    assert_eq!(
        flags,
        [("back", false), ("front", true), ("living", false)]
            .into_iter()
            .collect::<std::collections::BTreeMap<&str, bool>>(),
        "status flags apply and an absent camera keeps its configuration value"
    );
    config.abort();
    env.finish();
}

#[tokio::test]
async fn the_camera_list_shows_configuration_values_when_no_socket_is_configured() {
    let (addr, config) = serve_config().await;
    let env = Env::build("stand", false, None, &addr);
    let (status, _headers, body) = send(
        &env,
        "GET",
        "/corvette/api/v1/cameras",
        &[USER, ADMIN],
        None,
    )
    .await;
    assert_eq!(status, "HTTP/1.0 200 OK");
    let list: CameraList = serde_json::from_slice(&body).expect("camera list");
    let enabled: std::collections::BTreeMap<&str, bool> = list
        .cameras
        .iter()
        .map(|entry| (entry.name.as_str(), entry.enabled))
        .collect();
    assert_eq!(
        enabled,
        [("back", true), ("front", true), ("living", false)]
            .into_iter()
            .collect::<std::collections::BTreeMap<&str, bool>>(),
        "without a status every camera keeps its fixture value"
    );
    config.abort();
    env.finish();
}

#[tokio::test]
async fn the_camera_list_shows_every_camera_off_when_the_status_cannot_be_read() {
    let (addr, config) = serve_config().await;
    let dir = std::env::temp_dir().join(format!(
        "corvette-camera-enabled-down-{}",
        std::process::id()
    ));
    let env = Env::build(
        "down",
        false,
        Some(dir.join("absent-detection.sock")),
        &addr,
    );
    let (status, _headers, body) = send(
        &env,
        "GET",
        "/corvette/api/v1/cameras",
        &[USER, ADMIN],
        None,
    )
    .await;
    assert_eq!(status, "HTTP/1.0 200 OK");
    let list: CameraList = serde_json::from_slice(&body).expect("camera list");
    let enabled: std::collections::BTreeMap<&str, bool> = list
        .cameras
        .iter()
        .map(|entry| (entry.name.as_str(), entry.enabled))
        .collect();
    assert_eq!(
        enabled,
        [("back", false), ("front", false), ("living", false)]
            .into_iter()
            .collect::<std::collections::BTreeMap<&str, bool>>(),
        "an on claim needs a status answer behind it"
    );
    config.abort();
    env.finish();
}

#[tokio::test]
async fn a_slow_toggle_answer_inside_the_relay_budget_arrives_unchanged() {
    let env = Env::build("slow", true, None, "127.0.0.1:1");
    let answer = br#"{"camera":"front","enabled":false}"#;
    let backend = env.backend.as_ref().expect("backend");
    backend
        .set(http_answer("200 OK", "application/json", &[], answer))
        .await;
    // 6 s outlasts the 5 s default connection bound and the 4 s shared relay total; the
    // toggle route's own connection bound and its 20 s relay budget cover the service join.
    backend.set_delay(Duration::from_secs(6));
    let (status, _headers, body) = send(
        &env,
        "POST",
        FRONT,
        &[USER, ADMIN],
        Some(r#"{"enabled":false}"#),
    )
    .await;
    assert_eq!(status, "HTTP/1.0 200 OK");
    assert_eq!(body, answer);
    env.finish();
}

#[tokio::test]
async fn a_percent_encoded_camera_name_reaches_the_service_still_encoded() {
    let env = Env::build("encode", true, None, "127.0.0.1:1");
    let answer = br#"{"camera":"living room","enabled":false}"#;
    env.backend
        .as_ref()
        .expect("backend")
        .set(http_answer("200 OK", "application/json", &[], answer))
        .await;
    let (status, _headers, body) = send(
        &env,
        "POST",
        "/corvette/api/v1/cameras/living%20room/enabled",
        &[USER, ADMIN],
        Some(r#"{"enabled":false}"#),
    )
    .await;
    assert_eq!(status, "HTTP/1.0 200 OK");
    assert_eq!(body, answer);
    let (request_line, _head, _sent_body) = env
        .backend
        .as_ref()
        .expect("backend")
        .seen()
        .into_iter()
        .next()
        .expect("one relayed request");
    assert_eq!(
        request_line, "POST /corvette/api/v1/cameras/living%20room/enabled HTTP/1.1",
        "the escaped space enters the forwarded path as %20"
    );
    env.finish();
}
