// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and the crate lints do not forbid them.
use corvette_detect::{
    CameraChannels, CameraRecipe, CameraStartup, CameraThreadSettings, Config, DEFAULT_POLICY,
    EventFeed, FrameObjects, ServiceState, SharedLink, TrackedObject, WorkerLink, serve,
    stream_address,
};
use corvette_detect_proto::{HEADER_LEN, Reply, Request, parse_header};
use std::fs;
use std::io::{Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixListener as StdUnixListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::time::timeout;

include!("support/mod.rs");

const ONE_FRAME_BYTES: usize = 28800;
const WORKER_HOLD: Duration = Duration::from_millis(1200);
const DRAIN: Duration = Duration::from_secs(6);

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "corvette-enabled-toggle-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn sleeping_ffmpeg(dir: &Path) -> PathBuf {
    let path = dir.join("sleep-ffmpeg");
    fs::write(&path, "#!/bin/sh\nexec sleep 30\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

// The stub ends itself when its reader is gone: the write fails once the pipe closes.
fn streaming_ffmpeg(dir: &Path) -> PathBuf {
    let path = dir.join("frame-ffmpeg");
    let emitter = format!("head -c {ONE_FRAME_BYTES} /dev/zero | tr '\\0' '\\200'");
    let script = forever_stream(&emitter, 32);
    fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

// Poll /proc for up to 5 s until no process still names this directory.
async fn no_stub_process_holds(dir: &Path) {
    let marker = dir.to_string_lossy().into_owned();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut holding = Vec::new();
        for entry in fs::read_dir("/proc").unwrap() {
            let name = entry.unwrap().file_name();
            let Ok(pid) = name.to_string_lossy().parse::<u32>() else {
                continue;
            };
            if let Ok(raw) = fs::read(format!("/proc/{pid}/cmdline"))
                && String::from_utf8_lossy(&raw).contains(&marker)
            {
                holding.push(pid);
            }
        }
        if holding.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "processes still hold {marker}: {holding:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// Answers one infer request a second after it arrives, so the runner is inside a request when a stop lands.
fn worker_holding(dir: &Path) {
    let listener = StdUnixListener::bind(dir.join("worker.sock")).unwrap();
    std::thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            std::thread::spawn(move || {
                loop {
                    let mut header = [0u8; HEADER_LEN];
                    if stream.read_exact(&mut header).is_err() {
                        break;
                    }
                    let Ok(parsed) = parse_header(&header) else {
                        break;
                    };
                    let mut body = vec![0u8; parsed.body_len];
                    if stream.read_exact(&mut body).is_err() {
                        break;
                    }
                    let mut request = header.to_vec();
                    request.extend_from_slice(&body);
                    let Ok(Request::InferRequest { id, .. }) = Request::decode(&request) else {
                        break;
                    };
                    std::thread::sleep(WORKER_HOLD);
                    let reply = Reply::Rows {
                        id,
                        worker_ms: 5,
                        rows: Vec::new(),
                    };
                    if stream
                        .write_all(&reply.encode().expect("reply encodes"))
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
    });
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

struct Setup {
    state: Arc<ServiceState>,
    socket: PathBuf,
}

async fn setup(dir: &Path, startups: Vec<CameraStartup>, join_timeout: Duration) -> Setup {
    let state = Arc::new(
        ServiceState::new(Vec::new())
            .with_cameras(startups)
            .with_stop_timeout(join_timeout),
    );
    let feed = Arc::new(EventFeed::new());
    feed.spawn_publishers(state.camera_object_receivers());
    let socket = dir.join("ipc.sock");
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).unwrap();
    tokio::spawn(serve(
        listener,
        Arc::clone(&state),
        Some(Arc::clone(&feed)),
        Some(state.toggles()),
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;
    Setup { state, socket }
}

fn startup(
    dir: &Path,
    ffmpeg: &Path,
    name: &str,
    config: &Config,
) -> (CameraStartup, CameraChannels) {
    let camera = camera_config(name);
    let address = stream_address(&camera, "restream.invalid", &|_| None).unwrap();
    let link = SharedLink::new(WorkerLink::new(
        dir.join("worker.sock"),
        Duration::from_secs(5),
        DEFAULT_POLICY,
    ));
    let channels = CameraChannels::new();
    let startup = CameraStartup {
        name: name.to_owned(),
        channels: channels.clone(),
        recipe: CameraRecipe::new(
            config.clone(),
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
    };
    (startup, channels)
}

async fn post(socket: &Path, camera: &str, enabled: bool) -> (u16, String) {
    let body = format!("{{\"enabled\": {enabled}}}");
    let request = format!(
        "POST /corvette/api/v1/cameras/{camera}/enabled HTTP/1.1\r\nRemote-User: admin\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut stream = UnixStream::connect(socket).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut answer = Vec::new();
    timeout(DRAIN, stream.read_to_end(&mut answer))
        .await
        .expect("the answer arrives")
        .expect("the read completes");
    let text = String::from_utf8_lossy(&answer).into_owned();
    let (head, answer_body) = text.split_once("\r\n\r\n").expect("a head and a body");
    let status: u16 = head
        .split(' ')
        .nth(1)
        .expect("a status")
        .parse()
        .expect("numeric");
    (status, answer_body.to_owned())
}

async fn open_event_stream(socket: &Path) -> UnixStream {
    let mut stream = UnixStream::connect(socket).await.unwrap();
    stream
        .write_all(b"GET /corvette/api/v1/events HTTP/1.1\r\n\r\n")
        .await
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        timeout(Duration::from_secs(5), stream.read(&mut byte))
            .await
            .expect("head bytes")
            .unwrap();
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            assert!(head.starts_with(b"HTTP/1.1 200"), "stream head: {head:?}");
            return stream;
        }
    }
}

fn decode_chunks(raw: &[u8]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = raw;
    while !rest.is_empty() {
        let Some(size_end) = rest.windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let Ok(size) = usize::from_str_radix(std::str::from_utf8(&rest[..size_end]).unwrap(), 16)
        else {
            break;
        };
        if rest.len() < size_end + 4 + size {
            break;
        }
        lines.push(String::from_utf8_lossy(&rest[size_end + 2..size_end + 2 + size]).into_owned());
        rest = &rest[size_end + 4 + size..];
    }
    lines
}

async fn read_lines(stream: &mut UnixStream, want: usize) -> Vec<String> {
    let mut raw = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let deadline = Instant::now() + DRAIN;
    while lines.len() < want && Instant::now() < deadline {
        let mut chunk = [0_u8; 4096];
        let remaining = deadline.saturating_duration_since(Instant::now());
        match timeout(remaining, stream.read(&mut chunk)).await {
            Ok(Ok(0) | Err(_)) | Err(_) => break,
            Ok(Ok(read)) => {
                raw.extend_from_slice(&chunk[..read]);
                lines = decode_chunks(&raw);
            }
        }
    }
    lines
}

fn frame(ids: &[(&str, f64)]) -> FrameObjects {
    FrameObjects {
        frame_time: 1.0,
        objects: ids
            .iter()
            .map(|(id, x)| TrackedObject {
                id: (*id).to_owned(),
                label: "person".to_owned(),
                score: 0.9,
                confirmed: true,
                bounds: [*x, 0.2, *x + 0.1, 0.4],
                edges: [10, 20, 30, 40],
                first_seen: 1.0,
                last_seen: 1.0,
                zones_now: Vec::new(),
                zones_entered: Vec::new(),
                stationary: false,
            })
            .collect(),
        ended: Vec::new(),
        motion_triggered: false,
    }
}

async fn status_text(socket: &Path) -> (Duration, String) {
    let started = Instant::now();
    let mut stream = UnixStream::connect(socket).await.unwrap();
    stream
        .write_all(b"GET /corvette/api/v1/detection/status HTTP/1.1\r\n\r\n")
        .await
        .unwrap();
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.unwrap();
    (
        started.elapsed(),
        String::from_utf8_lossy(&answer).into_owned(),
    )
}

#[tokio::test]
async fn toggling_off_delivers_the_end_line_of_the_open_track_after_the_threads_stopped() {
    let dir = temp_dir("end-line");
    let config = loaded_config(&dir);
    let (front, channels) = startup(&dir, &sleeping_ffmpeg(&dir), "front", &config);
    let setup = setup(&dir, vec![front], Duration::from_secs(10)).await;
    setup
        .state
        .start_camera("front")
        .expect("the camera starts");
    let mut subscriber = open_event_stream(&setup.socket).await;
    channels.publish_frame(frame(&[("7", 0.1)]));
    let opened = read_lines(&mut subscriber, 1).await;
    assert_eq!(opened.len(), 1, "the start line arrived: {opened:?}");
    assert!(opened[0].starts_with("1\ts\tfront\t7"), "{opened:?}");
    let (status, _) = post(&setup.socket, "front", false).await;
    assert_eq!(status, 200);
    let closed = read_lines(&mut subscriber, 1).await;
    assert!(
        closed
            .first()
            .is_some_and(|line| line.starts_with("1\te\tfront\t7")),
        "the end line followed the stop: {closed:?}"
    );
}

#[tokio::test]
async fn the_same_subscriber_sees_new_start_and_update_lines_after_off_then_on() {
    let dir = temp_dir("reopen");
    let config = loaded_config(&dir);
    let (front, channels) = startup(&dir, &sleeping_ffmpeg(&dir), "front", &config);
    let setup = setup(&dir, vec![front], Duration::from_secs(10)).await;
    setup
        .state
        .start_camera("front")
        .expect("the camera starts");
    let mut subscriber = open_event_stream(&setup.socket).await;
    channels.publish_frame(frame(&[("7", 0.1)]));
    assert_eq!(read_lines(&mut subscriber, 1).await.len(), 1);
    assert_eq!(post(&setup.socket, "front", false).await.0, 200);
    let closed = read_lines(&mut subscriber, 1).await;
    assert!(
        closed
            .first()
            .is_some_and(|line| line.starts_with("1\te\tfront\t7")),
        "the open track ended on the toggle: {closed:?}"
    );
    assert_eq!(post(&setup.socket, "front", true).await.0, 200);
    channels.publish_frame(frame(&[("9", 0.2)]));
    tokio::time::sleep(Duration::from_millis(100)).await;
    channels.publish_frame(frame(&[("9", 0.5)]));
    let reopened = read_lines(&mut subscriber, 2).await;
    assert_eq!(
        reopened.len(),
        2,
        "the same subscriber stayed: {reopened:?}"
    );
    assert!(reopened[0].starts_with("1\ts\tfront\t9"), "{reopened:?}");
    assert!(reopened[1].starts_with("1\tu\tfront\t9"), "{reopened:?}");
}

#[tokio::test]
async fn a_status_request_answers_within_a_second_while_a_toggle_join_is_running() {
    let dir = temp_dir("status-live");
    worker_holding(&dir);
    let config = loaded_config(&dir);
    let (front, _channels) = startup(&dir, &streaming_ffmpeg(&dir), "front", &config);
    let setup = setup(&dir, vec![front], Duration::from_secs(10)).await;
    setup
        .state
        .start_camera("front")
        .expect("the camera starts");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let off = tokio::spawn({
        let socket = setup.socket.clone();
        async move { post(&socket, "front", false).await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (elapsed, text) = status_text(&setup.socket).await;
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(
        elapsed < Duration::from_secs(1),
        "the route stayed free during the join: {elapsed:?}"
    );
    assert_eq!(off.await.expect("the toggle answered").0, 200);
}

#[tokio::test]
async fn a_toggle_on_one_camera_does_not_hold_up_a_toggle_on_another() {
    let dir = temp_dir("independent");
    worker_holding(&dir);
    let config = loaded_config(&dir);
    let (slow, _slow_channels) = startup(&dir, &streaming_ffmpeg(&dir), "slow", &config);
    let (fast, _fast_channels) = startup(&dir, &sleeping_ffmpeg(&dir), "fast", &config);
    let setup = setup(&dir, vec![slow, fast], Duration::from_secs(10)).await;
    setup.state.start_camera("slow").expect("slow starts");
    setup.state.start_camera("fast").expect("fast starts");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let slow_off = tokio::spawn({
        let socket = setup.socket.clone();
        async move { (Instant::now(), post(&socket, "slow", false).await) }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let fast_at = Instant::now();
    assert_eq!(post(&setup.socket, "fast", false).await.0, 200);
    let fast_done = fast_at.elapsed();
    let (slow_start, (slow_status, _)) = slow_off.await.expect("the slow toggle answered");
    assert_eq!(slow_status, 200);
    let slow_ran = slow_start.elapsed();
    assert!(
        fast_done < slow_ran,
        "fast finished in {fast_done:?} while the slow toggle had only just started at {slow_ran:?}"
    );
}

#[tokio::test]
async fn two_toggles_on_one_camera_serialize_behind_the_first() {
    let dir = temp_dir("serialized");
    worker_holding(&dir);
    let config = loaded_config(&dir);
    let (front, _channels) = startup(&dir, &streaming_ffmpeg(&dir), "front", &config);
    let setup = setup(&dir, vec![front], Duration::from_secs(10)).await;
    setup
        .state
        .start_camera("front")
        .expect("the camera starts");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let first = tokio::spawn({
        let socket = setup.socket.clone();
        async move { (Instant::now(), post(&socket, "front", false).await) }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second_issued = Instant::now();
    let second = tokio::spawn({
        let socket = setup.socket.clone();
        async move { (Instant::now(), post(&socket, "front", false).await) }
    });
    let (first_at, (first_status, first_body)) = first.await.expect("the first toggle answered");
    let (second_at, (second_status, second_body)) =
        second.await.expect("the second toggle answered");
    assert_eq!(first_status, 200, "{first_body}");
    assert!(
        first_at - second_issued < Duration::from_millis(50),
        "the first toggle never reached the slow worker: {first_at:?} vs {second_issued:?}"
    );
    assert_eq!(
        second_status, 200,
        "the queued toggle lands as a repeat, not a racing error: {second_body}"
    );
    assert!(
        second_at >= first_at,
        "the second answer ({second_at:?}) landed before the first ({first_at:?})"
    );
}

#[tokio::test]
async fn a_timed_out_join_keeps_the_camera_off_until_both_threads_have_ended() {
    let dir = temp_dir("join-timeout");
    worker_holding(&dir);
    let config = loaded_config(&dir);
    let (front, _channels) = startup(&dir, &streaming_ffmpeg(&dir), "front", &config);
    let setup = setup(&dir, vec![front], Duration::from_millis(200)).await;
    setup
        .state
        .start_camera("front")
        .expect("the camera starts");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let (status, _) = post(&setup.socket, "front", false).await;
    assert_eq!(status, 200, "the off answers even when the join runs over");
    let (status, body) = post(&setup.socket, "front", true).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body, "{\"reason\":\"stopping\"}");
    let mut on_after = 0_u16;
    let mut body_after = String::new();
    for _ in 0..30 {
        (on_after, body_after) = post(&setup.socket, "front", true).await;
        if on_after != 503 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(
        on_after, 200,
        "the restart landed once both threads ended: {body_after}"
    );
    let (status, _) = post(&setup.socket, "front", false).await;
    assert_eq!(status, 200, "the restarted camera goes off again");
    no_stub_process_holds(&dir).await;
}

#[tokio::test]
async fn shutdown_waits_for_the_toggle_in_progress_and_refuses_later_toggles() {
    let dir = temp_dir("shutdown-order");
    worker_holding(&dir);
    let config = loaded_config(&dir);
    let (front, _channels) = startup(&dir, &streaming_ffmpeg(&dir), "front", &config);
    let setup = setup(&dir, vec![front], Duration::from_secs(10)).await;
    setup
        .state
        .start_camera("front")
        .expect("the camera starts");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let off = tokio::spawn({
        let socket = setup.socket.clone();
        async move { (Instant::now(), post(&socket, "front", false).await) }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let toggles = setup.state.toggles();
    let closed_at = tokio::spawn(async move {
        toggles.close(Duration::from_secs(10)).await;
        Instant::now()
    });
    let (off_at, (status, _)) = off.await.expect("the in-flight toggle answered");
    assert_eq!(status, 200);
    let closed = closed_at.await.expect("shutdown completed");
    assert!(
        closed >= off_at,
        "shutdown joined before the in-flight toggle had answered"
    );
    let (status, body) = post(&setup.socket, "front", true).await;
    assert_eq!(status, 503, "a toggle after the closing flag: {body}");
}

#[test]
fn a_refusing_camera_thread_does_not_hold_runtime_drop_past_the_stop_budget() {
    let dir = temp_dir("drop-budget");
    let listener = StdUnixListener::bind(dir.join("worker.sock")).unwrap();
    std::thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_mins(10));
                drop(stream);
            });
        }
    });
    let config = loaded_config(&dir);
    let (front, _channels) = startup(&dir, &streaming_ffmpeg(&dir), "front", &config);
    let budget = Duration::from_millis(200);
    let margin = Duration::from_millis(400);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let toggles = rt.block_on(async {
        let setup = setup(&dir, vec![front], budget).await;
        setup
            .state
            .start_camera("front")
            .expect("the camera starts");
        tokio::time::sleep(Duration::from_millis(400)).await;
        let (status, _) = post(&setup.socket, "front", false).await;
        assert_eq!(status, 200, "the off answers even when the join runs over");
        setup.state.toggles()
    });
    let started = Instant::now();
    rt.block_on(toggles.close(budget));
    drop(rt);
    let waited = started.elapsed();
    assert!(
        waited < budget + margin,
        "runtime drop took {waited:?}, past the stop budget plus its margin"
    );
}
