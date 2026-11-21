use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use corvette_detect::{
    Config, SCHEMA_JSON, ServiceState, classify, render, revision, serve_with_timeout,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

const READ_TIMEOUT: Duration = Duration::from_millis(500);
const CONFIG_PATH: &str = "/corvette/api/v1/detection/config";
const VALIDATE_PATH: &str = "/corvette/api/v1/detection/config/validate";
const SCHEMA_PATH: &str = "/corvette/api/v1/detection/config/schema";
const BOUND_PLUS_ONE: usize = 262_144 + 1;

// The saved file omits confirm_threshold so the read route's normalization is visible.
const SAVED: &str = r#"worker_socket = "/run/detect.sock"
labels = "/run/labels.txt"
deadline_ms = 5000

[[cameras]]
name = "front"
width = 640
height = 480
fps = 5
"#;

const PROPOSED_SAME: &str = r#"{"worker_socket":"/run/detect.sock","labels":"/run/labels.txt","deadline_ms":5000,"cameras":[{"name":"front","width":640,"height":480,"fps":5,"confirm_threshold":0.7}]}"#;

const COMMENTED: &str = "# kept-top\nworker_socket = \"/run/detect.sock\"\nlabels = \"/run/labels.txt\"\ndeadline_ms = 5000\n\n[[cameras]]\n# kept-camera\nname = \"front\"\nwidth = 640\nheight = 480\nfps = 5\n";

fn put_request(body: &[u8], user: Option<&str>, if_match: Option<&str>) -> Vec<u8> {
    let user_line = user.map_or_else(String::new, |user| format!("Remote-User: {user}\r\n"));
    let match_line =
        if_match.map_or_else(String::new, |if_match| format!("If-Match: {if_match}\r\n"));
    let length_line = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut bytes =
        format!("PUT {CONFIG_PATH} HTTP/1.1\r\n{user_line}{match_line}{length_line}").into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn dir(test: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "corvette-config-routes-{test}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("temp directory");
    path
}

fn saved_file(test: &str, contents: &str) -> PathBuf {
    let path = dir(test).join("detection.toml");
    std::fs::write(&path, contents).expect("write saved configuration");
    path
}

fn start(test: &str, config: Option<PathBuf>) -> PathBuf {
    let socket = dir(test).join("s.sock");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind");
    let mut state = ServiceState::new(Vec::new());
    if let Some(path) = config {
        state = state.with_config_path(path);
    }
    tokio::spawn(serve_with_timeout(listener, Arc::new(state), READ_TIMEOUT));
    socket
}

async fn request(socket: &Path, bytes: &[u8]) -> String {
    let mut stream = UnixStream::connect(socket).await.expect("connect");
    stream.write_all(bytes).await.expect("write request");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("eof");
    String::from_utf8(response).expect("utf8 response")
}

fn get(path: &str) -> Vec<u8> {
    format!("GET {path} HTTP/1.1\r\n\r\n").into_bytes()
}

fn post(path: &str, body: &[u8]) -> Vec<u8> {
    let head = format!(
        "POST {path} HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn status_code(response: &str) -> u16 {
    let line = response.split("\r\n").next().expect("status line");
    line.split(' ')
        .nth(1)
        .expect("code")
        .parse()
        .expect("code number")
}

fn body(response: &str) -> &str {
    response.split_once("\r\n\r\n").expect("head separator").1
}

fn header_value(response: &str, name: &str) -> String {
    let head = response.split_once("\r\n\r\n").expect("head separator").0;
    head.lines()
        .skip(1)
        .find_map(|line| {
            line.split_once(": ")
                .filter(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.to_owned())
        })
        .unwrap_or_else(|| panic!("no {name} header in {response}"))
}

fn problem(response: &str) -> Value {
    serde_json::from_str(body(response)).expect("problem json body")
}

#[tokio::test]
async fn get_config_answers_the_normalized_value_and_an_etag_equal_to_the_file_revision() {
    let path = saved_file("get_value", SAVED);
    let socket = start("get_value", Some(path.clone()));
    let bytes = std::fs::read(&path).expect("read saved file");
    let response = request(&socket, &get(CONFIG_PATH)).await;
    assert_eq!(status_code(&response), 200);
    let expected = Config::from_toml(SAVED)
        .expect("valid saved configuration")
        .to_value();
    let value: Value = serde_json::from_str(body(&response)).expect("json body");
    assert_eq!(value, expected);
    assert_eq!(
        header_value(&response, "ETag"),
        format!("\"{}\"", revision(&bytes))
    );
    assert_eq!(header_value(&response, "Cache-Control"), "no-store");
    assert_eq!(header_value(&response, "Content-Type"), "application/json");
}

#[tokio::test]
async fn get_config_shows_a_defaulted_field_the_saved_file_omits_filled_in() {
    let path = saved_file("get_default", SAVED);
    let socket = start("get_default", Some(path));
    let response = request(&socket, &get(CONFIG_PATH)).await;
    assert_eq!(status_code(&response), 200);
    let value: Value = serde_json::from_str(body(&response)).expect("json body");
    assert_eq!(value["cameras"][0]["confirm_threshold"], 0.7);
}

#[tokio::test]
async fn validate_of_the_saved_value_answers_valid_and_changes_nothing() {
    let path = saved_file("validate_same", SAVED);
    let socket = start("validate_same", Some(path.clone()));
    let before = std::fs::read(&path).expect("read before");
    let response = request(&socket, &post(VALIDATE_PATH, PROPOSED_SAME.as_bytes())).await;
    assert_eq!(status_code(&response), 200);
    let value: Value = serde_json::from_str(body(&response)).expect("json body");
    assert_eq!(value, json!({"valid": true, "restart_required": []}));
    assert_eq!(std::fs::read(&path).expect("read after"), before);
}

#[tokio::test]
async fn validate_of_one_changed_camera_names_only_that_camera() {
    let path = saved_file("validate_change", SAVED);
    let socket = start("validate_change", Some(path));
    let proposed = PROPOSED_SAME.replace("\"fps\":5", "\"fps\":7");
    let response = request(&socket, &post(VALIDATE_PATH, proposed.as_bytes())).await;
    assert_eq!(status_code(&response), 200);
    let value: Value = serde_json::from_str(body(&response)).expect("json body");
    assert_eq!(
        value,
        json!({"valid": true, "restart_required": ["cameras[front]"]})
    );
}

#[tokio::test]
async fn validate_of_three_faults_lists_three_errors_in_the_loaders_order() {
    let path = saved_file("validate_three", SAVED);
    let socket = start("validate_three", Some(path));
    let body_json = r#"{"cameras":[{"name":"front","width":640,"height":480,"fps":5}]}"#;
    let response = request(&socket, &post(VALIDATE_PATH, body_json.as_bytes())).await;
    assert_eq!(status_code(&response), 422);
    assert_eq!(
        header_value(&response, "Content-Type"),
        "application/problem+json"
    );
    let problem = problem(&response);
    assert_eq!(problem["type"], "about:blank");
    assert_eq!(problem["title"], "Unprocessable Content");
    assert_eq!(problem["status"], 422);
    assert_eq!(problem["code"], "invalid_config");
    assert_eq!(problem["detail"], "3 configuration errors");
    assert_eq!(problem["instance"], VALIDATE_PATH);
    let errors = problem["errors"].as_array().expect("errors array");
    assert_eq!(errors.len(), 3);
    let paths: Vec<&str> = errors
        .iter()
        .map(|error| error["path"].as_str().expect("error path"))
        .collect();
    assert_eq!(paths, ["worker_socket", "labels", "deadline_ms"]);
    assert!(
        errors
            .iter()
            .all(|error| error["rule"] == "missing" && error.get("found").is_none())
    );
}

#[tokio::test]
async fn validate_reports_found_and_allowed_for_a_bounded_value() {
    let path = saved_file("validate_bounded", SAVED);
    let socket = start("validate_bounded", Some(path));
    let proposed = PROPOSED_SAME.replace("\"deadline_ms\":5000", "\"deadline_ms\":0");
    let response = request(&socket, &post(VALIDATE_PATH, proposed.as_bytes())).await;
    assert_eq!(status_code(&response), 422);
    let problem = problem(&response);
    assert_eq!(problem["detail"], "1 configuration error");
    let errors = problem["errors"].as_array().expect("errors array");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["path"], "deadline_ms");
    assert_eq!(errors[0]["rule"], "0 is below 1");
    assert_eq!(errors[0]["found"], 0);
    assert_eq!(errors[0]["allowed"], json!({"min": 1, "max": 600_000}));
}

#[tokio::test]
async fn validate_drops_a_rejected_stream_value_so_the_address_is_never_echoed() {
    let path = saved_file("validate_secret", SAVED);
    let socket = start("validate_secret", Some(path));
    let proposed = PROPOSED_SAME.replace(
        "\"confirm_threshold\":0.7}",
        "\"confirm_threshold\":0.7,\"stream\":\"rtsp://cam:secret123@host/live\"}",
    );
    let response = request(&socket, &post(VALIDATE_PATH, proposed.as_bytes())).await;
    assert_eq!(status_code(&response), 422);
    assert!(
        !response.contains("secret123"),
        "the response echoed the camera's address: {response}"
    );
    let problem = problem(&response);
    let errors = problem["errors"].as_array().expect("errors array");
    assert_eq!(errors[0]["path"], "cameras[0].stream");
    assert_eq!(errors[0]["rule"], "unknown key");
    assert!(errors[0].get("found").is_none());
    assert!(errors[0]["allowed"].is_array());
}

#[tokio::test]
async fn schema_answers_the_embedded_document_byte_for_byte() {
    let path = saved_file("schema", SAVED);
    let socket = start("schema", Some(path));
    let response = request(&socket, &get(SCHEMA_PATH)).await;
    assert_eq!(status_code(&response), 200);
    assert_eq!(body(&response), SCHEMA_JSON);
    assert_eq!(
        header_value(&response, "Content-Type"),
        "application/schema+json"
    );
    assert_eq!(header_value(&response, "Cache-Control"), "no-store");
}

#[tokio::test]
async fn oversized_missing_or_unparseable_bodies_each_answer_bad_request() {
    let path = saved_file("bad_bodies", SAVED);
    let socket = start("bad_bodies", Some(path));
    let over_bound =
        format!("POST {VALIDATE_PATH} HTTP/1.1\r\nContent-Length: {BOUND_PLUS_ONE}\r\n\r\n");
    for bytes in [
        over_bound.into_bytes(),
        format!("POST {VALIDATE_PATH} HTTP/1.1\r\n\r\n").into_bytes(),
        post(VALIDATE_PATH, b"abc"),
        post(VALIDATE_PATH, b"[1,2]"),
    ] {
        let response = request(&socket, &bytes).await;
        assert_eq!(status_code(&response), 400);
        let problem = problem(&response);
        assert_eq!(problem["code"], "bad_request");
        assert_eq!(problem["status"], 400);
        assert_eq!(problem["instance"], VALIDATE_PATH);
    }
}

#[tokio::test]
async fn wrong_methods_answer_not_allowed_with_the_methods_that_route_accepts() {
    let path = saved_file("methods", SAVED);
    let socket = start("methods", Some(path));
    for (bytes, allowed) in [
        (get(VALIDATE_PATH), "POST"),
        (
            format!("POST {CONFIG_PATH} HTTP/1.1\r\n\r\n").into_bytes(),
            "GET, PUT",
        ),
        (
            format!("POST {SCHEMA_PATH} HTTP/1.1\r\n\r\n").into_bytes(),
            "GET",
        ),
    ] {
        let response = request(&socket, &bytes).await;
        assert_eq!(status_code(&response), 405);
        assert_eq!(header_value(&response, "Allow"), allowed);
    }
}

#[tokio::test]
async fn every_configuration_route_without_a_config_path_answers_unavailable() {
    let socket = start("no_path", None);
    for (path, bytes) in [
        (CONFIG_PATH, get(CONFIG_PATH)),
        (SCHEMA_PATH, get(SCHEMA_PATH)),
        (VALIDATE_PATH, post(VALIDATE_PATH, b"{}")),
    ] {
        let response = request(&socket, &bytes).await;
        assert_eq!(status_code(&response), 503, "for {path}");
        let problem = problem(&response);
        assert_eq!(problem["code"], "config_unavailable");
        assert_eq!(problem["status"], 503);
        assert_eq!(problem["instance"], path);
    }
}

#[tokio::test]
async fn duplicate_content_length_headers_are_refused_whatever_their_order() {
    let path = saved_file("dup_cl", SAVED);
    let socket = start("dup_cl", Some(path));
    let body: &[u8] = br#"{"a":1}"#;
    let length = body.len();
    for headers in [
        format!("Content-Length: {length}\r\nContent-Length: {length}"),
        format!("Content-Length: x\r\nContent-Length: {length}"),
    ] {
        let mut bytes = format!("POST {VALIDATE_PATH} HTTP/1.1\r\n{headers}\r\n\r\n").into_bytes();
        bytes.extend_from_slice(body);
        let response = request(&socket, &bytes).await;
        assert_eq!(status_code(&response), 400, "for {headers:?}");
        let problem = problem(&response);
        assert_eq!(problem["code"], "bad_request");
        assert_eq!(problem["detail"], "a POST needs a valid Content-Length");
    }
}

#[tokio::test]
async fn signed_or_spaced_content_length_values_are_refused() {
    let path = saved_file("cl_values", SAVED);
    let socket = start("cl_values", Some(path));
    let body: &[u8] = br#"{"a":1}"#;
    let length = body.len();
    for value in [
        format!("+{length}"),
        format!("-{length}"),
        format!("{length} {length}"),
        format!("{length}x"),
        String::new(),
    ] {
        let mut bytes = format!("POST {VALIDATE_PATH} HTTP/1.1\r\nContent-Length: {value}\r\n\r\n")
            .into_bytes();
        bytes.extend_from_slice(body);
        let response = request(&socket, &bytes).await;
        assert_eq!(status_code(&response), 400, "for Content-Length: {value:?}");
        assert_eq!(problem(&response)["code"], "bad_request");
    }
}

#[tokio::test]
async fn an_announced_body_that_never_arrives_answers_bad_request() {
    let path = saved_file("short_body", SAVED);
    let socket = start("short_body", Some(path));
    let mut bytes =
        format!("POST {VALIDATE_PATH} HTTP/1.1\r\nContent-Length: 40\r\n\r\n").into_bytes();
    bytes.extend_from_slice(br#"{"a":1}"#);
    let response = request(&socket, &bytes).await;
    assert_eq!(status_code(&response), 400);
    let problem = problem(&response);
    assert_eq!(problem["code"], "bad_request");
    assert_eq!(problem["detail"], "the body is missing");
    assert_eq!(problem["instance"], VALIDATE_PATH);
}

#[tokio::test]
async fn a_body_exactly_at_the_size_bound_is_not_refused_for_size() {
    let path = saved_file("exact_bound", SAVED);
    let socket = start("exact_bound", Some(path));
    // The object wrapper costs 10 bytes, so the padding lands the body on the bound exactly.
    let json = format!(r#"{{"pad":"{}"}}"#, "x".repeat(262_144 - 10));
    assert_eq!(json.len(), 262_144);
    let response = request(&socket, &post(VALIDATE_PATH, json.as_bytes())).await;
    let problem = problem(&response);
    assert_eq!(status_code(&response), 422, "detail: {}", problem["detail"]);
    assert_eq!(problem["code"], "invalid_config");
}

#[tokio::test]
async fn a_non_string_value_under_an_address_field_is_not_echoed() {
    let path = saved_file("address_object", SAVED);
    let socket = start("address_object", Some(path));
    let proposed = PROPOSED_SAME.replace(
        "\"worker_socket\":\"/run/detect.sock\"",
        "\"worker_socket\":{\"login\":\"rtsp://user:s3cret@host\"}",
    );
    let response = request(&socket, &post(VALIDATE_PATH, proposed.as_bytes())).await;
    assert_eq!(status_code(&response), 422);
    assert!(
        !response.contains("s3cret"),
        "the response echoed the address: {response}"
    );
    let problem = problem(&response);
    let errors = problem["errors"].as_array().expect("errors array");
    let entry = errors
        .iter()
        .find(|error| error["path"] == "worker_socket")
        .expect("a worker_socket error");
    assert_eq!(entry["rule"], "expected a string, found a table");
    assert!(entry.get("found").is_none());
}

#[tokio::test]
async fn a_lowercase_content_length_header_is_accepted() {
    let path = saved_file("lower_cl", SAVED);
    let socket = start("lower_cl", Some(path));
    let body: &[u8] = br#"{"a":1}"#;
    let mut bytes = format!(
        "POST {VALIDATE_PATH} HTTP/1.1\r\ncontent-length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    let response = request(&socket, &bytes).await;
    assert_eq!(status_code(&response), 422);
    assert_eq!(problem(&response)["code"], "invalid_config");
}

#[tokio::test]
async fn a_saved_file_deleted_after_start_answers_unavailable() {
    let path = saved_file("deleted", SAVED);
    let socket = start("deleted", Some(path.clone()));
    std::fs::remove_file(&path).expect("delete the saved file");
    let response = request(&socket, &get(CONFIG_PATH)).await;
    assert_eq!(status_code(&response), 503);
    assert_eq!(problem(&response)["code"], "config_unavailable");
    let validated = request(&socket, &post(VALIDATE_PATH, PROPOSED_SAME.as_bytes())).await;
    assert_eq!(status_code(&validated), 503);
    assert_eq!(problem(&validated)["code"], "config_unavailable");
}

#[tokio::test]
async fn apply_a_changed_value_writes_the_rendered_file_and_answers_the_new_revision() {
    let path = saved_file("apply_ok", SAVED);
    let socket = start("apply_ok", Some(path.clone()));
    let before = std::fs::read(&path).expect("read before");
    let proposed = PROPOSED_SAME.replace("\"fps\":5", "\"fps\":7");
    let response = request(
        &socket,
        &put_request(
            proposed.as_bytes(),
            Some("operator"),
            Some(&format!("\"{}\"", revision(&before))),
        ),
    )
    .await;
    assert_eq!(status_code(&response), 200);
    assert_eq!(header_value(&response, "Content-Type"), "application/json");
    assert_eq!(header_value(&response, "Cache-Control"), "no-store");
    assert!(
        !response.contains("ETag:"),
        "the apply answer must carry no ETag: {response}"
    );
    let answer: Value = serde_json::from_str(body(&response)).expect("json body");
    let written = std::fs::read(&path).expect("read after");
    let proposed_config =
        Config::from_value(&serde_json::from_str::<Value>(&proposed).expect("proposed json"))
            .expect("proposed valid");
    assert_eq!(
        written,
        render(Some(SAVED), &proposed_config)
            .expect("render")
            .as_bytes()
    );
    assert_eq!(answer["revision"], revision(&written));
    let restart: Vec<Value> = classify(&Config::from_toml(SAVED).expect("saved"), &proposed_config)
        .into_iter()
        .map(Value::String)
        .collect();
    assert_eq!(answer["restart_required"], Value::Array(restart));
}

#[tokio::test]
async fn apply_of_the_saved_value_writes_nothing_and_answers_the_same_revision() {
    let path = saved_file("apply_same", SAVED);
    let socket = start("apply_same", Some(path.clone()));
    let before = std::fs::read(&path).expect("read before");
    let response = request(
        &socket,
        &put_request(
            PROPOSED_SAME.as_bytes(),
            Some("operator"),
            Some(&format!("\"{}\"", revision(&before))),
        ),
    )
    .await;
    assert_eq!(status_code(&response), 200);
    let answer: Value = serde_json::from_str(body(&response)).expect("json body");
    assert_eq!(answer["revision"], revision(&before));
    assert_eq!(answer["restart_required"], json!([]));
    assert_eq!(std::fs::read(&path).expect("read after"), before);
}

#[tokio::test]
async fn apply_without_if_match_answers_revision_required_and_writes_nothing() {
    let path = saved_file("apply_no_if_match", SAVED);
    let socket = start("apply_no_if_match", Some(path.clone()));
    let response = request(
        &socket,
        &put_request(PROPOSED_SAME.as_bytes(), Some("operator"), None),
    )
    .await;
    assert_eq!(status_code(&response), 428);
    let problem = problem(&response);
    assert_eq!(problem["code"], "revision_required");
    assert_eq!(problem["status"], 428);
    assert_eq!(problem["instance"], CONFIG_PATH);
    assert!(problem.get("config").is_none());
    assert_eq!(std::fs::read(&path).expect("read after"), SAVED.as_bytes());
}

#[tokio::test]
async fn apply_with_a_stale_or_weak_if_match_answers_stale_revision_carrying_the_current_value() {
    let path = saved_file("apply_stale", SAVED);
    let socket = start("apply_stale", Some(path.clone()));
    let before = std::fs::read(&path).expect("read before");
    let current = revision(&before);
    let read_value: Value =
        serde_json::from_str(body(&request(&socket, &get(CONFIG_PATH)).await)).expect("read value");
    let proposed = PROPOSED_SAME.replace("\"fps\":5", "\"fps\":7");
    for if_match in [
        format!("\"sha256:{}\"", "0".repeat(64)),
        format!("W/\"{current}\""),
    ] {
        let response = request(
            &socket,
            &put_request(proposed.as_bytes(), Some("operator"), Some(&if_match)),
        )
        .await;
        assert_eq!(status_code(&response), 412, "for If-Match: {if_match}");
        let problem = problem(&response);
        assert_eq!(problem["code"], "stale_revision");
        assert_eq!(problem["status"], 412);
        assert_eq!(problem["config"], read_value);
        assert_eq!(problem["revision"], current);
        assert_eq!(std::fs::read(&path).expect("file untouched"), before);
    }
}

#[tokio::test]
async fn apply_validates_the_body_before_the_revision_check_and_leaves_the_file() {
    let path = saved_file("apply_invalid", SAVED);
    let socket = start("apply_invalid", Some(path.clone()));
    let before = std::fs::read(&path).expect("read before");
    let invalid = PROPOSED_SAME.replace("\"deadline_ms\":5000", "\"deadline_ms\":0");
    let response = request(
        &socket,
        &put_request(
            invalid.as_bytes(),
            Some("operator"),
            Some("\"sha256:0000000000000000000000000000000000000000000000000000000000000000\""),
        ),
    )
    .await;
    assert_eq!(status_code(&response), 422);
    assert_eq!(problem(&response)["code"], "invalid_config");
    assert_eq!(std::fs::read(&path).expect("read after"), before);
    let read_again = request(&socket, &get(CONFIG_PATH)).await;
    assert_eq!(
        header_value(&read_again, "ETag"),
        format!("\"{}\"", revision(&before))
    );
}

#[tokio::test]
async fn apply_without_or_with_an_empty_remote_user_answers_unauthorized() {
    let path = saved_file("apply_user", SAVED);
    let socket = start("apply_user", Some(path.clone()));
    let current = revision(&std::fs::read(&path).expect("read"));
    for user in [None, Some("")] {
        let response = request(
            &socket,
            &put_request(
                PROPOSED_SAME.as_bytes(),
                user,
                Some(&format!("\"{current}\"")),
            ),
        )
        .await;
        assert_eq!(status_code(&response), 401);
        let problem = problem(&response);
        assert_eq!(problem["code"], "unauthorized");
        assert_eq!(problem["status"], 401);
        assert_eq!(problem["instance"], CONFIG_PATH);
        assert_eq!(std::fs::read(&path).expect("read after"), SAVED.as_bytes());
    }
}

#[tokio::test]
async fn apply_into_a_read_only_directory_answers_apply_incomplete_and_changes_nothing() {
    if std::fs::read_to_string("/proc/self/uid").is_ok_and(|uid| uid.trim() == "0") {
        println!("skipped: running as uid 0, which writes into a read-only directory anyway");
        return;
    }
    let path = saved_file("apply_ro", SAVED);
    let socket = start("apply_ro", Some(path.clone()));
    let dir = path.parent().expect("directory");
    let before = std::fs::read(&path).expect("read before");
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o500))
        .expect("make the directory read-only");
    let proposed = PROPOSED_SAME.replace("\"fps\":5", "\"fps\":7");
    let response = request(
        &socket,
        &put_request(
            proposed.as_bytes(),
            Some("operator"),
            Some(&format!("\"{}\"", revision(&before))),
        ),
    )
    .await;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .expect("restore the directory");
    assert_eq!(status_code(&response), 500);
    let problem = problem(&response);
    assert_eq!(problem["code"], "apply_incomplete");
    assert_eq!(problem["status"], 500);
    assert!(
        !body(&response).contains(&dir.display().to_string()),
        "the answer body names a path: {}",
        body(&response)
    );
    assert_eq!(std::fs::read(&path).expect("read after"), before);
    let leftovers: Vec<String> = std::fs::read_dir(dir)
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_applies_at_one_revision_leave_exactly_one_winner() {
    for round in 0..20 {
        let test = format!("apply_race{round}");
        let path = saved_file(&test, SAVED);
        let socket = start(&test, Some(path.clone()));
        let current = revision(&std::fs::read(&path).expect("read"));
        let quoted = format!("\"{current}\"");
        let first = PROPOSED_SAME.replace("\"fps\":5", "\"fps\":7");
        let second = PROPOSED_SAME.replace("\"fps\":5", "\"fps\":9");
        let first_bytes = put_request(first.as_bytes(), Some("operator"), Some(&quoted));
        let second_bytes = put_request(second.as_bytes(), Some("operator"), Some(&quoted));
        let (one, two) = tokio::join!(
            request(&socket, &first_bytes),
            request(&socket, &second_bytes)
        );
        let (first_code, second_code) = (status_code(&one), status_code(&two));
        assert!(
            (first_code == 200 && second_code == 412) || (first_code == 412 && second_code == 200),
            "round {round}: codes {first_code} and {second_code}"
        );
        let winner = if first_code == 200 { &first } else { &second };
        let proposed = Config::from_value(&serde_json::from_str::<Value>(winner).expect("json"))
            .expect("valid");
        let expected = render(Some(SAVED), &proposed).expect("render");
        assert_eq!(
            std::fs::read(&path).expect("read final"),
            expected.as_bytes(),
            "round {round}"
        );
    }
}

#[tokio::test]
async fn apply_rewriting_a_camera_keeps_the_comments_around_it() {
    let path = saved_file("apply_comments", COMMENTED);
    let socket = start("apply_comments", Some(path.clone()));
    let current = revision(&std::fs::read(&path).expect("read"));
    let proposed = PROPOSED_SAME.replace("\"fps\":5", "\"fps\":7");
    let response = request(
        &socket,
        &put_request(
            proposed.as_bytes(),
            Some("operator"),
            Some(&format!("\"{current}\"")),
        ),
    )
    .await;
    assert_eq!(status_code(&response), 200);
    let text = std::fs::read_to_string(&path).expect("read back");
    assert!(text.contains("# kept-top"), "top comment lost: {text}");
    assert!(
        text.contains("# kept-camera"),
        "camera comment lost: {text}"
    );
}

#[tokio::test]
async fn apply_bodies_over_bound_missing_or_unshaped_each_answer_bad_request() {
    let path = saved_file("apply_bad_bodies", SAVED);
    let socket = start("apply_bad_bodies", Some(path.clone()));
    let current = revision(&std::fs::read(&path).expect("read"));
    let quoted = format!("\"{current}\"");
    let over = format!(
        "PUT {CONFIG_PATH} HTTP/1.1\r\nRemote-User: operator\r\nContent-Length: {BOUND_PLUS_ONE}\r\n\r\n"
    )
    .into_bytes();
    let no_length =
        format!("PUT {CONFIG_PATH} HTTP/1.1\r\nRemote-User: operator\r\n\r\n").into_bytes();
    for bytes in [
        over,
        no_length,
        put_request(b"abc", Some("operator"), Some(&quoted)),
        put_request(b"[1,2]", Some("operator"), Some(&quoted)),
    ] {
        let response = request(&socket, &bytes).await;
        assert_eq!(status_code(&response), 400);
        let problem = problem(&response);
        assert_eq!(problem["code"], "bad_request");
        assert_eq!(problem["instance"], CONFIG_PATH);
    }
}

#[tokio::test]
async fn patch_and_delete_on_the_config_path_answer_not_allowed_with_get_and_put() {
    let path = saved_file("apply_methods", SAVED);
    let socket = start("apply_methods", Some(path));
    for method in ["PATCH", "DELETE"] {
        let response = request(
            &socket,
            format!("{method} {CONFIG_PATH} HTTP/1.1\r\n\r\n").as_bytes(),
        )
        .await;
        assert_eq!(status_code(&response), 405);
        assert_eq!(header_value(&response, "Allow"), "GET, PUT");
    }
}

#[tokio::test]
async fn a_get_after_an_apply_answers_the_new_value_with_the_new_etag() {
    let path = saved_file("apply_then_get", SAVED);
    let socket = start("apply_then_get", Some(path.clone()));
    let before = std::fs::read(&path).expect("read before");
    let proposed = PROPOSED_SAME.replace("\"fps\":5", "\"fps\":7");
    let applied = request(
        &socket,
        &put_request(
            proposed.as_bytes(),
            Some("operator"),
            Some(&format!("\"{}\"", revision(&before))),
        ),
    )
    .await;
    assert_eq!(status_code(&applied), 200);
    let read = request(&socket, &get(CONFIG_PATH)).await;
    let written = std::fs::read(&path).expect("read final");
    assert_eq!(
        header_value(&read, "ETag"),
        format!("\"{}\"", revision(&written))
    );
    let value: Value = serde_json::from_str(body(&read)).expect("json body");
    assert_eq!(value["cameras"][0]["fps"], 7);
}

#[tokio::test]
async fn apply_without_a_config_path_answers_config_unavailable() {
    let socket = start("apply_no_path", None);
    let response = request(
        &socket,
        &put_request(
            PROPOSED_SAME.as_bytes(),
            Some("operator"),
            Some("\"sha256:0000000000000000000000000000000000000000000000000000000000000000\""),
        ),
    )
    .await;
    assert_eq!(status_code(&response), 503);
    assert_eq!(problem(&response)["code"], "config_unavailable");
    assert_eq!(problem(&response)["instance"], CONFIG_PATH);
}
