//! Minimal HTTP/1.1 over a Unix socket, one request per connection: one camera's objects answer, the detection status answer, and the configuration routes; the API server maps any non-`200` here to its public `503 detection_unavailable`.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;

use crate::camera_runner::CameraHandle;
use crate::enabled::{
    CameraSlot, DEFAULT_TOGGLE_JOIN_TIMEOUT, SlotState, ToggleService, slot_state,
};
use crate::object_stage::{CameraObjects, FrameObjects};
use crate::service_bodies::{ObjectsAnswer, WorkerReport, objects_answer, off_entry, status_entry};
use crate::supervised::Status;
use crate::track_events::{EventFeed, EventStream};

const MAX_HEAD_BYTES: usize = 8192;
const STATUS_PATH: &str = "/corvette/api/v1/detection/status";
const EVENTS_PATH: &str = "/corvette/api/v1/events";
const CONFIG_PATH: &str = "/corvette/api/v1/detection/config";
const VALIDATE_PATH: &str = "/corvette/api/v1/detection/config/validate";
const SCHEMA_PATH: &str = "/corvette/api/v1/detection/config/schema";
const CAMERAS_PREFIX: &str = "/corvette/api/v1/cameras/";
const OBJECTS_SUFFIX: &str = "/objects";
const ENABLED_SUFFIX: &str = "/enabled";
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub enum CameraEntry {
    Off,
    Live(CameraHandle),
}

#[derive(Debug)]
pub struct ServiceState {
    pub(crate) cameras: Arc<Mutex<Vec<Arc<CameraSlot>>>>,
    pub(crate) closing: Arc<AtomicBool>,
    pub(crate) stop_timeout: Duration,
    worker: Option<watch::Receiver<Status>>,
    config_path: Option<PathBuf>,
    apply_lock: Arc<Mutex<()>>,
}

impl ServiceState {
    #[must_use]
    pub fn new(cameras: Vec<(String, CameraEntry)>) -> Self {
        Self {
            cameras: Arc::new(Mutex::new(
                cameras
                    .into_iter()
                    .map(|(name, entry)| {
                        Arc::new(CameraSlot {
                            name,
                            channels: None,
                            recipe: None,
                            toggle: tokio::sync::Mutex::new(()),
                            stopping: AtomicBool::new(false),
                            state: Mutex::new(SlotState {
                                entry,
                                threads: None,
                            }),
                        })
                    })
                    .collect(),
            )),
            closing: Arc::new(AtomicBool::new(false)),
            stop_timeout: DEFAULT_TOGGLE_JOIN_TIMEOUT,
            worker: None,
            config_path: None,
            apply_lock: Arc::new(Mutex::new(())),
        }
    }

    #[must_use]
    pub const fn with_stop_timeout(mut self, stop_timeout: Duration) -> Self {
        self.stop_timeout = stop_timeout;
        self
    }

    #[must_use]
    pub fn with_worker(self, worker: watch::Receiver<Status>) -> Self {
        Self {
            worker: Some(worker),
            ..self
        }
    }

    #[must_use]
    pub fn with_config_path(self, config_path: PathBuf) -> Self {
        Self {
            config_path: Some(config_path),
            ..self
        }
    }

    pub(crate) fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// Process-wide apply lock: held from the revision check through the file rename, so no second apply reads a stale revision underneath a write.
    pub(crate) fn apply_lock(&self) -> &Mutex<()> {
        &self.apply_lock
    }
}

/// Accept connections until one `accept` fails; each connection is answered by its own task, closed after one request except an event stream, which pumps until it ends.
///
/// When `events` is `Some`, this listener serves `GET /corvette/api/v1/events` from that feed and, when `toggles` is `Some`, `POST /corvette/api/v1/cameras/<name>/enabled`; a `None` listener answers both `404`.
/// # Errors
/// Returns `Err` when `accept` fails.
pub async fn serve(
    listener: UnixListener,
    state: Arc<ServiceState>,
    events: Option<Arc<EventFeed>>,
    toggles: Option<ToggleService>,
) -> std::io::Result<()> {
    serve_with_timeout(listener, state, DEFAULT_READ_TIMEOUT, events, toggles).await
}

/// # Errors
/// Returns `Err` when `accept` fails.
pub async fn serve_with_timeout(
    listener: UnixListener,
    state: Arc<ServiceState>,
    read_timeout: Duration,
    events: Option<Arc<EventFeed>>,
    toggles: Option<ToggleService>,
) -> std::io::Result<()> {
    loop {
        let (stream, _peer) = listener.accept().await?;
        let state = Arc::clone(&state);
        let events = events.clone();
        let toggles = toggles.clone();
        tokio::spawn(async move {
            answer_one(
                stream,
                &state,
                events.as_ref(),
                toggles.as_ref(),
                read_timeout,
            )
            .await;
        });
    }
}

enum HeadRead {
    TooLarge,
    Closed,
}

enum BodyRead {
    Complete(Vec<u8>),
    Missing,
    Broken,
}

/// Reads the request head and any body bytes already buffered with it; the body prefix follows the terminator.
async fn read_head(stream: &mut UnixStream) -> Result<(Vec<u8>, Vec<u8>), HeadRead> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let prefix = buffer.split_off(end + 4);
            buffer.truncate(end);
            return Ok((buffer, prefix));
        }
        // Reads are sized to the room left, so a terminator past the limit is never seen.
        let remaining = MAX_HEAD_BYTES - buffer.len();
        if remaining == 0 {
            return Err(HeadRead::TooLarge);
        }
        let room = remaining.min(chunk.len());
        match stream.read(&mut chunk[..room]).await {
            Ok(0) => return Err(HeadRead::Closed),
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(err) => {
                crate::log_event("service", "head-read-error", err);
                return Err(HeadRead::Closed);
            }
        }
    }
}

enum Answered {
    Response(String),
    Stream(Arc<EventFeed>),
}

async fn answer_one(
    mut stream: UnixStream,
    state: &ServiceState,
    events: Option<&Arc<EventFeed>>,
    toggles: Option<&ToggleService>,
    read_timeout: Duration,
) {
    let Some(answered) = answer(&mut stream, state, events, toggles, read_timeout).await else {
        return;
    };
    match answered {
        Answered::Response(response) => {
            if let Err(err) = write_response(&mut stream, &response).await {
                crate::log_event("service", "response-write-error", err);
            }
        }
        Answered::Stream(feed) => stream_events(&mut stream, &feed).await,
    }
}

/// Parses one request and prepares its answer; the validation route needs its body first, and `None` ends the connection unanswered.
async fn answer(
    stream: &mut UnixStream,
    state: &ServiceState,
    events: Option<&Arc<EventFeed>>,
    toggles: Option<&ToggleService>,
    read_timeout: Duration,
) -> Option<Answered> {
    let (head, prefix) = match tokio::time::timeout(read_timeout, read_head(stream)).await {
        Err(_elapsed) => {
            crate::log_event(
                "service",
                "request-timeout",
                format!("head did not arrive within {read_timeout:?}"),
            );
            return None;
        }
        Ok(Err(HeadRead::TooLarge)) => {
            drain_buffered(stream);
            return Some(Answered::Response(http_response(
                431,
                "Request Header Fields Too Large",
                "{\"reason\":\"head_too_large\"}",
                None,
            )));
        }
        Ok(Err(HeadRead::Closed)) => return None,
        Ok(Ok(head_and_prefix)) => head_and_prefix,
    };
    let Some((method, target)) = parse_request_line(&head) else {
        return Some(Answered::Response(http_response(
            400,
            "Bad Request",
            "{\"reason\":\"bad_request\"}",
            None,
        )));
    };
    match route(state, method, target, &head, events, toggles) {
        RouteAnswer::Done(response) => Some(Answered::Response(response)),
        RouteAnswer::EventStream(feed) => Some(Answered::Stream(feed)),
        RouteAnswer::EnabledBody { camera, length } => {
            match read_body(stream, length, &prefix, read_timeout).await {
                BodyRead::Complete(body) => Some(Answered::Response(
                    state.enabled_answer(target, &camera, &head, &body).await,
                )),
                BodyRead::Missing => Some(Answered::Response(ServiceState::missing_body_answer(
                    target,
                ))),
                BodyRead::Broken => None,
            }
        }
        RouteAnswer::ValidateBody { length } => {
            match read_body(stream, length, &prefix, read_timeout).await {
                BodyRead::Complete(body) => {
                    Some(Answered::Response(state.validate_answer(target, &body)))
                }
                BodyRead::Missing => Some(Answered::Response(ServiceState::missing_body_answer(
                    target,
                ))),
                BodyRead::Broken => None,
            }
        }
        RouteAnswer::ApplyBody { length } => {
            match read_body(stream, length, &prefix, read_timeout).await {
                BodyRead::Complete(body) => {
                    Some(Answered::Response(state.apply_answer(target, &head, &body)))
                }
                BodyRead::Missing => Some(Answered::Response(ServiceState::missing_body_answer(
                    target,
                ))),
                BodyRead::Broken => None,
            }
        }
    }
}

/// Reads exactly `length` body bytes, taking what the head read buffered first; `Missing` is the read timeout, `Broken` an early close.
async fn read_body(
    stream: &mut UnixStream,
    length: usize,
    prefix: &[u8],
    read_timeout: Duration,
) -> BodyRead {
    let mut body: Vec<u8> = prefix.iter().copied().take(length).collect();
    let missing = length - body.len();
    if missing == 0 {
        return BodyRead::Complete(body);
    }
    let mut tail = vec![0_u8; missing];
    match tokio::time::timeout(read_timeout, stream.read_exact(&mut tail)).await {
        Err(_elapsed) => {
            crate::log_event(
                "service",
                "body-timeout",
                format!("body did not arrive within {read_timeout:?}"),
            );
            BodyRead::Missing
        }
        Ok(Err(err)) => {
            crate::log_event("service", "body-read-error", err);
            BodyRead::Broken
        }
        Ok(Ok(_)) => {
            body.extend_from_slice(&tail);
            BodyRead::Complete(body)
        }
    }
}

fn parse_request_line(head: &[u8]) -> Option<(&str, &str)> {
    let line = head
        .windows(2)
        .position(|window| window == b"\r\n")
        .map_or(head, |end| &head[..end]);
    let line = std::str::from_utf8(line).ok()?;
    let mut parts = line.split(' ');
    let method = parts.next().filter(|part| !part.is_empty())?;
    let target = parts.next().filter(|part| !part.is_empty())?;
    let version = parts.next().filter(|part| !part.is_empty())?;
    if parts.next().is_some() {
        return None;
    }
    let version = version.as_bytes();
    if version.len() != 8 || version[..7] != *b"HTTP/1." || !version[7].is_ascii_digit() {
        return None;
    }
    Some((method, target))
}

enum RouteAnswer {
    Done(String),
    EventStream(Arc<EventFeed>),
    ValidateBody { length: usize },
    ApplyBody { length: usize },
    EnabledBody { camera: String, length: usize },
}

fn route(
    state: &ServiceState,
    method: &str,
    target: &str,
    head: &[u8],
    events: Option<&Arc<EventFeed>>,
    toggles: Option<&ToggleService>,
) -> RouteAnswer {
    use RouteAnswer::Done;
    if target.contains(['?', '#']) {
        return Done(not_found());
    }
    if let Some(camera) = enabled_camera(target) {
        if toggles.is_none() {
            return Done(not_found());
        }
        if method != "POST" {
            return Done(method_not_allowed("POST"));
        }
        return match ServiceState::required_length(target, "POST", head) {
            Ok(length) => RouteAnswer::EnabledBody { camera, length },
            Err(answer) => Done(answer),
        };
    }
    if target == EVENTS_PATH {
        if method != "GET" {
            return Done(method_not_allowed("GET"));
        }
        return events.map_or_else(
            || Done(not_found()),
            |feed| RouteAnswer::EventStream(Arc::clone(feed)),
        );
    }
    if target == STATUS_PATH {
        if method != "GET" {
            return Done(method_not_allowed("GET"));
        }
        return Done(http_response(200, "OK", &status_body(state), None));
    }
    if target == CONFIG_PATH {
        return match method {
            "GET" => Done(state.config_answer(target)),
            "PUT" => match ServiceState::apply_length(target, head) {
                Ok(length) => RouteAnswer::ApplyBody { length },
                Err(answer) => Done(answer),
            },
            _ => Done(method_not_allowed("GET, PUT")),
        };
    }
    if target == SCHEMA_PATH {
        if method != "GET" {
            return Done(method_not_allowed("GET"));
        }
        return Done(state.schema_answer(target));
    }
    if target == VALIDATE_PATH {
        if method != "POST" {
            return Done(method_not_allowed("POST"));
        }
        return match ServiceState::validate_length(target, head) {
            Ok(length) => RouteAnswer::ValidateBody { length },
            Err(answer) => Done(answer),
        };
    }
    if let Some(name) = camera_name_in(target, OBJECTS_SUFFIX) {
        if method != "GET" {
            return Done(method_not_allowed("GET"));
        }
        let Some(slot) = state.slot_for(&name) else {
            return Done(not_found());
        };
        return Done(objects_response(&slot));
    }
    Done(not_found())
}

/// The camera name in `/corvette/api/v1/cameras/<name><suffix>`, percent-decoded; `None` for any other shape.
fn camera_name_in(target: &str, suffix: &str) -> Option<String> {
    let segment = target.strip_prefix(CAMERAS_PREFIX)?.strip_suffix(suffix)?;
    if segment.is_empty() || segment.contains('/') {
        return None;
    }
    percent_decode(segment)
}

fn enabled_camera(target: &str) -> Option<String> {
    camera_name_in(target, ENABLED_SUFFIX)
}

#[must_use]
pub fn not_found() -> String {
    http_response(404, "Not Found", "{\"reason\":\"not_found\"}", None)
}

#[must_use]
pub fn method_not_allowed(method: &str) -> String {
    http_response(
        405,
        "Method Not Allowed",
        "{\"reason\":\"method_not_allowed\"}",
        Some(method),
    )
}

/// Percent-decodes a raw path segment; `None` on a malformed escape or non-UTF-8 bytes; `+` stays `+`.
fn percent_decode(segment: &str) -> Option<String> {
    let mut decoded = Vec::with_capacity(segment.len());
    let mut bytes = segment.as_bytes().iter();
    while let Some(&byte) = bytes.next() {
        if byte != b'%' {
            decoded.push(byte);
            continue;
        }
        let (Some(&first), Some(&second)) = (bytes.next(), bytes.next()) else {
            return None;
        };
        let (Some(high), Some(low)) = (hex_digit(first), hex_digit(second)) else {
            return None;
        };
        decoded.push(high * 16 + low);
    }
    String::from_utf8(decoded).ok()
}

const fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn objects_response(slot: &Arc<CameraSlot>) -> String {
    let live = match &slot_state(slot).entry {
        CameraEntry::Off => None,
        CameraEntry::Live(handle) => Some(handle.clone()),
    };
    let Some(handle) = live else {
        return http_response(200, "OK", "{\"objects\":[]}", None);
    };
    let published_now = handle.objects.borrow();
    let published: Option<&FrameObjects> = match &*published_now {
        CameraObjects::Frame(known) => Some(known),
        CameraObjects::Cleared | CameraObjects::Stopped => None,
    };
    let answer = objects_answer(published);
    drop(published_now);
    match answer {
        Ok(ObjectsAnswer::Objects(body)) => http_response(200, "OK", &body, None),
        Ok(ObjectsAnswer::Unknown) => http_response(
            503,
            "Service Unavailable",
            "{\"reason\":\"objects_unknown\"}",
            None,
        ),
        Err(err) => {
            crate::log_event("service", "objects-serialisation", err);
            http_response(
                500,
                "Internal Server Error",
                "{\"reason\":\"serialisation_failed\"}",
                None,
            )
        }
    }
}

fn status_body(state: &ServiceState) -> String {
    let worker: Option<WorkerReport> = state
        .worker
        .as_ref()
        .map(|receiver| WorkerReport::from_status(&receiver.borrow()));
    let cameras = state.cameras_locked();
    let entries: Vec<String> = cameras
        .iter()
        .map(|slot| match &slot_state(slot).entry {
            CameraEntry::Off => off_entry(&slot.name),
            CameraEntry::Live(handle) => status_entry(&slot.name, &handle.status.borrow(), worker),
        })
        .collect();
    drop(cameras);
    format!("{{\"cameras\":[{}]}}", entries.join(","))
}

pub fn http_response(status: u16, reason: &str, body: &str, allow: Option<&str>) -> String {
    let allow = allow.map_or_else(String::new, |methods| format!("Allow: {methods}\r\n"));
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nCache-Control: no-store\r\n{allow}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn drain_buffered(stream: &UnixStream) {
    let mut scratch = [0_u8; 4096];
    while stream.try_read(&mut scratch).is_ok_and(|read| read > 0) {}
}

async fn write_response(stream: &mut UnixStream, response: &str) -> std::io::Result<()> {
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    stream.shutdown().await
}

const EVENT_STREAM_HEAD: &[u8] = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\n\r\n";

/// Streams `GET /corvette/api/v1/events` from the feed: register a subscriber after the head parsed, answer 200 chunked, then pump until the queue closes, the client's read half closes, or a write fails; the slot is held for the whole stream and released here.
async fn stream_events(stream: &mut UnixStream, feed: &EventFeed) {
    let Some(lines) = feed.admit_stream() else {
        crate::log_event("service", "event-stream-limit", "every stream slot is held");
        let refusal = http_response(
            503,
            "Service Unavailable",
            "{\"reason\":\"event_streams_full\"}",
            None,
        );
        if let Err(err) = write_response(stream, &refusal).await {
            crate::log_event("service", "refusal-write-error", err);
        }
        return;
    };
    if let Err(err) = pump_events(stream, &lines).await {
        crate::log_event("service", "event-stream-write", err);
    }
    feed.release_stream(&lines);
}

async fn pump_events(stream: &mut UnixStream, lines: &EventStream) -> std::io::Result<()> {
    stream.write_all(EVENT_STREAM_HEAD).await?;
    let mut watch_half = [0_u8; 1];
    loop {
        tokio::select! {
            () = lines.closed() => break,
            line = lines.next_line() => {
                let Some(line) = line else { break };
                stream.write_all(&chunk_frame(&line)).await?;
            }
            read = stream.read(&mut watch_half) => {
                if read? != 0 {
                    continue;
                }
                break;
            }
        }
    }
    Ok(())
}

/// Wraps one track line as exactly one chunk: hex length, the line's bytes untouched, terminator.
fn chunk_frame(line: &[u8]) -> Vec<u8> {
    let mut framed = format!("{:x}\r\n", line.len()).into_bytes();
    framed.extend_from_slice(line);
    framed.extend_from_slice(b"\r\n");
    framed
}

#[cfg(test)]
mod tests {
    use super::chunk_frame;

    #[test]
    fn a_line_shaped_like_a_chunk_header_frames_as_one_chunk_of_its_own_length() {
        let line = b"3\r\nfoo\r\n";
        let framed = chunk_frame(line);
        let size_end = framed.windows(2).position(|w| w == b"\r\n").unwrap();
        let size =
            usize::from_str_radix(std::str::from_utf8(&framed[..size_end]).unwrap(), 16).unwrap();
        let body = &framed[size_end + 2..size_end + 2 + size];
        assert_eq!(
            body, line,
            "the chunk holds the whole line, chunk-shaped bytes included"
        );
        assert_eq!(
            &framed[size_end + 2 + size..],
            b"\r\n",
            "one chunk terminator, nothing after"
        );
    }
}
