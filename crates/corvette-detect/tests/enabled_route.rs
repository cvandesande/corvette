// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and the crate lints do not forbid them.
use corvette_detect::{
    CameraChannels, CameraRecipe, CameraStartup, CameraThreadSettings, Config, DEFAULT_POLICY,
    EventFeed, ServiceState, SharedLink, WorkerLink, serve, stream_address,
};
use std::fmt::Write;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::time::timeout;

const CALL: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(5);

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "corvette-enabled-route-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn stub_ffmpeg(dir: &Path) -> PathBuf {
    let path = dir.join("stub-ffmpeg");
    fs::write(&path, "#!/bin/sh\nexec sleep 30\n").unwrap();
    fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn camera_config(name: &str) -> corvette_detect::CameraConfig {
    corvette_detect::CameraConfig {
        name: name.to_owned(),
        width: 160,
        height: 120,
        fps: 10,
        enabled: true,
        confirm_threshold: 0.7,
        area: None,
        source: None,
    }
}

fn loaded_config(dir: &Path) -> Config {
    let labels = dir.join("labels.txt");
    fs::write(&labels, "0 person\n").unwrap();
    Config {
        worker_socket: dir.join("worker.sock"),
        labels,
        deadline: Duration::from_secs(5),
        cameras: vec![],
    }
}

fn startup(dir: &Path, ffmpeg: &Path, name: &str, config: Config) -> CameraStartup {
    let camera = camera_config(name);
    let address = stream_address(&camera, "restream.invalid", &|_| None).unwrap();
    let link = SharedLink::new(WorkerLink::new(
        dir.join("worker.sock"),
        Duration::from_secs(5),
        DEFAULT_POLICY,
    ));
    CameraStartup {
        name: name.to_owned(),
        channels: CameraChannels::new(),
        recipe: CameraRecipe::new(
            config,
            camera,
            ffmpeg.to_path_buf(),
            address,
            link,
            CameraThreadSettings {
                frame_timeout: Duration::from_millis(500),
                backoff: DEFAULT_POLICY,
            },
        ),
        enabled: true,
    }
}

async fn serve_ipc(dir: &Path, startups: Vec<CameraStartup>) -> (PathBuf, Arc<ServiceState>) {
    let state = Arc::new(ServiceState::new(Vec::new()).with_cameras(startups));
    let feed = Arc::new(EventFeed::new());
    let socket = dir.join("ipc.sock");
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).unwrap();
    tokio::spawn(serve(
        listener,
        Arc::clone(&state),
        Some(feed),
        Some(state.toggles()),
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;
    (socket, state)
}

async fn serve_plain_no_toggles(dir: &Path, tag: &str) -> PathBuf {
    let socket = dir.join(format!("{tag}.sock"));
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).unwrap();
    tokio::spawn(serve(
        listener,
        Arc::new(ServiceState::new(Vec::new())),
        None,
        None,
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;
    socket
}

async fn post(
    socket: &Path,
    camera: &str,
    body: &str,
    user: Option<&str>,
) -> (u16, String, String) {
    let mut request = format!("POST /corvette/api/v1/cameras/{camera}/enabled HTTP/1.1\r\n");
    if let Some(user) = user {
        request
            .write_fmt(format_args!("Remote-User: {user}\r\n"))
            .unwrap();
    }
    request
        .write_fmt(format_args!("Content-Length: {}\r\n\r\n{body}", body.len()))
        .unwrap();
    let mut stream = UnixStream::connect(socket).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    timeout(READ_TIMEOUT, stream.read_to_end(&mut response))
        .await
        .expect("the answer arrives")
        .expect("the read completes");
    let text = String::from_utf8_lossy(&response).into_owned();
    let (head, answer_body) = text.split_once("\r\n\r\n").expect("a head and a body");
    let status: u16 = head
        .split(' ')
        .nth(1)
        .expect("a status code")
        .parse()
        .expect("a numeric status code");
    (status, head.to_owned(), answer_body.to_owned())
}

fn enabled_body(camera: &str, value: bool) -> String {
    format!("{{\"camera\":\"{camera}\",\"enabled\":{value}}}")
}

async fn status_state_of(socket: &Path, camera: &str) -> String {
    let mut stream = UnixStream::connect(socket).await.unwrap();
    stream
        .write_all(b"GET /corvette/api/v1/detection/status HTTP/1.1\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let text = String::from_utf8_lossy(&response).into_owned();
    let camera_part = text
        .split(&format!("\"camera\":\"{camera}\""))
        .nth(1)
        .unwrap_or("")
        .to_owned();
    camera_part
        .split('"')
        .nth(3)
        .unwrap_or("<no state>")
        .to_owned()
}

#[tokio::test]
async fn off_then_on_each_answer_twenty_ok_with_the_state_and_repeat_unchanged() {
    let dir = temp_dir("transitions");
    let ffmpeg = stub_ffmpeg(&dir);
    let config = loaded_config(&dir);
    let front = startup(&dir, &ffmpeg, "front", config.clone());
    let (socket, state) = serve_ipc(&dir, vec![front]).await;
    state.start_camera("front").expect("the camera starts");
    assert_eq!(
        status_state_of(&socket, "front").await,
        "waiting_for_frames",
        "a live camera that has seen no frame yet"
    );
    let (status, _, body) = post(&socket, "front", "{\"enabled\": false}", Some("admin")).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, enabled_body("front", false));
    assert_eq!(status_state_of(&socket, "front").await, "off");
    let (status, _, body) = post(&socket, "front", "{\"enabled\": false}", Some("admin")).await;
    assert_eq!(status, 200, "a repeat of the state is a no-op");
    assert_eq!(
        body,
        enabled_body("front", false),
        "the repeat answers unchanged"
    );
    let (status, _, body) = post(&socket, "front", "{\"enabled\": true}", Some("admin")).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, enabled_body("front", true));
    let (status, _, body) = post(&socket, "front", "{\"enabled\": true}", Some("admin")).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, enabled_body("front", true));
}

#[tokio::test]
async fn an_unknown_camera_answers_not_found() {
    let dir = temp_dir("unknown");
    let ffmpeg = stub_ffmpeg(&dir);
    let config = loaded_config(&dir);
    let front = startup(&dir, &ffmpeg, "front", config);
    let (socket, _state) = serve_ipc(&dir, vec![front]).await;
    let (status, _, body) = post(&socket, "garage", "{\"enabled\": false}", Some("admin")).await;
    assert_eq!(status, 404);
    assert_eq!(body, "{\"reason\":\"not_found\"}");
}

#[tokio::test]
async fn bodies_that_are_not_one_boolean_enabled_answer_bad_request() {
    let dir = temp_dir("bodies");
    let ffmpeg = stub_ffmpeg(&dir);
    let config = loaded_config(&dir);
    let front = startup(&dir, &ffmpeg, "front", config);
    let (socket, _state) = serve_ipc(&dir, vec![front]).await;
    for body in [
        "{}",
        "{\"enabled\":\"yes\"}",
        "{\"enabled\":true,\"extra\":1}",
        "{\"nope\":true}",
        "true",
        "[1]",
        "not json",
    ] {
        let (status, _, _) = post(&socket, "front", body, Some("admin")).await;
        assert_eq!(status, 400, "for body {body:?}");
    }
}

#[tokio::test]
async fn a_missing_or_unauditable_remote_user_is_refused_like_a_config_put() {
    let dir = temp_dir("user");
    let ffmpeg = stub_ffmpeg(&dir);
    let config = loaded_config(&dir);
    let front = startup(&dir, &ffmpeg, "front", config);
    let (socket, _state) = serve_ipc(&dir, vec![front]).await;
    let (status, head, body) = post(&socket, "front", "{\"enabled\": false}", None).await;
    assert_eq!(status, 401);
    assert!(
        head.contains("application/problem+json"),
        "refused like a config PUT: {head}"
    );
    assert!(body.contains("\"code\":\"unauthorized\""), "{body}");
    let (status, _, _) = post(&socket, "front", "{\"enabled\": false}", Some("bad\nuser")).await;
    assert_eq!(
        status, 401,
        "a user that could forge an audit line is refused too"
    );
}

#[tokio::test]
async fn other_methods_answer_method_not_allowed_with_post_allowed() {
    let dir = temp_dir("methods");
    let ffmpeg = stub_ffmpeg(&dir);
    let config = loaded_config(&dir);
    let front = startup(&dir, &ffmpeg, "front", config);
    let (socket, _state) = serve_ipc(&dir, vec![front]).await;
    let mut stream = UnixStream::connect(&socket).await.unwrap();
    stream
        .write_all(b"GET /corvette/api/v1/cameras/front/enabled HTTP/1.1\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let text = String::from_utf8_lossy(&response).into_owned();
    assert!(text.starts_with("HTTP/1.1 405"), "{text}");
    assert!(text.contains("Allow: POST"), "{text}");
}

#[tokio::test]
async fn the_listener_without_the_toggle_state_answers_not_found() {
    let dir = temp_dir("status-socket");
    let socket = serve_plain_no_toggles(&dir, "status").await;
    let (status, _, body) = post(&socket, "front", "{\"enabled\": false}", Some("admin")).await;
    assert_eq!(status, 404);
    assert_eq!(body, "{\"reason\":\"not_found\"}");
}

#[tokio::test]
async fn a_target_carrying_a_query_or_fragment_is_not_found() {
    let dir = temp_dir("query");
    let ffmpeg = stub_ffmpeg(&dir);
    let config = loaded_config(&dir);
    let front = startup(&dir, &ffmpeg, "front", config);
    let (socket, _state) = serve_ipc(&dir, vec![front]).await;
    for suffix in ["?x=1", "#frag"] {
        let mut stream = UnixStream::connect(&socket).await.unwrap();
        stream
            .write_all(format!("POST /corvette/api/v1/cameras/front/enabled{suffix} HTTP/1.1\r\nRemote-User: admin\r\nContent-Length: 17\r\n\r\n{{\"enabled\": false}}").as_bytes())
            .await
            .unwrap();
        let mut response = Vec::new();
        timeout(CALL, stream.read_to_end(&mut response))
            .await
            .expect("an answer")
            .expect("the read completes");
        let text = String::from_utf8_lossy(&response).into_owned();
        assert!(text.starts_with("HTTP/1.1 404"), "for {suffix}: {text}");
    }
}

#[tokio::test]
async fn a_failed_restart_answers_start_failed_and_the_status_stays_off() {
    let dir = temp_dir("start-failed");
    let ffmpeg = stub_ffmpeg(&dir);
    let config = loaded_config(&dir);
    let camera = camera_config("front");
    let address = stream_address(&camera, "restream.invalid", &|_| None).unwrap();
    let link = SharedLink::new(WorkerLink::new(
        dir.join("worker.sock"),
        Duration::from_secs(5),
        DEFAULT_POLICY,
    ));
    let broken_labels = Config {
        labels: dir.join("labels-missing.txt"),
        ..config.clone()
    };
    let mut camera_startup = startup(&dir, &ffmpeg, "front", config);
    camera_startup.recipe = CameraRecipe::new(
        broken_labels,
        camera,
        ffmpeg,
        address,
        link,
        CameraThreadSettings {
            frame_timeout: Duration::from_millis(500),
            backoff: DEFAULT_POLICY,
        },
    );
    camera_startup.enabled = false;
    let (socket, _state) = serve_ipc(&dir, vec![camera_startup]).await;
    assert_eq!(status_state_of(&socket, "front").await, "off");
    let (status, _, body) = post(&socket, "front", "{\"enabled\": true}", Some("admin")).await;
    assert_eq!(status, 503);
    assert_eq!(body, "{\"reason\":\"start_failed\"}");
    assert_eq!(
        status_state_of(&socket, "front").await,
        "off",
        "a failed restart keeps the camera off"
    );
}
