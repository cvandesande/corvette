//! Tests the recordings route against a fixture config server and a recording-index fixture.

use corvette_api::ErrorBody;
use corvette_api_server::config::{ConfigSource, DEFAULT_CACHE_TTL};
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixStream};
use tokio::task::JoinHandle;

/// Tables as crates/corvette-media-bridge/src/index.rs:14-29 creates them.
const INDEX_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS pieces (
    camera TEXT NOT NULL,
    seq INTEGER NOT NULL,
    start_ts INTEGER NOT NULL,
    end_ts INTEGER NOT NULL,
    duration_s REAL NOT NULL,
    path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    codec TEXT NOT NULL,
    upload_state TEXT NOT NULL DEFAULT 'local',
    PRIMARY KEY (camera, seq)
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

struct TestEnv {
    socket: PathBuf,
    dir: PathBuf,
    index_path: PathBuf,
    handle: JoinHandle<()>,
}

impl TestEnv {
    async fn start(tag: &str) -> Self {
        let config_addr = start_config_backing().await;
        let dir = unique_dir(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test directory");
        let socket = dir.join("api.sock");
        let index_path = dir.join("index.sqlite");
        let config = Arc::new(ConfigSource::new(config_addr, DEFAULT_CACHE_TTL));
        let server = ApiServer::bind(&socket, CONNECTION_TIMEOUT, config, &dir.join("absent.db"))
            .expect("bind")
            .with_index_path(&index_path);
        let socket = server.socket_path().to_path_buf();
        let handle = tokio::spawn(server.serve_until(std::future::pending::<()>()));
        Self {
            socket,
            dir,
            index_path,
            handle,
        }
    }

    fn finish(self) {
        self.handle.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// Creates the recording index and inserts pieces of `(camera, start_ms, end_ms)`.
    fn seed_index(&self, pieces: &[(&str, i64, i64)]) {
        let index = Connection::open(&self.index_path).expect("create index");
        index
            .execute_batch(INDEX_SCHEMA)
            .expect("create index schema");
        for (n, (camera, start, end)) in pieces.iter().enumerate() {
            let seq = i64::try_from(n).expect("piece count fits i64") + 1;
            index
                .execute(
                    "INSERT INTO pieces \
                     (camera, seq, start_ts, end_ts, duration_s, path, size_bytes, codec) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 'h264')",
                    params![
                        camera,
                        seq,
                        start,
                        end,
                        1.0,
                        format!("/{camera}/{start}.mp4")
                    ],
                )
                .expect("insert piece");
        }
    }

    async fn recordings(&self, role: &str, query: &str) -> Vec<u8> {
        exchange(&self.socket, &recordings_request(role, query)).await
    }

    /// The parsed `ranges` of a `200` answer.
    async fn ranges(&self, role: &str, query: &str) -> Value {
        let raw = self.recordings(role, query).await;
        let (status, _headers, body) = http_parts(&raw);
        assert_eq!(status_code(&status), "200", "status line {status:?}");
        let answer: Value = serde_json::from_slice(&body).expect("recordings answer is JSON");
        answer["ranges"].clone()
    }
}

fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "corvette-recordings-test-{}-{tag}-{id}",
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

fn recordings_request(role: &str, query: &str) -> Vec<u8> {
    get(
        &format!("/corvette/api/v1/recordings?{query}"),
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

fn assert_error_shape(raw: &[u8], status_prefix: &str, code: &str) {
    let (status, _headers, body) = http_parts(raw);
    assert_eq!(
        status_code(&status),
        status_prefix,
        "status line {status:?}"
    );
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(error.code, code);
}

/// Enables `front` and `back`; `frontonly` sees only `front`.
fn fixture_config() -> String {
    json!({
        "auth": { "roles": {
            "admin": [],
            "frontonly": ["front"]
        } },
        "cameras": {
            "front": { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                       "detect": { "width": 704, "height": 576 } },
            "back":  { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                       "detect": { "width": 704, "height": 576 } }
        }
    })
    .to_string()
}

/// Answers only `GET /api/config` with the fixture configuration.
async fn start_config_backing() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind config backing");
    let addr = listener
        .local_addr()
        .expect("config backing addr")
        .to_string();
    let body = format!(
        "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        fixture_config().len(),
        fixture_config()
    );
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _peer)) = listener.accept().await else {
                break;
            };
            let response = body.clone().into_bytes();
            tokio::spawn(async move {
                let mut request = [0_u8; 512];
                let _ = stream.read(&mut request).await;
                let _ = stream.write_all(&response).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    addr
}

/// The byte contents of the index file and its side files, present ones only.
fn index_files(index_path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    ["", "-wal", "-shm"]
        .iter()
        .map(|suffix| format!("{}{suffix}", index_path.display()))
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .map(|path| {
            let bytes = std::fs::read(&path).expect("read index file");
            (path, bytes)
        })
        .collect()
}

const fn piece(camera: &str, start_seconds: i64, end_seconds: i64) -> (&str, i64, i64) {
    (camera, start_seconds * 1000, end_seconds * 1000)
}

#[tokio::test]
async fn two_disjoint_pieces_answer_their_own_ranges() {
    let env = TestEnv::start("disjoint").await;
    env.seed_index(&[piece("front", 1000, 1050), piece("front", 1060, 1100)]);

    assert_eq!(
        env.ranges("admin", "camera=front&start=0&end=2000").await,
        json!([
            {"camera": "front", "start": 1000.0, "end": 1050.0},
            {"camera": "front", "start": 1060.0, "end": 1100.0}
        ])
    );
    env.finish();
}

#[tokio::test]
async fn exactly_touching_pieces_answer_one_range() {
    let env = TestEnv::start("touching").await;
    env.seed_index(&[piece("front", 1000, 1050), piece("front", 1050, 1100)]);

    assert_eq!(
        env.ranges("admin", "camera=front&start=0&end=2000").await,
        json!([{"camera": "front", "start": 1000.0, "end": 1100.0}])
    );
    env.finish();
}

#[tokio::test]
async fn overlapping_pieces_answer_one_range_without_double_counting() {
    let env = TestEnv::start("overlapping").await;
    env.seed_index(&[
        piece("front", 1000, 1080),
        piece("front", 1060, 1120),
        piece("front", 1100, 1110),
    ]);

    assert_eq!(
        env.ranges("admin", "camera=front&start=0&end=2000").await,
        json!([{"camera": "front", "start": 1000.0, "end": 1120.0}])
    );
    env.finish();
}

#[tokio::test]
async fn a_piece_ending_exactly_at_the_start_is_excluded() {
    let env = TestEnv::start("ends-at-start").await;
    env.seed_index(&[piece("front", 800, 1000)]);

    assert_eq!(
        env.ranges("admin", "camera=front&start=1000&end=2000")
            .await,
        json!([])
    );
    env.finish();
}

#[tokio::test]
async fn a_piece_starting_exactly_at_the_end_is_excluded() {
    let env = TestEnv::start("starts-at-end").await;
    env.seed_index(&[piece("front", 2000, 2100)]);

    assert_eq!(
        env.ranges("admin", "camera=front&start=1000&end=2000")
            .await,
        json!([])
    );
    env.finish();
}

#[tokio::test]
async fn a_piece_straddling_a_bound_is_clipped_to_it() {
    let env = TestEnv::start("straddling").await;
    env.seed_index(&[piece("front", 900, 1050), piece("front", 1900, 2100)]);

    assert_eq!(
        env.ranges("admin", "camera=front&start=1000&end=2000")
            .await,
        json!([
            {"camera": "front", "start": 1000.0, "end": 1050.0},
            {"camera": "front", "start": 1900.0, "end": 2000.0}
        ])
    );
    env.finish();
}

#[tokio::test]
async fn another_cameras_pieces_never_appear() {
    let env = TestEnv::start("other-camera").await;
    env.seed_index(&[piece("front", 1000, 1050), piece("back", 1020, 1080)]);

    assert_eq!(
        env.ranges("admin", "camera=front&start=0&end=2000").await,
        json!([{"camera": "front", "start": 1000.0, "end": 1050.0}])
    );
    env.finish();
}

#[tokio::test]
async fn a_known_camera_without_pieces_answers_no_ranges() {
    let env = TestEnv::start("no-pieces").await;
    env.seed_index(&[]);

    assert_eq!(
        env.ranges("admin", "camera=front&start=0&end=2000").await,
        json!([])
    );
    env.finish();
}

#[tokio::test]
async fn a_role_answers_like_the_range_link_route_for_a_camera_it_may_not_see() {
    let env = TestEnv::start("role-filter").await;
    env.seed_index(&[piece("front", 1000, 1050), piece("back", 1000, 1050)]);

    // The route answers 404 not_found with the range-link route's message.
    let refused = env
        .recordings("frontonly", "camera=back&start=0&end=2000")
        .await;
    assert_error_shape(&refused, "404", "not_found");
    let (_status, _headers, body) = http_parts(&refused);
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(
        error.message,
        "No recorded footage is available for this camera and range."
    );

    let ghost = env
        .recordings("admin", "camera=ghost&start=0&end=2000")
        .await;
    assert_error_shape(&ghost, "404", "not_found");

    assert_eq!(
        env.ranges("frontonly", "camera=front&start=0&end=2000")
            .await,
        json!([{"camera": "front", "start": 1000.0, "end": 1050.0}])
    );
    env.finish();
}

#[tokio::test]
async fn an_absent_index_answers_recording_unavailable_without_creating_it() {
    let env = TestEnv::start("absent-index").await;

    let raw = env
        .recordings("admin", "camera=front&start=0&end=2000")
        .await;
    assert_error_shape(&raw, "503", "recording_unavailable");
    assert!(
        !env.index_path.exists(),
        "the read path must never create the index file"
    );
    env.finish();
}

#[tokio::test]
async fn a_file_that_is_not_a_database_answers_recording_unavailable_without_naming_paths() {
    let env = TestEnv::start("not-a-database").await;
    std::fs::write(&env.index_path, "these bytes are not a database").expect("write file");

    let raw = env
        .recordings("admin", "camera=front&start=0&end=2000")
        .await;
    assert_error_shape(&raw, "503", "recording_unavailable");
    let (_status, _headers, body) = http_parts(&raw);
    assert!(
        !String::from_utf8_lossy(&body).contains(&env.dir.display().to_string()),
        "the index path must not reach the answer: {body:?}"
    );
    env.finish();
}

#[tokio::test]
async fn malformed_queries_answer_bad_request() {
    let env = TestEnv::start("bad-queries").await;
    let cases: [&str; 9] = [
        "",
        "camera=front",
        "camera=front&start=100",
        "camera=front&end=200",
        "camera=&start=100&end=200",
        "camera=front&start=abc&end=200",
        "camera=front&start=-1&end=200",
        "camera=front&start=200&end=200",
        "camera=front&start=0&end=3000000",
    ];
    for query in cases {
        let raw = env.recordings("admin", query).await;
        assert_error_shape(&raw, "400", "bad_request");
    }
    env.finish();
}

#[tokio::test]
async fn the_route_reads_an_index_the_service_cannot_write() {
    use std::os::unix::fs::PermissionsExt;
    let env = TestEnv::start("write-refused-open").await;
    env.seed_index(&[piece("front", 1000, 1050)]);
    // The deployed media mount is read-only for this service; a route that
    // opened read-write could not answer on it at all.
    std::fs::set_permissions(&env.index_path, std::fs::Permissions::from_mode(0o444))
        .expect("make index read-only");

    assert_eq!(
        env.ranges("admin", "camera=front&start=0&end=2000").await,
        json!([{"camera": "front", "start": 1000.0, "end": 1050.0}])
    );
    env.finish();
}

#[tokio::test]
async fn a_range_read_leaves_the_wal_index_bytes_unchanged() {
    let env = TestEnv::start("read-only-open").await;
    // The bridge keeps the index open in WAL mode while the service reads it.
    let writer = Connection::open(&env.index_path).expect("create index");
    writer
        .execute_batch("PRAGMA journal_mode=WAL;")
        .expect("set WAL mode");
    writer
        .execute_batch(INDEX_SCHEMA)
        .expect("create index schema");
    writer
        .execute(
            "INSERT INTO pieces \
             (camera, seq, start_ts, end_ts, duration_s, path, size_bytes, codec) \
             VALUES ('front', 1, 1000000, 1050000, 50.0, '/front/1000000.mp4', 1, 'h264')",
            params![],
        )
        .expect("insert piece");

    let before = index_files(&env.index_path);
    let side_files = before
        .iter()
        .filter(|(path, _)| {
            let name = path.to_string_lossy();
            name.ends_with("-wal") || name.ends_with("-shm")
        })
        .count();
    assert_eq!(
        side_files, 2,
        "WAL side files must be present while the writer is open"
    );

    let raw = env
        .recordings("admin", "camera=front&start=0&end=2000")
        .await;
    let (status, _headers, body) = http_parts(&raw);
    assert_eq!(status_code(&status), "200", "status line {status:?}");
    let answer: Value = serde_json::from_slice(&body).expect("recordings answer is JSON");
    assert_eq!(
        answer["ranges"],
        json!([{"camera": "front", "start": 1000.0, "end": 1050.0}])
    );

    let after = index_files(&env.index_path);
    // A WAL reader registers read marks in the `-shm` shared-memory file:
    // SQLite reader scratch, not the database. The database and its log are
    // the bytes a read-open must not change.
    let database_bytes = |files: &[(PathBuf, Vec<u8>)]| -> Vec<(String, Vec<u8>)> {
        files
            .iter()
            .filter(|(path, _)| !path.to_string_lossy().ends_with("-shm"))
            .map(|(path, bytes)| (path.display().to_string(), bytes.clone()))
            .collect()
    };
    assert_eq!(
        database_bytes(&before),
        database_bytes(&after),
        "the read changed no database bytes"
    );
    let shm_after = after
        .iter()
        .filter(|(path, _)| path.to_string_lossy().ends_with("-shm"))
        .count();
    assert_eq!(shm_after, 1, "the shared-memory file stays in place");
    drop(writer);
    env.finish();
}
