//! The identity-gated, read-only API over a Unix socket nginx alone can reach,
//! so `Remote-User`/`Remote-Role` cannot be spoofed from outside it.

use corvette_api::{ActivityResponse, ErrorBody, Health};
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

pub mod activity;
pub mod config;
pub mod database;

use crate::activity::{ActivityQuery, ActivityRows};
use crate::config::ConfigSource;

const API_PREFIX: &str = "/corvette/api/v1/";

pub const SOCKET_PATH_ENV: &str = "CORVETTE_API_SOCKET";

const MAX_REQUEST_HEADER_BYTES: usize = 8192;

pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

/// One structured log line; this workspace has no logging framework.
pub(crate) fn log_event(unit: &str, event: &str, detail: impl std::fmt::Display) {
    eprintln!("corvette-api-server unit=\"{unit}\" event={event} detail=\"{detail}\"");
}

struct Request {
    path: String,
    query: String,
    remote_user: Option<String>,
    remote_role: String,
}

struct Response {
    status: &'static str,
    body: Vec<u8>,
}

enum Outcome {
    Responded,
    ClosedEarly,
}

#[derive(Debug)]
pub struct ApiServer {
    listener: UnixListener,
    socket_path: PathBuf,
    connection_timeout: Duration,
    config: Arc<ConfigSource>,
    db_path: PathBuf,
    media_root: PathBuf,
}

impl ApiServer {
    /// Binds the socket at `socket_path`, removing a stale file first; accepts
    /// nothing until [`Self::serve_until`]. `db_path` is opened read-only.
    ///
    /// # Errors
    ///
    /// Returns an error if removing a stale file, binding, or setting permissions fails.
    pub fn bind(
        socket_path: &Path,
        connection_timeout: Duration,
        config: Arc<ConfigSource>,
        db_path: &Path,
    ) -> std::io::Result<Self> {
        // A crashed run can leave the socket file here; bind would fail on it.
        if socket_path.exists() {
            std::fs::remove_file(socket_path)?;
        }
        let listener = UnixListener::bind(socket_path)?;
        // nginx lacks CAP_DAC_OVERRIDE, so it needs write access to the socket
        // file itself; the socket's directory is the access boundary.
        std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o666))?;
        Ok(Self {
            listener,
            socket_path: socket_path.to_path_buf(),
            connection_timeout,
            config,
            db_path: db_path.to_path_buf(),
            media_root: PathBuf::from(activity::DEFAULT_MEDIA_ROOT),
        })
    }

    #[must_use]
    pub fn with_media_root(mut self, media_root: &Path) -> Self {
        self.media_root = media_root.to_path_buf();
        self
    }

    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub async fn serve_until(self, shutdown: impl Future<Output = ()> + Send) {
        let Self {
            listener,
            socket_path,
            connection_timeout,
            config,
            db_path,
            media_root,
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
                        let media_root = media_root.clone();
                        tokio::spawn(async move {
                            handle_connection(
                                stream,
                                connection_timeout,
                                config,
                                &db_path,
                                &media_root,
                            )
                            .await;
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

pub async fn shutdown_signal() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
        }
        Err(_) => {
            // No SIGTERM on this platform: stop on Ctrl-C alone.
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

async fn handle_connection(
    mut stream: UnixStream,
    timeout: Duration,
    config: Arc<ConfigSource>,
    db_path: &Path,
    media_root: &Path,
) {
    let outcome = tokio::time::timeout(
        timeout,
        read_route_respond(&mut stream, &config, db_path, media_root),
    )
    .await;
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

async fn read_route_respond(
    stream: &mut UnixStream,
    config: &ConfigSource,
    db_path: &Path,
    media_root: &Path,
) -> std::io::Result<Outcome> {
    let block = match read_header_block(stream).await {
        HeaderRead::Complete(block) => block,
        HeaderRead::TooLarge => {
            let response = bad_request("request header block exceeded the size limit");
            write_response(stream, &response).await?;
            drain_buffered(stream);
            return Ok(Outcome::Responded);
        }
        HeaderRead::Closed => return Ok(Outcome::ClosedEarly),
    };
    let response = match parse_request(&block) {
        Ok(request) => route(&request, config, db_path, media_root).await,
        Err(reason) => bad_request(&reason),
    };
    write_response(stream, &response).await?;
    Ok(Outcome::Responded)
}

enum HeaderRead {
    Complete(Vec<u8>),
    TooLarge,
    Closed,
}

async fn read_header_block(stream: &mut UnixStream) -> HeaderRead {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; MAX_REQUEST_HEADER_BYTES];
    loop {
        if let Some(end) = find_header_end(&buffer) {
            buffer.truncate(end);
            return HeaderRead::Complete(buffer);
        }
        // Reads are sized to the room left, so a terminator past the limit is never seen.
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

pub(crate) fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

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

fn parse_request(block: &[u8]) -> Result<Request, String> {
    let mut lines = split_header_lines(block).into_iter();
    let request_line = lines
        .next()
        .filter(|line| !line.is_empty())
        .ok_or_else(|| "missing request line".to_owned())?;
    let (path, query) = parse_request_line(request_line)?;

    let mut remote_user: Option<String> = None;
    let mut remote_role: Option<String> = None;
    let mut user_count = 0_u32;
    let mut role_count = 0_u32;

    for line in lines {
        // A leading space or tab is an obs-fold continuation; never accepted.
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
        query: query.to_owned(),
        remote_user,
        remote_role: remote_role.unwrap_or_default(),
    })
}

fn parse_request_line(line: &[u8]) -> Result<(&str, &str), String> {
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
    Ok(match target.split_once('?') {
        Some((path, query)) => (path, query),
        None => (target, ""),
    })
}

pub(crate) fn split_at_byte(bytes: &[u8], byte: u8) -> Option<(&[u8], &[u8])> {
    bytes
        .iter()
        .position(|candidate| *candidate == byte)
        .map(|index| (&bytes[..index], &bytes[index + 1..]))
}

fn is_valid_header_name(name: &[u8]) -> bool {
    !name.is_empty()
        && !name
            .iter()
            .any(|byte| *byte < 0x20 || *byte == 0x7F || *byte == b' ')
}

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

pub(crate) fn utf8(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes).ok()
}

/// Every route the service serves; the router matches nothing else.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Route {
    Activity,
    Cameras,
    Health,
}

impl Route {
    pub const ALL: [Self; 3] = [Self::Activity, Self::Cameras, Self::Health];

    #[must_use]
    pub fn path(self) -> String {
        match self {
            Self::Activity => format!("{API_PREFIX}activity"),
            Self::Cameras => format!("{API_PREFIX}cameras"),
            Self::Health => format!("{API_PREFIX}health"),
        }
    }
}

async fn route(
    request: &Request,
    config: &ConfigSource,
    db_path: &Path,
    media_root: &Path,
) -> Response {
    let Some(_user) = request
        .remote_user
        .as_deref()
        .filter(|user| !user.is_empty())
    else {
        return unauthorized();
    };
    let Some(route) = Route::ALL
        .into_iter()
        .find(|candidate| candidate.path() == request.path)
    else {
        return not_found();
    };
    match route {
        Route::Activity => activity(request, config, db_path, media_root).await,
        Route::Cameras => cameras(config, &request.remote_role).await,
        Route::Health => health(config, db_path).await,
    }
}

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

async fn activity(
    request: &Request,
    config: &ConfigSource,
    db_path: &Path,
    media_root: &Path,
) -> Response {
    let query = match ActivityQuery::parse(&request.query) {
        Ok(query) => query,
        Err(reason) => return bad_request(&reason),
    };
    let mut cameras = match config.allowed_camera_names(&request.remote_role).await {
        Ok(cameras) => cameras,
        Err(err) => {
            return error_response(
                "503 Service Unavailable",
                "config_unavailable",
                &format!("Frigate configuration is unavailable: {err}"),
            );
        }
    };
    if let Some(requested) = &query.cameras {
        cameras.retain(|name| requested.contains(name));
    }
    let response = if cameras.is_empty() {
        activity::activity_response(&query, ActivityRows::default(), media_root)
    } else {
        // rusqlite and the media `stat` calls are synchronous: off the async worker.
        let db_path = db_path.to_path_buf();
        let media_root = media_root.to_path_buf();
        let read = tokio::task::spawn_blocking(move || -> rusqlite::Result<ActivityResponse> {
            let mut connection = database::open_read_only(&db_path)?;
            let rows = activity::read_activity(&mut connection, &query, &cameras)?;
            drop(connection);
            Ok(activity::activity_response(&query, rows, &media_root))
        })
        .await;
        match read {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                log_event("database", "activity-read-error", err);
                return database_unavailable();
            }
            Err(err) => {
                log_event("database", "activity-task-error", err);
                return database_unavailable();
            }
        }
    };
    // ActivityResponse is strings and numbers only; this cannot fail to serialize.
    Response {
        status: "200 OK",
        body: serde_json::to_vec(&response).expect("activity body serializes"),
    }
}

async fn health(config: &ConfigSource, db_path: &Path) -> Response {
    let mut failing = Vec::new();
    if let Err(err) = probe_database(db_path).await {
        log_event("database", "probe-error", err);
        failing.push("database unreadable");
    }
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

fn database_unavailable() -> Response {
    error_response(
        "503 Service Unavailable",
        "database_unavailable",
        "Frigate's database could not be read.",
    )
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

/// Drains buffered bytes so the close sends a FIN, not an RST that drops the response.
fn drain_buffered(stream: &UnixStream) {
    let mut scratch = [0_u8; 4096];
    while stream.try_read(&mut scratch).is_ok_and(|read| read > 0) {}
}

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
        assert_eq!(request.query, "");
        assert_eq!(request.remote_user, None);
        assert_eq!(request.remote_role, "");
    }

    #[test]
    fn request_line_splits_the_path_from_the_query() {
        let request = parse("GET /corvette/api/v1/cameras?a=b HTTP/1.1", &[]).expect("valid");
        assert_eq!(request.path, "/corvette/api/v1/cameras");
        assert_eq!(request.query, "a=b");
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
