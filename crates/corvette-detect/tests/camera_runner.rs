// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and the crate lints do not forbid them.
use corvette_detect::{
    CameraRunner, CameraState, CandidateStage, Decode, EngineError, FrameClock, FrameSource,
    FrameSpec, Infer, Labels, LiveError, LivePipeline, ObjectStage, Policy, RegionStage,
    RegionStageSettings, RunnerError, StepEnd,
};
use corvette_detect_proto::{Row, TENSOR_BYTES};
use corvette_motion::{MotionSettings, Shape, SizeGrid};
use corvette_track::{IdSource, ObjectFilter, ObjectFilters, TrackerSettings, ZoneSet};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::process::Stdio;
use std::rc::Rc;
use std::time::Duration;
use tokio::process::Command;

const WIDTH: usize = 160;
const HEIGHT: usize = 120;

// 0x80-filled frames, as the motion stage fixture uses: an all-zero frame matches the
// detector's initial black background and produces no region, so nothing reaches inference.
const FLAT_4: &str = "head -c 115200 /dev/zero | tr '\\0' '\\200'";
const FLAT_2: &str = "head -c 57600 /dev/zero | tr '\\0' '\\200'";

const SMALL: Policy = Policy {
    initial: Duration::from_millis(10),
    max: Duration::from_millis(40),
    healthy_after: Duration::from_mins(1),
};

fn spec() -> FrameSpec {
    FrameSpec::new(160, 120, 10, Decode::Cpu).expect("valid frame spec")
}

fn shell(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(script);
    command.stdout(Stdio::piped());
    command
}

fn source_with(script: &'static str, frame_spec: FrameSpec) -> FrameSource {
    FrameSource::spawn(
        "test".to_string(),
        SMALL,
        frame_spec,
        FrameClock::replay(Duration::from_secs(1000), frame_spec),
        move || shell(script),
    )
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
}

struct Engine {
    cells: Cells,
}

impl Infer for Engine {
    fn infer(&mut self, _tensor: &[u8; TENSOR_BYTES]) -> Result<Vec<Row>, EngineError> {
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

#[tokio::test]
async fn working_frames_publish_a_confirmed_object_and_running_state() {
    let (pipeline, cells) = pipeline_with(true);
    let source = source_with(FLAT_4, spec());
    let (mut runner, handle) = CameraRunner::new(source, pipeline, Duration::from_secs(1));
    for _ in 0..3 {
        assert!(matches!(runner.step_once().await, Ok(StepEnd::Frame)));
    }
    let status = *handle.status.borrow();
    assert_eq!(status.state, CameraState::Running);
    assert_eq!(status.counters.frames_in, 3);
    let objects = handle.objects.borrow().clone().expect("objects published");
    assert!(
        objects.objects.iter().any(|object| object.confirmed),
        "a confirmed person after three frames, cells said person={}",
        cells.person.get()
    );
}

#[tokio::test]
async fn a_frame_with_no_rows_publishes_seen_empty_objects_not_unknown() {
    let (pipeline, _cells) = pipeline_with(false);
    let source = source_with("head -c 28800 /dev/zero", spec());
    let (mut runner, handle) = CameraRunner::new(source, pipeline, Duration::from_secs(1));
    assert!(matches!(runner.step_once().await, Ok(StepEnd::Frame)));
    let objects = handle.objects.borrow().clone();
    let objects = objects.expect("a seen frame publishes Some, not None");
    assert!(objects.objects.is_empty(), "seen but nothing there");
}

#[tokio::test]
async fn a_failing_inference_reports_unavailable_then_recovers() {
    let (pipeline, cells) = pipeline_with(true);
    let source = source_with(FLAT_2, spec());
    let (mut runner, handle) = CameraRunner::new(source, pipeline, Duration::from_secs(1));
    cells.failing.set(true);
    assert!(matches!(runner.step_once().await, Ok(StepEnd::Frame)));
    let status = *handle.status.borrow();
    assert_eq!(status.state, CameraState::InferenceUnavailable);
    assert_eq!(status.counters.frames_inference_failed, 1);
    assert!(handle.objects.borrow().is_none(), "unknown, not empty");
    cells.failing.set(false);
    assert!(matches!(runner.step_once().await, Ok(StepEnd::Frame)));
    assert_eq!(handle.status.borrow().state, CameraState::Running);
    assert!(handle.objects.borrow().is_some(), "recovered to Some");
}

#[tokio::test]
async fn a_quiet_source_past_the_timeout_reports_a_gap() {
    let (pipeline, _cells) = pipeline_with(true);
    let source = source_with("sleep 2", spec());
    let (mut runner, handle) = CameraRunner::new(source, pipeline, Duration::from_millis(50));
    assert!(matches!(runner.step_once().await, Ok(StepEnd::Gap)));
    let status = *handle.status.borrow();
    assert_eq!(status.state, CameraState::WaitingForFrames);
    assert_eq!(status.source_gaps, 1);
    assert!(handle.objects.borrow().is_none());
}

#[tokio::test]
async fn a_gap_after_a_seen_frame_clears_the_objects() {
    let (pipeline, _cells) = pipeline_with(true);
    let source = source_with("head -c 28800 /dev/zero; sleep 2", spec());
    let (mut runner, handle) = CameraRunner::new(source, pipeline, Duration::from_millis(500));
    assert!(matches!(runner.step_once().await, Ok(StepEnd::Frame)));
    assert!(handle.objects.borrow().is_some(), "the frame was seen");
    assert!(matches!(runner.step_once().await, Ok(StepEnd::Gap)));
    let status = *handle.status.borrow();
    assert_eq!(status.state, CameraState::WaitingForFrames);
    assert_eq!(status.source_gaps, 1);
    assert!(
        handle.objects.borrow().is_none(),
        "the gap clears to unknown"
    );
}

#[tokio::test]
async fn an_exiting_child_reports_a_gap_after_its_last_frame() {
    let (pipeline, _cells) = pipeline_with(true);
    let source = source_with("head -c 28800 /dev/zero", spec());
    let (mut runner, handle) = CameraRunner::new(source, pipeline, Duration::from_secs(1));
    assert!(matches!(runner.step_once().await, Ok(StepEnd::Frame)));
    assert!(handle.objects.borrow().is_some(), "the frame was seen");
    assert!(
        matches!(runner.step_once().await, Ok(StepEnd::Gap)),
        "end of stream is a gap; the source reconnects on its own"
    );
    let status = *handle.status.borrow();
    assert_eq!(status.source_gaps, 1, "the gap is counted");
    assert!(
        handle.objects.borrow().is_none(),
        "end of stream clears objects to unknown"
    );
}

#[tokio::test]
async fn a_frame_of_the_wrong_length_is_a_fatal_pipeline_error() {
    let (pipeline, _cells) = pipeline_with(true);
    let short = FrameSpec::new(16, 12, 10, Decode::Cpu).expect("valid small spec");
    let source = source_with("head -c 288 /dev/zero", short);
    let (mut runner, handle) = CameraRunner::new(source, pipeline, Duration::from_secs(1));
    let error = runner.step_once().await.expect_err("wrong frame length");
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

#[tokio::test]
async fn run_publishes_failed_when_a_step_ends_on_a_fatal_error() {
    let (pipeline, _cells) = pipeline_with(true);
    let short = FrameSpec::new(16, 12, 10, Decode::Cpu).expect("valid small spec");
    let source = source_with("head -c 288 /dev/zero", short);
    let (runner, handle) = CameraRunner::new(source, pipeline, Duration::from_secs(1));
    let error = runner.run().await.expect_err("wrong frame length");
    assert!(
        matches!(error, RunnerError::Pipeline(LiveError::Region(_))),
        "not a region error: {error:?}"
    );
    assert_eq!(handle.status.borrow().state, CameraState::Failed);
}

#[tokio::test]
async fn run_returns_ok_when_the_source_reports_stopped() {
    let (pipeline, _cells) = pipeline_with(true);
    let source = FrameSource::spawn(
        "panicking".to_string(),
        SMALL,
        spec(),
        FrameClock::replay(Duration::from_secs(1000), spec()),
        || panic!("the frame command closure panicked"),
    );
    let (runner, handle) = CameraRunner::new(source, pipeline, Duration::from_secs(1));
    assert!(runner.run().await.is_ok());
    assert_eq!(
        handle.status.borrow().state,
        CameraState::WaitingForFrames,
        "a clean stop leaves the last published state, not failed"
    );
}

#[tokio::test]
async fn a_runner_keeps_stepping_after_its_handle_is_dropped() {
    let (pipeline, _cells) = pipeline_with(true);
    let source = source_with("head -c 57600 /dev/zero", spec());
    let (mut runner, handle) = CameraRunner::new(source, pipeline, Duration::from_secs(1));
    drop(handle);
    assert!(matches!(runner.step_once().await, Ok(StepEnd::Frame)));
    assert!(matches!(runner.step_once().await, Ok(StepEnd::Frame)));
}
