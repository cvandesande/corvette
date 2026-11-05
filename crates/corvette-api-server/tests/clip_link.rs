//! Tests the clip-link route against a fixture config server and a Frigate-shaped database.

use corvette_api::ErrorBody;
use corvette_api_server::config::ConfigSource;
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::task::JoinHandle;

/// Long enough that every request in one test shares a single config fetch.
const CACHE_TTL: Duration = Duration::from_mins(5);

/// Frigate's own DDL for the table the footage check reads.
const RECORDINGS_DDL: &str = r#"CREATE TABLE "recordings" ("id" VARCHAR(30) NOT NULL PRIMARY KEY, "camera" VARCHAR(20) NOT NULL, "path" VARCHAR(255) NOT NULL, "start_time" DATETIME NOT NULL, "end_time" DATETIME NOT NULL, "duration" REAL NOT NULL, "motion" INTEGER)"#;

struct TestEnv {
    socket: PathBuf,
    dir: PathBuf,
    db_path: PathBuf,
    handle: JoinHandle<()>,
}

impl TestEnv {
    async fn start(tag: &str) -> Self {
        Self::with_config(tag, Some(config_response(&fixture_config()))).await
    }

    /// A config address where nothing answers, as a stopped backing would.
    async fn refusing(tag: &str) -> Self {
        Self::with_config(tag, None).await
    }

    async fn with_config(tag: &str, config_response: Option<Vec<u8>>) -> Self {
        let config_addr = Fixture::start(config_response).await;
        let dir = unique_dir(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test directory");
        let socket = dir.join("api.sock");
        let db_path = dir.join("frigate.db");
        let config = Arc::new(ConfigSource::new(config_addr, CACHE_TTL));
        let server = ApiServer::bind(&socket, CONNECTION_TIMEOUT, config, &db_path).expect("bind");
        let socket = server.socket_path().to_path_buf();
        let handle = tokio::spawn(server.serve_until(std::future::pending::<()>()));
        Self {
            socket,
            dir,
            db_path,
            handle,
        }
    }

    fn finish(self) {
        self.handle.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// Creates Frigate's `recordings` table and inserts `(id, camera, start, end)` rows.
    fn seed(&self, rows: &[(&str, &str, f64, f64)]) {
        let db = Connection::open(&self.db_path).expect("create test database");
        db.execute_batch(RECORDINGS_DDL).expect("create recordings");
        for (id, camera, start, end) in rows {
            db.execute(
                "INSERT INTO recordings \
                 (id, camera, path, start_time, end_time, duration, motion) \
                 VALUES (?, ?, ?, ?, ?, 50.0, NULL)",
                params![id, camera, format!("/{camera}/{start}.mp4"), start, end],
            )
            .expect("insert recording");
        }
    }

    async fn clip_link(&self, role: &str, query: &str) -> Vec<u8> {
        exchange(&self.socket, &clip_link_request(role, query)).await
    }
}

fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "corvette-clip-link-test-{}-{tag}-{id}",
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

fn clip_link_request(role: &str, query: &str) -> Vec<u8> {
    get(
        &format!("/corvette/api/v1/clip-link?{query}"),
        &["Remote-User: alice", &format!("Remote-Role: {role}")],
    )
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

fn assert_playlist(raw: &[u8], url: &str) {
    let (status, headers, body) = http_parts(raw);
    assert_eq!(status_code(&status), "200", "status line {status:?}");
    assert_framing(&headers, &body);
    assert_eq!(
        String::from_utf8_lossy(&body),
        format!(r#"{{"url":"{url}"}}"#),
        "body must be the playlist address and nothing else"
    );
}

/// A table row's answer, with the query named in every failure message.
fn assert_bad_request(raw: &[u8], query: &str) {
    let (status, headers, body) = http_parts(raw);
    assert_eq!(
        status_code(&status),
        "400",
        "query {query:?} answered {status:?}"
    );
    assert_framing(&headers, &body);
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(
        error.code, "bad_request",
        "query {query:?} answered {error:?}"
    );
}

fn assert_error_shape(raw: &[u8], status_prefix: &str, code: &str) {
    let (status, headers, body) = http_parts(raw);
    assert_eq!(
        status_code(&status),
        status_prefix,
        "status line {status:?}"
    );
    assert_framing(&headers, &body);
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(error.code, code);
}

/// A config server: `Some` answers every fetch, `None` closes each connection at once.
struct Fixture;

impl Fixture {
    async fn start(response: Option<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fixture server");
        let addr = listener.local_addr().expect("fixture addr").to_string();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _peer)) = listener.accept().await else {
                    break;
                };
                let response = response.clone();
                tokio::spawn(async move {
                    let Some(response) = response else {
                        drop(stream);
                        return;
                    };
                    read_request(&mut stream).await;
                    let _ = stream.write_all(&response).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        addr
    }
}

async fn read_request(stream: &mut TcpStream) {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 512];
    loop {
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return;
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
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
async fn clip_link_answers_the_playlist_address_for_a_recorded_range() {
    let env = TestEnv::start("playlist").await;
    env.seed(&[("r1", "front", 1_000.0, 1_500.0)]);

    let raw = env
        .clip_link("admin", "camera=front&start=1100&end=1400")
        .await;
    assert_playlist(
        &raw,
        "/corvette/api/v1/media/front/start/1100/end/1400/master.m3u8",
    );
    env.finish();
}

#[tokio::test]
async fn clip_link_percent_encodes_the_camera_name_in_the_address() {
    let env = TestEnv::start("space-name").await;
    env.seed(&[("r1", "back yard", 1_000.0, 1_500.0)]);

    let raw = env
        .clip_link("admin", "camera=back%20yard&start=1100&end=1400")
        .await;
    assert_playlist(
        &raw,
        "/corvette/api/v1/media/back%20yard/start/1100/end/1400/master.m3u8",
    );
    env.finish();
}

#[tokio::test]
async fn clip_link_for_a_camera_outside_the_role_answers_not_found() {
    let env = TestEnv::start("outside-role").await;
    env.seed(&[("r1", "back", 1_000.0, 1_500.0)]);

    let raw = env
        .clip_link("frontonly", "camera=back&start=1100&end=1400")
        .await;
    assert_error_shape(&raw, "404", "not_found");
    env.finish();
}

#[tokio::test]
async fn clip_link_answers_the_same_body_for_every_not_found_case() {
    let env = TestEnv::start("same-not-found").await;
    env.seed(&[
        ("r1", "front", 1_000.0, 1_500.0),
        ("r2", "back", 1_000.0, 1_500.0),
    ]);

    let unknown_camera = env
        .clip_link("admin", "camera=ghost&start=1100&end=1400")
        .await;
    let outside_role = env
        .clip_link("frontonly", "camera=back&start=1100&end=1400")
        .await;
    let no_footage_in_range = env
        .clip_link("admin", "camera=front&start=5000&end=5100")
        .await;

    let mut first: Option<(String, Vec<u8>)> = None;
    for raw in [&unknown_camera, &outside_role, &no_footage_in_range] {
        let (status, headers, body) = http_parts(raw);
        assert_eq!(status_code(&status), "404", "status line {status:?}");
        assert_framing(&headers, &body);
        let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
        assert_eq!(error.code, "not_found");
        match &first {
            Some((first_status, first_body)) => {
                assert_eq!(status, *first_status);
                assert_eq!(body, *first_body);
            }
            None => first = Some((status, body)),
        }
    }
    env.finish();
}

#[tokio::test]
async fn clip_link_overlap_is_half_open_at_both_ends() {
    let env = TestEnv::start("half-open").await;
    env.seed(&[("r1", "front", 1_000.0, 1_500.0)]);

    for query in [
        "camera=front&start=1500&end=1600",
        "camera=front&start=900&end=1000",
    ] {
        let raw = env.clip_link("admin", query).await;
        assert_error_shape(&raw, "404", "not_found");
    }
    for query in [
        "camera=front&start=1499&end=1600",
        "camera=front&start=900&end=1001",
    ] {
        let raw = env.clip_link("admin", query).await;
        let (status, _, _) = http_parts(&raw);
        assert_eq!(status_code(&status), "200", "status line {status:?}");
    }
    env.finish();
}

#[tokio::test]
async fn clip_link_serves_a_camera_that_is_switched_off_when_the_role_may_see_it() {
    let env = TestEnv::start("off-camera").await;
    env.seed(&[("r1", "living", 1_000.0, 1_500.0)]);

    let raw = env
        .clip_link("admin", "camera=living&start=1100&end=1400")
        .await;
    assert_playlist(
        &raw,
        "/corvette/api/v1/media/living/start/1100/end/1400/master.m3u8",
    );
    env.finish();
}

#[tokio::test]
async fn clip_link_uses_the_requested_range_in_the_address_not_the_recording() {
    let env = TestEnv::start("requested-range").await;
    env.seed(&[("r1", "front", 1_000.0, 5_000.0)]);

    let raw = env
        .clip_link("admin", "camera=front&start=1250.5&end=1260")
        .await;
    assert_playlist(
        &raw,
        "/corvette/api/v1/media/front/start/1250.5/end/1260/master.m3u8",
    );
    env.finish();
}

#[tokio::test]
async fn clip_link_answers_bad_request_for_every_malformed_query() {
    const BAD_QUERIES: &[&str] = &[
        "start=1100&end=1400",
        "camera=front&end=1400",
        "camera=front&start=1100",
        "",
        "camera=front&start=1100&end=1400&after=1",
        "camera=front&camera=back&start=1100&end=1400",
        "camera=front&start=1100&start=1200&end=1400",
        "camera=front&start=1100&end=1400&end=1500",
        "camera&start=1100&end=1400",
        "camera=front&start&end=1400",
        "camera=front&start=1100&end",
        "&camera=front&start=1100&end=1400",
        "camera=front&&start=1100&end=1400",
        "camera=front&start=1100&end=1400&",
        "camera=&start=1100&end=1400",
        "camera=front&start=&end=1400",
        "camera=%zz&start=1100&end=1400",
        "camera=front%&start=1100&end=1400",
        "camera=front%2&start=1100&end=1400",
        "camera=%ff&start=1100&end=1400",
        "camera=front&start=abc&end=1400",
        "camera=front&start=1100&end=1e",
        "camera=front&start=-1&end=1400",
        "camera=front&start=inf&end=1400",
        "camera=front&start=1100&end=NaN",
        "camera=front&start=1100&end=infinity",
        "camera=front&start=1400&end=1400",
        "camera=front&start=1400&end=1100",
        "camera=front&start=0&end=2678400.5",
    ];
    let env = TestEnv::start("bad-request").await;
    env.seed(&[("r1", "front", 1_000.0, 1_500.0)]);

    for query in BAD_QUERIES {
        assert_bad_request(&env.clip_link("admin", query).await, query);
    }

    let no_question_mark = exchange(
        &env.socket,
        &get(
            "/corvette/api/v1/clip-link",
            &["Remote-User: alice", "Remote-Role: admin"],
        ),
    )
    .await;
    assert_error_shape(&no_question_mark, "400", "bad_request");
    env.finish();
}

#[tokio::test]
async fn clip_link_names_start_and_end_in_its_bad_request_messages() {
    let env = TestEnv::start("bad-request-messages").await;
    env.seed(&[("r1", "front", 1_000.0, 1_500.0)]);

    for (query, message) in [
        ("start=1100&end=1400", "missing query parameter `camera`"),
        (
            "camera=&start=1100&end=1400",
            "`camera` must name one camera",
        ),
        (
            "camera=front&start=1400&end=1100",
            "`end` must be greater than `start`",
        ),
        (
            "camera=front&start=0&end=2678400.5",
            "`end` is more than 2678400 seconds after `start`",
        ),
    ] {
        let raw = env.clip_link("admin", query).await;
        assert_error_shape(&raw, "400", "bad_request");
        let (_, _, body) = http_parts(&raw);
        let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
        assert_eq!(error.message, message, "query {query:?}");
    }
    env.finish();
}

#[tokio::test]
async fn clip_link_accepts_a_range_of_exactly_thirty_one_days() {
    let env = TestEnv::start("thirty-one-days").await;
    env.seed(&[("r1", "front", 1_000.0, 3_000_000.0)]);

    assert_playlist(
        &env.clip_link("admin", "camera=front&start=1000&end=2679400")
            .await,
        "/corvette/api/v1/media/front/start/1000/end/2679400/master.m3u8",
    );
    assert_error_shape(
        &env.clip_link("admin", "camera=front&start=1000&end=2679400.5")
            .await,
        "400",
        "bad_request",
    );
    env.finish();
}

#[tokio::test]
async fn clip_link_keeps_a_plus_sign_in_the_camera_name() {
    let env = TestEnv::start("plus-in-name").await;
    env.seed(&[("r1", "back yard", 1_000.0, 1_500.0)]);

    assert_error_shape(
        &env.clip_link("admin", "camera=back+yard&start=1100&end=1400")
            .await,
        "404",
        "not_found",
    );
    assert_playlist(
        &env.clip_link("admin", "camera=back%20yard&start=1100&end=1400")
            .await,
        "/corvette/api/v1/media/back%20yard/start/1100/end/1400/master.m3u8",
    );
    env.finish();
}

#[tokio::test]
async fn clip_link_reports_a_malformed_query_before_the_config_fetch() {
    let env = TestEnv::refusing("malformed-first").await;

    assert_error_shape(
        &env.clip_link("admin", "camera=front&start=x&end=1400")
            .await,
        "400",
        "bad_request",
    );
    env.finish();
}

#[tokio::test]
async fn clip_link_with_a_refusing_config_server_answers_config_unavailable() {
    let env = TestEnv::refusing("config-down").await;
    env.seed(&[("r1", "front", 1_000.0, 1_500.0)]);

    assert_error_shape(
        &env.clip_link("admin", "camera=front&start=1100&end=1400")
            .await,
        "503",
        "config_unavailable",
    );
    env.finish();
}

#[tokio::test]
async fn clip_link_with_a_missing_database_answers_database_unavailable() {
    let env = TestEnv::start("db-missing").await;

    let raw = env
        .clip_link("admin", "camera=front&start=1100&end=1400")
        .await;
    let (status, headers, body) = http_parts(&raw);
    assert_eq!(status_code(&status), "503", "status line {status:?}");
    assert_framing(&headers, &body);
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(error.code, "database_unavailable");
    assert_eq!(error.message, "Frigate's database could not be read.");
    env.finish();
}
