//! Integration tests: the service over a real Unix socket in a temp directory
//! (issue #4 A2 — invariants I2, I7, I8).

use corvette_api::ErrorBody;
use corvette_api_server::config::{ConfigSource, DEFAULT_CACHE_TTL};
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use std::collections::HashMap;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::task::JoinHandle;

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
    std::env::temp_dir().join(format!("corvette-api-test-{}-{}", std::process::id(), id))
}

/// A source no test here can reach: none of these requests survives the
/// identity gate, the prefix check, or the parser, so no fetch is ever made.
fn unreachable_config() -> std::sync::Arc<ConfigSource> {
    std::sync::Arc::new(ConfigSource::new(
        "127.0.0.1:1".to_owned(),
        DEFAULT_CACHE_TTL,
    ))
}

/// Binds a service on a temp socket and serves it until the test aborts it.
/// The database path names no file: none of these tests reaches a route that
/// probes it.
fn start_server(timeout: Duration) -> TestServer {
    let dir = unique_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let socket = dir.join("api.sock");
    let server = ApiServer::bind(
        &socket,
        timeout,
        unreachable_config(),
        &dir.join("absent.db"),
    )
    .expect("bind");
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

/// Sends `request` and reads the whole response. A write error is ignored: a
/// server that rejects a request early may close before the last byte is sent.
async fn exchange(socket: &Path, request: &[u8]) -> Vec<u8> {
    let mut stream = UnixStream::connect(socket).await.expect("connect");
    let _ = stream.write_all(request).await;
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

/// A well-formed request whose header block totals exactly `total` bytes,
/// including the terminating `\r\n\r\n`.
fn sized_request(total: usize) -> Vec<u8> {
    let mut request = b"GET /x HTTP/1.0\r\nRemote-User: alice\r\nX-Pad: ".to_vec();
    let pad = total - request.len() - b"\r\n\r\n".len();
    request.extend(std::iter::repeat_n(b'a', pad));
    request.extend_from_slice(b"\r\n\r\n");
    assert_eq!(request.len(), total);
    request
}

/// Asserts the single error shape: `application/json`, an exact
/// `Content-Length`, and a body that parses as the expected `ErrorBody` code.
fn assert_error_shape(raw: &[u8], status_prefix: &str, code: &str) {
    let (status, headers, body) = http_parts(raw);
    let actual = status.split_whitespace().nth(1).unwrap_or_default();
    assert_eq!(actual, status_prefix, "status line {status:?}");
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json"),
        "Content-Type must be exactly application/json"
    );
    let declared: usize = headers
        .get("content-length")
        .expect("response carries a Content-Length")
        .parse()
        .expect("Content-Length is a number");
    assert_eq!(declared, body.len(), "Content-Length must match the body");
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(error.code, code);
}

#[tokio::test]
async fn every_route_requires_remote_user() {
    let server = start_server(CONNECTION_TIMEOUT);
    for path in [
        "/corvette/api/v1/cameras",
        "/corvette/api/v1/health",
        "/corvette/api/v1/unknown-path",
    ] {
        let response = exchange(&server.socket, &get(path, &[])).await;
        assert_error_shape(&response, "401", "unauthorized");
    }
    // An empty `Remote-User` is treated the same as an absent one.
    let empty = exchange(
        &server.socket,
        &get("/corvette/api/v1/cameras", &["Remote-User:"]),
    )
    .await;
    assert_error_shape(&empty, "401", "unauthorized");
    server.finish();
}

#[tokio::test]
async fn authenticated_requests_to_unknown_routes_are_not_found() {
    let server = start_server(CONNECTION_TIMEOUT);
    let with_role = exchange(
        &server.socket,
        &get(
            "/corvette/api/v1/unknown-path",
            &["Remote-User: alice", "Remote-Role: viewer"],
        ),
    )
    .await;
    assert_error_shape(&with_role, "404", "not_found");

    // `Remote-User` alone passes the gate; the path is unknown either way.
    let without_role = exchange(
        &server.socket,
        &get("/corvette/api/v1/unknown-path", &["Remote-User: alice"]),
    )
    .await;
    assert_error_shape(&without_role, "404", "not_found");
    server.finish();
}

#[tokio::test]
async fn the_remote_user_header_name_is_case_insensitive() {
    let server = start_server(CONNECTION_TIMEOUT);
    // Any non-401 answer proves the header passed the identity gate; an
    // unknown path answers without touching any upstream.
    let response = exchange(
        &server.socket,
        &get("/corvette/api/v1/unknown-path", &["remote-user: alice"]),
    )
    .await;
    assert_error_shape(&response, "404", "not_found");
    server.finish();
}

#[tokio::test]
async fn every_malformed_request_is_bad_request() {
    let server = start_server(CONNECTION_TIMEOUT);

    let mut oversized = b"GET /x HTTP/1.0\r\nX-Pad: ".to_vec();
    oversized.extend(std::iter::repeat_n(b'a', 9000));
    oversized.extend_from_slice(b"\r\n\r\n");

    let bad_requests: Vec<Vec<u8>> = vec![
        b"POST /x HTTP/1.0\r\nRemote-User: a\r\n\r\n".to_vec(),
        b"GET /x\r\n\r\n".to_vec(),
        b"GET  /x HTTP/1.0\r\n\r\n".to_vec(),
        b"GET /x HTTP/2.0\r\n\r\n".to_vec(),
        b"GET /x HTTP/1.0\r\nnocolon\r\nRemote-User: a\r\n\r\n".to_vec(),
        b"GET /x HTTP/1.0\r\n folded-continuation\r\n\r\n".to_vec(),
        oversized,
        b"GET /x HTTP/1.0\r\nRemote-User: a\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec(),
        b"GET /x HTTP/1.0\r\nRemote-User: a\r\nContent-Length: 5\r\n\r\n".to_vec(),
        b"GET /x HTTP/1.0\r\nRemote-User: a\r\nRemote-User: b\r\n\r\n".to_vec(),
        b"GET /x HTTP/1.0\r\nRemote-User: a\r\nRemote-Role: r\r\nRemote-Role: s\r\n\r\n".to_vec(),
        b"GET /x HTTP/1.0\r\nRemote-User: \xff\r\n\r\n".to_vec(),
    ];

    for request in bad_requests {
        let response = exchange(&server.socket, &request).await;
        assert_error_shape(&response, "400", "bad_request");
    }
    server.finish();
}

#[tokio::test]
async fn a_stalled_sender_is_closed_by_the_timeout() {
    let server = start_server(Duration::from_millis(50));
    let mut stream = UnixStream::connect(&server.socket).await.expect("connect");
    // Half a request, then nothing: no terminating blank line is ever sent.
    stream
        .write_all(b"GET /corvette/api/v1/cameras HTTP/1.0\r\nRemote-Us")
        .await
        .expect("partial write");

    let started = Instant::now();
    let mut response = Vec::new();
    let closed =
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response)).await;
    let elapsed = started.elapsed();

    assert!(
        closed.is_ok(),
        "the server must close a stalled connection within its timeout"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "closed after {elapsed:?}, not promptly"
    );
    assert!(
        response.is_empty(),
        "a stalled request must get no response: {response:?}"
    );
    server.finish();
}

#[tokio::test]
async fn a_stale_socket_file_at_the_path_is_replaced() {
    let dir = unique_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let socket = dir.join("api.sock");
    std::fs::write(&socket, b"left over from a previous run").expect("write stale file");

    let server = ApiServer::bind(
        &socket,
        CONNECTION_TIMEOUT,
        unreachable_config(),
        &dir.join("absent.db"),
    )
    .expect("binding must replace a stale socket file");
    let handle = tokio::spawn(server.serve_until(std::future::pending::<()>()));

    let response = exchange(
        &socket,
        &get("/corvette/api/v1/unknown-path", &["Remote-User: alice"]),
    )
    .await;
    assert_error_shape(&response, "404", "not_found");

    handle.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_bound_socket_file_is_writable_by_every_user() {
    let dir = unique_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let socket = dir.join("api.sock");

    let server = ApiServer::bind(
        &socket,
        CONNECTION_TIMEOUT,
        unreachable_config(),
        &dir.join("absent.db"),
    )
    .expect("bind");

    let metadata = std::fs::metadata(&socket).expect("stat the socket file");
    assert!(
        metadata.file_type().is_socket(),
        "bound path is not a socket"
    );
    assert_eq!(
        metadata.permissions().mode() & 0o777,
        0o666,
        "nginx has no CAP_DAC_OVERRIDE, so it needs write on the socket file"
    );

    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_header_block_exactly_at_the_size_limit_is_accepted() {
    let server = start_server(CONNECTION_TIMEOUT);
    let response = exchange(&server.socket, &sized_request(8192)).await;
    assert_error_shape(&response, "404", "not_found");
    server.finish();
}

#[tokio::test]
async fn a_header_block_at_the_limit_split_across_writes_is_accepted() {
    let server = start_server(CONNECTION_TIMEOUT);
    let request = sized_request(8192);
    let mut stream = UnixStream::connect(&server.socket).await.expect("connect");
    // Pause between writes so the server has consumed the first part; back-to-
    // back writes would let the kernel deliver both in one read.
    let (head, tail) = request.split_at(request.len() - 1);
    stream.write_all(head).await.expect("first write");
    tokio::time::sleep(Duration::from_millis(200)).await;
    stream.write_all(tail).await.expect("second write");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read to end");
    assert_error_shape(&response, "404", "not_found");
    server.finish();
}

#[tokio::test]
async fn a_header_block_one_byte_over_the_size_limit_is_rejected() {
    let server = start_server(CONNECTION_TIMEOUT);
    let response = exchange(&server.socket, &sized_request(8193)).await;
    assert_error_shape(&response, "400", "bad_request");
    server.finish();
}

#[tokio::test]
async fn an_over_limit_terminator_split_across_writes_is_rejected() {
    let server = start_server(CONNECTION_TIMEOUT);
    let request = sized_request(8193);
    let mut stream = UnixStream::connect(&server.socket).await.expect("connect");
    // Pause so the server reads the first part alone; an uncapped follow-up
    // read would then grab the tail and push the terminator past the limit.
    let (head, tail) = request.split_at(request.len() - 2);
    stream.write_all(head).await.expect("first write");
    tokio::time::sleep(Duration::from_millis(200)).await;
    stream.write_all(tail).await.expect("second write");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read to end");
    assert_error_shape(&response, "400", "bad_request");
    server.finish();
}

#[tokio::test]
async fn a_peer_that_closes_before_the_header_block_ends_gets_no_response() {
    let server = start_server(CONNECTION_TIMEOUT);
    let mut stream = UnixStream::connect(&server.socket).await.expect("connect");
    stream
        .write_all(b"GET /corvette/api/v1/cameras HTTP/1.0\r\nRemote-User: al")
        .await
        .expect("partial write");
    // Half-close: the server sees EOF mid-block, not a timeout.
    stream.shutdown().await.expect("shutdown");

    let started = Instant::now();
    let mut response = Vec::new();
    let closed =
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response)).await;
    assert!(
        closed.is_ok() && started.elapsed() < Duration::from_secs(2),
        "the server must close an early-closed connection promptly"
    );
    assert!(
        response.is_empty(),
        "an early close must get no response: {response:?}"
    );
    server.finish();
}
