//! The read, apply, validate and schema answers over the saved detection configuration; every failure is RFC 9457 problem details under the contract's codes.

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::config::{Config, Violation};
use crate::config_change::classify;
use crate::config_file::render;
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
            Ok(copy) => respond(
                200,
                "OK",
                "application/json",
                Some(&revision(&copy.bytes)),
                &copy.config.to_value().to_string(),
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
        required_length(target, "POST", head)
    }

    /// Decides a `PUT config` from its head: `Ok` is the body length to read, `Err` is the finished `400` answer.
    pub(crate) fn apply_length(target: &str, head: &[u8]) -> Result<usize, String> {
        required_length(target, "PUT", head)
    }

    /// Answers `POST config/validate`: the loader's whole rule set against the proposed value, changing nothing.
    pub(crate) fn validate_answer(&self, target: &str, body: &[u8]) -> String {
        if self.config_path().is_none() {
            return unavailable(target);
        }
        let value = match config_body(target, body) {
            Ok(value) => value,
            Err(answer) => return answer,
        };
        let proposed = match Config::validate_all(&value) {
            Ok(proposed) => proposed,
            Err(violations) => return invalid_config(target, &violations),
        };
        let copy = match saved(self) {
            Ok(copy) => copy,
            Err(reason) => {
                crate::log_event("service", "config-read-failed", reason);
                return unavailable(target);
            }
        };
        let restart_required: Vec<Value> = classify(&copy.config, &proposed)
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

    pub(crate) fn missing_body_answer(target: &str) -> String {
        bad_request(target, "the body is missing")
    }

    /// Answers `PUT config`: replaces the saved file when `If-Match` names its revision, in the contract's order — validate, lock, revision, diff, write.
    /// Concurrency: one apply at a time service-wide, on the lock; the guard spans the revision check and the rename, so a second apply cannot write over a raced check.
    pub(crate) fn apply_answer(&self, target: &str, head: &[u8], body: &[u8]) -> String {
        let value = match config_body(target, body) {
            Ok(value) => value,
            Err(answer) => return answer,
        };
        let Some(user) = header(head, "Remote-User").filter(|user| remote_user_is_auditable(user))
        else {
            return unauthorized(target);
        };
        let proposed = match Config::validate_all(&value) {
            Ok(proposed) => proposed,
            Err(violations) => return invalid_config(target, &violations),
        };
        // A panic elsewhere poisoned the lock; the lock only orders applies and a rename either landed or did not, so the recovered guard stays valid.
        let _apply_guard = match self.apply_lock().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let copy = match saved(self) {
            Ok(copy) => copy,
            Err(reason) => {
                crate::log_event("service", "config-read-failed", reason);
                return unavailable(target);
            }
        };
        let current = revision(&copy.bytes);
        let expected = format!("\"{current}\"");
        let Some(if_match) = header(head, "If-Match") else {
            return problem(
                428,
                "revision_required",
                "a PUT needs an If-Match revision",
                target,
                None,
            );
        };
        if if_match != expected {
            return problem_with(
                412,
                "stale_revision",
                "the If-Match revision is not the saved one",
                target,
                None,
                &[
                    ("config", copy.config.to_value()),
                    ("revision", Value::String(current)),
                ],
            );
        }
        let changed = classify(&copy.config, &proposed);
        let rendered = match render(Some(&copy.text), &proposed) {
            Ok(rendered) => rendered,
            Err(error) => {
                crate::log_event("service", "config-apply-render-failed", error);
                return apply_incomplete(target);
            }
        };
        let new_revision = if rendered.as_bytes() == copy.bytes.as_slice() {
            current.clone()
        } else {
            match write_saved(&copy.path, &rendered) {
                Ok(()) => revision(rendered.as_bytes()),
                Err(error) => {
                    crate::log_event("service", "config-apply-write-failed", error);
                    return apply_incomplete(target);
                }
            }
        };
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        crate::log_event(
            "service",
            "config-applied",
            format_args!(
                "user=\"{user}\" when_unix_ms={} previous=\"{current}\" new=\"{new_revision}\" changed=\"{}\"",
                stamp.as_millis(),
                changed.join(",")
            ),
        );
        let restart_required: Vec<Value> = changed.into_iter().map(Value::String).collect();
        let mut answer = Map::new();
        answer.insert("revision".to_owned(), Value::String(new_revision));
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

fn required_length(target: &str, verb: &str, head: &[u8]) -> Result<usize, String> {
    match content_length(head) {
        None => Err(bad_request(
            target,
            &format!("a {verb} needs a valid Content-Length"),
        )),
        Some(length) if length > MAX_BODY_BYTES => Err(bad_request(
            target,
            &format!("body over the {MAX_BODY_BYTES} byte bound"),
        )),
        Some(length) => Ok(length),
    }
}

fn config_body(target: &str, body: &[u8]) -> Result<Value, String> {
    match serde_json::from_slice::<Value>(body) {
        Err(_) => Err(bad_request(target, "the body is not JSON")),
        Ok(value) if value.is_object() => Ok(value),
        Ok(_) => Err(bad_request(target, "the body is not a JSON object")),
    }
}

/// Whether a `Remote-User` is safe inside the audit line's quotes: a control
/// character or a double quote there could split or forge a line.
fn remote_user_is_auditable(user: &str) -> bool {
    !user.is_empty()
        && user
            .bytes()
            .all(|byte| (0x20..0x7f).contains(&byte) && byte != b'"')
}

fn header<'a>(head: &'a [u8], name: &str) -> Option<&'a str> {
    let text = std::str::from_utf8(head).ok()?;
    text.split("\r\n").skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then_some(value.trim())
    })
}

/// Lands `text` at `path` so a failure leaves the saved bytes: temp file in the same directory, one `fsync`, then `rename` over the original with the original's mode.
fn write_saved(path: &Path, text: &str) -> std::io::Result<()> {
    let mode = std::fs::metadata(path)?.permissions().mode();
    let temp = temp_path(path);
    let landed = write_temp_then_rename(&temp, path, text, mode);
    if landed.is_err() {
        // best effort: the saved file already stands unchanged whatever this cleanup does
        let _ = std::fs::remove_file(&temp);
    }
    landed
}

#[cfg(test)]
thread_local! { static FAIL_RENAME_ONCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

fn write_temp_then_rename(temp: &Path, path: &Path, text: &str, mode: u32) -> std::io::Result<()> {
    let mut file = std::fs::File::create(temp)?;
    file.write_all(text.as_bytes())?;
    file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    file.sync_all()?;
    drop(file);
    #[cfg(test)]
    if FAIL_RENAME_ONCE.with(std::cell::Cell::take) {
        return Err(std::io::Error::other("injected rename failure"));
    }
    std::fs::rename(temp, path)
}

fn temp_path(path: &Path) -> PathBuf {
    static TEMP_SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let stem = path.file_name().map_or_else(
        || "config".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    path.with_file_name(format!(
        ".{stem}.tmp-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn unauthorized(instance: &str) -> String {
    problem(
        401,
        "unauthorized",
        "no Remote-User arrived with the request",
        instance,
        None,
    )
}

fn apply_incomplete(instance: &str) -> String {
    problem(
        500,
        "apply_incomplete",
        "the configuration file did not land; nothing took effect",
        instance,
        None,
    )
}

fn content_length(head: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(head).ok()?;
    let mut announced: Option<&str> = None;
    for line in text.split("\r\n").skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        if announced.is_some() {
            return None;
        }
        announced = Some(value.trim());
    }
    let digits = announced?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse::<usize>().ok()
}

/// One read of the saved configuration; the apply needs the path, bytes, text and value from the same read, so a reader never mixes copies.
struct SavedCopy {
    path: PathBuf,
    bytes: Vec<u8>,
    text: String,
    config: Config,
}

fn saved(state: &ServiceState) -> Result<SavedCopy, String> {
    let path = state.config_path().ok_or("no configuration path")?;
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| format!("{} is not UTF-8: {error}", path.display()))?
        .to_owned();
    let config =
        Config::from_toml(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(SavedCopy {
        path: path.to_owned(),
        bytes,
        text,
        config,
    })
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
        && !holds_address_value(found, &violation.path)
    {
        entry.insert("found".to_owned(), found.clone());
    }
    if let Some(allowed) = &violation.allowed {
        entry.insert("allowed".to_owned(), allowed.clone());
    }
    Value::Object(entry)
}

fn holds_address_value(found: &Value, path: &str) -> bool {
    let last = path.rsplit('.').next().unwrap_or(path);
    let field = last.split('[').next().unwrap_or(last);
    let could_hold_address = found.is_string() || found.is_object() || found.is_array();
    could_hold_address && ADDRESS_FIELDS.contains(&field)
}

fn problem(
    status: u16,
    code: &str,
    detail: &str,
    instance: &str,
    errors: Option<Vec<Value>>,
) -> String {
    problem_with(status, code, detail, instance, errors, &[])
}

fn problem_with(
    status: u16,
    code: &str,
    detail: &str,
    instance: &str,
    errors: Option<Vec<Value>>,
    extras: &[(&str, Value)],
) -> String {
    let reason = match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        412 => "Precondition Failed",
        422 => "Unprocessable Content",
        428 => "Precondition Required",
        500 => "Internal Server Error",
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
    for (name, value) in extras {
        body.insert((*name).to_owned(), (*value).clone());
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

#[cfg(test)]
mod apply_unit_tests {
    use super::*;

    const TARGET: &str = "/corvette/api/v1/detection/config";
    const FILE: &str = "worker_socket = \"/run/detect.sock\"\nlabels = \"/run/labels.txt\"\ndeadline_ms = 5000\n\n[[cameras]]\nname = \"front\"\nwidth = 640\nheight = 480\nfps = 5\n";
    const GOOD: &str = r#"{"worker_socket":"/run/detect.sock","labels":"/run/labels.txt","deadline_ms":5000,"cameras":[{"name":"front","width":640,"height":480,"fps":5,"confirm_threshold":0.7}]}"#;

    fn state_with_saved(test: &str) -> (ServiceState, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("corvette-apply-unit-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp directory");
        let path = dir.join("detection.toml");
        std::fs::write(&path, FILE).expect("write saved configuration");
        let state = ServiceState::new(Vec::new()).with_config_path(path.clone());
        (state, path)
    }

    fn put_head(user: &str, if_match: &str) -> Vec<u8> {
        format!("PUT {TARGET} HTTP/1.1\r\nRemote-User: {user}\r\nIf-Match: {if_match}\r\n\r\n")
            .into_bytes()
    }

    #[test]
    fn a_poisoned_apply_lock_is_recovered_and_the_apply_succeeds() {
        let (state, path) = state_with_saved("poison");
        let poisoned = std::panic::AssertUnwindSafe(state.apply_lock());
        let outcome = std::panic::catch_unwind(move || {
            let _guard = poisoned.lock().expect("fresh lock");
            panic!("poison the apply lock on purpose");
        });
        assert!(outcome.is_err(), "the poisoning panic must have run");
        let head = put_head(
            "unit-poison",
            &format!("\"{}\"", revision(&std::fs::read(&path).expect("read"))),
        );
        let answer = state.apply_answer(TARGET, &head, GOOD.as_bytes());
        assert!(
            answer.starts_with("HTTP/1.1 200 "),
            "recovery should apply: {answer}"
        );
    }

    #[test]
    fn a_successful_apply_logs_one_audit_line_with_user_revisions_and_no_secret() {
        let (state, path) = state_with_saved("audit");
        let previous = revision(&std::fs::read(&path).expect("read"));
        let body = r#"{"worker_socket":"/run/detect.sock","labels":"/run/labels.txt","deadline_ms":5000,"cameras":[{"name":"front","width":640,"height":480,"fps":7,"area":{"name":"rtsp://op:sekret9@host/live","polygon":[[0.0,0.0],[0.9,0.0],[0.9,0.9]]}}]}"#;
        let head = put_head("audit-user", &format!("\"{previous}\""));
        let answer = state.apply_answer(TARGET, &head, body.as_bytes());
        assert!(
            answer.starts_with("HTTP/1.1 200 "),
            "apply should succeed: {answer}"
        );
        let new = revision(&std::fs::read(&path).expect("read back"));
        let lines: Vec<String> = crate::TEST_LOG
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|line| line.contains("user=\"audit-user\""))
            .cloned()
            .collect();
        assert_eq!(lines.len(), 1, "exactly one audit line: {lines:?}");
        let line = &lines[0];
        assert!(line.contains("event=config-applied"));
        assert!(line.contains(&previous), "previous revision: {line}");
        assert!(line.contains(&new), "new revision: {line}");
        assert!(
            line.contains("changed=\"cameras[front]\""),
            "changed paths: {line}"
        );
        assert!(
            !line.contains("sekret9"),
            "the audit line echoed a value: {line}"
        );
    }

    #[test]
    fn a_remote_user_with_a_control_character_or_quote_is_refused() {
        for (user, auditable) in [
            ("operator", true),
            ("first last.name", true),
            ("", false),
            ("first\nlast", false),
            ("first\rlast", false),
            ("first\tlast", false),
            ("first\u{7f}last", false),
            ("first\"last", false),
        ] {
            let head = format!("PUT {TARGET} HTTP/1.1\r\nRemote-User: {user}\r\n\r\n").into_bytes();
            let accepted = header(&head, "Remote-User").is_some_and(remote_user_is_auditable);
            assert_eq!(accepted, auditable, "for user {user:?}");
        }
    }

    #[test]
    fn a_rename_failure_after_the_temp_landed_removes_the_temp_and_answers_apply_incomplete() {
        let (state, path) = state_with_saved("rename_fail");
        let current = revision(&std::fs::read(&path).expect("read"));
        let head = put_head("operator", &format!("\"{current}\""));
        FAIL_RENAME_ONCE.with(|armed| armed.set(true));
        let body = GOOD.replace("\"fps\":5", "\"fps\":7");
        let answer = state.apply_answer(TARGET, &head, body.as_bytes());
        assert!(
            answer.starts_with("HTTP/1.1 500 "),
            "the injected rename failure should answer 500: {answer}"
        );
        assert!(
            answer.contains("\"code\":\"apply_incomplete\""),
            "problem body: {answer}"
        );
        assert_eq!(
            std::fs::read(&path).expect("read after"),
            FILE.as_bytes(),
            "the saved file must stand unchanged"
        );
        let leftovers: Vec<String> = std::fs::read_dir(path.parent().expect("directory"))
            .expect("read directory")
            .filter_map(|entry| {
                entry
                    .ok()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
            })
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }
}
