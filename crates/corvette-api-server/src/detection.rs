//! The relay of the detection routes to the detection service's Unix socket, so the
//! API server stays the single public door and the service's own answer never appears.

use crate::config::ConfigSource;
use crate::{
    API_PREFIX, Request, Response, activity, bad_request, camera_not_found, config_unavailable,
    error_response, find_header_end, log_event, utf8,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub const DETECTION_SOCKET_ENV: &str = "CORVETTE_DETECTION_SOCKET";

pub(crate) const DETECTION_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) const DETECTION_TOTAL_TIMEOUT: Duration = Duration::from_secs(5);

const MAX_ANSWER_BYTES: usize = 1024 * 1024;

/// Where the detection service is; `timeouts` `None` means the default consts.
#[derive(Clone, Debug, Default)]
pub(crate) struct Relay {
    pub(crate) socket: Option<PathBuf>,
    pub(crate) timeouts: Option<(Duration, Duration)>,
}

pub(crate) fn objects_segment(path: &str) -> Option<&str> {
    let prefix = format!("{API_PREFIX}cameras/");
    let rest = path.strip_prefix(prefix.as_str())?;
    let segment = rest.strip_suffix("/objects")?;
    (!segment.is_empty() && !segment.contains('/')).then_some(segment)
}

fn is_objects_answer(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body)
        .is_ok_and(|value| value.get("objects").is_some_and(Value::is_array))
}

fn empty_objects() -> Response {
    Response::json("200 OK", br#"{"objects":[]}"#.to_vec())
}

fn detection_unavailable() -> Response {
    error_response(
        "503 Service Unavailable",
        "detection_unavailable",
        "The detection service could not be reached.",
    )
}

pub(crate) async fn fetch(socket: &Path, target: &str) -> Result<(u16, Vec<u8>), String> {
    fetch_timed(
        socket,
        target,
        DETECTION_CONNECT_TIMEOUT,
        DETECTION_TOTAL_TIMEOUT,
    )
    .await
}

async fn fetch_via(relay: &Relay, socket: &Path, target: &str) -> Result<(u16, Vec<u8>), String> {
    match relay.timeouts {
        Some((connect, total)) => fetch_timed(socket, target, connect, total).await,
        None => fetch(socket, target).await,
    }
}

async fn fetch_timed(
    socket: &Path,
    target: &str,
    connect: Duration,
    total: Duration,
) -> Result<(u16, Vec<u8>), String> {
    tokio::time::timeout(total, fetch_untimed(socket, target, connect))
        .await
        .map_err(|_elapsed| {
            format!("no complete answer from the detection service within {total:?}")
        })?
}

async fn fetch_untimed(
    socket: &Path,
    target: &str,
    connect: Duration,
) -> Result<(u16, Vec<u8>), String> {
    let mut stream = tokio::time::timeout(connect, UnixStream::connect(socket))
        .await
        .map_err(|_elapsed| format!("connecting to the detection service took over {connect:?}"))?
        .map_err(|err| format!("could not connect to the detection service: {err}"))?;
    let request = format!("GET {target} HTTP/1.1\r\nHost: detection\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|err| format!("could not send the detection request: {err}"))?;
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

fn parse_answer(answer: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let Some(head_end) = find_header_end(answer) else {
        return Err("detection answer ended before its header block did".to_owned());
    };
    let status_line = utf8(&answer[..head_end])
        .and_then(|head| head.split('\n').next())
        .and_then(|line| line.strip_suffix('\r'))
        .ok_or("detection status line is not valid UTF-8")?;
    let Some((version, rest)) = status_line.split_once(' ') else {
        return Err(format!(
            "detection status line {status_line:?} is malformed"
        ));
    };
    if version != "HTTP/1.0" && version != "HTTP/1.1" {
        return Err(format!("detection status line names version {version:?}"));
    }
    let code = rest
        .split(' ')
        .next()
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or("detection status code is not a number")?;
    Ok((code, answer[head_end + 4..].to_vec()))
}

fn unavailable_without_socket() -> Response {
    log_event(
        "detection",
        "socket-unset",
        "the detection socket is not configured",
    );
    detection_unavailable()
}

pub(crate) async fn objects(
    request: &Request,
    config: &ConfigSource,
    segment: &str,
    relay: &Relay,
) -> Response {
    let camera = match activity::percent_decode(segment) {
        Ok(camera) => camera,
        Err(reason) => return bad_request(&reason),
    };
    match config.camera_for_role(&request.remote_role, &camera).await {
        Ok(Some(_)) => {}
        Ok(None) => return camera_not_found(),
        Err(err) => return config_unavailable(&err),
    }
    let Some(socket) = relay.socket.as_deref() else {
        return unavailable_without_socket();
    };
    // The service matches camera names verbatim, so every byte outside the
    // unreserved set goes on the wire as an upper-case percent escape.
    let target = format!(
        "{API_PREFIX}cameras/{}/objects",
        activity::percent_encode_segment(&camera)
    );
    let (status, body) = match fetch_via(relay, socket, &target).await {
        Ok(answer) => answer,
        Err(reason) => {
            log_event("detection", "objects-fetch-error", reason);
            return detection_unavailable();
        }
    };
    match status {
        200 if is_objects_answer(&body) => Response::json("200 OK", body),
        404 => empty_objects(),
        other => {
            log_event(
                "detection",
                "objects-answer",
                format!("service answered {other}"),
            );
            detection_unavailable()
        }
    }
}

fn service_camera_entries(body: &[u8]) -> Option<HashMap<String, Value>> {
    let list = serde_json::from_slice::<Value>(body)
        .ok()?
        .get("cameras")?
        .as_array()
        .cloned()?;
    let mut entries = HashMap::with_capacity(list.len());
    for entry in &list {
        let name = entry.get("camera")?.as_str()?.to_owned();
        entries.insert(name, entry.clone());
    }
    Some(entries)
}

pub(crate) async fn status(config: &ConfigSource, role: &str, relay: &Relay) -> Response {
    let entries = match config.camera_entries_for_role(role).await {
        Ok(entries) => entries,
        Err(err) => return config_unavailable(&err),
    };
    let Some(socket) = relay.socket.as_deref() else {
        return unavailable_without_socket();
    };
    let target = format!("{API_PREFIX}detection/status");
    let (status, body) = match fetch_via(relay, socket, &target).await {
        Ok(answer) => answer,
        Err(reason) => {
            log_event("detection", "status-fetch-error", reason);
            return detection_unavailable();
        }
    };
    if status != 200 {
        log_event(
            "detection",
            "status-answer",
            format!("service answered {status}"),
        );
        return detection_unavailable();
    }
    let Some(listed) = service_camera_entries(&body) else {
        log_event(
            "detection",
            "status-shape",
            "service answered a body that is not a camera list",
        );
        return detection_unavailable();
    };
    let cameras: Vec<Value> = entries
        .iter()
        .map(|entry| {
            listed.get(&entry.name).map_or_else(
                || serde_json::json!({ "camera": entry.name, "state": "off" }),
                Clone::clone,
            )
        })
        .collect();
    Response::json(
        "200 OK",
        serde_json::to_vec(&serde_json::json!({ "cameras": cameras }))
            .expect("status body serializes"),
    )
}
