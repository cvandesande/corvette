//! The read-only Corvette API service (issue #4 A2, A3).
//!
//! Serves [`corvette_api`] wire types over a Unix domain socket with a
//! hand-written `HTTP/1.0` request parser and an identity-gated router under
//! `/corvette/api/v1/`. nginx is the only client: it forwards `GET` requests
//! with no body and sets `Remote-User`/`Remote-Role` from Frigate's auth
//! response, so the parser accepts exactly that narrow form and rejects
//! everything else. The service listens on a Unix socket only (no TCP) because
//! that is what keeps the identity headers unspoofable (issue #4 D4). The data
//! routes answer from [`config::ConfigSource`] and, for `health`, from a
//! read-only probe of [`database`].

use corvette_api::{ErrorBody, Health};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

pub mod config;
pub mod database;

use crate::config::ConfigSource;

/// Every Corvette route lives under this prefix (issue #4 D6).
const API_PREFIX: &str = "/corvette/api/v1/";

/// Environment variable naming the Unix socket to listen on.
pub const SOCKET_PATH_ENV: &str = "CORVETTE_API_SOCKET";

/// Largest accepted header block, so one connection cannot grow this server's
/// read buffer without bound.
const MAX_REQUEST_HEADER_BYTES: usize = 8192;

/// Overall budget for reading one request and writing its response, so a peer
/// that stalls cannot leak a task and a file descriptor forever.
pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

/// Writes one structured log line, matching `corvette-media-bridge`'s own
/// `eprintln!`-based convention: no logging framework exists in this workspace.
pub(crate) fn log_event(unit: &str, event: &str, detail: impl std::fmt::Display) {
    eprintln!("corvette-api-server unit=\"{unit}\" event={event} detail=\"{detail}\"");
}

/// A request that passed the parser: the fields the identity gate and router
/// need, with the identity headers already validated and case-folded.
struct Request {
    path: String,
    remote_user: Option<String>,
    remote_role: String,
}

/// A JSON error response: the status line and the serialized [`ErrorBody`].
struct Response {
    status: &'static str,
    body: Vec<u8>,
}

/// The outcome of one connection's read/route/respond body, so the caller can
/// tell a silently-dropped connection from one that got a response.
enum Outcome {
    Responded,
    ClosedEarly,
}

/// The whole-process socket listener (issue #4 D4).
#[derive(Debug)]
pub struct ApiServer {
    listener: UnixListener,
    socket_path: PathBuf,
    connection_timeout: Duration,
    config: Arc<ConfigSource>,
    db_path: PathBuf,
}

impl ApiServer {
    /// Binds a Unix socket at `socket_path`, first removing any stale file a
    /// previous run left behind. Accepts nothing until [`Self::serve_until`].
    /// `db_path` is Frigate's database file, opened read-only per request
    /// (issue #4 D5).
    ///
    /// # Errors
    ///
    /// Returns an error if removing a stale file or binding the socket fails.
    pub fn bind(
        socket_path: &Path,
        connection_timeout: Duration,
        config: Arc<ConfigSource>,
        db_path: &Path,
    ) -> std::io::Result<Self> {
        // A socket file left by a crashed run makes `bind` fail `EADDRINUSE`;
        // the private `emptyDir` volume keeps it across a container restart.
        if socket_path.exists() {
            std::fs::remove_file(socket_path)?;
        }
        let listener = UnixListener::bind(socket_path)?;
        Ok(Self {
            listener,
            socket_path: socket_path.to_path_buf(),
            connection_timeout,
            config,
            db_path: db_path.to_path_buf(),
        })
    }

    /// The socket path this server is listening on.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Accepts connections until `shutdown` resolves, then stops accepting and
    /// removes the socket file so the next run can bind the same path.
    pub async fn serve_until(self, shutdown: impl Future<Output = ()> + Send) {
        let Self {
            listener,
            socket_path,
            connection_timeout,
            config,
            db_path,
        } = self;
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, _peer)) => {
                        let config = Arc::clone(&config);
                        let db_path = db_path.clone();
                        tokio::spawn(async move {
                            handle_connection(stream, connection_timeout, config, &db_path).await;
                        });
                    }
                    Err(err) => log_event("listener", "accept-error", err),
                },
            }
        }
        drop(listener);
        let _ = std::fs::remove_file(&socket_path);
    }
}

/// Resolves when the process is asked to terminate: `SIGTERM` (the container
/// stop signal) or Ctrl-C at a terminal.
pub async fn shutdown_signal() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
        }
        // A platform without `SIGTERM` still stops on Ctrl-C.
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

/// Handles one connection within `timeout`: read, route, respond, close. A
/// timeout or an early peer close ends the connection with no response.
async fn handle_connection(
    mut stream: UnixStream,
    timeout: Duration,
    config: Arc<ConfigSource>,
    db_path: &Path,
) {
    let outcome =
        tokio::time::timeout(timeout, read_route_respond(&mut stream, &config, db_path)).await;
    match outcome {
        Ok(Ok(Outcome::Responded)) => {}
        Ok(Ok(Outcome::ClosedEarly)) => {
            log_event(
                "http",
                "request-incomplete",
                "peer closed before a complete request header block arrived",
            );
        }
        Ok(Err(err)) => log_event("http", "response-write-error", err),
        Err(_elapsed) => log_event(
            "http",
            "connection-timeout",
            format!(
                "peer did not complete a request within {timeout:?}; closed to avoid leaking the connection"
            ),
        ),
    }
}

/// Reads one request from `stream`, routes it, and writes the response.
async fn read_route_respond(
    stream: &mut UnixStream,
    config: &ConfigSource,
    db_path: &Path,
) -> std::io::Result<Outcome> {
    let block = match read_header_block(stream).await {
        HeaderRead::Complete(block) => block,
        HeaderRead::TooLarge => {
            let response = bad_request("request header block exceeded the size limit");
            write_response(stream, &response).await?;
            // Consuming the unread bytes lets the close send a FIN instead of
            // an RST that would discard the unread response.
            drain_buffered(stream);
            return Ok(Outcome::Responded);
        }
        HeaderRead::Closed => return Ok(Outcome::ClosedEarly),
    };
    let response = match parse_request(&block) {
        Ok(request) => route(&request, config, db_path).await,
        Err(reason) => bad_request(&reason),
    };
    write_response(stream, &response).await?;
    Ok(Outcome::Responded)
}

/// What reading a request header block produced.
enum HeaderRead {
    /// The bytes up to (not including) the terminating `\r\n\r\n`.
    Complete(Vec<u8>),
    /// More than [`MAX_REQUEST_HEADER_BYTES`] arrived before the terminator.
    TooLarge,
    /// The peer closed (or errored) before a complete header block arrived.
    Closed,
}

/// Reads the header block, whose bytes INCLUDING its `\r\n\r\n` terminator must
/// total at most [`MAX_REQUEST_HEADER_BYTES`].
async fn read_header_block(stream: &mut UnixStream) -> HeaderRead {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; MAX_REQUEST_HEADER_BYTES];
    loop {
        // A terminator found here ends within the buffer, which is capped at the
        // limit below, so its block plus terminator is always within the limit.
        if let Some(end) = find_header_end(&buffer) {
            buffer.truncate(end);
            return HeaderRead::Complete(buffer);
        }
        // Size each read to what remains, so a terminator that would land past
        // the limit can never be observed: the buffer fills to the limit first.
        let remaining = MAX_REQUEST_HEADER_BYTES - buffer.len();
        if remaining == 0 {
            return HeaderRead::TooLarge;
        }
        match stream.read(&mut chunk[..remaining]).await {
            Ok(0) => return HeaderRead::Closed,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(err) => {
                log_event("http", "request-read-error", err);
                return HeaderRead::Closed;
            }
        }
    }
}

/// Byte index of the first `\r\n\r\n`, which ends the header block.
pub(crate) fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

/// Splits `block` (already free of the terminating `\r\n\r\n`) into the
/// request line and header lines, on each `\r\n`.
pub(crate) fn split_header_lines(block: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index + 1 < block.len() {
        if block[index] == b'\r' && block[index + 1] == b'\n' {
            lines.push(&block[start..index]);
            start = index + 2;
            index += 2;
        } else {
            index += 1;
        }
    }
    lines.push(&block[start..]);
    lines
}

/// Parses the request line and header block into a [`Request`], or a
/// human-readable reason for the `400` to return.
fn parse_request(block: &[u8]) -> Result<Request, String> {
    let mut lines = split_header_lines(block).into_iter();
    let request_line = lines
        .next()
        .filter(|line| !line.is_empty())
        .ok_or_else(|| "missing request line".to_owned())?;
    let path = parse_request_line(request_line)?;

    let mut remote_user: Option<String> = None;
    let mut remote_role: Option<String> = None;
    let mut user_count = 0_u32;
    let mut role_count = 0_u32;

    for line in lines {
        // A leading space or tab is an obs-fold continuation, which this
        // narrow parser never accepts.
        if line
            .first()
            .is_some_and(|byte| *byte == b' ' || *byte == b'\t')
        {
            return Err("obs-fold header continuation is not accepted".to_owned());
        }
        let (name, value) =
            split_at_byte(line, b':').ok_or_else(|| "header line without a colon".to_owned())?;
        if !is_valid_header_name(name) {
            return Err("malformed header name".to_owned());
        }
        let value = trim_spaces(value);
        match name.to_ascii_lowercase().as_slice() {
            b"transfer-encoding" => {
                return Err("Transfer-Encoding is not accepted".to_owned());
            }
            b"content-length" => {
                if value != b"0" {
                    return Err("Content-Length other than 0 is not accepted".to_owned());
                }
            }
            b"remote-user" => {
                user_count += 1;
                if remote_user.is_none() {
                    let user = utf8(value)
                        .ok_or_else(|| "Remote-User value is not valid UTF-8".to_owned())?;
                    remote_user = Some(user.to_owned());
                }
            }
            b"remote-role" => {
                role_count += 1;
                if remote_role.is_none() {
                    let role = utf8(value)
                        .ok_or_else(|| "Remote-Role value is not valid UTF-8".to_owned())?;
                    remote_role = Some(role.to_owned());
                }
            }
            _ => {}
        }
    }

    if user_count > 1 {
        return Err("more than one Remote-User header".to_owned());
    }
    if role_count > 1 {
        return Err("more than one Remote-Role header".to_owned());
    }
    Ok(Request {
        path: path.to_owned(),
        remote_user,
        remote_role: remote_role.unwrap_or_default(),
    })
}

/// Validates `GET <target> HTTP/1.0`/`HTTP/1.1` (single spaces, target starts
/// with `/`) and returns the path (the target up to any `?`).
fn parse_request_line(line: &[u8]) -> Result<&str, String> {
    let line =
        std::str::from_utf8(line).map_err(|_| "request line is not valid UTF-8".to_owned())?;
    let parts: Vec<&str> = line.split(' ').collect();
    if parts.len() != 3 {
        return Err("request line is not `GET <target> HTTP/1.0`".to_owned());
    }
    if parts[0] != "GET" {
        return Err(format!("method {} is not GET", parts[0]));
    }
    if parts[2] != "HTTP/1.0" && parts[2] != "HTTP/1.1" {
        return Err(format!("version {} is not HTTP/1.0 or HTTP/1.1", parts[2]));
    }
    let target = parts[1];
    if !target.starts_with('/') {
        return Err("request target does not start with /".to_owned());
    }
    Ok(target.split_once('?').map_or(target, |(path, _)| path))
}

/// Splits `bytes` at the first `byte`, returning the parts either side of it.
pub(crate) fn split_at_byte(bytes: &[u8], byte: u8) -> Option<(&[u8], &[u8])> {
    bytes
        .iter()
        .position(|candidate| *candidate == byte)
        .map(|index| (&bytes[..index], &bytes[index + 1..]))
}

/// A header name is a non-empty token with no spaces and no control characters.
fn is_valid_header_name(name: &[u8]) -> bool {
    !name.is_empty()
        && !name
            .iter()
            .any(|byte| *byte < 0x20 || *byte == 0x7F || *byte == b' ')
}

/// Trims leading and trailing ASCII spaces from a header value.
pub(crate) fn trim_spaces(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| *byte != b' ')
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| *byte != b' ')
        .map_or(start, |index| index + 1);
    &value[start..end]
}

/// Interprets `bytes` as UTF-8 text, or `None`.
pub(crate) fn utf8(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes).ok()
}

/// The identity gate and router: a missing or empty `Remote-User` is `401`
/// whatever the path (issue #4 D4). A data route may fetch, so routing is
/// async; the gate still runs first, so an anonymous request never reaches a
/// fetch.
async fn route(request: &Request, config: &ConfigSource, db_path: &Path) -> Response {
    // Identity gate runs before any routing decision, so an anonymous caller
    // learns nothing about which paths exist (issue #4 D4).
    let Some(_user) = request
        .remote_user
        .as_deref()
        .filter(|user| !user.is_empty())
    else {
        return unauthorized();
    };
    // Everything the service serves lives under the API prefix; a path outside
    // it can match no route (issue #4 D6).
    let Some(rest) = request.path.strip_prefix(API_PREFIX) else {
        return not_found();
    };
    match rest {
        "cameras" => cameras(config, &request.remote_role).await,
        "health" => health(config, db_path).await,
        _ => not_found(),
    }
}

/// `GET /corvette/api/v1/cameras` (issue #4 D3): the caller's role sees the
/// cameras it may access, or the route reports `503` when no configuration
/// copy fresh enough to serve exists.
async fn cameras(config: &ConfigSource, role: &str) -> Response {
    match config.camera_list_json(role).await {
        Ok(body) => Response {
            status: "200 OK",
            body,
        },
        Err(err) => error_response(
            "503 Service Unavailable",
            "config_unavailable",
            &format!("Frigate configuration is unavailable: {err}"),
        ),
    }
}

/// `GET /corvette/api/v1/health` (issue #4 D2): `200` when Frigate's database
/// answers the read-only probe and a configuration copy fresh enough to serve
/// exists; `503` naming the failing parts otherwise. The underlying errors
/// are logged, never sent to the caller.
async fn health(config: &ConfigSource, db_path: &Path) -> Response {
    let mut failing = Vec::new();
    if let Err(err) = probe_database(db_path).await {
        log_event("database", "probe-error", err);
        failing.push("database unreadable");
    }
    // A fetch failure is already logged with unit `config` where it happens.
    if config.available().await.is_err() {
        failing.push("config unavailable");
    }
    if !failing.is_empty() {
        return error_response("503 Service Unavailable", "unhealthy", &failing.join("; "));
    }
    let body = Health {
        database: "ok".to_owned(),
        config: "ok".to_owned(),
    };
    // `Health` is two String fields; its derived Serialize never fails.
    Response {
        status: "200 OK",
        body: serde_json::to_vec(&body).expect("health body serializes"),
    }
}

/// Runs the read-only database probe on a blocking thread; rusqlite is
/// synchronous and must not run on an async worker (issue #4 D5).
async fn probe_database(db_path: &Path) -> Result<(), String> {
    let db_path = db_path.to_path_buf();
    match tokio::task::spawn_blocking(move || database::probe(&db_path)).await {
        Ok(result) => result,
        Err(err) => Err(format!("the database probe task failed: {err}")),
    }
}

fn error_response(status: &'static str, code: &str, message: &str) -> Response {
    let body = ErrorBody {
        code: code.to_owned(),
        message: message.to_owned(),
    };
    // ErrorBody is two String fields; its derived Serialize never fails.
    Response {
        status,
        body: serde_json::to_vec(&body).expect("error body serializes"),
    }
}

fn bad_request(message: &str) -> Response {
    error_response("400 Bad Request", "bad_request", message)
}

fn unauthorized() -> Response {
    error_response(
        "401 Unauthorized",
        "unauthorized",
        "no identity was supplied",
    )
}

fn not_found() -> Response {
    error_response("404 Not Found", "not_found", "no such route")
}

/// Non-blockingly consumes whatever the peer already sent, so closing sends a
/// clean FIN rather than an RST that would discard an unwritten response.
fn drain_buffered(stream: &UnixStream) {
    let mut scratch = [0_u8; 4096];
    while stream.try_read(&mut scratch).is_ok_and(|read| read > 0) {}
}

/// Writes `response` as an `HTTP/1.0` response and closes the connection.
async fn write_response(stream: &mut UnixStream, response: &Response) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.0 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        response.status,
        response.body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&response.body).await?;
    stream.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(request_line: &str, headers: &[&str]) -> Result<Request, String> {
        let mut block = request_line.as_bytes().to_vec();
        for header in headers {
            block.extend_from_slice(b"\r\n");
            block.extend_from_slice(header.as_bytes());
        }
        parse_request(&block)
    }

    #[test]
    fn request_line_accepts_get_with_a_leading_slash_target() {
        let request = parse("GET /corvette/api/v1/cameras HTTP/1.0", &[]).expect("valid");
        assert_eq!(request.path, "/corvette/api/v1/cameras");
        assert_eq!(request.remote_user, None);
        assert_eq!(request.remote_role, "");
    }

    #[test]
    fn request_line_strips_the_query_string_from_the_path() {
        let request = parse("GET /corvette/api/v1/cameras?a=b HTTP/1.1", &[]).expect("valid");
        assert_eq!(request.path, "/corvette/api/v1/cameras");
    }

    #[test]
    fn request_line_rejects_extra_spaces_short_forms_and_bad_versions() {
        assert!(parse("GET  /x HTTP/1.0", &[]).is_err());
        assert!(parse("GET /x", &[]).is_err());
        assert!(parse("GET /x HTTP/2.0", &[]).is_err());
        assert!(parse("POST /x HTTP/1.0", &[]).is_err());
        assert!(parse("GET x HTTP/1.0", &[]).is_err());
    }

    #[test]
    fn identity_headers_are_read_case_insensitively_and_trimmed() {
        let request = parse(
            "GET /x HTTP/1.0",
            &["remote-user: alice ", "Remote-Role:  viewer"],
        )
        .expect("valid");
        assert_eq!(request.remote_user.as_deref(), Some("alice"));
        assert_eq!(request.remote_role, "viewer");
    }

    #[test]
    fn duplicate_identity_headers_are_rejected() {
        assert!(parse("GET /x HTTP/1.0", &["Remote-User: a", "Remote-User: b"]).is_err());
        assert!(parse("GET /x HTTP/1.0", &["Remote-Role: a", "Remote-Role: b"]).is_err());
    }

    #[test]
    fn malformed_headers_and_rejected_headers_are_errors() {
        assert!(parse("GET /x HTTP/1.0", &["no-coline-here"]).is_err());
        assert!(parse("GET /x HTTP/1.0", &[" folded continuation"]).is_err());
        assert!(parse("GET /x HTTP/1.0", &["Transfer-Encoding: chunked"]).is_err());
        assert!(parse("GET /x HTTP/1.0", &["Content-Length: 5"]).is_err());
        assert!(parse("GET /x HTTP/1.0", &["Content-Length: 0"]).is_ok());
    }
}
