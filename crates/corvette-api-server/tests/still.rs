//! Tests the still route against a fixture backing serving config and picture requests.

use corvette_api::ErrorBody;
use corvette_api_server::config::ConfigSource;
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::task::JoinHandle;

/// Long enough that every request in one test shares a single config fetch.
const CACHE_TTL: Duration = Duration::from_mins(5);

/// JPEG magic, printable bytes, and one `0D 0A 0D 0A` sequence inside the body.
const FAKE_JPEG: &[u8] = b"\xFF\xD8\xFF\xE0 fake jpeg body \x0D\x0A\x0D\x0A tail";

/// Text the backing puts in unusable answers; it must never reach the client.
const BACKING_SECRET: &str = "backing-secret-detail";

/// What the backing does with a picture request, after recording its request line.
enum Picture {
    Answer(Vec<u8>),
    CloseWithoutAnswer,
    /// Holds the stream open long enough for the server's own fetch bound to end.
    NeverAnswer,
}

struct TestEnv {
    socket: PathBuf,
    dir: PathBuf,
    handle: JoinHandle<()>,
    /// The request line of every request the backing received.
    backing: Arc<Mutex<Vec<String>>>,
}

impl TestEnv {
    async fn start(tag: &str) -> Self {
        Self::start_with(tag, Picture::Answer(still_response(FAKE_JPEG))).await
    }

    async fn start_with(tag: &str, picture: Picture) -> Self {
        let (addr, backing) = start_backing(picture).await;
        Self::serve(tag, addr, backing)
    }

    /// Points the configuration source at an address nothing listens on.
    async fn start_refusing_config(tag: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve a dead config address");
        let addr = listener.local_addr().expect("dead config addr").to_string();
        drop(listener);
        Self::serve(tag, addr, Arc::new(Mutex::new(Vec::new())))
    }

    fn serve(tag: &str, addr: String, backing: Arc<Mutex<Vec<String>>>) -> Self {
        let dir = unique_dir(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test directory");
        let socket = dir.join("api.sock");
        let db_path = dir.join("frigate.db");
        let config = Arc::new(ConfigSource::new(addr, CACHE_TTL));
        let server = ApiServer::bind(&socket, CONNECTION_TIMEOUT, config, &db_path).expect("bind");
        let socket = server.socket_path().to_path_buf();
        let handle = tokio::spawn(server.serve_until(std::future::pending::<()>()));
        Self {
            socket,
            dir,
            handle,
            backing,
        }
    }

    fn finish(self) {
        self.handle.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    async fn still(&self, role: &str, path: &str) -> Vec<u8> {
        exchange(
            &self.socket,
            &get(
                path,
                &["Remote-User: alice", &format!("Remote-Role: {role}")],
            ),
        )
        .await
    }

    /// The recorded backing request lines other than the config fetch.
    fn still_request_lines(&self) -> Vec<String> {
        self.backing_lines()
            .into_iter()
            .filter(|line| !line.starts_with("GET /api/config"))
            .collect()
    }

    fn backing_lines(&self) -> Vec<String> {
        self.backing.lock().expect("backing log").clone()
    }
}

fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "corvette-still-test-{}-{tag}-{id}",
        std::process::id()
    ))
}

fn get(path: &str, headers: &[&str]) -> Vec<u8> {
    let mut request = format!("GET {path} HTTP/1.0\r\n");
    for header in headers {
        request.push_str(header);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.into_bytes()
}

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

fn status_code(status: &str) -> &str {
    status.split_whitespace().nth(1).unwrap_or_default()
}

fn assert_framing(headers: &HashMap<String, String>, content_type: &str, body: &[u8]) {
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some(content_type),
        "Content-Type must be exactly {content_type}"
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

fn assert_error_shape(raw: &[u8], status_prefix: &str, code: &str) {
    let (status, headers, body) = http_parts(raw);
    assert_eq!(
        status_code(&status),
        status_prefix,
        "status line {status:?}"
    );
    assert_framing(&headers, "application/json", &body);
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(error.code, code);
}

/// The backing: always answers config requests, answers each picture request per
/// `picture`, and records every request line before answering.
async fn start_backing(picture: Picture) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind backing fixture");
    let addr = listener.local_addr().expect("backing addr").to_string();
    let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    let picture = Arc::new(picture);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _peer)) = listener.accept().await else {
                break;
            };
            let recorded = Arc::clone(&recorded);
            let picture = Arc::clone(&picture);
            tokio::spawn(async move {
                let request_line = read_request_line(&mut stream).await;
                recorded.lock().expect("record").push(request_line.clone());
                if request_line.starts_with("GET /api/config") {
                    let response = config_response(&fixture_config());
                    let _ = stream.write_all(&response).await;
                    let _ = stream.shutdown().await;
                    return;
                }
                match &*picture {
                    Picture::Answer(response) => {
                        let _ = stream.write_all(response).await;
                        let _ = stream.shutdown().await;
                    }
                    Picture::CloseWithoutAnswer => {}
                    Picture::NeverAnswer => {
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        let _ = stream.shutdown().await;
                    }
                }
            });
        }
    });
    (addr, requests)
}

async fn read_request_line(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 512];
    loop {
        if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = &request[..end];
            let line_end = head
                .windows(2)
                .position(|window| window == b"\r\n")
                .unwrap_or(head.len());
            return String::from_utf8_lossy(&head[..line_end]).into_owned();
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return String::from_utf8_lossy(&request).into_owned(),
            Ok(read) => request.extend_from_slice(&chunk[..read]),
        }
    }
}

fn config_response(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn still_response(body: &[u8]) -> Vec<u8> {
    let head = format!(
        "HTTP/1.0 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    [head.into_bytes(), body.to_vec()].concat()
}

fn still_head(content_length: usize) -> Vec<u8> {
    format!(
        "HTTP/1.0 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {content_length}\r\n\r\n"
    )
    .into_bytes()
}

/// A `200` `image/jpeg` response whose whole length (head plus JPEG-headed body) is
/// exactly `total` bytes. The digit count of the body length is stable near 1 MiB, so
/// the fixed point on the head length converges after one correction.
fn response_of_exact_total_length(total: usize) -> Vec<u8> {
    let mut body_len = total - still_head(0).len();
    let head = loop {
        let candidate = still_head(body_len);
        if candidate.len() == total - body_len {
            break candidate;
        }
        body_len = total - candidate.len();
    };
    let mut body = vec![b'.'; total - head.len()];
    body[..3].copy_from_slice(&[0xFF, 0xD8, 0xFF]);
    [head, body].concat()
}

fn backing_error_response(status_line: &str) -> Vec<u8> {
    format!(
        "HTTP/1.0 {status_line}\r\nContent-Length: {}\r\n\r\n{BACKING_SECRET}",
        BACKING_SECRET.len()
    )
    .into_bytes()
}

/// Enables `front`, `back` and `back yard`; disables `living`; `ghost` names no camera.
fn fixture_config() -> String {
    serde_json::json!({
        "auth": { "roles": {
            "admin": [],
            "frontonly": ["front"]
        } },
        "cameras": {
            "front":     { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                           "detect": { "width": 704, "height": 576 } },
            "back":      { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                           "detect": { "width": 704, "height": 576 } },
            "back yard": { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                           "detect": { "width": 704, "height": 576 } },
            "living":    { "enabled": false, "friendly_name": null, "ui": { "order": 0 },
                           "detect": { "width": 640, "height": 480 } }
        }
    })
    .to_string()
}

#[tokio::test]
async fn still_proxies_the_backing_picture_with_the_default_height() {
    let env = TestEnv::start("default-height").await;

    let raw = env
        .still("admin", "/corvette/api/v1/cameras/front/still.jpg")
        .await;
    let (status, headers, body) = http_parts(&raw);
    assert_eq!(status_code(&status), "200", "status line {status:?}");
    assert_framing(&headers, "image/jpeg", &body);
    assert_eq!(body, FAKE_JPEG, "the backing's bytes must arrive unchanged");
    assert_eq!(
        env.still_request_lines(),
        ["GET /api/front/latest.jpg?height=270 HTTP/1.0"]
    );
    env.finish();
}

#[tokio::test]
async fn still_passes_the_requested_height_to_the_backing() {
    let env = TestEnv::start("heights").await;

    env.still(
        "admin",
        "/corvette/api/v1/cameras/front/still.jpg?height=120",
    )
    .await;
    env.still(
        "admin",
        "/corvette/api/v1/cameras/front/still.jpg?height=576",
    )
    .await;
    assert_eq!(
        env.still_request_lines(),
        [
            "GET /api/front/latest.jpg?height=120 HTTP/1.0",
            "GET /api/front/latest.jpg?height=576 HTTP/1.0",
        ]
    );
    env.finish();
}

#[tokio::test]
async fn still_of_a_camera_outside_the_role_answers_not_found() {
    let env = TestEnv::start("outside-role").await;

    let raw = env
        .still("frontonly", "/corvette/api/v1/cameras/back/still.jpg")
        .await;
    assert_error_shape(&raw, "404", "not_found");
    assert!(
        env.still_request_lines().is_empty(),
        "a refused camera must not reach the backing"
    );
    env.finish();
}

#[tokio::test]
async fn still_answers_the_same_body_for_an_unknown_camera_and_a_camera_outside_the_role() {
    let env = TestEnv::start("same-404").await;

    let outside_role = env
        .still("frontonly", "/corvette/api/v1/cameras/back/still.jpg")
        .await;
    let unknown_camera = env
        .still("frontonly", "/corvette/api/v1/cameras/ghost/still.jpg")
        .await;

    let (outside_status, _, outside_body) = http_parts(&outside_role);
    let (unknown_status, _, unknown_body) = http_parts(&unknown_camera);
    assert_eq!(
        unknown_status, outside_status,
        "both answers share the status line"
    );
    assert_eq!(
        unknown_body, outside_body,
        "both answers share the body bytes"
    );
    env.finish();
}

#[tokio::test]
async fn still_of_a_camera_that_is_off_answers_no_picture_without_a_backing_call() {
    let env = TestEnv::start("off-camera").await;

    let raw = env
        .still("admin", "/corvette/api/v1/cameras/living/still.jpg")
        .await;
    assert_error_shape(&raw, "404", "no_picture");
    assert!(
        env.still_request_lines().is_empty(),
        "a camera that is off must not be fetched"
    );
    env.finish();
}

#[tokio::test]
async fn still_percent_encodes_the_camera_name_for_the_backing() {
    let env = TestEnv::start("encoded-name").await;

    env.still("admin", "/corvette/api/v1/cameras/back%20yard/still.jpg")
        .await;
    assert_eq!(
        env.still_request_lines(),
        ["GET /api/back%20yard/latest.jpg?height=270 HTTP/1.0"]
    );
    env.finish();
}

#[tokio::test]
async fn still_requires_an_identity() {
    let env = TestEnv::start("no-identity").await;

    for headers in [
        &["Remote-Role: admin"][..],
        &["Remote-User:", "Remote-Role: admin"][..],
    ] {
        let raw = exchange(
            &env.socket,
            &get("/corvette/api/v1/cameras/front/still.jpg", headers),
        )
        .await;
        assert_error_shape(&raw, "401", "unauthorized");
    }
    assert!(
        env.backing_lines().is_empty(),
        "an unauthorized still must not reach the backing at all"
    );
    env.finish();
}

#[tokio::test]
async fn still_refuses_a_malformed_height_before_any_backing_request() {
    let env = TestEnv::start("bad-height").await;

    for query in [
        "height=0",
        "height=577",
        "height=abc",
        "height=",
        "height=+5",
        "height=-1",
        "height=1.5",
        "height=99999999999999999999",
    ] {
        let raw = env
            .still(
                "admin",
                &format!("/corvette/api/v1/cameras/front/still.jpg?{query}"),
            )
            .await;
        assert_error_shape(&raw, "400", "bad_request");
        let (_, _, body) = http_parts(&raw);
        let error: ErrorBody = serde_json::from_slice(&body).expect("error body");
        assert_eq!(
            error.message, "`height` must be a whole number from 1 to 576",
            "query {query:?}"
        );
    }
    for query in [
        "width=1",
        "height=5&height=6",
        "height",
        "height=5&",
        "&height=5",
    ] {
        let raw = env
            .still(
                "admin",
                &format!("/corvette/api/v1/cameras/front/still.jpg?{query}"),
            )
            .await;
        assert_error_shape(&raw, "400", "bad_request");
        let (_, _, body) = http_parts(&raw);
        let error: ErrorBody = serde_json::from_slice(&body).expect("error body");
        assert!(
            !error.message.is_empty(),
            "query {query:?} must name a reason"
        );
    }
    assert!(
        env.backing_lines().is_empty(),
        "a refused height must not reach the backing at all: {:?}",
        env.backing_lines()
    );
    env.finish();
}

#[tokio::test]
async fn still_refuses_a_malformed_camera_escape_before_any_backing_request() {
    let env = TestEnv::start("bad-escape").await;

    for segment in ["bad%2", "%zz", "%ff"] {
        let raw = env
            .still(
                "admin",
                &format!("/corvette/api/v1/cameras/{segment}/still.jpg"),
            )
            .await;
        assert_error_shape(&raw, "400", "bad_request");
        assert!(
            env.backing_lines().is_empty(),
            "escape {segment:?} must not reach the backing: {:?}",
            env.backing_lines()
        );
    }
    env.finish();
}

#[tokio::test]
async fn still_answers_config_unavailable_when_no_configuration_can_be_fetched() {
    let env = TestEnv::start_refusing_config("config-unavailable").await;

    let raw = env
        .still("admin", "/corvette/api/v1/cameras/front/still.jpg")
        .await;
    assert_error_shape(&raw, "503", "config_unavailable");
    env.finish();
}

#[tokio::test]
async fn still_answers_picture_unavailable_for_every_unusable_backing_answer() {
    let mut transfer_encoded =
        b"HTTP/1.0 200 OK\r\nContent-Type: image/jpeg\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_vec();
    transfer_encoded.extend_from_slice(FAKE_JPEG);
    let content_length = format!("Content-Length: {}\r\n\r\n", FAKE_JPEG.len());
    let mut content_encoded =
        b"HTTP/1.0 200 OK\r\nContent-Type: image/jpeg\r\nContent-Encoding: gzip\r\n".to_vec();
    content_encoded.extend_from_slice(content_length.as_bytes());
    content_encoded.extend_from_slice(FAKE_JPEG);
    let truncated_head =
        "HTTP/1.0 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: 5000\r\n\r\n";
    let truncated = [truncated_head.as_bytes(), FAKE_JPEG].concat();

    let cases = [
        (
            "500 with a body",
            Picture::Answer(backing_error_response("500 Internal Server Error")),
        ),
        (
            "404 with a body",
            Picture::Answer(backing_error_response("404 Not Found")),
        ),
        (
            "200 that is not a JPEG",
            Picture::Answer(still_response(
                format!("{BACKING_SECRET} is not a picture").as_bytes(),
            )),
        ),
        (
            "body shorter than Content-Length",
            Picture::Answer(truncated),
        ),
        (
            "transfer-encoded body that starts like a JPEG",
            Picture::Answer(transfer_encoded),
        ),
        (
            "content-encoded body that starts like a JPEG",
            Picture::Answer(content_encoded),
        ),
        (
            "response over the 1 MiB bound",
            Picture::Answer(response_of_exact_total_length(1024 * 1024 + 1)),
        ),
        (
            "connection closed without an answer",
            Picture::CloseWithoutAnswer,
        ),
    ];
    for (label, picture) in cases {
        let env = TestEnv::start_with(label, picture).await;
        let raw = env
            .still("admin", "/corvette/api/v1/cameras/front/still.jpg")
            .await;
        let (status, _, body) = http_parts(&raw);
        assert_eq!(
            status_code(&status),
            "503",
            "{label}: status line {status:?}"
        );
        assert_error_shape(&raw, "503", "picture_unavailable");
        let error: ErrorBody = serde_json::from_slice(&body).expect("error body");
        assert_eq!(
            error.message, "The camera's picture could not be fetched.",
            "{label} must answer the fixed message"
        );
        assert!(
            !String::from_utf8_lossy(&raw).contains(BACKING_SECRET),
            "{label}: the backing's own body must never be returned"
        );
        assert_eq!(
            env.still_request_lines(),
            ["GET /api/front/latest.jpg?height=270 HTTP/1.0"],
            "{label}: the refusal came from the backing's answer, not an earlier check"
        );
        env.finish();
    }
}

#[tokio::test]
async fn still_answers_picture_unavailable_when_the_backing_never_answers() {
    let env = TestEnv::start_with("never-answers", Picture::NeverAnswer).await;

    let started = std::time::Instant::now();
    let raw = env
        .still("admin", "/corvette/api/v1/cameras/front/still.jpg")
        .await;
    let elapsed = started.elapsed();

    assert_error_shape(&raw, "503", "picture_unavailable");
    assert!(
        elapsed < CONNECTION_TIMEOUT,
        "the answer must arrive before the connection bound, took {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(1500),
        "the fetch bound, not an early refusal, ended the wait: {elapsed:?}"
    );
    env.finish();
}

#[tokio::test]
async fn still_accepts_a_response_of_exactly_the_one_mebibyte_bound() {
    let picture = response_of_exact_total_length(1024 * 1024);
    let (_, _, fixture_body) = http_parts(&picture);
    let env = TestEnv::start_with("one-mebibyte", Picture::Answer(picture)).await;

    let raw = env
        .still("admin", "/corvette/api/v1/cameras/front/still.jpg")
        .await;
    let (status, headers, body) = http_parts(&raw);
    assert_eq!(status_code(&status), "200", "status line {status:?}");
    assert_framing(&headers, "image/jpeg", &body);
    assert_eq!(
        body, fixture_body,
        "the picture bytes must arrive unchanged"
    );
    env.finish();
}
