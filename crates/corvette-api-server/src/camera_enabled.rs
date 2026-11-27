//! The public camera on/off route relayed to the detection service, and the
//! enabled flag the camera list takes from the service's status answer.

use crate::activity;
use crate::detection::{self, Relay};
use crate::detection_config::{
    ADMIN_ROLE, backend_request, body_length_refusal, detection_unavailable, exchange, passthrough,
    problem,
};
use crate::{API_PREFIX, Request, Response, bad_request, log_event};
use serde_json::Value;
use std::time::Duration;

const ENABLED_SUFFIX: &str = "/enabled";
const OFF_STATE: &str = "off";
const STATUS_TARGET: &str = "detection/status";

// The service's toggle join may take up to 15 s, so this total outlasts the shared default.
pub(crate) const TOGGLE_TOTAL_TIMEOUT: Duration = Duration::from_secs(20);

pub(crate) struct CameraFlag {
    pub(crate) name: String,
    pub(crate) enabled: bool,
}

pub(crate) enum StatusFlags {
    NoSocket,
    Answered(Vec<CameraFlag>),
    Unreadable,
}

pub(crate) fn serves(path: &str) -> bool {
    enabled_segment(path).is_some()
}

pub(crate) fn enabled_segment(path: &str) -> Option<&str> {
    let rest = path
        .strip_prefix(API_PREFIX)?
        .strip_prefix("cameras/")?
        .strip_suffix(ENABLED_SUFFIX)?;
    (!rest.is_empty() && !rest.contains('/')).then_some(rest)
}

pub(crate) async fn forward(
    request: &Request,
    segment: &str,
    user: &str,
    relay: &Relay,
) -> Response {
    if request.remote_role != ADMIN_ROLE {
        return problem(
            403,
            "Forbidden",
            "forbidden",
            "the caller's role may not toggle a camera",
            &request.path,
        );
    }
    if let Some(refusal) = body_length_refusal(request) {
        return refusal;
    }
    let camera = match activity::percent_decode(segment) {
        Ok(camera) => camera,
        Err(reason) => return bad_request(&reason),
    };
    let Some(socket) = relay.socket.as_deref() else {
        log_event(
            "camera-enabled",
            "socket-unset",
            "the detection socket is not configured",
        );
        return detection_unavailable(&request.path);
    };
    let target = format!(
        "{API_PREFIX}cameras/{}/enabled",
        activity::percent_encode_segment(&camera)
    );
    let timeouts = relay
        .timeouts
        .unwrap_or((detection::DETECTION_CONNECT_TIMEOUT, TOGGLE_TOTAL_TIMEOUT));
    match exchange(
        socket,
        &backend_request(request, user, &target),
        Some(timeouts),
    )
    .await
    {
        Ok(answer) => passthrough(answer),
        Err(reason) => {
            log_event("camera-enabled", "relay-error", reason);
            detection_unavailable(&request.path)
        }
    }
}

/// `NoSocket` when the socket is unset, `Answered` when the status read, `Unreadable` otherwise.
pub(crate) async fn camera_flags(relay: &Relay) -> StatusFlags {
    let Some(socket) = relay.socket.as_deref() else {
        return StatusFlags::NoSocket;
    };
    let target = format!("{API_PREFIX}{STATUS_TARGET}");
    let body = match detection::fetch(socket, &target).await {
        Ok((200, body)) => body,
        Ok((status, _body)) => {
            log_event(
                "camera-enabled",
                "status-answer",
                format!("service answered {status}"),
            );
            return StatusFlags::Unreadable;
        }
        Err(reason) => {
            log_event("camera-enabled", "status-fetch-error", reason);
            return StatusFlags::Unreadable;
        }
    };
    let listed = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|value| value.get("cameras").and_then(Value::as_array).cloned());
    let Some(list) = listed else {
        log_event(
            "camera-enabled",
            "status-shape",
            "service answered a body that is not a camera list",
        );
        return StatusFlags::Unreadable;
    };
    StatusFlags::Answered(
        list.iter()
            .filter_map(|entry| {
                Some(CameraFlag {
                    name: entry.get("camera")?.as_str()?.to_owned(),
                    enabled: entry.get("state").and_then(Value::as_str) != Some(OFF_STATE),
                })
            })
            .collect(),
    )
}
