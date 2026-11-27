use corvette_detect::{
    CameraEntry, CameraHandle, CameraObjects, CameraRunner, CandidateStage, Decode, EventFeed,
    Frame, FrameObjects, FrameSpec, Infer, Labels, LivePipeline, ObjectStage, ServiceState,
    StepEnd, TRACK_EVENTS_ACCEPT_BACKOFF, TRACK_EVENTS_MAX_SUBSCRIBERS, TrackAccept, TrackedObject,
    frame_slot, objects_answer, serve, serve_track_events,
};
use corvette_detect_proto::{Row, TrackFrame, decode_track_line};
use corvette_motion::{MotionSettings, Shape, SizeGrid};
use corvette_track::{
    FrameDetection, IdSource, ObjectFilter, ObjectFilters, TrackerSettings, ZoneSet,
};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::SocketAddr;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;
use tokio::time::timeout;

const TICK: Duration = Duration::from_millis(20);
const DRAIN: Duration = Duration::from_millis(200);

fn temp_socket(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "corvette-track-events-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("events.sock")
}

async fn read_lines(stream: &mut UnixStream) -> Vec<Vec<u8>> {
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 4096];
    while let Ok(Ok(read)) = timeout(DRAIN, stream.read(&mut chunk)).await {
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
    }
    raw.split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

fn decode_lines(lines: &[Vec<u8>]) -> Vec<TrackFrame> {
    lines
        .iter()
        .map(|line| decode_track_line(line).expect("each received line decodes"))
        .collect()
}

fn kinds_of(frames: &[TrackFrame]) -> Vec<char> {
    frames
        .iter()
        .map(|frame| match frame {
            TrackFrame::Start(_) => 's',
            TrackFrame::Update(_) => 'u',
            TrackFrame::End { .. } => 'e',
            TrackFrame::Motion { .. } => 'm',
        })
        .collect()
}

const fn tracker_settings(
    min_initialized: u32,
    max_disappeared: u32,
    stationary_threshold: u64,
) -> TrackerSettings {
    TrackerSettings {
        width: 640,
        height: 480,
        min_initialized,
        max_disappeared,
        stationary_threshold,
        max_frames_default: None,
        max_frames_overrides: Vec::new(),
        own_labels: Vec::new(),
    }
}

fn person(score: f64, edges: [i32; 4]) -> FrameDetection {
    FrameDetection {
        label: "person".to_owned(),
        score,
        edges,
        area: 20_000,
        ratio: 0.5,
        region: Vec::new(),
    }
}

const STAY: [i32; 4] = [100, 140, 200, 340];
const MOVE: [i32; 4] = [300, 140, 400, 340];

async fn serve_bound(tag: &str, receiver: watch::Receiver<CameraObjects>) -> PathBuf {
    let path = temp_socket(tag);
    let listener = UnixListener::bind(&path).unwrap();
    let feed = Arc::new(EventFeed::new());
    feed.spawn_publishers(vec![("front".to_owned(), receiver)]);
    tokio::spawn(async move {
        let () = serve_track_events(listener, feed).await;
    });
    tokio::time::sleep(TICK).await;
    path
}

async fn connect(path: &Path) -> UnixStream {
    let stream = UnixStream::connect(path).await.unwrap();
    tokio::time::sleep(TICK).await;
    stream
}

#[tokio::test]
async fn a_track_gets_start_update_end_lines_and_first_motion_flips_after_it_moves() {
    let mut ids = IdSource::default();
    let mut stage = ObjectStage::new(
        tracker_settings(0, 1, 2),
        &mut ids,
        ZoneSet::new(Vec::new()),
        0.5,
    )
    .unwrap();
    let (sender, receiver) = watch::channel(CameraObjects::Cleared);
    let path = serve_bound("lifecycle", receiver).await;
    let mut client = connect(&path).await;

    let steps: [(f64, Vec<FrameDetection>); 6] = [
        (0.0, vec![person(0.9, STAY)]),
        (0.1, vec![person(0.9, STAY)]),
        (0.2, vec![person(0.9, STAY)]),
        (0.3, vec![person(0.9, MOVE)]),
        (0.4, Vec::new()),
        (0.5, Vec::new()),
    ];
    for (time, detections) in steps {
        let frame = stage.step(time, detections).unwrap();
        sender.send(CameraObjects::Frame(frame)).unwrap();
        tokio::time::sleep(TICK).await;
    }

    let frames = decode_lines(&read_lines(&mut client).await);
    let kinds = kinds_of(&frames);
    assert_eq!(
        &kinds[..4],
        ['s', 'u', 'u', 'u'],
        "start then updates: {kinds:?}"
    );
    assert_eq!(
        kinds.last().copied(),
        Some('e'),
        "last line is an end: {kinds:?}"
    );
    let info = match &frames[0] {
        TrackFrame::Start(info) => info,
        other => panic!("first line is a start, got {other:?}"),
    };
    assert_eq!(info.camera, "front");
    assert_eq!(info.label, "person");
    assert_eq!(info.bounds[0].to_bits(), (100_f64 / 640_f64).to_bits());
    assert_eq!(info.first_motion, None, "a new track has no first_motion");
    let track_id = info.track_id.clone();
    let motions: Vec<Option<f64>> = frames
        .iter()
        .filter_map(|frame| match frame {
            TrackFrame::Start(info) | TrackFrame::Update(info) => Some(info.first_motion),
            TrackFrame::End { .. } | TrackFrame::Motion { .. } => None,
        })
        .collect();
    let flip = motions
        .iter()
        .position(Option::is_some)
        .expect("first_motion flips after the track moves");
    assert_eq!(
        flip, 3,
        "the flip follows the stationary frames: {motions:?}"
    );
    assert_eq!(motions[flip], Some(0.3));
    match frames.last().expect("an end line") {
        TrackFrame::End {
            camera,
            track_id: ended,
        } => {
            assert_eq!(camera, "front");
            assert_eq!(ended, &track_id, "the end line names the started track");
        }
        other => panic!("last line is an end, got {other:?}"),
    }
}

#[tokio::test]
async fn two_subscribers_receive_identical_lines() {
    let mut ids = IdSource::default();
    let mut stage = ObjectStage::new(
        tracker_settings(0, 1, 2),
        &mut ids,
        ZoneSet::new(Vec::new()),
        0.5,
    )
    .unwrap();
    let (sender, receiver) = watch::channel(CameraObjects::Cleared);
    let path = serve_bound("two", receiver).await;
    let mut first = connect(&path).await;
    let mut second = connect(&path).await;

    for (time, detections) in [
        (0.0, vec![person(0.9, STAY)]),
        (0.1, vec![person(0.9, STAY)]),
        (0.2, vec![person(0.9, MOVE)]),
    ] {
        let frame = stage.step(time, detections).unwrap();
        sender.send(CameraObjects::Frame(frame)).unwrap();
        tokio::time::sleep(TICK).await;
    }

    let first_lines = read_lines(&mut first).await;
    let second_lines = read_lines(&mut second).await;
    assert!(
        !first_lines.is_empty(),
        "the first subscriber received lines"
    );
    assert_eq!(
        first_lines, second_lines,
        "both subscribers saw the same lines"
    );
}

fn moving_track(id: &str, time: f64) -> FrameObjects {
    FrameObjects {
        frame_time: time,
        objects: vec![TrackedObject {
            id: id.to_owned(),
            label: "person".to_owned(),
            score: 0.9,
            confirmed: true,
            bounds: [0.5, 0.5, 0.6, 0.7],
            edges: [500, 500, 600, 700],
            first_seen: 1.0,
            last_seen: time,
            zones_now: Vec::new(),
            zones_entered: Vec::new(),
            stationary: false,
        }],
        ended: Vec::new(),
        motion_triggered: false,
    }
}

fn count_newlines(stream: &UnixStream) -> usize {
    let mut lines = 0;
    let mut chunk = [0_u8; 8192];
    loop {
        match stream.try_read(&mut chunk) {
            Ok(0) => return lines,
            Ok(read) => {
                lines += chunk[..read]
                    .iter()
                    .fold(0, |count, byte| count + usize::from(*byte == b'\n'));
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => return lines,
            Err(error) => panic!("reading the live subscriber failed: {error}"),
        }
    }
}

#[tokio::test]
async fn a_stalled_subscriber_drops_updates_keeps_boundaries_and_the_loop_keeps_publishing() {
    let (sender, receiver) = watch::channel(CameraObjects::Cleared);
    let path = serve_bound("stalled", receiver).await;
    let mut stalled = connect(&path).await;
    let live = connect(&path).await;

    let updates: u32 = 6_000;
    sender
        .send(CameraObjects::Frame(moving_track("t1", 1.0)))
        .unwrap();
    let mut live_lines = count_newlines(&live);
    for step in 1u32..=updates {
        sender
            .send(CameraObjects::Frame(moving_track(
                "t1",
                1.0 + f64::from(step),
            )))
            .unwrap();
        tokio::time::sleep(Duration::from_micros(100)).await;
        live_lines += count_newlines(&live);
    }
    sender
        .send(CameraObjects::Frame(FrameObjects {
            frame_time: 2.0 + f64::from(updates),
            objects: Vec::new(),
            ended: vec!["t1".to_owned()],
            motion_triggered: false,
        }))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    live_lines += count_newlines(&live);

    let stalled_frames = decode_lines(&read_lines(&mut stalled).await);
    let kinds = kinds_of(&stalled_frames);
    assert_eq!(
        kinds.first().copied(),
        Some('s'),
        "the stalled subscriber kept the start line"
    );
    assert!(
        kinds.contains(&'e'),
        "the stalled subscriber kept the end line"
    );
    let stalled_updates = kinds.iter().filter(|kind| **kind == 'u').count();
    assert!(
        live_lines >= updates as usize,
        "the live subscriber saw every frame processed: {live_lines} < {updates}"
    );
    assert!(
        stalled_updates < live_lines.saturating_sub(2),
        "the stalled queue dropped updates: {stalled_updates} stalled vs {live_lines} live"
    );
}

struct Engine;

impl Infer for Engine {
    fn infer(
        &mut self,
        _tensor: &[u8; corvette_detect_proto::TENSOR_BYTES],
    ) -> Result<Vec<Row>, corvette_detect::EngineError> {
        Ok(vec![Row {
            class_id: 0,
            score: 0.9,
            y_min: 0.25,
            x_min: 0.5,
            y_max: 0.75,
            x_max: 0.875,
        }])
    }
}

fn flat_frame(spec: &FrameSpec, index: u64) -> Frame {
    Frame {
        index,
        time: Duration::from_millis(100 * (index + 1)),
        data: vec![0x80; spec.frame_bytes()],
    }
}

fn running_camera() -> (CameraHandle, Option<FrameObjects>) {
    let settings = TrackerSettings {
        width: 160,
        height: 120,
        min_initialized: 0,
        max_disappeared: 5,
        stationary_threshold: 100,
        max_frames_default: None,
        max_frames_overrides: Vec::new(),
        own_labels: Vec::new(),
    };
    let ids: &'static mut IdSource = Box::leak(Box::new(IdSource::default()));
    let objects = ObjectStage::new(settings, ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    let stage = corvette_detect::RegionStage::new(
        corvette_detect::RegionStageSettings {
            motion: MotionSettings::default(),
            floor: 32,
            grid: SizeGrid::empty(),
            model: Shape::new(640, 640).unwrap(),
            motion_enabled: true,
        },
        Shape::new(160, 120).unwrap(),
        None,
    )
    .unwrap();
    let mut filters = std::collections::BTreeMap::new();
    filters.insert("person".to_owned(), ObjectFilter::default());
    let candidates = CandidateStage::new(
        Labels::parse("0 person").unwrap(),
        ObjectFilters::new(vec!["person".to_owned()], filters),
        160,
        120,
    )
    .unwrap();
    let pipeline = LivePipeline::new(stage, candidates, objects, Engine);
    let spec = FrameSpec::new(160, 120, 10, Decode::Cpu).unwrap();
    let (slot, receiver) = frame_slot();
    let (mut runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    for index in 0..3 {
        slot.put(flat_frame(&spec, index));
        let step = runner.step_once();
        assert!(matches!(step, Ok(StepEnd::Frame)), "step {index}: {step:?}");
    }
    let published: Option<FrameObjects> = match &*handle.objects.borrow() {
        CameraObjects::Frame(objects) => Some(objects.clone()),
        CameraObjects::Cleared | CameraObjects::Stopped => None,
    };
    assert!(published.is_some(), "the mock pipeline published objects");
    (handle, published)
}

async fn serve_front_state(handle: &CameraHandle) -> (PathBuf, PathBuf) {
    let objects_rx = handle.objects.clone();
    let events_path = temp_socket("none");
    let events_listener = UnixListener::bind(&events_path).unwrap();
    let events_feed = Arc::new(EventFeed::new());
    events_feed.spawn_publishers(vec![("front".to_owned(), objects_rx)]);
    tokio::spawn(async move {
        let () = serve_track_events(events_listener, events_feed).await;
    });
    let service_state = ServiceState::new(vec![(
        "front".to_owned(),
        CameraEntry::Live(handle.clone()),
    )]);
    let status_path = temp_socket("none-status");
    let status_listener = UnixListener::bind(&status_path).unwrap();
    tokio::spawn(async move {
        let _ = serve(status_listener, Arc::new(service_state), None, None).await;
    });
    tokio::time::sleep(TICK).await;
    (status_path, events_path)
}

async fn ask_status(status_path: &Path) -> String {
    let mut watcher = UnixStream::connect(status_path).await.unwrap();
    watcher
        .write_all(
            b"GET /corvette/api/v1/detection/status HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    let mut answer = Vec::new();
    watcher.read_to_end(&mut answer).await.unwrap();
    String::from_utf8_lossy(&answer).into_owned()
}

async fn ask_objects(status_path: &Path) -> (String, String) {
    let mut client = UnixStream::connect(status_path).await.unwrap();
    client
        .write_all(
            b"GET /corvette/api/v1/cameras/front/objects HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    let mut answer = Vec::new();
    client.read_to_end(&mut answer).await.unwrap();
    let text = String::from_utf8_lossy(&answer).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("a header");
    (head.to_owned(), body.to_owned())
}

#[tokio::test]
async fn with_no_subscriber_the_status_answer_stays_the_pipeline_value() {
    let (handle, _published) = running_camera();
    let (status_path, _events) = serve_front_state(&handle).await;
    let text = ask_status(&status_path).await;
    assert!(
        text.starts_with("HTTP/1.1 200") && text.contains("\"camera\":\"front\""),
        "the status answer is unchanged: {text}"
    );
}

#[tokio::test]
async fn with_no_subscriber_the_objects_answer_stays_the_pipeline_value() {
    let (handle, published) = running_camera();
    let (status_path, _events) = serve_front_state(&handle).await;
    let (head, body) = ask_objects(&status_path).await;
    assert!(
        head.starts_with("HTTP/1.1 200"),
        "the objects answer is 200: {head}"
    );
    match objects_answer(published.as_ref()) {
        Ok(corvette_detect::ObjectsAnswer::Objects(expected)) => {
            assert_eq!(
                body, expected,
                "the objects answer is the published pipeline value"
            );
        }
        other => panic!("an objects answer expected, got {other:?}"),
    }
}

#[tokio::test]
async fn a_track_moving_from_its_first_frame_gets_first_motion_on_the_moving_frame() {
    let mut ids = IdSource::default();
    let mut stage = ObjectStage::new(
        tracker_settings(0, 1, 2),
        &mut ids,
        ZoneSet::new(Vec::new()),
        0.5,
    )
    .unwrap();
    let (sender, receiver) = watch::channel(CameraObjects::Cleared);
    let path = serve_bound("moves-first", receiver).await;
    let mut client = connect(&path).await;
    for (time, detections) in [
        (0.0, vec![person(0.9, STAY)]),
        (0.1, vec![person(0.9, MOVE)]),
    ] {
        let frame = stage.step(time, detections).unwrap();
        sender.send(CameraObjects::Frame(frame)).unwrap();
        tokio::time::sleep(TICK).await;
    }
    let frames = decode_lines(&read_lines(&mut client).await);
    let kinds = kinds_of(&frames);
    assert_eq!(
        &kinds[..2],
        ['s', 'u'],
        "start then the moving update: {kinds:?}"
    );
    let motion = match &frames[1] {
        TrackFrame::Update(info) => info.first_motion,
        other => panic!("the second line is an update, got {other:?}"),
    };
    assert_eq!(
        motion,
        Some(0.1),
        "a track that moved from its first frame stamps first_motion there"
    );
}

#[tokio::test]
async fn connections_beyond_the_subscriber_bound_are_closed() {
    let (sender, receiver) = watch::channel(CameraObjects::Cleared);
    let path = serve_bound("count", receiver).await;
    let mut bound = Vec::new();
    for _ in 0..TRACK_EVENTS_MAX_SUBSCRIBERS {
        bound.push(connect(&path).await);
    }
    let mut extra = connect(&path).await;
    let mut probe = [0_u8; 1];
    let seen = timeout(DRAIN, extra.read(&mut probe))
        .await
        .expect("the extra connection answers");
    assert_eq!(
        seen.expect("the read succeeds"),
        0,
        "a connection past the bound is closed"
    );
    sender
        .send(CameraObjects::Frame(moving_track("t1", 1.0)))
        .unwrap();
    tokio::time::sleep(TICK).await;
    assert!(
        count_newlines(&bound[0]) >= 1,
        "a subscriber inside the bound still receives lines"
    );
}

#[derive(Clone)]
struct RetryAccept {
    failures: Arc<Mutex<u32>>,
    pair_given: Arc<AtomicBool>,
    accepted: Arc<Mutex<Vec<UnixStream>>>,
}

impl TrackAccept for RetryAccept {
    async fn accept(&self) -> std::io::Result<(UnixStream, SocketAddr)> {
        let injected_failure = {
            let mut left = self.failures.lock().unwrap();
            if *left > 0 {
                *left -= 1;
                true
            } else {
                false
            }
        };
        if injected_failure {
            return Err(std::io::Error::other("injected accept failure"));
        }
        if !self.pair_given.swap(true, Ordering::SeqCst) {
            let (client, server) = UnixStream::pair().unwrap();
            self.accepted.lock().unwrap().push(client);
            let addr = std::os::unix::net::SocketAddr::from_pathname("retry").unwrap();
            return Ok((server, SocketAddr::from(addr)));
        }
        std::future::pending::<std::io::Result<(UnixStream, SocketAddr)>>().await
    }
}

#[tokio::test]
async fn a_failed_accept_is_logged_retried_and_acceptance_keeps_working() {
    let (sender, receiver) = watch::channel(CameraObjects::Cleared);
    let fake = RetryAccept {
        failures: Arc::new(Mutex::new(2)),
        pair_given: Arc::new(AtomicBool::new(false)),
        accepted: Arc::new(Mutex::new(Vec::new())),
    };
    let accepted = Arc::clone(&fake.accepted);
    let feed = Arc::new(EventFeed::new());
    feed.spawn_publishers(vec![("front".to_owned(), receiver)]);
    tokio::spawn(serve_track_events(fake, feed));
    tokio::time::sleep(TRACK_EVENTS_ACCEPT_BACKOFF * 3).await;
    let mut client = accepted
        .lock()
        .unwrap()
        .pop()
        .expect("the loop accepted after the injected failures");
    sender
        .send(CameraObjects::Frame(moving_track("t1", 1.0)))
        .unwrap();
    let frames = decode_lines(&read_lines(&mut client).await);
    assert!(
        kinds_of(&frames).contains(&'s'),
        "the retried accept still serves a subscriber"
    );
}
