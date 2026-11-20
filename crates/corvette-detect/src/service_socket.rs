//! Minimal HTTP/1.1 over a Unix socket, one request per connection: one camera's objects answer and the detection status answer; the API server maps any non-`200` here to its public `503 detection_unavailable`.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::camera_runner::CameraHandle;
use crate::object_stage::FrameObjects;
use crate::service_bodies::{ObjectsAnswer, objects_answer, off_entry, status_entry};

const MAX_HEAD_BYTES: usize = 8192;
const STATUS_PATH: &str = "/corvette/api/v1/detection/status";
const CAMERAS_PREFIX: &str = "/corvette/api/v1/cameras/";
const OBJECTS_SUFFIX: &str = "/objects";
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub enum CameraEntry {
    Off,
    Live(CameraHandle),
}

#[derive(Clone, Debug)]
pub struct ServiceState {
    cameras: Vec<(String, CameraEntry)>,
}

impl ServiceState {
    #[must_use]
    pub const fn new(cameras: Vec<(String, CameraEntry)>) -> Self {
        Self { cameras }
    }
}

/// Accept connections until one `accept` fails; each connection is answered by its own task and closed after one request.
/// # Errors
/// Returns `Err` when `accept` fails.
pub async fn serve(listener: UnixListener, state: Arc<ServiceState>) -> std::io::Result<()> {
    serve_with_timeout(listener, state, DEFAULT_READ_TIMEOUT).await
}

/// # Errors
/// Returns `Err` when `accept` fails.
pub async fn serve_with_timeout(
    listener: UnixListener,
    state: Arc<ServiceState>,
    read_timeout: Duration,
) -> std::io::Result<()> {
    loop {
        let (stream, _peer) = listener.accept().await?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            answer_one(stream, &state, read_timeout).await;
        });
    }
}

enum HeadRead {
    TooLarge,
    Closed,
}

async fn read_head(stream: &mut UnixStream) -> Result<Vec<u8>, HeadRead> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            buffer.truncate(end);
            return Ok(buffer);
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

async fn answer_one(mut stream: UnixStream, state: &ServiceState, read_timeout: Duration) {
    let response = match tokio::time::timeout(read_timeout, read_head(&mut stream)).await {
        Err(_elapsed) => {
            crate::log_event(
                "service",
                "request-timeout",
                format!("head did not arrive within {read_timeout:?}"),
            );
            return;
        }
        Ok(Err(HeadRead::TooLarge)) => {
            drain_buffered(&stream);
            http_response(
                431,
                "Request Header Fields Too Large",
                "{\"reason\":\"head_too_large\"}",
                false,
            )
        }
        Ok(Err(HeadRead::Closed)) => return,
        Ok(Ok(head)) => match parse_request_line(&head) {
            None => http_response(400, "Bad Request", "{\"reason\":\"bad_request\"}", false),
            Some((method, target)) => route(state, method, target),
        },
    };
    if let Err(err) = write_response(&mut stream, &response).await {
        crate::log_event("service", "response-write-error", err);
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

fn route(state: &ServiceState, method: &str, target: &str) -> String {
    if target.contains(['?', '#']) {
        return not_found();
    }
    if target == STATUS_PATH {
        if method != "GET" {
            return method_not_allowed();
        }
        return http_response(200, "OK", &status_body(state), false);
    }
    if let Some(index) = objects_index(state, target) {
        if method != "GET" {
            return method_not_allowed();
        }
        return objects_response(state, index);
    }
    not_found()
}

fn not_found() -> String {
    http_response(404, "Not Found", "{\"reason\":\"not_found\"}", false)
}

fn method_not_allowed() -> String {
    http_response(
        405,
        "Method Not Allowed",
        "{\"reason\":\"method_not_allowed\"}",
        true,
    )
}

fn objects_index(state: &ServiceState, target: &str) -> Option<usize> {
    let segment = target
        .strip_prefix(CAMERAS_PREFIX)?
        .strip_suffix(OBJECTS_SUFFIX)?;
    if segment.is_empty() || segment.contains('/') {
        return None;
    }
    let decoded = percent_decode(segment)?;
    state.cameras.iter().position(|(name, _)| *name == decoded)
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

fn objects_response(state: &ServiceState, index: usize) -> String {
    match &state.cameras[index].1 {
        CameraEntry::Off => http_response(200, "OK", "{\"objects\":[]}", false),
        CameraEntry::Live(handle) => {
            let published: Option<FrameObjects> = (*handle.objects.borrow()).clone();
            match objects_answer(published.as_ref()) {
                Ok(ObjectsAnswer::Objects(body)) => http_response(200, "OK", &body, false),
                Ok(ObjectsAnswer::Unknown) => http_response(
                    503,
                    "Service Unavailable",
                    "{\"reason\":\"objects_unknown\"}",
                    false,
                ),
                Err(err) => {
                    crate::log_event("service", "objects-serialisation", err);
                    http_response(
                        500,
                        "Internal Server Error",
                        "{\"reason\":\"serialisation_failed\"}",
                        false,
                    )
                }
            }
        }
    }
}

fn status_body(state: &ServiceState) -> String {
    let entries: Vec<String> = state
        .cameras
        .iter()
        .map(|(name, entry)| match entry {
            CameraEntry::Off => off_entry(name),
            CameraEntry::Live(handle) => status_entry(name, &handle.status.borrow()),
        })
        .collect();
    format!("{{\"cameras\":[{}]}}", entries.join(","))
}

fn http_response(status: u16, reason: &str, body: &str, allow_get: bool) -> String {
    let allow = if allow_get { "Allow: GET\r\n" } else { "" };
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
