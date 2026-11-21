//! The detection configuration routes through the public door: the admin gate on
//! the write, the refusals answered before any relay, and the byte-for-byte
//! passthrough against a fake detection service on a Unix socket.

use corvette_api_server::config::{ConfigSource, DEFAULT_CACHE_TTL};
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;

const CONFIG_PATH: &str = "/corvette/api/v1/detection/config";
const VALIDATE_PATH: &str = "/corvette/api/v1/detection/config/validate";
const SCHEMA_PATH: &str = "/corvette/api/v1/detection/config/schema";
const USER: &str = "Remote-User: door-front";
const ADMIN: &str = "Remote-Role: admin";
const UNAVAILABLE: &str = "the detection service could not be reached over its local socket";

type SeenRequest = (String, String, Vec<u8>);
type Answered = (String, Vec<(String, String)>, Vec<u8>);
type RefusalRow<'a> = (
    &'a [&'a str],
    Option<&'a str>,
    &'static str,
    (String, String),
);

enum Answer {
    Bytes(Vec<u8>),
    Stall,
}

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
    answer: Arc<tokio::sync::Mutex<Answer>>,
    requests: Arc<Mutex<Vec<SeenRequest>>>,
    connections: Arc<AtomicUsize>,
    handle: JoinHandle<()>,
}

impl Backend {
    fn start(dir: &Path) -> Self {
        let socket = dir.join("detection.sock");
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind fake detection service");
        let (answer, requests, connections) = (
            Arc::new(tokio::sync::Mutex::new(Answer::Stall)),
            Arc::new(Mutex::new(Vec::new())),
            Arc::new(AtomicUsize::new(0)),
        );
        let (answer_loop, requests_loop, connections_loop) = (
            Arc::clone(&answer),
            Arc::clone(&requests),
            Arc::clone(&connections),
        );
        let handle = tokio::spawn(async move {
            let mut sink = [0_u8; 1];
            while let Ok((mut stream, _peer)) = listener.accept().await {
                connections_loop.fetch_add(1, Ordering::SeqCst);
                let seen = read_request(&mut stream).await;
                requests_loop.lock().expect("requests").push(seen);
                match &*answer_loop.lock().await {
                    Answer::Bytes(bytes) => {
                        let _ = stream.write_all(bytes).await;
                    }
                    Answer::Stall => {
                        let _ = stream.read(&mut sink).await;
                    }
                }
            }
        });
        Self {
            socket,
            answer,
            requests,
            connections,
            handle,
        }
    }

    async fn set(&self, answer: Answer) {
        *self.answer.lock().await = answer;
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
        timeouts: Option<(Duration, Duration)>,
    ) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "corvette-detection-config-{}-{tag}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create test directory");
        let backend = with_backend.then(|| Backend::start(&dir));
        let socket = socket_override.or_else(|| backend.as_ref().map(|b| b.socket.clone()));
        let source = Arc::new(ConfigSource::new(
            "127.0.0.1:1".to_owned(),
            DEFAULT_CACHE_TTL,
        ));
        let mut server = ApiServer::bind(
            &dir.join("api.sock"),
            CONNECTION_TIMEOUT,
            source,
            &dir.join("frigate.db"),
        )
        .expect("bind");
        if let Some(path) = socket {
            server = server.with_detection_socket(path);
        }
        if let Some((connect, total)) = timeouts {
            server = server.with_detection_timeouts(connect, total);
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

    const fn backend(&self) -> &Backend {
        self.backend.as_ref().expect("test has a fake backend")
    }

    fn finish(self) {
        self.server.abort();
        if let Some(backend) = self.backend {
            backend.handle.abort();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Sends one caller request and returns status line, headers and body of the answer.
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

/// Queues one canned backend answer for the next relayed request.
async fn canned(env: &Env, status: &str, content_type: &str, extras: &[&str], body: &[u8]) {
    env.backend()
        .set(Answer::Bytes(http_answer(
            status,
            content_type,
            extras,
            body,
        )))
        .await;
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn assert_unavailable(status: &str, headers: &[(String, String)], body: &[u8]) {
    assert_eq!(status, "HTTP/1.0 503 Service Unavailable");
    assert_eq!(
        header(headers, "content-type"),
        Some("application/problem+json")
    );
    let parsed: Value = serde_json::from_slice(body).expect("problem body");
    assert_eq!(
        parsed,
        json!({
            "type": "about:blank", "title": "Service Unavailable", "status": 503,
            "detail": UNAVAILABLE, "code": "detection_unavailable", "instance": CONFIG_PATH,
        })
    );
}

#[tokio::test]
async fn refusals_answer_before_the_backend_is_contacted() {
    let env = Env::build("refusals", true, None, None);
    let rows: [RefusalRow; 5] = [
        (
            &[ADMIN],
            None,
            "HTTP/1.0 401 Unauthorized",
            ("code".into(), "unauthorized".into()),
        ),
        (
            &[USER, "Remote-Role: viewer"],
            Some("{\"a\":1}"),
            "HTTP/1.0 403 Forbidden",
            ("code".into(), "forbidden".into()),
        ),
        (
            &[USER],
            Some("{\"a\":1}"),
            "HTTP/1.0 403 Forbidden",
            ("code".into(), "forbidden".into()),
        ),
        (
            &[USER, ADMIN],
            None,
            "HTTP/1.0 400 Bad Request",
            ("detail".into(), "a PUT needs a valid Content-Length".into()),
        ),
        (
            &[USER, ADMIN, "Content-Length: 262145"],
            None,
            "HTTP/1.0 400 Bad Request",
            ("detail".into(), "body over the 262144 byte bound".into()),
        ),
    ];
    for (headers, body, expected_status, (member, value)) in rows {
        let (status, _, got_body) = send(&env, "PUT", CONFIG_PATH, headers, body).await;
        assert_eq!(status, expected_status);
        let parsed: Value = serde_json::from_slice(&got_body).expect("body is JSON");
        assert_eq!(parsed[member.as_str()], value, "row {expected_status}");
        assert_eq!(env.backend().connections(), 0, "row {expected_status}");
    }
    let (status, _, got_body) = send(&env, "GET", CONFIG_PATH, &[], None).await;
    assert_eq!(status, "HTTP/1.0 401 Unauthorized");
    assert_eq!(
        got_body,
        br#"{"code":"unauthorized","message":"no identity was supplied"}"#.as_slice()
    );
    env.finish();
}

#[tokio::test]
async fn put_as_admin_relays_if_match_body_and_the_servers_identity_to_the_backend() {
    let env = Env::build("put-relay", true, None, None);
    let answer_body = br#"{"revision":"sha256:bb","restart_required":[]}"#;
    canned(&env, "200 OK", "application/json", &[], answer_body).await;
    let body = "{\"a\":1}";
    let (status, _, got_body) = send(
        &env,
        "PUT",
        CONFIG_PATH,
        &[
            "Remote-User:   door-front  ",
            ADMIN,
            "If-Match: \"sha256:aa\"",
            "Content-Type: application/json",
        ],
        Some(body),
    )
    .await;
    assert_eq!(status, "HTTP/1.0 200 OK");
    assert_eq!(got_body, answer_body.to_vec());
    let seen = env.backend().seen();
    assert_eq!(seen.len(), 1);
    let (line, head, sent_body) = &seen[0];
    assert_eq!(line, "PUT /corvette/api/v1/detection/config HTTP/1.1");
    assert!(head.contains("Remote-User: door-front\r\n"), "head: {head}");
    assert!(
        !head.contains("Remote-Role"),
        "role reached the backend: {head}"
    );
    assert!(
        head.contains("If-Match: \"sha256:aa\"\r\nContent-Type: application/json\r\n"),
        "head: {head}"
    );
    assert_eq!(sent_body, body.as_bytes());
    env.finish();
}

#[tokio::test]
async fn backend_error_bodies_pass_through_byte_for_byte() {
    let env = Env::build("errors", true, None, None);
    let cases: [(&str, &[u8]); 3] = [
        (
            "412 Precondition Failed",
            b"{\"code\":\"stale_revision\",\"detail\":\"stale\",\"revision\":\"sha256:cc\"}",
        ),
        (
            "428 Precondition Required",
            b"{\"code\":\"revision_required\",\"detail\":\"a PUT needs an If-Match revision\"}",
        ),
        (
            "422 Unprocessable Content",
            b"{\"code\":\"invalid_config\",\"errors\":[{\"path\":\"cameras[1].fps\"}]}",
        ),
    ];
    for (status_line, body) in cases {
        canned(&env, status_line, "application/problem+json", &[], body).await;
        let (status, headers, got_body) =
            send(&env, "GET", CONFIG_PATH, &[USER, ADMIN], None).await;
        assert_eq!(status, format!("HTTP/1.0 {status_line}"));
        assert_eq!(
            header(&headers, "content-type"),
            Some("application/problem+json")
        );
        assert_eq!(got_body, body.to_vec(), "body for {status_line}");
    }
    env.finish();
}

#[tokio::test]
async fn reads_relay_and_a_method_mismatch_passes_the_backend_answer_through() {
    let env = Env::build("reads", true, None, None);
    let body = br#"{"cameras":[]}"#;
    canned(
        &env,
        "200 OK",
        "application/json",
        &["ETag: \"sha256:aa\""],
        body,
    )
    .await;
    let (status, headers, got_body) = send(&env, "GET", CONFIG_PATH, &[USER, ADMIN], None).await;
    assert_eq!(status, "HTTP/1.0 200 OK");
    assert_eq!(got_body, body.to_vec());
    assert_eq!(header(&headers, "etag"), Some("\"sha256:aa\""));
    assert_eq!(header(&headers, "content-type"), Some("application/json"));
    assert_eq!(header(&headers, "cache-control"), Some("no-store"));
    canned(&env, "200 OK", "application/json", &[], b"{\"valid\":true}").await;
    let (status, _, got_body) = send(
        &env,
        "POST",
        VALIDATE_PATH,
        &[USER, ADMIN, "Content-Type: application/json"],
        Some("{\"a\":1}"),
    )
    .await;
    assert_eq!(status, "HTTP/1.0 200 OK");
    assert_eq!(got_body, b"{\"valid\":true}".to_vec());
    let seen = env.backend().seen();
    assert_eq!(
        seen[1].0,
        "POST /corvette/api/v1/detection/config/validate HTTP/1.1"
    );
    assert_eq!(seen[1].2, b"{\"a\":1}".to_vec());
    canned(
        &env,
        "200 OK",
        "application/schema+json",
        &[],
        b"{\"$schema\":\"x\"}",
    )
    .await;
    let (status, headers, _) = send(&env, "GET", SCHEMA_PATH, &[USER], None).await;
    assert_eq!(status, "HTTP/1.0 200 OK");
    assert_eq!(
        header(&headers, "content-type"),
        Some("application/schema+json")
    );
    canned(
        &env,
        "405 Method Not Allowed",
        "application/json",
        &["Allow: POST"],
        b"{}",
    )
    .await;
    let (status, headers, _) = send(&env, "GET", VALIDATE_PATH, &[USER], None).await;
    assert_eq!(status, "HTTP/1.0 405 Method Not Allowed");
    assert_eq!(header(&headers, "allow"), Some("POST"));
    env.finish();
}

#[tokio::test]
async fn absent_backend_answers_the_contracts_unavailable_answer() {
    let absent = std::env::temp_dir().join(format!("corvette-absent-{}.sock", std::process::id()));
    let env = Env::build("absent", false, Some(absent), None);
    let (status, headers, body) = send(&env, "GET", CONFIG_PATH, &[USER], None).await;
    assert_unavailable(&status, &headers, &body);
    env.finish();
}

#[tokio::test]
async fn nonsense_stall_and_early_close_are_all_unavailable_and_the_server_survives() {
    let env = Env::build(
        "unusable",
        true,
        None,
        Some((Duration::from_millis(200), Duration::from_millis(300))),
    );
    let truncated = String::from_utf8(http_answer("200 OK", "application/json", &[], b"short"))
        .expect("answer is UTF-8")
        .replace("Content-Length: 5", "Content-Length: 99");
    for answer in [
        Answer::Bytes(b"complete nonsense without any http shape".to_vec()),
        Answer::Bytes(b"HTTP/1.1 200 OK\r\n".to_vec()),
        Answer::Bytes(truncated.clone().into_bytes()),
        Answer::Stall,
    ] {
        env.backend().set(answer).await;
        let (status, headers, body) = send(&env, "GET", CONFIG_PATH, &[USER], None).await;
        assert_unavailable(&status, &headers, &body);
    }
    canned(&env, "200 OK", "application/json", &[], b"{}").await;
    let (status, _, _) = send(&env, "GET", CONFIG_PATH, &[USER], None).await;
    assert_eq!(status, "HTTP/1.0 200 OK", "the server kept serving");
    env.finish();
}
