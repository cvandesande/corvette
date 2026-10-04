//! Integration tests for `GET /corvette/api/v1/cameras` against a fixture
//! configuration server (issue #4 A3 — D3 cache, D4 role rule, I2, I7).

use corvette_api::{CameraEntry, CameraList, ErrorBody};
use corvette_api_server::config::ConfigSource;
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::task::JoinHandle;

/// Frigate's `/api/config` answer, trimmed from the live capture (issue #4 A3).
const FRIGATE_CONFIG: &str = include_str!("fixtures/frigate-config.json");

/// Short enough that the refetch path runs in well under a second, long
/// enough that two immediate requests always land inside one cache window.
const CACHE_TTL: Duration = Duration::from_secs(1);
const EXPIRY_SLEEP: Duration = Duration::from_millis(1400);

/// A running service plus the temp dir to clean up afterwards.
struct TestServer {
    socket: PathBuf,
    dir: PathBuf,
    handle: JoinHandle<()>,
}

impl TestServer {
    fn finish(self) {
        self.handle.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A fresh temp directory path, unique within this test process.
fn unique_dir() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "corvette-cameras-test-{}-{}",
        std::process::id(),
        id
    ))
}

/// Binds a service on a temp socket, reading its configuration from `addr`.
/// The database path names no file: no test here probes it.
fn start_server(config_addr: String) -> TestServer {
    let dir = unique_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let socket = dir.join("api.sock");
    let config = Arc::new(ConfigSource::new(config_addr, CACHE_TTL));
    let server =
        ApiServer::bind(&socket, CONNECTION_TIMEOUT, config, &dir.join("absent.db")).expect("bind");
    let path = server.socket_path().to_path_buf();
    let handle = tokio::spawn(server.serve_until(std::future::pending::<()>()));
    TestServer {
        socket: path,
        dir,
        handle,
    }
}

/// A well-formed `GET` request line followed by the given header lines.
fn get(path: &str, headers: &[&str]) -> Vec<u8> {
    let mut request = format!("GET {path} HTTP/1.0\r\n");
    for header in headers {
        request.push_str(header);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.into_bytes()
}

/// Sends `request` and reads the whole response.
async fn exchange(socket: &Path, request: &[u8]) -> Vec<u8> {
    let mut stream = UnixStream::connect(socket).await.expect("connect");
    stream.write_all(request).await.expect("write");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read to end");
    response
}

/// Splits a raw response into its status line, header map (lower-cased names),
/// and body bytes.
fn http_parts(raw: &[u8]) -> (String, HashMap<String, String>, Vec<u8>) {
    let separator = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("response has no header/body separator");
    let head = &raw[..separator];
    let body = raw[separator + 4..].to_vec();
    let mut lines = std::str::from_utf8(head)
        .expect("response head is ASCII")
        .split("\r\n");
    let status = lines.next().unwrap_or_default().to_owned();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    (status, headers, body)
}

/// Asserts I7's framing on every response: exactly the headers the service
/// promises, and a `Content-Length` that matches the body.
fn assert_framing(headers: &HashMap<String, String>, body: &[u8]) {
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json"),
        "Content-Type must be exactly application/json"
    );
    assert_eq!(
        headers.get("cache-control").map(String::as_str),
        Some("no-store")
    );
    assert_eq!(headers.get("connection").map(String::as_str), Some("close"));
    let declared: usize = headers
        .get("content-length")
        .expect("response carries a Content-Length")
        .parse()
        .expect("Content-Length is a number");
    assert_eq!(declared, body.len(), "Content-Length must match the body");
}

/// Asserts the single error shape (I7) and returns nothing further.
fn assert_error_shape(raw: &[u8], status_prefix: &str, code: &str) {
    let (status, headers, body) = http_parts(raw);
    let actual = status.split_whitespace().nth(1).unwrap_or_default();
    assert_eq!(actual, status_prefix, "status line {status:?}");
    assert_framing(&headers, &body);
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(error.code, code);
}

/// Asserts a `200` camera response and returns the parsed list.
fn assert_camera_list(raw: &[u8]) -> CameraList {
    let (status, headers, body) = http_parts(raw);
    let actual = status.split_whitespace().nth(1).unwrap_or_default();
    assert_eq!(actual, "200", "status line {status:?}");
    assert_framing(&headers, &body);
    serde_json::from_slice(&body).expect("body parses as CameraList")
}

/// How the fixture config server answers a connection.
#[derive(Clone)]
enum Behavior {
    /// Read the request, write these response bytes, close.
    Respond(Vec<u8>),
    /// Read the request, then keep the connection open without answering.
    Stall,
    /// Close at once, as nothing behind the address would.
    Refuse,
}

/// A fixture for Frigate's config HTTP interface: it answers connections with
/// the queued behaviors (the last one repeats), records each request's bytes,
/// and counts answered requests.
struct Fixture {
    behaviors: Mutex<VecDeque<Behavior>>,
    requests: Mutex<Vec<Vec<u8>>>,
}

impl Fixture {
    async fn start(behaviors: Vec<Behavior>) -> (Arc<Self>, String) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fixture server");
        let addr = listener.local_addr().expect("fixture addr").to_string();
        let fixture = Arc::new(Self {
            behaviors: Mutex::new(behaviors.into()),
            requests: Mutex::new(Vec::new()),
        });
        let served = Arc::clone(&fixture);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _peer)) = listener.accept().await else {
                    break;
                };
                let behavior = served.next_behavior();
                let fixture = Arc::clone(&served);
                tokio::spawn(async move {
                    match behavior {
                        Behavior::Refuse => drop(stream),
                        Behavior::Respond(response) => {
                            record(&fixture, read_request(&mut stream).await);
                            let _ = stream.write_all(&response).await;
                            let _ = stream.shutdown().await;
                        }
                        Behavior::Stall => {
                            record(&fixture, read_request(&mut stream).await);
                            tokio::time::sleep(Duration::from_hours(1)).await;
                        }
                    }
                });
            }
        });
        (fixture, addr)
    }

    fn next_behavior(&self) -> Behavior {
        let mut behaviors = self.behaviors.lock().expect("behaviors lock");
        if behaviors.len() > 1 {
            behaviors.pop_front().expect("more than one behavior")
        } else {
            behaviors.front().expect("at least one behavior").clone()
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("requests lock").len()
    }

    fn requests(&self) -> Vec<Vec<u8>> {
        self.requests.lock().expect("requests lock").clone()
    }
}

/// Notes one request the fixture answered, so tests can count fetches.
fn record(fixture: &Fixture, request: Option<Vec<u8>>) {
    if let Some(request) = request {
        fixture
            .requests
            .lock()
            .expect("requests lock")
            .push(request);
    }
}

/// Reads until the request's `\r\n\r\n` terminator; `None` if the peer closes
/// first.
async fn read_request(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 512];
    loop {
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return Some(request);
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return None,
            Ok(read) => request.extend_from_slice(&chunk[..read]),
        }
    }
}

/// The `HTTP/1.0` `200` framing the config source must accept around `body`.
fn config_response(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn admin_request() -> Vec<u8> {
    get(
        "/corvette/api/v1/cameras",
        &["Remote-User: alice", "Remote-Role: admin"],
    )
}

#[tokio::test]
async fn admin_sees_all_enabled_cameras_in_dashboard_order() {
    let (_fixture, addr) =
        Fixture::start(vec![Behavior::Respond(config_response(FRIGATE_CONFIG))]).await;
    let server = start_server(addr);

    let response = exchange(&server.socket, &admin_request()).await;
    let list = assert_camera_list(&response);
    assert_eq!(
        list.cameras,
        vec![
            // Equal `ui.order`, so the name decides: the disabled `living`
            // is absent and `display_name` falls back to the name.
            CameraEntry {
                name: "back".to_owned(),
                display_name: "back".to_owned(),
                order: 0,
                detect_width: 704,
                detect_height: 576,
            },
            CameraEntry {
                name: "front".to_owned(),
                display_name: "front".to_owned(),
                order: 0,
                detect_width: 704,
                detect_height: 576,
            },
        ]
    );
    server.finish();
}

#[tokio::test]
async fn the_fetch_sends_the_documented_config_request() {
    let (fixture, addr) =
        Fixture::start(vec![Behavior::Respond(config_response(FRIGATE_CONFIG))]).await;
    let server = start_server(addr.clone());

    exchange(&server.socket, &admin_request()).await;
    assert_eq!(
        fixture.requests(),
        vec![
            format!("GET /api/config HTTP/1.0\r\nHost: {addr}\r\nX-Cache-Bypass: 1\r\nConnection: close\r\n\r\n").into_bytes()
        ]
    );
    server.finish();
}

/// A config where `keeper` lists one enabled camera, one disabled camera, and
/// one that does not exist.
fn role_filter_fixture() -> String {
    serde_json::json!({
        "auth": { "roles": {
            "admin": [],
            "viewer": [],
            "keeper": ["front", "garage", "living"]
        } },
        "cameras": {
            "front": { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                       "detect": { "width": 704, "height": 576 } },
            "back":  { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                       "detect": { "width": 704, "height": 576 } },
            "living": { "enabled": false, "friendly_name": null, "ui": { "order": 0 },
                        "detect": { "width": 640, "height": 480 } }
        }
    })
    .to_string()
}

#[tokio::test]
async fn the_route_filters_cameras_by_role() {
    let (_fixture, addr) = Fixture::start(vec![Behavior::Respond(config_response(
        &role_filter_fixture(),
    ))])
    .await;
    let server = start_server(addr);

    let cases: &[(&str, &[&str])] = &[
        // Listed and enabled survives; the disabled and unknown names drop.
        ("keeper", &["front"]),
        // An empty role list grants every enabled camera.
        ("viewer", &["back", "front"]),
        // A role absent from `auth.roles` sees nothing (fail closed).
        ("auditor", &[]),
    ];
    for (role, expected) in cases {
        let request = get(
            "/corvette/api/v1/cameras",
            &["Remote-User: alice", &format!("Remote-Role: {role}")],
        );
        let list = assert_camera_list(&exchange(&server.socket, &request).await);
        assert_eq!(
            list.cameras
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            *expected,
            "role {role:?}"
        );
    }

    // No `Remote-Role` header at all is the empty role: also nothing.
    let anonymous_role = get("/corvette/api/v1/cameras", &["Remote-User: alice"]);
    let list = assert_camera_list(&exchange(&server.socket, &anonymous_role).await);
    assert!(list.cameras.is_empty());
    server.finish();
}

#[tokio::test]
async fn a_request_without_identity_is_refused_before_any_fetch() {
    let (fixture, addr) =
        Fixture::start(vec![Behavior::Respond(config_response(FRIGATE_CONFIG))]).await;
    let server = start_server(addr);

    let response = exchange(&server.socket, &get("/corvette/api/v1/cameras", &[])).await;
    assert_error_shape(&response, "401", "unauthorized");
    assert_eq!(
        fixture.request_count(),
        0,
        "the identity gate must run before the config fetch"
    );
    server.finish();
}

#[tokio::test]
async fn cached_config_answers_requests_until_the_ttl_expires() {
    let (fixture, addr) =
        Fixture::start(vec![Behavior::Respond(config_response(FRIGATE_CONFIG))]).await;
    let server = start_server(addr);

    assert_camera_list(&exchange(&server.socket, &admin_request()).await);
    assert_camera_list(&exchange(&server.socket, &admin_request()).await);
    assert_eq!(
        fixture.request_count(),
        1,
        "two requests inside the TTL share one fetch"
    );

    tokio::time::sleep(EXPIRY_SLEEP).await;
    assert_camera_list(&exchange(&server.socket, &admin_request()).await);
    assert_eq!(
        fixture.request_count(),
        2,
        "an expired copy forces a refetch"
    );
    server.finish();
}

#[tokio::test]
async fn an_expired_copy_is_never_served_when_the_refetch_fails() {
    let (fixture, addr) = Fixture::start(vec![
        Behavior::Respond(config_response(FRIGATE_CONFIG)),
        Behavior::Stall,
    ])
    .await;
    let server = start_server(addr);

    assert_camera_list(&exchange(&server.socket, &admin_request()).await);
    tokio::time::sleep(EXPIRY_SLEEP).await;

    // The upstream accepts the refetch and stops answering, so the fetch hits
    // its timeout; the held copy is older than the TTL and must not be served.
    let response = exchange(&server.socket, &admin_request()).await;
    assert_error_shape(&response, "503", "config_unavailable");
    assert_eq!(fixture.request_count(), 2);
    server.finish();
}

#[tokio::test]
async fn no_cameras_without_any_configuration_copy() {
    let (_fixture, addr) = Fixture::start(vec![Behavior::Refuse]).await;
    let server = start_server(addr);

    let response = exchange(&server.socket, &admin_request()).await;
    assert_error_shape(&response, "503", "config_unavailable");
    server.finish();
}

#[tokio::test]
async fn each_unacceptable_upstream_answer_is_unavailable() {
    // The framing-rule cases carry an otherwise-valid config body, so the
    // framing defect is the only reason the answer can be refused.
    let bad_responses: Vec<Vec<u8>> = vec![
        b"HTTP/1.0 500 Internal Server Error\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        format!(
            "HTTP/1.0 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: {}\r\n\r\n{FRIGATE_CONFIG}",
            FRIGATE_CONFIG.len()
        )
        .into_bytes(),
        format!(
            "HTTP/1.0 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n{FRIGATE_CONFIG}",
            FRIGATE_CONFIG.len()
        )
        .into_bytes(),
        format!(
            "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{FRIGATE_CONFIG}",
            FRIGATE_CONFIG.len() + 1
        )
        .into_bytes(),
        b"HTTP/1.0 200 OK\r\nContent-Length: 8\r\n\r\nnope!!??".to_vec(),
    ];
    let (_fixture, addr) =
        Fixture::start(bad_responses.into_iter().map(Behavior::Respond).collect()).await;
    let server = start_server(addr);

    for _ in 0..5 {
        let response = exchange(&server.socket, &admin_request()).await;
        assert_error_shape(&response, "503", "config_unavailable");
    }
    server.finish();
}
