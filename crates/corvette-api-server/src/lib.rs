//! The identity-gated, read-only API over a Unix socket nginx alone can reach,
//! so `Remote-User`/`Remote-Role` cannot be spoofed from outside it.

use corvette_api::{ActivityResponse, ErrorBody, Health};
use std::borrow::Cow;
use std::fmt::Write as _;
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

pub mod activity;
pub mod clip_link;
pub mod config;
pub mod database;
pub mod detection;
pub mod detection_config;
pub mod download;
pub mod events;
pub mod playlist;
pub mod rebase;
pub mod recordings;
pub mod status;
pub mod still;

use crate::activity::{ActivityQuery, ActivityRows};
use crate::config::ConfigSource;
use crate::status::StatusState;

const API_PREFIX: &str = "/corvette/api/v1/";

pub const SOCKET_PATH_ENV: &str = "CORVETTE_API_SOCKET";

const MAX_REQUEST_HEADER_BYTES: usize = 8192;

pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

/// Seconds since the Unix epoch, the unit of every stored `start` and `end`.
type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

fn system_clock() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// One structured log line; this workspace has no logging framework.
pub(crate) fn log_event(unit: &str, event: &str, detail: impl std::fmt::Display) {
    eprintln!("corvette-api-server unit=\"{unit}\" event={event} detail=\"{detail}\"");
}

struct Request {
    method: String,
    path: String,
    query: String,
    body: Vec<u8>,
    body_len: Option<usize>,
    if_match: Option<String>,
    content_type: Option<String>,
    range: Option<String>,
    remote_user: Option<String>,
    remote_role: String,
}

struct Response {
    status: Cow<'static, str>,
    body: Vec<u8>,
    content_type: Cow<'static, str>,
    cache_control: Cow<'static, str>,
    extra: Vec<(Cow<'static, str>, String)>,
    stream: Option<download::StreamBody>,
}

impl Response {
    const fn json(status: &'static str, body: Vec<u8>) -> Self {
        Self {
            status: Cow::Borrowed(status),
            body,
            content_type: Cow::Borrowed("application/json"),
            cache_control: Cow::Borrowed("no-store"),
            extra: Vec::new(),
            stream: None,
        }
    }

    const fn jpeg(body: Vec<u8>) -> Self {
        Self {
            status: Cow::Borrowed("200 OK"),
            body,
            content_type: Cow::Borrowed("image/jpeg"),
            cache_control: Cow::Borrowed("no-store"),
            extra: Vec::new(),
            stream: None,
        }
    }
}

enum Ready {
    ClosedEarly,
    Answer {
        response: Box<Response>,
        drain_after_write: bool,
    },
}

/// Per-connection settings handed to every request the connection carries.
#[derive(Clone)]
struct ConnectionContext {
    timeout: Duration,
    detection: detection::Relay,
    config: Arc<ConfigSource>,
    db_path: PathBuf,
    index_path: PathBuf,
    activity_backing: activity::ActivityBacking,
    media_root: PathBuf,
    clock: Clock,
    status_state: Arc<StatusState>,
}

pub struct ApiServer {
    listener: UnixListener,
    socket_path: PathBuf,
    connection_timeout: Duration,
    config: Arc<ConfigSource>,
    db_path: PathBuf,
    index_path: PathBuf,
    activity_backing: activity::ActivityBacking,
    media_root: PathBuf,
    clock: Clock,
    status_state: Arc<StatusState>,
    detection: detection::Relay,
}

impl std::fmt::Debug for ApiServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiServer")
            .field("socket_path", &self.socket_path)
            .field("connection_timeout", &self.connection_timeout)
            .field("db_path", &self.db_path)
            .field("index_path", &self.index_path)
            .field("media_root", &self.media_root)
            .finish_non_exhaustive()
    }
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
            index_path: PathBuf::from(recordings::DEFAULT_INDEX_PATH),
            activity_backing: activity::ActivityBacking::default(),
            media_root: PathBuf::from(activity::DEFAULT_MEDIA_ROOT),
            clock: Arc::new(system_clock),
            status_state: Arc::new(StatusState::default()),
            detection: detection::Relay::default(),
        })
    }

    #[must_use]
    pub fn with_media_root(mut self, media_root: &Path) -> Self {
        self.media_root = media_root.to_path_buf();
        self
    }

    #[must_use]
    pub fn with_index_path(mut self, index_path: &Path) -> Self {
        self.index_path = index_path.to_path_buf();
        self
    }

    #[must_use]
    pub const fn with_activity_backing(mut self, backing: activity::ActivityBacking) -> Self {
        self.activity_backing = backing;
        self
    }

    #[must_use]
    pub fn with_detection_socket(mut self, path: PathBuf) -> Self {
        self.detection.socket = Some(path);
        self
    }

    #[must_use]
    pub const fn with_detection_timeouts(mut self, connect: Duration, total: Duration) -> Self {
        self.detection.timeouts = Some((connect, total));
        self
    }

    #[must_use]
    pub fn with_clock(mut self, clock: impl Fn() -> f64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
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
            index_path,
            activity_backing,
            media_root,
            clock,
            status_state,
            detection,
        } = self;
        let context = ConnectionContext {
            timeout: connection_timeout,
            detection,
            config,
            db_path,
            index_path,
            activity_backing,
            media_root,
            clock,
            status_state,
        };
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, _peer)) => {
                        let context = context.clone();
                        tokio::spawn(async move {
                            handle_connection(stream, &context).await;
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

async fn handle_connection(mut stream: UnixStream, context: &ConnectionContext) {
    // The connection deadline covers everything up to the answer being ready to write.
    // A streamed body then continues under the per-write inactivity bound in `download`.
    let deadline = tokio::time::Instant::now() + context.timeout;
    match tokio::time::timeout_at(deadline, read_and_route(&mut stream, context)).await {
        Err(_elapsed) => connection_timeout_log(context),
        Ok(Ready::ClosedEarly) => log_event(
            "http",
            "request-incomplete",
            "peer closed before a complete request header block arrived",
        ),
        Ok(Ready::Answer {
            mut response,
            drain_after_write,
        }) => {
            if response.stream.is_some() {
                if let Err(err) = write_response(&mut stream, &mut response).await {
                    log_event("http", "response-write-error", err);
                }
            } else {
                match tokio::time::timeout_at(deadline, write_response(&mut stream, &mut response))
                    .await
                {
                    Err(_elapsed) => connection_timeout_log(context),
                    Ok(Err(err)) => log_event("http", "response-write-error", err),
                    Ok(Ok(())) => {}
                }
            }
            if drain_after_write {
                drain_buffered(&stream);
            }
        }
    }
}

fn connection_timeout_log(context: &ConnectionContext) {
    log_event(
        "http",
        "connection-timeout",
        format!(
            "peer did not complete a request within {:?}; closed to avoid leaking the connection",
            context.timeout
        ),
    );
}

async fn read_and_route(stream: &mut UnixStream, context: &ConnectionContext) -> Ready {
    let (block, body_prefix) = match read_header_block(stream).await {
        HeaderRead::Complete(block, prefix) => (block, prefix),
        HeaderRead::TooLarge => {
            return Ready::Answer {
                response: Box::new(bad_request("request header block exceeded the size limit")),
                drain_after_write: true,
            };
        }
        HeaderRead::Closed => return Ready::ClosedEarly,
    };
    let mut request = match parse_request(&block) {
        Ok(request) => request,
        Err(reason) => {
            return Ready::Answer {
                response: Box::new(bad_request(&reason)),
                drain_after_write: false,
            };
        }
    };
    if let Some(length) = request
        .body_len
        .filter(|length| *length > 0 && *length <= detection_config::MAX_BODY_BYTES)
    {
        let mut body = body_prefix;
        body.truncate(length);
        let missing = length - body.len();
        if missing > 0 {
            let mut tail = vec![0_u8; missing];
            match stream.read_exact(&mut tail).await {
                Ok(_read) => body.extend_from_slice(&tail),
                Err(err) => {
                    log_event("http", "request-read-error", err);
                    return Ready::ClosedEarly;
                }
            }
        }
        request.body = body;
    }
    let drain_after_write = request
        .body_len
        .is_some_and(|len| len > detection_config::MAX_BODY_BYTES);
    let response = route(&request, context).await;
    Ready::Answer {
        response: Box::new(response),
        drain_after_write,
    }
}

enum HeaderRead {
    Complete(Vec<u8>, Vec<u8>),
    TooLarge,
    Closed,
}

async fn read_header_block(stream: &mut UnixStream) -> HeaderRead {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; MAX_REQUEST_HEADER_BYTES];
    loop {
        if let Some(end) = find_header_end(&buffer) {
            // Body bytes can already sit in the buffer after the terminator.
            let prefix = buffer.split_off(end + 4);
            buffer.truncate(end);
            return HeaderRead::Complete(buffer, prefix);
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

/// The body length one `Content-Length` line announces; overflow counts as over the bound.
fn declared_body_length(value: &[u8]) -> Result<usize, String> {
    let digits = utf8(value).ok_or_else(|| "Content-Length is not valid UTF-8".to_owned())?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Content-Length must be a plain number for a request with a body".to_owned());
    }
    Ok(digits.parse::<usize>().unwrap_or(usize::MAX))
}

/// Stores a single-occurrence header value; the second line of the same name is an error.
fn capture_once(seen: &mut Option<String>, value: &[u8], label: &str) -> Result<(), String> {
    if seen.is_some() {
        return Err(format!("more than one {label} header"));
    }
    let text = utf8(value).ok_or_else(|| format!("{label} value is not valid UTF-8"))?;
    *seen = Some(text.to_owned());
    Ok(())
}

fn parse_request(block: &[u8]) -> Result<Request, String> {
    let mut lines = split_header_lines(block).into_iter();
    let request_line = lines
        .next()
        .filter(|line| !line.is_empty())
        .ok_or_else(|| "missing request line".to_owned())?;
    let (method, path, query) = parse_request_line(request_line)?;
    let body_method = matches!(method, "PUT" | "POST");

    let mut remote_user: Option<String> = None;
    let mut remote_role: Option<String> = None;
    let mut user_count = 0_u32;
    let mut role_count = 0_u32;
    let mut body_len: Option<usize> = None;
    let mut if_match: Option<String> = None;
    let mut content_type: Option<String> = None;
    let mut range: Option<String> = None;

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
                if body_method {
                    if body_len.is_some() {
                        return Err("more than one Content-Length header".to_owned());
                    }
                    body_len = Some(declared_body_length(value)?);
                } else if value != b"0" {
                    return Err("Content-Length other than 0 is not accepted".to_owned());
                }
            }
            b"if-match" => capture_once(&mut if_match, value, "If-Match")?,
            b"content-type" => capture_once(&mut content_type, value, "Content-Type")?,
            b"range" => capture_once(&mut range, value, "Range")?,
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
        method: method.to_owned(),
        path: path.to_owned(),
        query: query.to_owned(),
        body: Vec::new(),
        body_len,
        if_match,
        content_type,
        range,
        remote_user,
        remote_role: remote_role.unwrap_or_default(),
    })
}

fn parse_request_line(line: &[u8]) -> Result<(&str, &str, &str), String> {
    let line =
        std::str::from_utf8(line).map_err(|_| "request line is not valid UTF-8".to_owned())?;
    let parts: Vec<&str> = line.split(' ').collect();
    if parts.len() != 3 {
        return Err("request line is not `GET <target> HTTP/1.0`".to_owned());
    }
    let target_ok = parts[1].starts_with('/');
    let body_method = matches!(parts[0], "PUT" | "POST");
    if parts[0] != "GET" && !(body_method && target_ok && detection_config::serves(parts[1])) {
        return Err(format!("method {} is not GET", parts[0]));
    }
    if parts[2] != "HTTP/1.0" && parts[2] != "HTTP/1.1" {
        return Err(format!("version {} is not HTTP/1.0 or HTTP/1.1", parts[2]));
    }
    let target = parts[1];
    if !target.starts_with('/') {
        return Err("request target does not start with /".to_owned());
    }
    let path_and_query = match target.split_once('?') {
        Some((path, query)) => (path, query),
        None => (target, ""),
    };
    Ok((parts[0], path_and_query.0, path_and_query.1))
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
    CameraStatus,
    Cameras,
    ClipLink,
    DetectionStatus,
    Events,
    Health,
    Recordings,
}

impl Route {
    pub const ALL: [Self; 8] = [
        Self::Activity,
        Self::CameraStatus,
        Self::Cameras,
        Self::ClipLink,
        Self::DetectionStatus,
        Self::Events,
        Self::Health,
        Self::Recordings,
    ];

    #[must_use]
    pub fn path(self) -> String {
        match self {
            Self::Activity => format!("{API_PREFIX}activity"),
            Self::CameraStatus => format!("{API_PREFIX}cameras/status"),
            Self::Cameras => format!("{API_PREFIX}cameras"),
            Self::ClipLink => format!("{API_PREFIX}clip-link"),
            Self::DetectionStatus => format!("{API_PREFIX}detection/status"),
            Self::Events => format!("{API_PREFIX}events"),
            Self::Health => format!("{API_PREFIX}health"),
            Self::Recordings => format!("{API_PREFIX}recordings"),
        }
    }
}

async fn route(request: &Request, context: &ConnectionContext) -> Response {
    let ConnectionContext {
        detection,
        config,
        db_path,
        index_path,
        activity_backing,
        media_root,
        clock,
        status_state,
        ..
    } = context;
    let Some(user) = request
        .remote_user
        .as_deref()
        .filter(|user| !user.is_empty())
    else {
        return unauthorized();
    };
    if detection_config::serves(&request.path) {
        return detection_config::forward(request, user, detection).await;
    }
    if let Some(segment) = still::camera_segment(&request.path) {
        return still_picture(request, config, segment, clock, status_state).await;
    }
    if let Some(segment) = detection::objects_segment(&request.path) {
        return detection::objects(request, config, segment, detection).await;
    }
    if let Some(media) = playlist::media_parts(&request.path) {
        return playlist::handle(request, config, index_path, media).await;
    }
    if let Some(segment) = events::review_id_segment(&request.path) {
        return event_detail(request, config, index_path, segment, clock).await;
    }
    let Some(route) = Route::ALL
        .into_iter()
        .find(|candidate| candidate.path() == request.path)
    else {
        return not_found();
    };
    match route {
        Route::Activity => {
            activity(
                request,
                config,
                db_path,
                index_path,
                media_root,
                *activity_backing,
                clock,
            )
            .await
        }
        Route::CameraStatus => {
            camera_status(
                config,
                status_state,
                index_path,
                &request.remote_role,
                clock,
            )
            .await
        }
        Route::Cameras => cameras(config, &request.remote_role).await,
        Route::ClipLink => clip_link(request, config, db_path, clock).await,
        Route::DetectionStatus => detection::status(config, &request.remote_role, detection).await,
        Route::Events => events_list(request, config, index_path, clock).await,
        Route::Health => health(config, db_path).await,
        Route::Recordings => recordings(request, config, index_path).await,
    }
}

async fn cameras(config: &ConfigSource, role: &str) -> Response {
    match config.camera_list_json(role).await {
        Ok(body) => Response::json("200 OK", body),
        Err(err) => config_unavailable(&err),
    }
}

async fn camera_status(
    config: &ConfigSource,
    status_state: &StatusState,
    index_path: &Path,
    role: &str,
    clock: &Clock,
) -> Response {
    let entries = match config.camera_entries_for_role(role).await {
        Ok(entries) => entries,
        Err(err) => return config_unavailable(&err),
    };
    let fps = match status::fetch_camera_fps(config.backing_addr()).await {
        Ok(fps) => fps,
        Err(reason) => {
            log_event("status", "fetch-error", &reason);
            return status_unavailable();
        }
    };
    let now = clock();
    // rusqlite is synchronous: the recording-index read moves off the async worker.
    let camera_names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
    let index_path = index_path.to_path_buf();
    let read = tokio::task::spawn_blocking(move || {
        recordings::recording_states(&index_path, &camera_names, now)
    })
    .await;
    let recording = if let Ok(Ok(flags)) = read {
        if status_state.note_index_success() {
            log_event("status", "recording-index-readable", "");
        }
        flags
    } else {
        if status_state.note_index_failure() {
            log_event("status", "recording-index-unreadable", "");
        }
        vec![false; entries.len()]
    };
    Response::json(
        "200 OK",
        status::status_body(&entries, &fps, status_state, &recording, now),
    )
}

async fn recordings(request: &Request, config: &ConfigSource, index_path: &Path) -> Response {
    let query = match recordings::RecordingsQuery::parse(&request.query) {
        Ok(query) => query,
        Err(reason) => return bad_request(&reason),
    };
    let allowed = match config.allowed_camera_names(&request.remote_role).await {
        Ok(cameras) => cameras,
        Err(err) => return config_unavailable(&err),
    };
    if !allowed.contains(&query.camera) {
        return no_footage();
    }
    // rusqlite is synchronous: the index read moves off the async worker.
    let index_path = index_path.to_path_buf();
    let camera = query.camera.clone();
    let start_ms = recordings::to_millis(query.start);
    let end_ms = recordings::to_millis(query.end);
    let read =
        tokio::task::spawn_blocking(move || -> Result<Vec<(i64, i64)>, recordings::IndexError> {
            let connection = database::open_read_only(&index_path)?;
            recordings::overlapping_ranges(&connection, &camera, start_ms, end_ms)
        })
        .await;
    match read {
        Ok(Ok(ranges)) => Response::json("200 OK", recordings::ranges_body(&query.camera, &ranges)),
        Ok(Err(_)) | Err(_) => recording_unavailable(),
    }
}

async fn events_list(
    request: &Request,
    config: &ConfigSource,
    index_path: &Path,
    clock: &Clock,
) -> Response {
    let query = match events::EventsQuery::parse(&request.query) {
        Ok(query) => query,
        Err(reason) => return bad_request(&reason),
    };
    let mut cameras = match config.allowed_camera_names(&request.remote_role).await {
        Ok(cameras) => cameras,
        Err(err) => return config_unavailable(&err),
    };
    if let Some(requested) = &query.cameras {
        cameras.retain(|name| requested.contains(name));
    }
    if cameras.is_empty() {
        return Response::json("200 OK", events::empty_list_body());
    }
    let now = clock();
    // rusqlite is synchronous: the index read moves off the async worker.
    let index_path = index_path.to_path_buf();
    let read = tokio::task::spawn_blocking(move || -> rusqlite::Result<Vec<u8>> {
        let connection = database::open_read_only(&index_path)?;
        events::list_body(&connection, &query, &cameras, now)
    })
    .await;
    match read {
        Ok(Ok(body)) => Response::json("200 OK", body),
        Ok(Err(_)) | Err(_) => recording_unavailable(),
    }
}

async fn event_detail(
    request: &Request,
    config: &ConfigSource,
    index_path: &Path,
    segment: &str,
    clock: &Clock,
) -> Response {
    if let Err(reason) = activity::parse_parameters(&request.query, &[]) {
        return bad_request(&reason);
    }
    let Ok(id) = segment.parse::<i64>() else {
        return review_not_found();
    };
    let cameras = match config.allowed_camera_names(&request.remote_role).await {
        Ok(cameras) => cameras,
        Err(err) => return config_unavailable(&err),
    };
    let now = clock();
    // rusqlite is synchronous: the index read moves off the async worker.
    let index_path = index_path.to_path_buf();
    let read = tokio::task::spawn_blocking(move || -> rusqlite::Result<Option<Vec<u8>>> {
        let connection = database::open_read_only(&index_path)?;
        events::detail_body(&connection, id, &cameras, now)
    })
    .await;
    match read {
        Ok(Ok(Some(body))) => Response::json("200 OK", body),
        Ok(Ok(None)) => review_not_found(),
        Ok(Err(_)) | Err(_) => recording_unavailable(),
    }
}

async fn still_picture(
    request: &Request,
    config: &ConfigSource,
    segment: &str,
    clock: &Clock,
    status_state: &StatusState,
) -> Response {
    let camera = match activity::percent_decode(segment) {
        Ok(camera) => camera,
        Err(reason) => return bad_request(&reason),
    };
    let height = match still::parse_height(&request.query) {
        Ok(height) => height,
        Err(reason) => return bad_request(&reason),
    };
    let entry = match config.camera_for_role(&request.remote_role, &camera).await {
        Ok(Some(entry)) => entry,
        Ok(None) => return camera_not_found(),
        Err(err) => return config_unavailable(&err),
    };
    if !entry.enabled {
        return no_picture();
    }
    match still::fetch_still(config.backing_addr(), &camera, height).await {
        Ok(bytes) => {
            status_state.still_delivered(&entry.name);
            Response::jpeg(bytes)
        }
        Err(detail) => {
            status_state.still_failed(&entry.name, clock());
            log_event("still", "fetch-error", &detail);
            picture_unavailable()
        }
    }
}

async fn activity(
    request: &Request,
    config: &ConfigSource,
    db_path: &Path,
    index_path: &Path,
    media_root: &Path,
    backing: activity::ActivityBacking,
    clock: &Clock,
) -> Response {
    let now = clock();
    let query = match ActivityQuery::parse(&request.query) {
        Ok(query) => query,
        Err(reason) => return bad_request(&reason),
    };
    let mut cameras = match config.allowed_camera_names(&request.remote_role).await {
        Ok(cameras) => cameras,
        Err(err) => return config_unavailable(&err),
    };
    if let Some(requested) = &query.cameras {
        cameras.retain(|name| requested.contains(name));
    }
    let body = if cameras.is_empty() {
        serde_json::to_vec(&activity::activity_response(
            &query,
            ActivityRows::default(),
            media_root,
            now,
        ))
        .expect("activity body serializes")
    } else {
        match backing {
            activity::ActivityBacking::Frigate => {
                // rusqlite and the media `stat` calls are synchronous: off the async worker.
                let db_path = db_path.to_path_buf();
                let media_root = media_root.to_path_buf();
                let read =
                    tokio::task::spawn_blocking(move || -> rusqlite::Result<ActivityResponse> {
                        let mut connection = database::open_read_only(&db_path)?;
                        let rows = activity::read_activity(&mut connection, &query, &cameras, now)?;
                        drop(connection);
                        Ok(activity::activity_response(&query, rows, &media_root, now))
                    })
                    .await;
                let response = match read {
                    Ok(Ok(response)) => response,
                    Ok(Err(err)) => {
                        log_event("database", "activity-read-error", err);
                        return database_unavailable();
                    }
                    Err(err) => {
                        log_event("database", "activity-task-error", err);
                        return database_unavailable();
                    }
                };
                // ActivityResponse is strings and numbers only; this cannot fail to serialize.
                serde_json::to_vec(&response).expect("activity body serializes")
            }
            activity::ActivityBacking::Corvette => {
                let index_path = index_path.to_path_buf();
                let read = tokio::task::spawn_blocking(move || -> rusqlite::Result<Vec<u8>> {
                    let connection = database::open_read_only(&index_path)?;
                    activity::corvette_body(&connection, &query, &cameras, now)
                })
                .await;
                match read {
                    Ok(Ok(body)) => body,
                    Ok(Err(_)) | Err(_) => return recording_unavailable(),
                }
            }
        }
    };
    Response::json("200 OK", body)
}

async fn clip_link(
    request: &Request,
    config: &ConfigSource,
    db_path: &Path,
    clock: &Clock,
) -> Response {
    let query = match clip_link::ClipLinkQuery::parse(&request.query) {
        Ok(query) => query,
        Err(reason) => return bad_request(&reason),
    };
    let allowed = match config.allowed_camera_names(&request.remote_role).await {
        Ok(cameras) => cameras,
        Err(err) => return config_unavailable(&err),
    };
    if !allowed.contains(&query.camera) {
        return no_footage();
    }
    let db_path = db_path.to_path_buf();
    let camera = query.camera.clone();
    let read = tokio::task::spawn_blocking(move || -> rusqlite::Result<bool> {
        let connection = database::open_read_only(&db_path)?;
        clip_link::has_recorded_footage(&connection, &camera, query.start, query.end)
    })
    .await;
    let has_footage = match read {
        Ok(Ok(has_footage)) => has_footage,
        Ok(Err(err)) => {
            log_event("database", "clip-link-read-error", err);
            return database_unavailable();
        }
        Err(err) => {
            log_event("database", "clip-link-task-error", err);
            return database_unavailable();
        }
    };
    if !has_footage {
        return no_footage();
    }
    // MediaLink is one String field; its derived Serialize never fails.
    Response::json(
        "200 OK",
        serde_json::to_vec(&activity::playlist_link(
            &query.camera,
            activity::padded_clip_start(query.start),
            activity::padded_clip_end(query.end, clock()),
        ))
        .expect("media link serializes"),
    )
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
    Response::json(
        "200 OK",
        serde_json::to_vec(&body).expect("health body serializes"),
    )
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
    Response::json(
        status,
        serde_json::to_vec(&body).expect("error body serializes"),
    )
}

fn bad_request(message: &str) -> Response {
    error_response("400 Bad Request", "bad_request", message)
}

fn config_unavailable(err: &impl std::fmt::Display) -> Response {
    error_response(
        "503 Service Unavailable",
        "config_unavailable",
        &format!("Frigate configuration is unavailable: {err}"),
    )
}

fn database_unavailable() -> Response {
    error_response(
        "503 Service Unavailable",
        "database_unavailable",
        "Frigate's database could not be read.",
    )
}

fn recording_unavailable() -> Response {
    error_response(
        "503 Service Unavailable",
        "recording_unavailable",
        "The recording index could not be read.",
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

fn no_footage() -> Response {
    error_response(
        "404 Not Found",
        "not_found",
        "No recorded footage is available for this camera and range.",
    )
}

fn camera_not_found() -> Response {
    error_response("404 Not Found", "not_found", "No such camera.")
}

fn review_not_found() -> Response {
    error_response("404 Not Found", "not_found", "No such review.")
}

fn no_picture() -> Response {
    error_response(
        "404 Not Found",
        "no_picture",
        "The camera is off and has no picture.",
    )
}

fn picture_unavailable() -> Response {
    error_response(
        "503 Service Unavailable",
        "picture_unavailable",
        "The camera's picture could not be fetched.",
    )
}

fn status_unavailable() -> Response {
    error_response(
        "503 Service Unavailable",
        "status_unavailable",
        "The camera status could not be read.",
    )
}

/// Drains buffered bytes so the close sends a FIN, not an RST that drops the response.
fn drain_buffered(stream: &UnixStream) {
    let mut scratch = [0_u8; 4096];
    while stream.try_read(&mut scratch).is_ok_and(|read| read > 0) {}
}

async fn write_response(stream: &mut UnixStream, response: &mut Response) -> std::io::Result<()> {
    if let Some(body) = response.stream.take() {
        return download::write_streamed(stream, response, body, download::WRITE_INACTIVITY).await;
    }
    let mut header = format!(
        "HTTP/1.0 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: {}\r\n",
        response.status,
        response.content_type,
        response.body.len(),
        response.cache_control
    );
    for (name, value) in &response.extra {
        let _ = write!(header, "{name}: {value}\r\n");
    }
    header.push_str("Connection: close\r\n\r\n");
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
