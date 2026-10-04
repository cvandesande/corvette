//! Integration tests for `GET /corvette/api/v1/health` (issue #4 A4 —
//! invariants I2, I3, I7): the database probe against real `SQLite` files and
//! the configuration source behind a fixture.

use corvette_api::{ErrorBody, Health};
use corvette_api_server::config::ConfigSource;
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use rusqlite::Connection;
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

/// A running service plus the temp dir to clean up afterwards.
struct TestServer {
    socket: PathBuf,
    dir: PathBuf,
    handle: JoinHandle<()>,
}

impl TestServer {
    fn finish(self) {
        self.handle.abort();
        restore_writable(&self.dir);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A fresh temp directory path, unique within this test process.
fn unique_dir() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "corvette-health-test-{}-{}",
        std::process::id(),
        id
    ))
}

/// Binds a service on a temp socket with the given database path and config
/// address.
fn start_server(db_path: &Path, config_addr: String) -> TestServer {
    let dir = unique_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let socket = dir.join("api.sock");
    let config = Arc::new(ConfigSource::new(config_addr, Duration::from_mins(1)));
    let server = ApiServer::bind(&socket, CONNECTION_TIMEOUT, config, db_path).expect("bind");
    let path = server.socket_path().to_path_buf();
    let handle = tokio::spawn(server.serve_until(std::future::pending::<()>()));
    TestServer {
        socket: path,
        dir,
        handle,
    }
}

/// Makes `path` (a directory or file) writable again, ignoring failures.
fn restore_writable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Creates a WAL-mode database at `path` holding one `reviewsegment` row and
/// returns the writer connection: keeping it open is what leaves `-wal` and
/// `-shm` present with an uncheckpointed write, as they are while Frigate runs.
fn create_wal_database(path: &Path) -> Connection {
    let writer = Connection::open(path).expect("create test database");
    writer
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE reviewsegment (id INTEGER PRIMARY KEY);
             INSERT INTO reviewsegment (id) VALUES (1);",
        )
        .expect("set up WAL database");
    writer
}

/// How the fixture config server answers a connection.
#[derive(Clone)]
enum Behavior {
    /// Read the request, write these response bytes, close.
    Respond(Vec<u8>),
    /// Close at once, as nothing behind the address would.
    Refuse,
}

/// A fixture for Frigate's config HTTP interface: it answers connections with
/// the queued behaviors (the last one repeats) and counts answered requests.
struct Fixture {
    behaviors: Mutex<VecDeque<Behavior>>,
    request_count: AtomicUsize,
}

impl Fixture {
    async fn start(behaviors: Vec<Behavior>) -> (Arc<Self>, String) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fixture server");
        let addr = listener.local_addr().expect("fixture addr").to_string();
        let fixture = Arc::new(Self {
            behaviors: Mutex::new(behaviors.into()),
            request_count: AtomicUsize::new(0),
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
                            if read_request(&mut stream).await.is_some() {
                                fixture.request_count.fetch_add(1, Ordering::SeqCst);
                            }
                            let _ = stream.write_all(&response).await;
                            let _ = stream.shutdown().await;
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
        self.request_count.load(Ordering::SeqCst)
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

/// A well-formed authenticated `GET` of the health route.
fn health_request() -> Vec<u8> {
    b"GET /corvette/api/v1/health HTTP/1.0\r\nRemote-User: alice\r\nRemote-Role: admin\r\n\r\n"
        .to_vec()
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

/// Asserts the framing every response carries (I7): exactly the promised
/// headers and a `Content-Length` that matches the body.
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

#[tokio::test]
async fn health_reports_database_unreadable_when_the_file_is_missing() {
    let (_fixture, addr) =
        Fixture::start(vec![Behavior::Respond(config_response(FRIGATE_CONFIG))]).await;
    let dir = unique_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let db_path = dir.join("absent.db");
    let server = start_server(&db_path, addr);

    let response = exchange(&server.socket, &health_request()).await;
    let (status, headers, body) = http_parts(&response);
    assert_eq!(status, "HTTP/1.0 503 Service Unavailable");
    assert_framing(&headers, &body);
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(error.code, "unhealthy");
    assert_eq!(error.message, "database unreadable");
    assert!(
        !db_path.exists(),
        "a health probe must never create the database file"
    );
    server.finish();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn health_names_both_failing_parts_when_both_are_down() {
    let (_fixture, addr) = Fixture::start(vec![Behavior::Refuse]).await;
    let dir = unique_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let server = start_server(&dir.join("absent.db"), addr);

    let response = exchange(&server.socket, &health_request()).await;
    let (status, headers, body) = http_parts(&response);
    assert_eq!(status, "HTTP/1.0 503 Service Unavailable");
    assert_framing(&headers, &body);
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(error.code, "unhealthy");
    assert_eq!(error.message, "database unreadable; config unavailable");
    server.finish();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_missing_reviewsegment_table_is_unhealthy() {
    let (_fixture, addr) =
        Fixture::start(vec![Behavior::Respond(config_response(FRIGATE_CONFIG))]).await;
    let dir = unique_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let db_path = dir.join("frigate.db");
    {
        let writer = Connection::open(&db_path).expect("create test database");
        writer
            .execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE event (id INTEGER);")
            .expect("set up WAL database without reviewsegment");
    }
    let server = start_server(&db_path, addr);

    let response = exchange(&server.socket, &health_request()).await;
    let (status, headers, body) = http_parts(&response);
    assert_eq!(status, "HTTP/1.0 503 Service Unavailable");
    assert_framing(&headers, &body);
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(error.code, "unhealthy");
    assert_eq!(error.message, "database unreadable");
    server.finish();
    restore_writable(&dir);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_good_database_alone_is_not_enough_when_config_is_down() {
    let (_fixture, addr) = Fixture::start(vec![Behavior::Refuse]).await;
    let dir = unique_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let db_path = dir.join("frigate.db");
    create_wal_database(&db_path)
        .close()
        .expect("close the seeded writer");
    let server = start_server(&db_path, addr);

    let response = exchange(&server.socket, &health_request()).await;
    let (status, headers, body) = http_parts(&response);
    assert_eq!(status, "HTTP/1.0 503 Service Unavailable");
    assert_framing(&headers, &body);
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(error.code, "unhealthy");
    assert_eq!(error.message, "config unavailable");
    server.finish();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_unauthenticated_health_request_touches_no_upstream() {
    let (fixture, addr) =
        Fixture::start(vec![Behavior::Respond(config_response(FRIGATE_CONFIG))]).await;
    let dir = unique_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let db_path = dir.join("absent.db");
    let server = start_server(&db_path, addr);

    let request = b"GET /corvette/api/v1/health HTTP/1.0\r\n\r\n";
    let response = exchange(&server.socket, request).await;
    let (status, headers, body) = http_parts(&response);
    assert_eq!(status, "HTTP/1.0 401 Unauthorized");
    assert_framing(&headers, &body);
    assert_eq!(fixture.request_count(), 0, "the gate runs before any fetch");
    assert!(
        !db_path.exists(),
        "an anonymous request must not open (or create) the database"
    );
    server.finish();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn health_answers_ok_from_a_read_only_wal_database() {
    // Reproduces the read-only `frigate-config` mount of issue #4 D5: the
    // open writer keeps the readable `-wal`/`-shm` files present.
    let (_fixture, addr) =
        Fixture::start(vec![Behavior::Respond(config_response(FRIGATE_CONFIG))]).await;
    let dir = unique_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let db_path = dir.join("frigate.db");
    let writer = create_wal_database(&db_path);
    assert!(dir.join("frigate.db-wal").exists());
    assert!(dir.join("frigate.db-shm").exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["frigate.db", "frigate.db-wal", "frigate.db-shm"] {
            std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o444))
                .expect("make file read-only");
        }
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))
            .expect("make directory read-only");
    }

    let server = start_server(&db_path, addr);
    let response = exchange(&server.socket, &health_request()).await;
    let (status, headers, body) = http_parts(&response);
    assert_eq!(status, "HTTP/1.0 200 OK");
    assert_framing(&headers, &body);
    assert_eq!(
        serde_json::from_slice::<Health>(&body).expect("body parses as Health"),
        Health {
            database: "ok".to_owned(),
            config: "ok".to_owned(),
        }
    );

    restore_writable(&dir);
    drop(writer);
    let _ = std::fs::remove_dir_all(&dir);
}
