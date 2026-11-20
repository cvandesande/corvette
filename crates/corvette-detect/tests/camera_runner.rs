// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and the crate lints do not forbid them.
use corvette_detect::{
    CameraRunner, CameraState, CandidateStage, DEFAULT_POLICY, Decode, EngineError, Frame,
    FrameSpec, Infer, Labels, LiveError, LivePipeline, ObjectStage, RegionStage,
    RegionStageSettings, RunnerError, SharedLink, StepEnd, WorkerLink, frame_slot,
};
use corvette_detect_proto::{HEADER_LEN, Reply, Request, Row, TENSOR_BYTES, parse_header};
use corvette_motion::{MotionSettings, Shape, SizeGrid};
use corvette_track::{IdSource, ObjectFilter, ObjectFilters, TrackerSettings, ZoneSet};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

const WIDTH: usize = 160;
const HEIGHT: usize = 120;

fn spec() -> FrameSpec {
    FrameSpec::new(160, 120, 10, Decode::Cpu).expect("valid frame spec")
}

// 0x80-filled frames, as the motion stage fixture uses: an all-zero frame matches the
// detector's initial black background and produces no region, so nothing reaches inference.
fn flat_frame(index: u64) -> Frame {
    Frame {
        index,
        time: Duration::from_millis(100 * (index + 1)),
        data: vec![0x80; spec().frame_bytes()],
    }
}

fn black_frame(index: u64) -> Frame {
    Frame {
        index,
        time: Duration::from_millis(100 * (index + 1)),
        data: vec![0; spec().frame_bytes()],
    }
}

fn short_frame() -> Frame {
    Frame {
        index: 0,
        time: Duration::from_millis(100),
        data: vec![0; 288],
    }
}

const fn person_row() -> Row {
    Row {
        class_id: 0,
        score: 0.9,
        y_min: 0.25,
        x_min: 0.5,
        y_max: 0.75,
        x_max: 0.875,
    }
}

fn stage() -> RegionStage {
    let settings = RegionStageSettings {
        motion: MotionSettings::default(),
        floor: 32,
        grid: SizeGrid::empty(),
        model: Shape::new(640, 640).unwrap(),
        motion_enabled: true,
    };
    RegionStage::new(settings, Shape::new(WIDTH, HEIGHT).unwrap(), None).unwrap()
}

fn candidate_stage() -> CandidateStage {
    let mut filters = BTreeMap::new();
    filters.insert("person".to_owned(), ObjectFilter::default());
    let track = vec!["person".to_owned()];
    CandidateStage::new(
        Labels::parse("0 person").unwrap(),
        ObjectFilters::new(track, filters),
        160,
        120,
    )
    .unwrap()
}

const fn tracker() -> TrackerSettings {
    TrackerSettings {
        width: 160,
        height: 120,
        min_initialized: 0,
        max_disappeared: 5,
        stationary_threshold: 100,
        max_frames_default: None,
        max_frames_overrides: Vec::new(),
        own_labels: Vec::new(),
    }
}

#[derive(Clone)]
struct Cells {
    person: Rc<Cell<bool>>,
    failing: Rc<Cell<bool>>,
    inferences: Rc<Cell<u32>>,
}

struct Engine {
    cells: Cells,
}

impl Infer for Engine {
    fn infer(&mut self, _tensor: &[u8; TENSOR_BYTES]) -> Result<Vec<Row>, EngineError> {
        self.cells.inferences.set(self.cells.inferences.get() + 1);
        if self.cells.failing.get() {
            return Err(EngineError::Io("scripted engine failure".to_owned()));
        }
        if self.cells.person.get() {
            Ok(vec![person_row()])
        } else {
            Ok(Vec::new())
        }
    }
}

fn pipeline_with(person: bool) -> (LivePipeline<'static, Engine>, Cells) {
    let cells = Cells {
        person: Rc::new(Cell::new(person)),
        failing: Rc::new(Cell::new(false)),
        inferences: Rc::new(Cell::new(0)),
    };
    let ids: &'static mut IdSource = Box::leak(Box::new(IdSource::default()));
    let objects = ObjectStage::new(tracker(), ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    let engine = Engine {
        cells: cells.clone(),
    };
    (
        LivePipeline::new(stage(), candidate_stage(), objects, engine),
        cells,
    )
}

#[test]
fn working_frames_publish_a_confirmed_object_and_running_state() {
    let (pipeline, cells) = pipeline_with(true);
    let (sender, receiver) = frame_slot();
    let (mut runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    for index in 0..3 {
        sender.put(flat_frame(index));
        assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
    }
    let status = *handle.status.borrow();
    assert_eq!(status.state, CameraState::Running);
    assert_eq!(status.counters.frames_in, 3);
    assert_eq!(status.frames_dropped, 0, "one frame in, one frame out");
    let objects = handle.objects.borrow().clone().expect("objects published");
    assert!(
        objects.objects.iter().any(|object| object.confirmed),
        "a confirmed person after three frames, cells said person={}",
        cells.person.get()
    );
}

#[test]
fn a_frame_with_no_rows_publishes_seen_empty_objects_not_unknown() {
    let (pipeline, _cells) = pipeline_with(false);
    let (sender, receiver) = frame_slot();
    let (mut runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    sender.put(black_frame(0));
    assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
    let objects = handle.objects.borrow().clone();
    let objects = objects.expect("a seen frame publishes Some, not None");
    assert!(objects.objects.is_empty(), "seen but nothing there");
}

#[test]
fn a_failing_inference_reports_unavailable_then_recovers() {
    let (pipeline, cells) = pipeline_with(true);
    let (sender, receiver) = frame_slot();
    let (mut runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    cells.failing.set(true);
    sender.put(flat_frame(0));
    assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
    let status = *handle.status.borrow();
    assert_eq!(status.state, CameraState::InferenceUnavailable);
    assert_eq!(status.counters.frames_inference_failed, 1);
    assert!(handle.objects.borrow().is_none(), "unknown, not empty");
    cells.failing.set(false);
    sender.put(flat_frame(1));
    assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
    assert_eq!(handle.status.borrow().state, CameraState::Running);
    assert!(handle.objects.borrow().is_some(), "recovered to Some");
}

#[test]
fn a_quiet_source_past_the_timeout_reports_a_gap() {
    let (pipeline, _cells) = pipeline_with(true);
    let (_sender, receiver) = frame_slot();
    let (mut runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_millis(50));
    assert!(matches!(runner.step_once(), Ok(StepEnd::Gap)));
    let status = *handle.status.borrow();
    assert_eq!(status.state, CameraState::WaitingForFrames);
    assert_eq!(status.source_gaps, 1);
    assert!(handle.objects.borrow().is_none());
}

#[test]
fn a_gap_after_a_seen_frame_clears_the_objects() {
    let (pipeline, _cells) = pipeline_with(true);
    let (sender, receiver) = frame_slot();
    let (mut runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_millis(500));
    sender.put(black_frame(0));
    assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
    assert!(handle.objects.borrow().is_some(), "the frame was seen");
    sender.gap();
    assert!(matches!(runner.step_once(), Ok(StepEnd::Gap)));
    let status = *handle.status.borrow();
    assert_eq!(status.state, CameraState::WaitingForFrames);
    assert_eq!(status.source_gaps, 1);
    assert!(
        handle.objects.borrow().is_none(),
        "the gap clears to unknown"
    );
}

#[test]
fn an_exiting_child_reports_a_gap_after_its_last_frame() {
    let (pipeline, _cells) = pipeline_with(true);
    let (sender, receiver) = frame_slot();
    let (mut runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    sender.put(black_frame(0));
    assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
    assert!(handle.objects.borrow().is_some(), "the frame was seen");
    sender.gap();
    assert!(
        matches!(runner.step_once(), Ok(StepEnd::Gap)),
        "end of stream is a gap; the producer reconnects on its own"
    );
    let status = *handle.status.borrow();
    assert_eq!(status.source_gaps, 1, "the gap is counted");
    assert!(
        handle.objects.borrow().is_none(),
        "end of stream clears objects to unknown"
    );
}

#[test]
fn a_frame_of_the_wrong_length_is_a_fatal_pipeline_error() {
    let (pipeline, _cells) = pipeline_with(true);
    let (sender, receiver) = frame_slot();
    let (mut runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    sender.put(short_frame());
    let error = runner.step_once().expect_err("wrong frame length");
    assert!(
        matches!(error, RunnerError::Pipeline(LiveError::Region(_))),
        "not a region error: {error:?}"
    );
    let status = *handle.status.borrow();
    assert_eq!(
        status.counters.frames_in, 0,
        "nothing published after a fatal error"
    );
    assert_eq!(status.state, CameraState::WaitingForFrames);
}

#[test]
fn run_publishes_failed_when_a_step_ends_on_a_fatal_error() {
    let (pipeline, _cells) = pipeline_with(true);
    let (sender, receiver) = frame_slot();
    let (runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    sender.put(short_frame());
    let error = runner.run().expect_err("wrong frame length");
    assert!(
        matches!(error, RunnerError::Pipeline(LiveError::Region(_))),
        "not a region error: {error:?}"
    );
    assert_eq!(handle.status.borrow().state, CameraState::Failed);
}

#[test]
fn run_returns_ok_when_the_source_reports_stopped() {
    let (pipeline, _cells) = pipeline_with(true);
    let (sender, receiver) = frame_slot();
    let (runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    sender.close();
    assert!(runner.run().is_ok());
    assert_eq!(
        handle.status.borrow().state,
        CameraState::WaitingForFrames,
        "a clean stop leaves the last published state, not failed"
    );
}

#[test]
fn a_runner_keeps_stepping_after_its_handle_is_dropped() {
    let (pipeline, _cells) = pipeline_with(true);
    let (sender, receiver) = frame_slot();
    let (mut runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    drop(handle);
    sender.put(flat_frame(0));
    assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
    sender.put(flat_frame(1));
    assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
}

#[test]
fn a_slow_consumer_steps_the_newest_frame_and_counts_the_drops() {
    let (pipeline, cells) = pipeline_with(true);
    let (sender, receiver) = frame_slot();
    let (mut runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    for index in 0..4 {
        sender.put(black_frame(index));
    }
    sender.put(flat_frame(4));
    assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
    assert_eq!(
        cells.inferences.get(),
        1,
        "the stepped frame is the newest, the flat one"
    );
    let status = *handle.status.borrow();
    assert_eq!(status.counters.frames_in, 1);
    assert_eq!(status.frames_dropped, 4);
}

struct QuietEngine;

impl Infer for QuietEngine {
    fn infer(&mut self, _tensor: &[u8; TENSOR_BYTES]) -> Result<Vec<Row>, EngineError> {
        Ok(Vec::new())
    }
}

#[test]
fn run_keeps_stepping_after_a_gap_and_ends_only_on_a_closed_slot() {
    let ids: &'static mut IdSource = Box::leak(Box::new(IdSource::default()));
    let objects = ObjectStage::new(tracker(), ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    let pipeline = LivePipeline::new(stage(), candidate_stage(), objects, QuietEngine);
    let (sender, receiver) = frame_slot();
    let (runner, mut handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    let thread = std::thread::spawn(move || runner.run());
    sender.gap();
    let gap_deadline = Instant::now() + Duration::from_secs(5);
    while handle.status.borrow_and_update().source_gaps != 1 {
        assert!(
            Instant::now() < gap_deadline,
            "the runner did not consume the gap"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    sender.put(flat_frame(0));
    let frame_deadline = Instant::now() + Duration::from_secs(5);
    while handle.status.borrow_and_update().counters.frames_in != 1 {
        assert!(
            Instant::now() < frame_deadline,
            "run ended at the gap without stepping the frame"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    sender.close();
    assert!(thread.join().expect("the runner thread finished").is_ok());
}

#[test]
fn a_closed_slot_with_an_untaken_frame_ends_the_run() {
    let (pipeline, _cells) = pipeline_with(true);
    let (sender, receiver) = frame_slot();
    let (runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    sender.put(flat_frame(0));
    sender.close();
    assert!(runner.run().is_ok(), "closure outranks the queued frame");
    let status = *handle.status.borrow();
    assert_eq!(
        status.counters.frames_in, 0,
        "the untaken frame was never stepped"
    );
    assert_eq!(status.state, CameraState::WaitingForFrames);
}

fn temp_worker_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("camrunner-{tag}-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let _ = fs::remove_file(dir.join("worker.sock"));
    dir
}

fn sample_tensor() -> Box<[u8; TENSOR_BYTES]> {
    let bytes = vec![0x5Au8; TENSOR_BYTES];
    <Box<[u8; TENSOR_BYTES]>>::try_from(bytes.into_boxed_slice()).expect("tensor length")
}

fn answer_one_infer(listener: UnixListener, worker_ms: u32, delay: Duration) {
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut header_bytes = [0u8; HEADER_LEN];
        if stream.read_exact(&mut header_bytes).is_err() {
            return;
        }
        let header = parse_header(&header_bytes).expect("header decodes");
        let mut body = vec![0u8; header.body_len];
        if stream.read_exact(&mut body).is_err() {
            return;
        }
        let mut frame = header_bytes.to_vec();
        frame.extend_from_slice(&body);
        let id = match Request::decode(&frame).expect("request decodes") {
            Request::InferRequest { id, .. } => id,
            other => panic!("expected an infer request, got {other:?}"),
        };
        std::thread::sleep(delay);
        let reply = Reply::Rows {
            id,
            worker_ms,
            rows: vec![person_row()],
        };
        let _ = stream.write_all(&reply.encode().expect("reply encodes"));
    });
}

#[test]
fn a_runner_with_link_timings_publishes_the_values_recorded_on_that_link() {
    let dir = temp_worker_dir("timings");
    let listener = UnixListener::bind(dir.join("worker.sock")).expect("bind");
    answer_one_infer(listener, 41, Duration::from_millis(60));
    let shared = SharedLink::new(WorkerLink::new(
        dir.join("worker.sock"),
        Duration::from_secs(5),
        DEFAULT_POLICY,
    ));
    let mut infer = shared.infer_handle();
    let timings = infer.timings();
    infer.infer(&sample_tensor()).expect("one answered request");
    assert_eq!(timings.worker_ms(), 41, "the fake worker carried its time");
    assert!(timings.roundtrip_ms() > 0, "the exchange was timed");
    let (pipeline, _cells) = pipeline_with(false);
    let (sender, receiver) = frame_slot();
    let (runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    let mut runner = runner.with_timings(timings.clone());
    sender.put(black_frame(0));
    assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
    let status = *handle.status.borrow();
    assert_eq!(status.worker_ms, timings.worker_ms());
    assert_eq!(status.roundtrip_ms, timings.roundtrip_ms());
    assert_eq!(status.lock_wait_ms, timings.lock_wait_ms());
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn a_completed_window_publishes_more_arrivals_than_the_pipeline_processed() {
    let (pipeline, _cells) = pipeline_with(true);
    let (sender, receiver) = frame_slot();
    let (mut runner, handle) = CameraRunner::new(receiver, pipeline, Duration::from_secs(1));
    for index in 0..4 {
        sender.put(black_frame(index));
    }
    sender.put(flat_frame(4));
    assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
    std::thread::sleep(Duration::from_millis(1_100));
    sender.gap();
    assert!(matches!(runner.step_once(), Ok(StepEnd::Gap)));
    let status = *handle.status.borrow();
    assert!(
        status.fps_processed_tenths >= 1,
        "the completed window published the processed frame, got {}",
        status.fps_processed_tenths
    );
    assert!(
        status.fps_in_tenths > status.fps_processed_tenths,
        "five frames arrived while one was processed, got in {} processed {}",
        status.fps_in_tenths,
        status.fps_processed_tenths
    );
}
