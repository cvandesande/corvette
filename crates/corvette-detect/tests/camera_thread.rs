// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and the crate lints do not forbid them.
use corvette_detect::{
    CameraChannels, CameraState, CameraThreadSettings, CameraThreads, CandidateStage,
    DEFAULT_POLICY, Decode, FrameClock, FrameSpec, JoinReport, Labels, LivePipeline, ObjectStage,
    Policy, ProducerPlan, RegionStage, RegionStageSettings, SharedInfer, SharedLink, ThreadEnd,
    WorkerLink,
};
use corvette_motion::{MotionSettings, Shape, SizeGrid};
use corvette_track::{IdSource, ObjectFilter, ObjectFilters, TrackerSettings, ZoneSet};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::process::Command;

include!("support/mod.rs");

const WIDTH: usize = 160;
const HEIGHT: usize = 120;
// One whole frame of the large spec: 160 * 120 * 3 / 2 bytes.
const BIG_FRAME_BYTES: usize = 28800;

const RESTART_FAST: Policy = Policy {
    initial: Duration::from_millis(10),
    max: Duration::from_millis(40),
    healthy_after: Duration::from_mins(1),
};
const SHORT_FATAL: Policy = Policy {
    initial: Duration::from_millis(30),
    max: Duration::from_millis(60),
    healthy_after: Duration::from_secs(30),
};
const LONG_BACKOFF: Policy = Policy {
    initial: Duration::from_secs(30),
    max: Duration::from_mins(1),
    healthy_after: Duration::from_mins(1),
};

fn spec() -> FrameSpec {
    FrameSpec::new(160, 120, 10, Decode::Cpu).expect("valid frame spec")
}

fn small_spec() -> FrameSpec {
    FrameSpec::new(4, 2, 5, Decode::Cpu).expect("valid frame spec")
}

fn shell(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(script);
    command.stdout(Stdio::piped());
    command
}

fn missing_worker_link() -> SharedLink {
    SharedLink::new(WorkerLink::new(
        PathBuf::from("/nonexistent/corvette-camera-thread-worker.sock"),
        Duration::from_secs(5),
        DEFAULT_POLICY,
    ))
}

fn stage_at(width: usize, height: usize) -> RegionStage {
    let settings = RegionStageSettings {
        motion: MotionSettings::default(),
        floor: 32,
        grid: SizeGrid::empty(),
        model: Shape::new(640, 640).unwrap(),
        motion_enabled: true,
    };
    RegionStage::new(settings, Shape::new(width, height).unwrap(), None).unwrap()
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

// Black frames match the detector's initial background, so no region and no inference: the missing worker socket is never reached.
fn build_live(
    ids: &mut IdSource,
    infer: SharedInfer,
) -> Result<LivePipeline<'_, SharedInfer>, String> {
    let objects = ObjectStage::new(tracker(), ids, ZoneSet::new(Vec::new()), 0.5)
        .ok_or("object stage refused")?;
    Ok(LivePipeline::new(
        stage_at(WIDTH, HEIGHT),
        candidate_stage(),
        objects,
        infer,
    ))
}

// The same pipeline with a stage that refuses every frame: the fatal error from the runner tests, repeated on every frame.
fn build_fatal(
    ids: &mut IdSource,
    infer: SharedInfer,
) -> Result<LivePipeline<'_, SharedInfer>, String> {
    let objects = ObjectStage::new(tracker(), ids, ZoneSet::new(Vec::new()), 0.5)
        .ok_or("object stage refused")?;
    Ok(LivePipeline::new(
        stage_at(64, 64),
        candidate_stage(),
        objects,
        infer,
    ))
}

fn build_panics(
    _ids: &mut IdSource,
    _infer: SharedInfer,
) -> Result<LivePipeline<'_, SharedInfer>, String> {
    panic!("the build closure panicked");
}

static BUILD_REFUSALS: AtomicUsize = AtomicUsize::new(0);

fn build_refused_twice_then_live(
    ids: &mut IdSource,
    infer: SharedInfer,
) -> Result<LivePipeline<'_, SharedInfer>, String> {
    if BUILD_REFUSALS.fetch_add(1, Ordering::SeqCst) < 2 {
        return Err(String::from("refused"));
    }
    build_live(ids, infer)
}

fn wait_until(what: &str, within: Duration, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + within;
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_healthy_camera_reaches_running_and_finishes_on_stop() {
    let channels = CameraChannels::new();
    let threads = CameraThreads::start(
        "healthy",
        CameraThreadSettings {
            frame_timeout: Duration::from_millis(500),
            backoff: DEFAULT_POLICY,
        },
        &channels,
        ProducerPlan {
            spec: spec(),
            clock: FrameClock::replay(Duration::ZERO, spec()),
        },
        move || {
            shell(&forever_stream(
                &format!("head -c {BIG_FRAME_BYTES} /dev/zero"),
                32,
            ))
        },
        &missing_worker_link(),
        build_live,
    )
    .expect("the camera threads start");
    let handle = channels.handle();
    wait_until(
        "the status to reach running",
        Duration::from_secs(5),
        || handle.status.borrow().state == CameraState::Running,
    );
    threads.stop();
    let report = threads.join(Duration::from_secs(5));
    assert_eq!(
        report,
        JoinReport {
            runner: ThreadEnd::Finished,
            producer: ThreadEnd::Finished,
        },
        "a healthy camera ends both threads"
    );
    let status = *handle.status.borrow();
    assert!(
        status.counters.frames_in >= 1,
        "frames flowed, got {:?}",
        status.counters
    );
}

#[test]
fn a_fatal_first_frame_restarts_the_runner_and_stop_ends_it() {
    let channels = CameraChannels::new();
    let threads = CameraThreads::start(
        "fatal",
        CameraThreadSettings {
            frame_timeout: Duration::from_millis(500),
            backoff: SHORT_FATAL,
        },
        &channels,
        ProducerPlan {
            spec: small_spec(),
            clock: FrameClock::replay(Duration::ZERO, small_spec()),
        },
        || shell(&forever_stream("printf abcdefghijkl || exit 1; sleep 1", 1)),
        &missing_worker_link(),
        build_fatal,
    )
    .expect("the camera threads start");
    let mut handle = channels.handle();
    wait_until("the first failure", Duration::from_secs(5), || {
        handle.status.borrow().state == CameraState::Failed
    });
    wait_until(
        "the restart handover of waiting_for_frames with one restart",
        Duration::from_secs(5),
        || {
            let status = *handle.status.borrow_and_update();
            status.state == CameraState::WaitingForFrames && status.runner_restarts == 1
        },
    );
    wait_until(
        "the restarted runner to fail again",
        Duration::from_secs(5),
        || {
            let status = *handle.status.borrow_and_update();
            status.state == CameraState::Failed && status.runner_restarts >= 1
        },
    );
    threads.stop();
    let report = threads.join(Duration::from_secs(5));
    assert_eq!(
        report,
        JoinReport {
            runner: ThreadEnd::Finished,
            producer: ThreadEnd::Finished,
        },
        "a failing camera stops on stop"
    );
}

#[test]
fn a_runner_in_a_long_backoff_returns_from_join_as_soon_as_stop_is_called() {
    let channels = CameraChannels::new();
    let threads = CameraThreads::start(
        "long-backoff",
        CameraThreadSettings {
            frame_timeout: Duration::from_millis(500),
            backoff: LONG_BACKOFF,
        },
        &channels,
        ProducerPlan {
            spec: small_spec(),
            clock: FrameClock::replay(Duration::ZERO, small_spec()),
        },
        || shell("printf abcdefghijkl; sleep 30"),
        &missing_worker_link(),
        build_fatal,
    )
    .expect("the camera threads start");
    let handle = channels.handle();
    wait_until("the first failure", Duration::from_secs(5), || {
        handle.status.borrow().state == CameraState::Failed
    });
    let stopped_at = Instant::now();
    threads.stop();
    let report = threads.join(Duration::from_secs(2));
    assert_eq!(
        report,
        JoinReport {
            runner: ThreadEnd::Finished,
            producer: ThreadEnd::Finished,
        },
        "a runner in back-off must stop at once"
    );
    assert!(
        stopped_at.elapsed() < Duration::from_secs(2),
        "join returned after {:?}",
        stopped_at.elapsed()
    );
}

#[test]
fn a_silent_child_stops_both_threads_and_dies() {
    let pid_file = std::env::temp_dir().join(format!(
        "corvette-camera-thread-child-{}.pid",
        std::process::id()
    ));
    let script = format!("echo $$ > '{}'; exec sleep 30", pid_file.display());
    let channels = CameraChannels::new();
    let threads = CameraThreads::start(
        "silent",
        CameraThreadSettings {
            frame_timeout: Duration::from_millis(500),
            backoff: SHORT_FATAL,
        },
        &channels,
        ProducerPlan {
            spec: small_spec(),
            clock: FrameClock::replay(Duration::ZERO, small_spec()),
        },
        move || shell(&script),
        &missing_worker_link(),
        build_live,
    )
    .expect("the camera threads start");
    wait_until(
        "the child to record its pid",
        Duration::from_secs(5),
        || pid_file.exists(),
    );
    let pid: u32 = fs::read_to_string(&pid_file)
        .expect("pid file")
        .trim()
        .parse()
        .expect("pid parses");
    threads.stop();
    let report = threads.join(Duration::from_secs(5));
    assert_eq!(
        report,
        JoinReport {
            runner: ThreadEnd::Finished,
            producer: ThreadEnd::Finished,
        },
        "stop must free a producer blocked on a silent child"
    );
    wait_until(
        "the child process to be gone",
        Duration::from_secs(5),
        || !Path::new(&format!("/proc/{pid}")).exists(),
    );
    fs::remove_file(&pid_file).expect("remove the pid file");
}

#[test]
fn stopping_twice_is_harmless() {
    let channels = CameraChannels::new();
    let threads = CameraThreads::start(
        "twice",
        CameraThreadSettings {
            frame_timeout: Duration::from_millis(500),
            backoff: DEFAULT_POLICY,
        },
        &channels,
        ProducerPlan {
            spec: spec(),
            clock: FrameClock::replay(Duration::ZERO, spec()),
        },
        move || {
            shell(&forever_stream(
                &format!("head -c {BIG_FRAME_BYTES} /dev/zero"),
                32,
            ))
        },
        &missing_worker_link(),
        build_live,
    )
    .expect("the camera threads start");
    let handle = channels.handle();
    wait_until(
        "the status to reach running",
        Duration::from_secs(5),
        || handle.status.borrow().state == CameraState::Running,
    );
    threads.stop();
    threads.stop();
    let report = threads.join(Duration::from_secs(5));
    assert_eq!(
        report,
        JoinReport {
            runner: ThreadEnd::Finished,
            producer: ThreadEnd::Finished,
        },
        "the second stop must be a no-op"
    );
}

#[test]
fn a_build_that_panics_reports_the_runner_as_panicked() {
    let channels = CameraChannels::new();
    let threads = CameraThreads::start(
        "panicking-build",
        CameraThreadSettings {
            frame_timeout: Duration::from_millis(500),
            backoff: SHORT_FATAL,
        },
        &channels,
        ProducerPlan {
            spec: spec(),
            clock: FrameClock::replay(Duration::ZERO, spec()),
        },
        || shell("exec sleep 30"),
        &missing_worker_link(),
        build_panics,
    )
    .expect("the camera threads start");
    threads.stop();
    let report = threads.join(Duration::from_secs(5));
    assert_eq!(
        report,
        JoinReport {
            runner: ThreadEnd::Panicked,
            producer: ThreadEnd::Finished,
        },
        "a panicking runner drops its done sender"
    );
}

#[test]
fn a_failing_pipeline_build_publishes_failed_and_retries_after_the_back_off() {
    BUILD_REFUSALS.store(0, Ordering::SeqCst);
    let channels = CameraChannels::new();
    let threads = CameraThreads::start(
        "failing-build",
        CameraThreadSettings {
            frame_timeout: Duration::from_millis(500),
            backoff: SHORT_FATAL,
        },
        &channels,
        ProducerPlan {
            spec: spec(),
            clock: FrameClock::replay(Duration::ZERO, spec()),
        },
        move || {
            shell(&forever_stream(
                &format!("head -c {BIG_FRAME_BYTES} /dev/zero"),
                32,
            ))
        },
        &missing_worker_link(),
        build_refused_twice_then_live,
    )
    .expect("the camera threads start");
    let mut handle = channels.handle();
    wait_until("the first build failure", Duration::from_secs(5), || {
        handle.status.borrow().state == CameraState::Failed
    });
    let failed_at = Instant::now();
    wait_until(
        "the third build to run and recover",
        Duration::from_secs(5),
        || {
            let status = *handle.status.borrow_and_update();
            status.state == CameraState::Running && status.runner_restarts == 2
        },
    );
    assert!(
        failed_at.elapsed() >= Duration::from_millis(30),
        "the retry must wait out the injected back-off, recovered after {:?}",
        failed_at.elapsed()
    );
    threads.stop();
    let report = threads.join(Duration::from_secs(5));
    assert_eq!(
        report,
        JoinReport {
            runner: ThreadEnd::Finished,
            producer: ThreadEnd::Finished,
        },
        "a recovered camera stops on stop"
    );
}

#[test]
fn a_child_that_exits_after_one_frame_restarts_and_feeds_a_second_frame() {
    let channels = CameraChannels::new();
    let threads = CameraThreads::start(
        "restart-feed",
        CameraThreadSettings {
            frame_timeout: Duration::from_millis(500),
            backoff: RESTART_FAST,
        },
        &channels,
        ProducerPlan {
            spec: spec(),
            clock: FrameClock::replay(Duration::ZERO, spec()),
        },
        move || shell(&format!("head -c {BIG_FRAME_BYTES} /dev/zero")),
        &missing_worker_link(),
        build_live,
    )
    .expect("the camera threads start");
    let handle = channels.handle();
    wait_until(
        "a second frame from the restarted child",
        Duration::from_secs(5),
        || handle.status.borrow().counters.frames_in >= 2,
    );
    threads.stop();
    let report = threads.join(Duration::from_secs(5));
    assert_eq!(
        report,
        JoinReport {
            runner: ThreadEnd::Finished,
            producer: ThreadEnd::Finished,
        },
        "the restart-feeding producer ends on stop"
    );
}
