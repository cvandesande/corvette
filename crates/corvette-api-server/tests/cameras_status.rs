//! Tests the camera status route against a fixture backing serving config and a replaceable stats answer.

use corvette_api::ErrorBody;
use corvette_api_server::config::ConfigSource;
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::task::JoinHandle;

/// Long enough that every request in one test shares a single config fetch.
const CACHE_TTL: Duration = Duration::from_mins(5);

const STATUS_PATH: &str = "/corvette/api/v1/cameras/status";

struct TestEnv {
    socket: PathBuf,
    dir: PathBuf,
    handle: JoinHandle<()>,
    /// The request line of every request the backing received.
    backing: Arc<Mutex<Vec<String>>>,
    /// The exact bytes the backing answers a stats request with.
    stats: Arc<Mutex<Vec<u8>>>,
    /// The exact bytes the backing answers `GET /api/config` with.
    config: Arc<Mutex<Vec<u8>>>,
    /// The exact bytes the backing answers a picture request with.
    still: Arc<Mutex<Vec<u8>>>,
    clock: Arc<AtomicU64>,
}

impl TestEnv {
    async fn start(tag: &str) -> Self {
        let stats = Arc::new(Mutex::new(default_stats()));
        let config = Arc::new(Mutex::new(config_response(&fixture_config())));
        let still = Arc::new(Mutex::new(default_still()));
        let (addr, backing) =
            start_backing(Arc::clone(&stats), Arc::clone(&config), Arc::clone(&still)).await;
        Self::build(tag, addr, backing, stats, config, still)
    }

    /// A server whose configuration address accepts no connection at all.
    async fn start_refused_config(tag: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve an address");
        let addr = listener.local_addr().expect("reserved address").to_string();
        drop(listener);
        let stats = Arc::new(Mutex::new(default_stats()));
        let config = Arc::new(Mutex::new(config_response(&fixture_config())));
        let still = Arc::new(Mutex::new(default_still()));
        let backing = Arc::new(Mutex::new(Vec::new()));
        Self::build(tag, addr, backing, stats, config, still)
    }

    fn build(
        tag: &str,
        addr: String,
        backing: Arc<Mutex<Vec<String>>>,
        stats: Arc<Mutex<Vec<u8>>>,
        config: Arc<Mutex<Vec<u8>>>,
        still: Arc<Mutex<Vec<u8>>>,
    ) -> Self {
        let clock = Arc::new(AtomicU64::new(1000));
        let dir = unique_dir(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test directory");
        let socket = dir.join("api.sock");
        let db_path = dir.join("frigate.db");
        let source = Arc::new(ConfigSource::new(addr, CACHE_TTL));
        let server = ApiServer::bind(&socket, CONNECTION_TIMEOUT, source, &db_path)
            .expect("bind")
            .with_clock({
                let clock = Arc::clone(&clock);
                move || Duration::from_secs(clock.load(Ordering::Relaxed)).as_secs_f64()
            });
        let socket = server.socket_path().to_path_buf();
        let handle = tokio::spawn(server.serve_until(std::future::pending::<()>()));
        Self {
            socket,
            dir,
            handle,
            backing,
            stats,
            config,
            still,
            clock,
        }
    }

    fn finish(self) {
        self.handle.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    fn set_stats(&self, cameras: &[(&str, f64)]) {
        let answer = stats_body(cameras).to_string();
        *self.stats.lock().expect("stats cell") = stats_response(answer.as_bytes());
    }

    fn set_stats_raw(&self, raw: &[u8]) {
        *self.stats.lock().expect("stats cell") = raw.to_vec();
    }

    fn set_still_raw(&self, raw: &[u8]) {
        *self.still.lock().expect("still cell") = raw.to_vec();
    }

    fn set_config_raw(&self, raw: &[u8]) {
        *self.config.lock().expect("config cell") = raw.to_vec();
    }

    fn set_clock(&self, seconds: u64) {
        self.clock.store(seconds, Ordering::Relaxed);
    }

    fn stats_request_lines(&self) -> Vec<String> {
        self.backing
            .lock()
            .expect("backing log")
            .iter()
            .filter(|line| line.starts_with("GET /api/stats"))
            .cloned()
            .collect()
    }
}

fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "corvette-cameras-status-test-{}-{tag}-{id}",
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

/// An `admin` request for the status route, as raw bytes.
fn status_request() -> Vec<u8> {
    get(STATUS_PATH, &["Remote-User: alice", "Remote-Role: admin"])
}

/// An `admin` request for one camera's still route, as raw bytes.
fn still_request(camera: &str) -> Vec<u8> {
    get(
        &format!("/corvette/api/v1/cameras/{camera}/still.jpg"),
        &["Remote-User: alice", "Remote-Role: admin"],
    )
}

/// The status code `alice` gets from one camera's still route.
async fn still_code(env: &TestEnv, camera: &str) -> String {
    let raw = exchange(&env.socket, &still_request(camera)).await;
    let (status_line, _headers, _body) = http_parts(&raw);
    status_code(&status_line).to_owned()
}

/// The request as `alice` with the given role; returns the parsed `200` answer.
async fn status(env: &TestEnv, role: &str) -> Value {
    let raw = exchange(
        &env.socket,
        &get(
            STATUS_PATH,
            &["Remote-User: alice", &format!("Remote-Role: {role}")],
        ),
    )
    .await;
    let (status_line, _headers, body) = http_parts(&raw);
    assert_eq!(
        status_code(&status_line),
        "200",
        "status line {status_line:?}"
    );
    serde_json::from_slice(&body).expect("status answer is JSON")
}

fn entry<'a>(answer: &'a Value, name: &str) -> &'a Value {
    answer["cameras"]
        .as_array()
        .expect("cameras array")
        .iter()
        .find(|camera| camera["name"] == name)
        .unwrap_or_else(|| panic!("{name} is absent from {answer}"))
}

/// The backing: answers config, stats and picture requests from its cells, and records every
/// request line.
async fn start_backing(
    stats: Arc<Mutex<Vec<u8>>>,
    config: Arc<Mutex<Vec<u8>>>,
    still: Arc<Mutex<Vec<u8>>>,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind backing fixture");
    let addr = listener.local_addr().expect("backing addr").to_string();
    let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _peer)) = listener.accept().await else {
                break;
            };
            let recorded = Arc::clone(&recorded);
            let stats = Arc::clone(&stats);
            let config = Arc::clone(&config);
            let still = Arc::clone(&still);
            tokio::spawn(async move {
                let request_line = read_request_line(&mut stream).await;
                recorded.lock().expect("record").push(request_line.clone());
                let response = if request_line.starts_with("GET /api/config") {
                    config.lock().expect("config cell").clone()
                } else if request_line.starts_with("GET /api/")
                    && request_line.contains("/latest.jpg")
                {
                    still.lock().expect("still cell").clone()
                } else {
                    stats.lock().expect("stats cell").clone()
                };
                let _ = stream.write_all(&response).await;
                let _ = stream.shutdown().await;
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

fn stats_response(body: &[u8]) -> Vec<u8> {
    let head = format!(
        "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    [head.into_bytes(), body.to_vec()].concat()
}

/// A raw response with the given status line and headers, so a case can declare a
/// head the route must refuse.
fn raw_response(status_line: &str, headers: &[&str], body: &[u8]) -> Vec<u8> {
    let mut head = format!("{status_line}\r\n");
    for header in headers {
        head.push_str(header);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    [head.into_bytes(), body.to_vec()].concat()
}

/// One camera of the stats answer; only the frame rate varies between tests.
fn camera_stats(camera_fps: f64) -> Value {
    json!({
        "camera_fps": camera_fps,
        "process_fps": 5.1,
        "skipped_fps": 0.0,
        "detection_fps": 0.0,
        "detection_enabled": true,
        "pid": 688,
        "capture_pid": 749,
        "ffmpeg_pid": 1158,
        "audio_rms": 0,
        "audio_dBFS": 0
    })
}

fn stats_body(cameras: &[(&str, f64)]) -> Value {
    let entries: serde_json::Map<String, Value> = cameras
        .iter()
        .map(|(name, fps)| ((*name).to_owned(), camera_stats(*fps)))
        .collect();
    json!({ "cameras": Value::Object(entries), "service": { "uptime": 100 } })
}

/// front live, back stalled, living off with frozen frames; back yard absent.
fn default_stats() -> Vec<u8> {
    stats_response(
        stats_body(&[("front", 5.1), ("back", 0.0), ("living", 5.0)])
            .to_string()
            .as_bytes(),
    )
}

/// A healthy picture answer: a `200` whose body starts with the JPEG magic bytes.
fn default_still() -> Vec<u8> {
    raw_response(
        "HTTP/1.0 200 OK",
        &["Content-Type: image/jpeg", "Content-Length: 4"],
        &[0xFF, 0xD8, 0xFF, 0xD9],
    )
}

/// A picture answer that fails, so a test can drive a run of failures.
fn still_failure() -> Vec<u8> {
    raw_response(
        "HTTP/1.0 500 Internal Server Error",
        &["Content-Length: 7"],
        b"failure",
    )
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
async fn status_lists_only_the_role_cameras_with_their_state() {
    let env = TestEnv::start("role-list").await;

    let listed = exchange(
        &env.socket,
        &get(
            "/corvette/api/v1/cameras",
            &["Remote-User: alice", "Remote-Role: admin"],
        ),
    )
    .await;
    let (status_line, _headers, body) = http_parts(&listed);
    assert_eq!(
        status_code(&status_line),
        "200",
        "status line {status_line:?}"
    );
    let listed: Value = serde_json::from_slice(&body).expect("cameras answer is JSON");
    let listed_names: Vec<&Value> = listed["cameras"]
        .as_array()
        .expect("cameras array")
        .iter()
        .map(|camera| &camera["name"])
        .collect();

    let answer = status(&env, "admin").await;
    let names: Vec<&Value> = answer["cameras"]
        .as_array()
        .expect("cameras array")
        .iter()
        .map(|camera| &camera["name"])
        .collect();
    assert_eq!(
        names, listed_names,
        "the names come in the cameras answer's order"
    );

    assert_eq!(
        answer["cameras"],
        json!([
            {"name": "back", "state": "no_picture", "since": 1000.0},
            {"name": "back yard", "state": "no_picture", "since": 1000.0},
            {"name": "front", "state": "live", "since": 1000.0},
            {"name": "living", "state": "off_by_choice", "since": 1000.0}
        ])
    );

    assert_eq!(
        status(&env, "frontonly").await,
        json!({"cameras": [{"name": "front", "state": "live", "since": 1000.0}]})
    );
    assert_eq!(status(&env, "ghost").await, json!({"cameras": []}));

    assert_eq!(
        env.stats_request_lines(),
        [
            "GET /api/stats HTTP/1.0",
            "GET /api/stats HTTP/1.0",
            "GET /api/stats HTTP/1.0"
        ],
        "one stats request per status request"
    );
    env.finish();
}

#[tokio::test]
async fn status_reports_a_switched_off_camera_as_off_by_choice_even_when_the_stats_omit_it() {
    let env = TestEnv::start("off-omitted-from-stats").await;
    env.set_stats(&[("front", 5.1), ("back", 0.0)]);

    let answer = status(&env, "admin").await;
    assert_eq!(entry(&answer, "living")["state"], "off_by_choice");
    env.finish();
}

#[tokio::test]
async fn status_reports_an_enabled_camera_missing_from_the_stats_as_no_picture() {
    let env = TestEnv::start("absent-from-stats").await;
    env.set_stats(&[("front", 5.1)]);

    let answer = status(&env, "admin").await;
    assert_eq!(entry(&answer, "back")["state"], "no_picture");
    assert_eq!(entry(&answer, "back yard")["state"], "no_picture");
    assert_eq!(entry(&answer, "front")["state"], "live");
    env.finish();
}

#[tokio::test]
async fn status_reports_an_enabled_camera_with_a_zero_frame_rate_as_no_picture() {
    let env = TestEnv::start("zero-frame-rate").await;
    env.set_stats(&[("front", 0.0), ("back", 5.1), ("living", 5.0)]);

    let answer = status(&env, "admin").await;
    assert_eq!(entry(&answer, "front")["state"], "no_picture");
    assert_eq!(entry(&answer, "back")["state"], "live");
    env.finish();
}

#[tokio::test]
async fn status_keeps_since_while_a_state_holds_and_moves_it_when_the_state_changes() {
    let env = TestEnv::start("since-moves").await;

    let answer = status(&env, "admin").await;
    for camera in answer["cameras"].as_array().expect("cameras array") {
        assert_eq!(camera["since"], 1000.0);
    }

    env.set_clock(1030);
    let answer = status(&env, "admin").await;
    for camera in answer["cameras"].as_array().expect("cameras array") {
        assert_eq!(camera["since"], 1000.0, "an unchanged state keeps its time");
    }

    env.set_stats(&[("front", 0.0), ("back", 0.0), ("living", 5.0)]);
    env.set_clock(1060);
    let answer = status(&env, "admin").await;
    assert_eq!(
        *entry(&answer, "front"),
        json!({"name": "front", "state": "no_picture", "since": 1060.0})
    );
    assert_eq!(entry(&answer, "back")["since"], 1000.0);
    assert_eq!(entry(&answer, "living")["since"], 1000.0);

    env.set_stats(&[("front", 5.1), ("back", 0.0), ("living", 5.0)]);
    env.set_clock(1090);
    let answer = status(&env, "admin").await;
    assert_eq!(
        *entry(&answer, "front"),
        json!({"name": "front", "state": "live", "since": 1090.0})
    );
    env.finish();
}

#[tokio::test]
async fn status_answers_config_unavailable_without_asking_the_backing_for_stats() {
    let env = TestEnv::start("config-500").await;
    env.set_config_raw(&raw_response(
        "HTTP/1.0 500 Internal Server Error",
        &["Content-Length: 7"],
        b"failure",
    ));

    let raw = exchange(&env.socket, &status_request()).await;
    assert_error_shape(&raw, "503", "config_unavailable");
    assert!(
        env.stats_request_lines().is_empty(),
        "no stats request may be made: {:?}",
        env.stats_request_lines()
    );
    env.finish();

    let refused = TestEnv::start_refused_config("config-refused").await;
    let raw = exchange(&refused.socket, &status_request()).await;
    assert_error_shape(&raw, "503", "config_unavailable");
    // stats come from the configuration address, which accepted no connection.
    assert!(refused.stats_request_lines().is_empty());
    refused.finish();
}

#[tokio::test]
async fn status_answers_status_unavailable_for_every_unusable_stats_answer() {
    const BACKING_SECRET: &str = "BACKING_SECRET";
    let cases: [(&str, Vec<u8>); 11] = [
        (
            "500 with a body",
            raw_response(
                "HTTP/1.0 500 Internal Server Error",
                &["Content-Length: 14"],
                BACKING_SECRET.as_bytes(),
            ),
        ),
        (
            "404 with a body",
            raw_response(
                "HTTP/1.0 404 Not Found",
                &["Content-Length: 14"],
                BACKING_SECRET.as_bytes(),
            ),
        ),
        (
            "200 that is not JSON",
            stats_response(BACKING_SECRET.as_bytes()),
        ),
        (
            "JSON without a cameras member",
            stats_response(br#"{"service":{}}"#),
        ),
        (
            "cameras is not an object",
            stats_response(br#"{"cameras":[]}"#),
        ),
        (
            "a camera without a numeric camera_fps",
            stats_response(br#"{"cameras":{"front":{"camera_fps":"5.1"}}}"#),
        ),
        (
            "a camera missing camera_fps",
            stats_response(br#"{"cameras":{"front":{}}}"#),
        ),
        (
            "body shorter than Content-Length",
            raw_response(
                "HTTP/1.0 200 OK",
                &["Content-Length: 5000"],
                br#"{"cameras":{"front":{"camera_fps":5.1}}}"#,
            ),
        ),
        (
            "transfer-encoded body that starts like a stats body",
            raw_response(
                "HTTP/1.0 200 OK",
                &["Transfer-Encoding: chunked"],
                br#"{"cameras":{"front":{"camera_fps":5.1}}}"#,
            ),
        ),
        ("connection closed without an answer", Vec::new()),
        ("stats answer over the 4 MiB limit", {
            let prefix = br#"{"cameras":{"front":{"camera_fps":5.1}},"pad":""#;
            let pad = vec![b'a'; 4 * 1024 * 1024];
            let body = [prefix.as_slice(), pad.as_slice(), b"\"}"].concat();
            stats_response(&body)
        }),
    ];
    for (index, (label, stats)) in cases.into_iter().enumerate() {
        let env = TestEnv::start(&format!("case-{index}")).await;
        env.set_stats_raw(&stats);

        let raw = exchange(&env.socket, &status_request()).await;
        assert_error_shape(&raw, "503", "status_unavailable");
        let (_status, _headers, body) = http_parts(&raw);
        let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
        assert_eq!(
            error.message, "The camera status could not be read.",
            "case {label}"
        );
        assert!(
            !String::from_utf8_lossy(&raw).contains(BACKING_SECRET),
            "case {label}: the backing's own text reached the answer"
        );
        assert_eq!(
            env.stats_request_lines(),
            ["GET /api/stats HTTP/1.0"],
            "case {label}: one stats request"
        );
        env.finish();
    }
}

#[tokio::test]
async fn status_reports_stale_when_a_live_cameras_stills_have_failed_for_over_a_minute() {
    let env = TestEnv::start("stale-after-a-minute").await;
    env.set_still_raw(&still_failure());

    let raw = exchange(&env.socket, &still_request("front")).await;
    assert_error_shape(&raw, "503", "picture_unavailable");
    let answer = status(&env, "admin").await;
    assert_eq!(
        *entry(&answer, "front"),
        json!({"name": "front", "state": "live", "since": 1000.0})
    );

    env.set_clock(1030);
    assert_eq!(still_code(&env, "front").await, "503");
    let answer = status(&env, "admin").await;
    assert_eq!(entry(&answer, "front")["state"], "live");

    env.set_clock(1060);
    assert_eq!(still_code(&env, "front").await, "503");
    // The run began at 1000, so at 1060 it is exactly 60 s, not over a minute.
    let answer = status(&env, "admin").await;
    assert_eq!(entry(&answer, "front")["state"], "live");

    env.set_clock(1070);
    assert_eq!(still_code(&env, "front").await, "503");
    let answer = status(&env, "admin").await;
    assert_eq!(
        *entry(&answer, "front"),
        json!({"name": "front", "state": "stale", "since": 1070.0})
    );
    assert_eq!(
        *entry(&answer, "back"),
        json!({"name": "back", "state": "no_picture", "since": 1000.0})
    );
    env.finish();
}

#[tokio::test]
async fn status_returns_a_stale_camera_to_live_when_a_still_is_delivered() {
    let env = TestEnv::start("stale-to-live").await;
    env.set_still_raw(&still_failure());
    for seconds in [1000, 1030, 1060, 1070] {
        env.set_clock(seconds);
        assert_eq!(still_code(&env, "front").await, "503");
    }
    let answer = status(&env, "admin").await;
    assert_eq!(entry(&answer, "front")["state"], "stale");

    env.set_still_raw(&default_still());
    env.set_clock(1080);
    assert_eq!(still_code(&env, "front").await, "200");
    let answer = status(&env, "admin").await;
    assert_eq!(
        *entry(&answer, "front"),
        json!({"name": "front", "state": "live", "since": 1080.0})
    );
    env.finish();
}

#[tokio::test]
async fn status_reports_stale_for_the_camera_whose_stills_failed_and_no_other() {
    let env = TestEnv::start("stale-attribution").await;
    env.set_stats(&[("front", 5.1), ("back", 5.1), ("living", 5.0)]);
    env.set_still_raw(&still_failure());

    assert_eq!(still_code(&env, "back").await, "503");
    env.set_clock(1070);
    assert_eq!(still_code(&env, "back").await, "503");

    let answer = status(&env, "admin").await;
    assert_eq!(
        entry(&answer, "back")["state"],
        "stale",
        "back's own failures made it stale"
    );
    assert_eq!(
        entry(&answer, "front")["state"],
        "live",
        "front never fetched a picture"
    );

    env.set_still_raw(&default_still());
    env.set_clock(1080);
    assert_eq!(still_code(&env, "front").await, "200");
    let answer = status(&env, "admin").await;
    assert_eq!(
        entry(&answer, "back")["state"],
        "stale",
        "back's run outlives front's delivery"
    );
    assert_eq!(
        entry(&answer, "front")["state"],
        "live",
        "front delivered, its state is live"
    );
    env.finish();
}

#[tokio::test]
async fn status_stops_reporting_stale_when_the_last_failure_is_over_a_minute_old() {
    let env = TestEnv::start("stale-evidence-ages-out").await;
    env.set_still_raw(&still_failure());
    for seconds in [1000, 1070] {
        env.set_clock(seconds);
        assert_eq!(still_code(&env, "front").await, "503");
    }
    let answer = status(&env, "admin").await;
    assert_eq!(entry(&answer, "front")["state"], "stale");

    // At 1130 the last failure is exactly 60 s old, so the evidence still counts.
    env.set_clock(1130);
    let answer = status(&env, "admin").await;
    assert_eq!(entry(&answer, "front")["state"], "stale");
    env.set_clock(1131);
    let answer = status(&env, "admin").await;
    assert_eq!(entry(&answer, "front")["state"], "live");
    env.finish();
}

#[tokio::test]
async fn status_does_not_report_stale_for_a_first_failure_after_a_long_quiet_time() {
    let env = TestEnv::start("first-failure-after-quiet").await;
    assert_eq!(still_code(&env, "front").await, "200");
    env.set_still_raw(&still_failure());
    env.set_clock(5000);
    assert_eq!(still_code(&env, "front").await, "503");
    let answer = status(&env, "admin").await;
    assert_eq!(entry(&answer, "front")["state"], "live");
    env.finish();
}

#[tokio::test]
async fn status_never_reports_stale_for_a_camera_that_is_off_or_has_no_picture_and_tracks_each_camera_alone()
 {
    let env = TestEnv::start("stale-per-camera").await;
    env.set_stats(&[("front", 5.1), ("back", 5.1), ("living", 5.0)]);
    env.set_still_raw(&still_failure());

    for camera in ["front", "back", "back%20yard"] {
        assert_eq!(still_code(&env, camera).await, "503");
    }

    env.set_clock(1070);
    for camera in ["front", "back%20yard"] {
        assert_eq!(still_code(&env, camera).await, "503");
    }
    env.set_still_raw(&default_still());
    assert_eq!(still_code(&env, "back").await, "200");
    let off = exchange(&env.socket, &still_request("living")).await;
    assert_error_shape(&off, "404", "no_picture");

    let answer = status(&env, "admin").await;
    assert_eq!(entry(&answer, "front")["state"], "stale");
    assert_eq!(entry(&answer, "back")["state"], "live");
    assert_eq!(entry(&answer, "living")["state"], "off_by_choice");
    assert_eq!(entry(&answer, "back yard")["state"], "no_picture");
    env.finish();
}
