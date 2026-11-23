use corvette_detect::{
    Area, CameraConfig, CameraPlan, Config, DEFAULT_POLICY, FrameClock, FrameOutcome,
    PipelineCounters, PlanError, SharedLink, WorkerLink,
};
use corvette_track::IdSource;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cplan-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn labels_in(dir: &Path) -> PathBuf {
    let path = dir.join("labels.txt");
    fs::write(&path, "0 person\n").expect("write labels");
    path
}

fn config(labels: &Path) -> Config {
    Config {
        worker_socket: PathBuf::from("/no/such/dir/plan-worker.sock"),
        labels: labels.to_path_buf(),
        deadline: Duration::from_secs(5),
        cameras: vec![front_camera()],
    }
}

fn front_camera() -> CameraConfig {
    CameraConfig {
        name: "front".to_owned(),
        width: 640,
        height: 480,
        fps: 10,
        confirm_threshold: 0.7,
        source: None,
        area: None,
    }
}

fn missing_socket_link() -> SharedLink {
    SharedLink::new(WorkerLink::new(
        PathBuf::from("/no/such/dir/plan-worker.sock"),
        Duration::from_secs(5),
        DEFAULT_POLICY,
    ))
}

#[test]
fn a_valid_plan_builds_two_fresh_pipelines_and_steps_one_frame() {
    let dir = temp_dir("valid");
    let plan = CameraPlan::new(&config(&labels_in(&dir)), &front_camera()).expect("the plan");
    assert_eq!(plan.name(), "front");
    let spec = plan.spec();
    assert_eq!((spec.width(), spec.height(), spec.fps()), (640, 480, 10));
    let link = missing_socket_link();
    let mut first_ids = IdSource::default();
    let mut first = plan
        .pipeline(&mut first_ids, link.infer_handle())
        .expect("the first pipeline");
    let mut second_ids = IdSource::default();
    let second = plan
        .pipeline(&mut second_ids, link.infer_handle())
        .expect("the second pipeline");
    assert_eq!(second.counters(), PipelineCounters::default());
    // An empty size grid gives the startup frame no region; the bright second frame makes motion, and motion gives a region to infer.
    let frame_bytes = spec.frame_bytes();
    let dark = vec![0u8; frame_bytes];
    let bright = vec![255u8; frame_bytes];
    let quiet = first.step(0.0, &dark).expect("the startup frame steps");
    assert!(matches!(quiet, FrameOutcome::Objects(_)), "{quiet:?}");
    let outcome = first
        .step(0.1, &bright)
        .expect("the motion frame steps through the missing worker socket");
    assert!(
        matches!(outcome, FrameOutcome::InferenceUnavailable),
        "the motion region reached the missing worker socket: {outcome:?}"
    );
    assert_eq!(
        first.counters(),
        PipelineCounters {
            frames_in: 2,
            frames_tracked: 1,
            frames_inference_failed: 1,
        }
    );
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn the_plan_carries_the_live_clock() {
    let dir = temp_dir("clock");
    let plan = CameraPlan::new(&config(&labels_in(&dir)), &front_camera()).expect("the plan");
    assert_eq!(plan.clock(), FrameClock::live());
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn an_unreadable_labels_path_is_a_labels_error_that_names_the_path() {
    let dir = temp_dir("no-labels");
    let missing = dir.join("absent.txt");
    let error = CameraPlan::new(&config(&missing), &front_camera())
        .expect_err("a missing labels file fails the plan");
    assert!(matches!(error, PlanError::Labels(_)), "{error}");
    assert!(
        error.to_string().contains("absent.txt"),
        "the error names the path: {error}"
    );
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn a_malformed_labels_file_is_a_labels_error() {
    let dir = temp_dir("bad-labels");
    let labels = dir.join("labels.txt");
    fs::write(&labels, "0 person\nnope person\n").expect("write labels");
    let error = CameraPlan::new(&config(&labels), &front_camera())
        .expect_err("a malformed label list fails the plan");
    assert!(matches!(error, PlanError::Labels(_)), "{error}");
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn an_area_point_outside_the_pixel_range_is_an_area_error() {
    let dir = temp_dir("bad-area");
    let camera = CameraConfig {
        name: "front".to_owned(),
        width: 640,
        height: 480,
        fps: 10,
        confirm_threshold: 0.7,
        source: None,
        area: Some(Area {
            name: "yard".to_owned(),
            // A point whose scaled pixel leaves i32: Config's own bounds refuse such a polygon, so the camera is built directly.
            polygon: vec![(0.0, 0.0), (1.0, 0.0), (1.0e10, 0.5)],
            pad_pixels: 0,
        }),
    };
    let error = CameraPlan::new(&config(&labels_in(&dir)), &camera)
        .expect_err("the refused polygon fails the plan");
    assert!(matches!(error, PlanError::Area(_)), "{error}");
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn a_frame_too_narrow_for_the_motion_shape_is_a_stage_error() {
    let dir = temp_dir("narrow");
    // Width 2 at height 1600: the motion frame scales the width to zero and the region stage refuses.
    let camera = CameraConfig {
        name: "tall".to_owned(),
        width: 2,
        height: 1600,
        fps: 10,
        confirm_threshold: 0.7,
        source: None,
        area: None,
    };
    let error = CameraPlan::new(&config(&labels_in(&dir)), &camera)
        .expect_err("the narrow frame fails the plan");
    assert!(matches!(error, PlanError::Stage("region")), "{error}");
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn the_decode_command_names_the_sub_stream_url_exactly_once() {
    let dir = temp_dir("command");
    let plan = CameraPlan::new(&config(&labels_in(&dir)), &front_camera()).expect("the plan");
    let mut factory = plan.command_factory(
        PathBuf::from("/usr/bin/ffmpeg"),
        String::from("127.0.0.1:8554"),
    );
    let url = "rtsp://127.0.0.1:8554/front_sub";
    let first = factory();
    assert_eq!(first.as_std().get_program(), "/usr/bin/ffmpeg");
    let args: Vec<String> = first
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(args.iter().filter(|arg| arg.as_str() == url).count(), 1);
    let second = factory();
    let second_args: Vec<String> = second
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(second_args, args);
    fs::remove_dir_all(&dir).expect("remove temp dir");
}
