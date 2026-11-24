use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use corvette_detect::{
    CameraEntry, CameraHandle, CameraState, CameraStatus, FrameObjects, PipelineCounters, Policy,
    ServiceState, Status, Supervised, TrackedObject, serve_with_timeout,
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Command;
use tokio::sync::watch;

const READ_TIMEOUT: Duration = Duration::from_millis(200);
const STATUS: &str = "/corvette/api/v1/detection/status";

fn socket_path(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("corvette-service-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp directory");
    let socket = dir.join("s.sock");
    let _ = std::fs::remove_file(&socket);
    socket
}

fn start(test: &str, cameras: Vec<(String, CameraEntry)>) -> PathBuf {
    let socket = socket_path(test);
    let listener = UnixListener::bind(&socket).expect("bind");
    let state = Arc::new(ServiceState::new(cameras));
    tokio::spawn(serve_with_timeout(listener, state, READ_TIMEOUT));
    socket
}

fn start_with_worker(
    test: &str,
    cameras: Vec<(String, CameraEntry)>,
    worker: watch::Receiver<Status>,
) -> PathBuf {
    let socket = socket_path(test);
    let listener = UnixListener::bind(&socket).expect("bind");
    let state = Arc::new(ServiceState::new(cameras).with_worker(worker));
    tokio::spawn(serve_with_timeout(listener, state, READ_TIMEOUT));
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

fn header_lines(response: &str) -> Vec<&str> {
    let head = response.split_once("\r\n\r\n").expect("head separator").0;
    head.lines().skip(1).collect()
}

fn assert_common_headers(response: &str) {
    assert!(
        response.starts_with("HTTP/1.1 "),
        "status line first: {response}"
    );
    let lines = header_lines(response);
    assert!(
        lines.contains(&"Content-Type: application/json"),
        "{lines:?}"
    );
    assert!(lines.contains(&"Cache-Control: no-store"), "{lines:?}");
    assert!(lines.contains(&"Connection: close"), "{lines:?}");
    let length = lines
        .iter()
        .find_map(|l| l.strip_prefix("Content-Length: "))
        .expect("length");
    assert_eq!(
        length.parse::<usize>().expect("length number"),
        body(response).len()
    );
}

const fn running(frames_in: u64) -> CameraStatus {
    CameraStatus {
        state: CameraState::Running,
        counters: PipelineCounters {
            frames_in,
            frames_tracked: frames_in,
            frames_inference_failed: 0,
            inferences_run: 0,
            frames_without_region: 0,
        },
        source_gaps: 0,
        frames_dropped: 0,
        runner_restarts: 0,
        fps_in_tenths: 0,
        fps_processed_tenths: 0,
        worker_ms: 0,
        roundtrip_ms: 0,
        lock_wait_ms: 0,
    }
}

fn live_handle(status: CameraStatus, objects: Option<FrameObjects>) -> CameraHandle {
    let (_status_tx, status_rx) = watch::channel(status);
    let (_objects_tx, objects_rx) = watch::channel(objects);
    CameraHandle {
        status: status_rx,
        objects: objects_rx,
    }
}

fn object(id: &str) -> TrackedObject {
    TrackedObject {
        id: id.to_owned(),
        label: "person".to_owned(),
        score: 0.9,
        confirmed: true,
        bounds: [1.0, 2.0, 3.0, 4.0],
        edges: [10, 20, 30, 40],
        first_seen: 0.0,
        last_seen: 1.0,
        zones_now: Vec::new(),
        zones_entered: Vec::new(),
        stationary: false,
    }
}

fn frame(ids: &[&str]) -> FrameObjects {
    FrameObjects {
        frame_time: 1.0,
        objects: ids.iter().copied().map(object).collect(),
        ended: Vec::new(),
    }
}

fn objects_of(response: &str) -> Vec<String> {
    let value: Value = serde_json::from_str(body(response)).expect("json body");
    let objects = value["objects"].as_array().expect("objects array");
    objects
        .iter()
        .map(|object| object["id"].as_str().expect("string id").to_owned())
        .collect()
}

#[tokio::test]
async fn objects_path_returns_the_published_objects_of_that_camera() {
    let handle = live_handle(running(4), Some(frame(&["a1"])));
    let socket = start(
        "objects_one",
        vec![("front".to_owned(), CameraEntry::Live(handle))],
    );
    let response = request(&socket, &get("/corvette/api/v1/cameras/front/objects")).await;
    assert_eq!(status_code(&response), 200);
    assert_eq!(objects_of(&response), vec!["a1".to_owned()]);
}

#[tokio::test]
async fn two_cameras_return_only_their_own_object_ids() {
    let front = live_handle(running(4), Some(frame(&["a1", "a2"])));
    let back = live_handle(running(4), Some(frame(&["b1"])));
    let socket = start(
        "two_cameras",
        vec![
            ("front".to_owned(), CameraEntry::Live(front)),
            ("back".to_owned(), CameraEntry::Live(back)),
        ],
    );
    let front_response = request(&socket, &get("/corvette/api/v1/cameras/front/objects")).await;
    let back_response = request(&socket, &get("/corvette/api/v1/cameras/back/objects")).await;
    assert_eq!(
        objects_of(&front_response),
        vec!["a1".to_owned(), "a2".to_owned()]
    );
    assert_eq!(objects_of(&back_response), vec!["b1".to_owned()]);
}

#[tokio::test]
async fn camera_without_any_published_frame_answers_objects_unknown() {
    let handle = live_handle(running(0), None);
    let socket = start(
        "unknown",
        vec![("front".to_owned(), CameraEntry::Live(handle))],
    );
    let response = request(&socket, &get("/corvette/api/v1/cameras/front/objects")).await;
    assert_eq!(status_code(&response), 503);
    assert_eq!(body(&response), "{\"reason\":\"objects_unknown\"}");
}

#[tokio::test]
async fn empty_frame_and_off_camera_both_answer_an_empty_list() {
    let empty = live_handle(running(3), Some(frame(&[])));
    let socket = start(
        "empty_off",
        vec![
            ("live".to_owned(), CameraEntry::Live(empty)),
            ("dark".to_owned(), CameraEntry::Off),
        ],
    );
    let live_response = request(&socket, &get("/corvette/api/v1/cameras/live/objects")).await;
    let off_response = request(&socket, &get("/corvette/api/v1/cameras/dark/objects")).await;
    assert_eq!(status_code(&live_response), 200);
    assert_eq!(body(&live_response), "{\"objects\":[]}");
    assert_eq!(status_code(&off_response), 200);
    assert_eq!(body(&off_response), "{\"objects\":[]}");
}

#[tokio::test]
async fn unregistered_query_and_extra_segment_paths_answer_not_found() {
    let handle = live_handle(running(4), Some(frame(&["a1"])));
    let socket = start(
        "not_found",
        vec![("front".to_owned(), CameraEntry::Live(handle))],
    );
    let paths = [
        "/corvette/api/v1/cameras/nosuch/objects",
        "/corvette/api/v1/cameras/front/objects?camera=back",
        "/corvette/api/v1/cameras/front/extra/objects",
        "/corvette/api/v1/detection/status/extra",
    ];
    for path in paths {
        let response = request(&socket, &get(path)).await;
        assert_eq!(status_code(&response), 404, "for {path}");
        assert_eq!(body(&response), "{\"reason\":\"not_found\"}", "for {path}");
    }
}

#[tokio::test]
async fn percent_escapes_decode_to_a_registered_name_in_either_hex_case() {
    let handle = live_handle(running(4), Some(frame(&["a1"])));
    let socket = start(
        "decode",
        vec![("front".to_owned(), CameraEntry::Live(handle))],
    );
    for path in [
        "/corvette/api/v1/cameras/fr%6Fnt/objects",
        "/corvette/api/v1/cameras/fr%6fnt/objects",
    ] {
        let response = request(&socket, &get(path)).await;
        assert_eq!(status_code(&response), 200, "for {path}");
        assert_eq!(objects_of(&response), vec!["a1".to_owned()]);
    }
}

#[tokio::test]
async fn space_and_slash_cameras_answer_only_through_their_encoded_forms() {
    let socket = start(
        "encoded_names",
        vec![
            (
                "a".to_owned(),
                CameraEntry::Live(live_handle(running(4), Some(frame(&["x1"])))),
            ),
            (
                "a/b".to_owned(),
                CameraEntry::Live(live_handle(running(4), Some(frame(&["y1"])))),
            ),
            (
                "back yard".to_owned(),
                CameraEntry::Live(live_handle(running(4), Some(frame(&["b1"])))),
            ),
        ],
    );
    let cases = [
        (
            "/corvette/api/v1/cameras/back%20yard/objects",
            vec!["b1".to_owned()],
        ),
        (
            "/corvette/api/v1/cameras/a%2Fb/objects",
            vec!["y1".to_owned()],
        ),
        (
            "/corvette/api/v1/cameras/a%2fb/objects",
            vec!["y1".to_owned()],
        ),
    ];
    for (path, ids) in cases {
        let response = request(&socket, &get(path)).await;
        assert_eq!(status_code(&response), 200, "for {path}");
        assert_eq!(
            objects_of(&response),
            ids,
            "only the named camera's ids: {path}"
        );
    }
    let raw = request(&socket, &get("/corvette/api/v1/cameras/a/b/objects")).await;
    assert_eq!(
        status_code(&raw),
        404,
        "the raw slash path never reaches a camera"
    );
    assert_eq!(body(&raw), "{\"reason\":\"not_found\"}");
}

#[tokio::test]
async fn malformed_escapes_and_invalid_utf8_answer_not_found_even_when_registered_verbatim() {
    let socket = start(
        "malformed",
        vec![
            (
                "front".to_owned(),
                CameraEntry::Live(live_handle(running(4), Some(frame(&["a1"])))),
            ),
            (
                "front%".to_owned(),
                CameraEntry::Live(live_handle(running(4), Some(frame(&["p1"])))),
            ),
            (
                "fr%zznt".to_owned(),
                CameraEntry::Live(live_handle(running(4), Some(frame(&["z1"])))),
            ),
            (
                "fr%2nt".to_owned(),
                CameraEntry::Live(live_handle(running(4), Some(frame(&["t1"])))),
            ),
        ],
    );
    for path in [
        "/corvette/api/v1/cameras/front%/objects",
        "/corvette/api/v1/cameras/fr%zznt/objects",
        "/corvette/api/v1/cameras/fr%2nt/objects",
        "/corvette/api/v1/cameras/fr%ffnt/objects",
    ] {
        let response = request(&socket, &get(path)).await;
        assert_eq!(status_code(&response), 404, "for {path}");
        assert_eq!(body(&response), "{\"reason\":\"not_found\"}", "for {path}");
    }
    // The only way to reach a name carrying a percent sign is its own escape.
    let escaped = request(&socket, &get("/corvette/api/v1/cameras/front%25/objects")).await;
    assert_eq!(status_code(&escaped), 200);
    assert_eq!(objects_of(&escaped), vec!["p1".to_owned()]);
}

#[tokio::test]
async fn status_lists_cameras_in_registered_order_and_tracks_the_published_status() {
    let (status_tx, status_rx) = watch::channel(running(3));
    let (_, objects_rx) = watch::channel::<Option<FrameObjects>>(None);
    let a = CameraHandle {
        status: status_rx,
        objects: objects_rx,
    };
    let unavailable = CameraStatus {
        state: CameraState::InferenceUnavailable,
        counters: PipelineCounters {
            frames_in: 5,
            frames_tracked: 0,
            frames_inference_failed: 5,
            inferences_run: 0,
            frames_without_region: 0,
        },
        source_gaps: 1,
        frames_dropped: 0,
        runner_restarts: 0,
        fps_in_tenths: 0,
        fps_processed_tenths: 0,
        worker_ms: 0,
        roundtrip_ms: 0,
        lock_wait_ms: 0,
    };
    let b = live_handle(unavailable, None);
    let socket = start(
        "status_order",
        vec![
            ("a".to_owned(), CameraEntry::Live(a)),
            ("b".to_owned(), CameraEntry::Live(b)),
            ("c".to_owned(), CameraEntry::Off),
        ],
    );
    let response = request(&socket, &get(STATUS)).await;
    assert_eq!(status_code(&response), 200);
    let value: Value = serde_json::from_str(body(&response)).expect("json body");
    let cameras = value["cameras"].as_array().expect("cameras array");
    assert_eq!(cameras.len(), 3);
    let names: Vec<&str> = cameras
        .iter()
        .map(|c| c["camera"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["a", "b", "c"]);
    assert_eq!(cameras[0]["state"], "running");
    assert_eq!(cameras[0]["frames_in"], 3);
    assert_eq!(cameras[1]["state"], "inference_unavailable");
    assert_eq!(cameras[1]["frames_inference_failed"], 5);
    assert_eq!(cameras[1]["source_gaps"], 1);
    assert_eq!(cameras[1]["frames_dropped"], 0);
    assert_eq!(cameras[1]["runner_restarts"], 0);
    assert_eq!(cameras[1]["fps_in"].to_string(), "0.0");
    assert_eq!(cameras[1]["fps_processed"].to_string(), "0.0");
    assert_eq!(cameras[1]["worker_ms"], 0);
    assert_eq!(cameras[1]["roundtrip_ms"], 0);
    assert_eq!(cameras[1]["lock_wait_ms"], 0);
    assert_eq!(cameras[2]["state"], "off");
    assert!(cameras[2].get("frames_in").is_none());
    assert!(cameras[2].get("fps_in").is_none());

    let updated = CameraStatus {
        state: CameraState::WaitingForFrames,
        counters: PipelineCounters {
            frames_in: 9,
            frames_tracked: 8,
            frames_inference_failed: 7,
            inferences_run: 0,
            frames_without_region: 0,
        },
        source_gaps: 6,
        frames_dropped: 0,
        runner_restarts: 0,
        fps_in_tenths: 0,
        fps_processed_tenths: 0,
        worker_ms: 0,
        roundtrip_ms: 0,
        lock_wait_ms: 0,
    };
    status_tx.send(updated).expect("send status");
    let response = request(&socket, &get(STATUS)).await;
    let value: Value = serde_json::from_str(body(&response)).expect("json body");
    let a = &value["cameras"].as_array().expect("cameras array")[0];
    assert_eq!(a["state"], "waiting_for_frames");
    assert_eq!(a["frames_in"], 9);
    assert_eq!(a["frames_tracked"], 8);
    assert_eq!(a["frames_inference_failed"], 7);
    assert_eq!(a["source_gaps"], 6);
}

#[tokio::test]
async fn worker_channel_marks_live_entries_only_and_follows_the_next_publish() {
    let initial = Status {
        starts: 2,
        running: true,
        ..Status::default()
    };
    let (sender, receiver) = watch::channel(initial);
    let socket = start_with_worker(
        "worker_channel",
        vec![
            (
                "live".to_owned(),
                CameraEntry::Live(live_handle(running(3), None)),
            ),
            ("dark".to_owned(), CameraEntry::Off),
        ],
        receiver,
    );
    let response = request(&socket, &get(STATUS)).await;
    assert_eq!(status_code(&response), 200);
    let value: Value = serde_json::from_str(body(&response)).expect("json body");
    let cameras = value["cameras"].as_array().expect("cameras array");
    assert_eq!(cameras[0]["worker_state"], "running");
    assert_eq!(cameras[0]["worker_restarts"], 1);
    assert!(cameras[1].get("worker_state").is_none());
    assert!(cameras[1].get("worker_restarts").is_none());
    sender
        .send(Status {
            running: false,
            ..initial
        })
        .expect("publish exited");
    let response = request(&socket, &get(STATUS)).await;
    let value: Value = serde_json::from_str(body(&response)).expect("json body");
    let cameras = value["cameras"].as_array().expect("cameras array");
    assert_eq!(cameras[0]["worker_state"], "exited");
    assert_eq!(cameras[0]["worker_restarts"], 1);
    assert!(cameras[1].get("worker_state").is_none());
}

fn shell(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(script);
    command
}

#[tokio::test]
async fn a_restarting_supervised_child_shows_exited_and_rising_restarts() {
    let policy = Policy {
        initial: Duration::from_millis(10),
        max: Duration::from_millis(40),
        healthy_after: Duration::from_mins(1),
    };
    let supervisor = Supervised::spawn(
        "status-worker".to_string(),
        policy,
        || shell("true"),
        |_| {},
    );
    let socket = start_with_worker(
        "worker_supervised",
        vec![(
            "a".to_owned(),
            CameraEntry::Live(live_handle(running(2), None)),
        )],
        supervisor.subscribe(),
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let response = request(&socket, &get(STATUS)).await;
        let value: Value = serde_json::from_str(body(&response)).expect("json body");
        let entry = &value["cameras"].as_array().expect("cameras array")[0];
        if entry["worker_state"] == "exited"
            && entry["worker_restarts"]
                .as_u64()
                .expect("worker_restarts number")
                >= 1
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the status never showed exited with a restart"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        supervisor.status().starts >= 2,
        "the subscribed status did not show starts rising"
    );
}

#[tokio::test]
async fn post_on_a_known_path_answers_method_not_allowed_with_allow_get() {
    let handle = live_handle(running(4), Some(frame(&["a1"])));
    let socket = start(
        "post",
        vec![("front".to_owned(), CameraEntry::Live(handle))],
    );
    for path in ["/corvette/api/v1/cameras/front/objects", STATUS] {
        let raw = format!("POST {path} HTTP/1.1\r\nContent-Length: 3\r\n\r\nabc");
        let response = request(&socket, raw.as_bytes()).await;
        assert_eq!(status_code(&response), 405, "for {path}");
        assert!(header_lines(&response).contains(&"Allow: GET"));
        assert_eq!(body(&response), "{\"reason\":\"method_not_allowed\"}");
    }
}

#[tokio::test]
async fn garbage_request_line_answers_bad_request() {
    let socket = start("garbage", Vec::new());
    let response = request(&socket, b"GARBAGE\r\n\r\n").await;
    assert_eq!(status_code(&response), 400);
    assert_eq!(body(&response), "{\"reason\":\"bad_request\"}");
}

#[tokio::test]
async fn request_head_over_the_cap_answers_too_large() {
    let socket = start("too_large", Vec::new());
    let pad = "x".repeat(8192);
    let head = format!("GET {STATUS} HTTP/1.1\r\nX-Pad: {pad}\r\n\r\n");
    let response = request(&socket, head.as_bytes()).await;
    assert_eq!(status_code(&response), 431);
}

#[tokio::test]
async fn stalled_client_does_not_block_another_client() {
    let handle = live_handle(running(4), Some(frame(&["a1"])));
    let socket = start(
        "stalled",
        vec![("front".to_owned(), CameraEntry::Live(handle))],
    );
    let stalled = UnixStream::connect(&socket).await.expect("connect");
    let response = request(&socket, &get("/corvette/api/v1/cameras/front/objects")).await;
    assert_eq!(status_code(&response), 200);
    drop(stalled);
}

#[tokio::test]
async fn every_response_carries_the_four_headers() {
    let live = live_handle(running(4), Some(frame(&["a1"])));
    let unknown = live_handle(running(0), None);
    let socket = start(
        "headers",
        vec![
            ("live".to_owned(), CameraEntry::Live(live)),
            ("unknown".to_owned(), CameraEntry::Live(unknown)),
        ],
    );
    let pad = "x".repeat(8192);
    let requests = [
        get("/corvette/api/v1/cameras/live/objects"),
        get("/corvette/api/v1/cameras/unknown/objects"),
        get(STATUS),
        get("/nope"),
        b"DELETE /corvette/api/v1/cameras/live/objects HTTP/1.1\r\n\r\n".to_vec(),
        b"GARBAGE\r\n\r\n".to_vec(),
        format!("GET {STATUS} HTTP/1.1\r\nX-Pad: {pad}\r\n\r\n").into_bytes(),
    ];
    for bytes in requests {
        assert_common_headers(&request(&socket, &bytes).await);
    }
}
