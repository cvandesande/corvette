//! The public door's side of the detection configuration routes: the write gate,
//! the refusals answered before any relay, and the byte-forwarding relay to the
//! detection service's Unix socket.

use crate::detection::{
    DETECTION_CONNECT_TIMEOUT, DETECTION_TOTAL_TIMEOUT, MAX_ANSWER_BYTES, Relay,
};
use crate::{API_PREFIX, Request, Response, find_header_end, log_event, utf8};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

// The contract's body bound, 256 KiB; the detection service refuses the same size.
pub(crate) const MAX_BODY_BYTES: usize = 262_144;

const VALIDATE_SUFFIX: &str = "detection/config/validate";
const SCHEMA_SUFFIX: &str = "detection/config/schema";
const ADMIN_ROLE: &str = "admin";

pub(crate) fn serves(path: &str) -> bool {
    path.strip_prefix(API_PREFIX)
        .is_some_and(|rest| matches!(rest, "detection/config" | VALIDATE_SUFFIX | SCHEMA_SUFFIX))
}

fn is_apply(request: &Request) -> bool {
    request.path.strip_prefix(API_PREFIX) == Some("detection/config") && request.method == "PUT"
}

/// The request the service sees: its own `Remote-User`, the caller's `If-Match`
/// and `Content-Type` unchanged, the body bytes, nothing else of the caller's headers.
fn backend_request(request: &Request, user: &str) -> Vec<u8> {
    let mut out = format!(
        "{} {} HTTP/1.1\r\nHost: detection\r\nRemote-User: {user}\r\nConnection: close\r\n",
        request.method, request.path
    )
    .into_bytes();
    if let Some(if_match) = &request.if_match {
        out.extend_from_slice(format!("If-Match: {if_match}\r\n").as_bytes());
    }
    if let Some(content_type) = &request.content_type {
        out.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
    }
    if matches!(request.method.as_str(), "PUT" | "POST") {
        out.extend_from_slice(format!("Content-Length: {}\r\n", request.body.len()).as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&request.body);
    out
}

struct BackendAnswer {
    status: String,
    content_type: String,
    cache_control: String,
    etag: Option<String>,
    allow: Option<String>,
    body: Vec<u8>,
}

fn parse_answer(answer: &[u8]) -> Result<BackendAnswer, String> {
    let Some(end) = find_header_end(answer) else {
        return Err("detection answer ended before its header block did".to_owned());
    };
    let head = utf8(&answer[..end]).ok_or("detection answer head is not valid UTF-8")?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let (version, rest) = status_line
        .split_once(' ')
        .ok_or("detection status line is malformed")?;
    if version != "HTTP/1.0" && version != "HTTP/1.1" {
        return Err(format!("detection status line names version {version:?}"));
    }
    let (code, reason) = rest.split_once(' ').unwrap_or((rest, ""));
    let code_number: u16 = code
        .parse()
        .map_err(|_err| format!("detection status code {code:?} is not a number"))?;
    let mut content_type: Option<String> = None;
    let mut cache_control: Option<String> = None;
    let mut etag: Option<String> = None;
    let mut allow: Option<String> = None;
    let mut declared: Option<usize> = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "content-type" if content_type.is_none() => content_type = Some(value.to_owned()),
            "cache-control" if cache_control.is_none() => cache_control = Some(value.to_owned()),
            "etag" if etag.is_none() => etag = Some(value.to_owned()),
            "allow" if allow.is_none() => allow = Some(value.to_owned()),
            "content-length" if declared.is_none() => {
                declared = Some(
                    value
                        .parse()
                        .map_err(|_err| "detection Content-Length is not a number".to_owned())?,
                );
            }
            _ => {}
        }
    }
    let body = &answer[end + 4..];
    match declared {
        Some(length) if length == body.len() => {}
        Some(_) => return Err("detection body does not match its declared length".to_owned()),
        None => return Err("detection answer carries no Content-Length".to_owned()),
    }
    Ok(BackendAnswer {
        status: if reason.is_empty() {
            code_number.to_string()
        } else {
            format!("{code_number} {reason}")
        },
        content_type: content_type.ok_or("detection answer carries no Content-Type")?,
        cache_control: cache_control.unwrap_or_else(|| "no-store".to_owned()),
        etag,
        allow,
        body: body.to_vec(),
    })
}

fn passthrough(answer: BackendAnswer) -> Response {
    let mut extra = Vec::new();
    if let Some(etag) = answer.etag {
        extra.push((Cow::Borrowed("ETag"), etag));
    }
    if let Some(allow) = answer.allow {
        extra.push((Cow::Borrowed("Allow"), allow));
    }
    Response {
        status: Cow::Owned(answer.status),
        body: answer.body,
        content_type: Cow::Owned(answer.content_type),
        cache_control: Cow::Owned(answer.cache_control),
        extra,
    }
}

async fn exchange(
    socket: &Path,
    request: &[u8],
    timeouts: Option<(Duration, Duration)>,
) -> Result<BackendAnswer, String> {
    let (connect, total) = timeouts.unwrap_or((DETECTION_CONNECT_TIMEOUT, DETECTION_TOTAL_TIMEOUT));
    tokio::time::timeout(total, exchange_bounded(socket, request, connect))
        .await
        .map_err(|_elapsed| {
            format!("no complete answer from the detection service within {total:?}")
        })?
}

async fn exchange_bounded(
    socket: &Path,
    request: &[u8],
    connect: Duration,
) -> Result<BackendAnswer, String> {
    let mut stream = tokio::time::timeout(connect, UnixStream::connect(socket))
        .await
        .map_err(|_elapsed| format!("connecting to the detection service took over {connect:?}"))?
        .map_err(|err| format!("could not connect to the detection service: {err}"))?;
    stream
        .write_all(request)
        .await
        .map_err(|err| format!("could not send the configuration request: {err}"))?;
    let mut answer = Vec::new();
    stream
        .take(MAX_ANSWER_BYTES as u64 + 1)
        .read_to_end(&mut answer)
        .await
        .map_err(|err| format!("could not read the detection answer: {err}"))?;
    if answer.len() > MAX_ANSWER_BYTES {
        return Err(format!(
            "detection answer is larger than the {MAX_ANSWER_BYTES}-byte cap"
        ));
    }
    parse_answer(&answer)
}

fn problem(status: u16, title: &str, code: &str, detail: &str, instance: &str) -> Response {
    let mut body = Map::new();
    body.insert("type".to_owned(), Value::String("about:blank".to_owned()));
    body.insert("title".to_owned(), Value::String(title.to_owned()));
    body.insert("status".to_owned(), Value::from(u64::from(status)));
    body.insert("detail".to_owned(), Value::String(detail.to_owned()));
    body.insert("code".to_owned(), Value::String(code.to_owned()));
    body.insert("instance".to_owned(), Value::String(instance.to_owned()));
    Response {
        status: Cow::Owned(format!("{status} {title}")),
        body: Value::Object(body).to_string().into_bytes(),
        content_type: Cow::Borrowed("application/problem+json"),
        cache_control: Cow::Borrowed("no-store"),
        extra: Vec::new(),
    }
}

fn detection_unavailable(instance: &str) -> Response {
    problem(
        503,
        "Service Unavailable",
        "detection_unavailable",
        "the detection service could not be reached over its local socket",
        instance,
    )
}

/// Forwards one configuration-route request; every refusal here precedes the relay.
pub(crate) async fn forward(request: &Request, user: &str, relay: &Relay) -> Response {
    if is_apply(request) && request.remote_role != ADMIN_ROLE {
        return problem(
            403,
            "Forbidden",
            "forbidden",
            "the caller's role may not write the detection configuration",
            &request.path,
        );
    }
    if matches!(request.method.as_str(), "PUT" | "POST") {
        match request.body_len {
            None => {
                return problem(
                    400,
                    "Bad Request",
                    "bad_request",
                    &format!("a {} needs a valid Content-Length", request.method),
                    &request.path,
                );
            }
            Some(length) if length > MAX_BODY_BYTES => {
                return problem(
                    400,
                    "Bad Request",
                    "bad_request",
                    &format!("body over the {MAX_BODY_BYTES} byte bound"),
                    &request.path,
                );
            }
            Some(_length) => {}
        }
    }
    let Some(socket) = relay.socket.as_deref() else {
        log_event(
            "detection-config",
            "socket-unset",
            "the detection socket is not configured",
        );
        return detection_unavailable(&request.path);
    };
    match exchange(socket, &backend_request(request, user), relay.timeouts).await {
        Ok(answer) => passthrough(answer),
        Err(reason) => {
            log_event("detection-config", "relay-error", reason);
            detection_unavailable(&request.path)
        }
    }
}
