//! The read, validate and schema answers over the saved detection configuration; every failure is RFC 9457 problem details under the contract's codes.

use serde_json::{Map, Value};

use crate::config::{Config, Violation};
use crate::config_change::classify;
use crate::config_schema::SCHEMA_JSON;
use crate::revision::revision;
use crate::service_socket::ServiceState;

// The contract's body bound, sized for a hand-authored multi-camera file, so an oversized body is refused before it is read.
const MAX_BODY_BYTES: usize = 262_144;
// A rejected address value can itself carry credentials (an rtsp URL does), so `found` is dropped on these paths.
const ADDRESS_FIELDS: [&str; 4] = ["worker_socket", "stream", "url", "address"];

impl ServiceState {
    /// Answers `GET config`: the saved file re-read at request time, normalized, tagged with its revision.
    pub(crate) fn config_answer(&self, target: &str) -> String {
        match saved(self) {
            Err(reason) => {
                crate::log_event("service", "config-read-failed", reason);
                unavailable(target)
            }
            Ok((bytes, config)) => respond(
                200,
                "OK",
                "application/json",
                Some(&revision(&bytes)),
                &config.to_value().to_string(),
            ),
        }
    }

    pub(crate) fn schema_answer(&self, target: &str) -> String {
        if self.config_path().is_none() {
            return unavailable(target);
        }
        respond(200, "OK", "application/schema+json", None, SCHEMA_JSON)
    }

    /// Decides a `POST config/validate` from its head: `Ok` is the body length to read, `Err` is the finished `400` answer.
    pub(crate) fn validate_length(target: &str, head: &[u8]) -> Result<usize, String> {
        match content_length(head) {
            None => Err(bad_request(target, "a POST needs a valid Content-Length")),
            Some(length) if length > MAX_BODY_BYTES => Err(bad_request(
                target,
                &format!("body over the {MAX_BODY_BYTES} byte bound"),
            )),
            Some(length) => Ok(length),
        }
    }

    /// Answers `POST config/validate`: the loader's whole rule set against the proposed value, changing nothing.
    pub(crate) fn validate_answer(&self, target: &str, body: &[u8]) -> String {
        if self.config_path().is_none() {
            return unavailable(target);
        }
        let Ok(value) = serde_json::from_slice::<Value>(body) else {
            return bad_request(target, "the body is not JSON");
        };
        if !value.is_object() {
            return bad_request(target, "the body is not a JSON object");
        }
        let proposed = match Config::validate_all(&value) {
            Ok(proposed) => proposed,
            Err(violations) => return invalid_config(target, &violations),
        };
        let saved = match saved(self) {
            Ok((_bytes, saved)) => saved,
            Err(reason) => {
                crate::log_event("service", "config-read-failed", reason);
                return unavailable(target);
            }
        };
        let restart_required: Vec<Value> = classify(&saved, &proposed)
            .into_iter()
            .map(Value::String)
            .collect();
        let mut answer = Map::new();
        answer.insert("valid".to_owned(), Value::Bool(true));
        answer.insert(
            "restart_required".to_owned(),
            Value::Array(restart_required),
        );
        respond(
            200,
            "OK",
            "application/json",
            None,
            &Value::Object(answer).to_string(),
        )
    }
}

fn content_length(head: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(head).ok()?;
    let mut found: Option<usize> = None;
    for line in text.split("\r\n").skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = value.trim().parse::<usize>().ok();
    }
    found
}

fn saved(state: &ServiceState) -> Result<(Vec<u8>, Config), String> {
    let path = state.config_path().ok_or("no configuration path")?;
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| format!("{} is not UTF-8: {error}", path.display()))?;
    Config::from_toml(text)
        .map(|config| (bytes, config))
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn invalid_config(instance: &str, violations: &[Violation]) -> String {
    let errors: Vec<Value> = violations.iter().map(violation_entry).collect();
    let detail = if errors.len() == 1 {
        "1 configuration error".to_owned()
    } else {
        format!("{} configuration errors", errors.len())
    };
    problem(422, "invalid_config", &detail, instance, Some(errors))
}

fn violation_entry(violation: &Violation) -> Value {
    let mut entry = Map::new();
    entry.insert("path".to_owned(), Value::String(violation.path.clone()));
    entry.insert("rule".to_owned(), Value::String(violation.rule.clone()));
    if let Some(found) = &violation.found
        && !(found.is_string() && names_address_field(&violation.path))
    {
        entry.insert("found".to_owned(), found.clone());
    }
    if let Some(allowed) = &violation.allowed {
        entry.insert("allowed".to_owned(), allowed.clone());
    }
    Value::Object(entry)
}

fn names_address_field(path: &str) -> bool {
    let last = path.rsplit('.').next().unwrap_or(path);
    let field = last.split('[').next().unwrap_or(last);
    ADDRESS_FIELDS.contains(&field)
}

fn problem(
    status: u16,
    code: &str,
    detail: &str,
    instance: &str,
    errors: Option<Vec<Value>>,
) -> String {
    let reason = match status {
        400 => "Bad Request",
        422 => "Unprocessable Content",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let mut body = Map::new();
    body.insert("type".to_owned(), Value::String("about:blank".to_owned()));
    body.insert("title".to_owned(), Value::String(reason.to_owned()));
    body.insert("status".to_owned(), Value::from(u64::from(status)));
    body.insert("detail".to_owned(), Value::String(detail.to_owned()));
    body.insert("code".to_owned(), Value::String(code.to_owned()));
    body.insert("instance".to_owned(), Value::String(instance.to_owned()));
    if let Some(errors) = errors {
        body.insert("errors".to_owned(), Value::Array(errors));
    }
    respond(
        status,
        reason,
        "application/problem+json",
        None,
        &Value::Object(body).to_string(),
    )
}

fn bad_request(instance: &str, detail: &str) -> String {
    problem(400, "bad_request", detail, instance, None)
}

fn unavailable(instance: &str) -> String {
    problem(
        503,
        "config_unavailable",
        "no usable configuration copy is available",
        instance,
        None,
    )
}

fn respond(
    status: u16,
    reason: &str,
    content_type: &str,
    etag: Option<&str>,
    body: &str,
) -> String {
    let etag = etag.map_or_else(String::new, |etag| format!("ETag: \"{etag}\"\r\n"));
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\n{etag}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
