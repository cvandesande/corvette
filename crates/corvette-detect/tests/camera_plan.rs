use corvette_detect::{
    Area, CameraConfig, CameraPlan, CameraSource, Config, DEFAULT_POLICY, FrameClock, FrameOutcome,
    PipelineCounters, PlanError, SharedLink, StreamSource, WorkerLink, stream_address,
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

const CANARY: &str = "p@ss:wor/d 1";

type SecretPair = (&'static str, &'static str);
type SecretRow = (&'static str, &'static [SecretPair], &'static str);

const fn no_secret(_: &str) -> Option<String> {
    None
}

fn env_lookup<'a>(pairs: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_owned())
    }
}

fn plain_stream(host: &str, path: &str) -> StreamSource {
    StreamSource {
        host: host.to_owned(),
        port: 554,
        path: path.to_owned(),
        username: String::new(),
        secret_env: None,
    }
}

fn secret_stream(host: &str, path: &str, variable: &str) -> StreamSource {
    StreamSource {
        host: host.to_owned(),
        port: 554,
        path: path.to_owned(),
        username: "u".to_owned(),
        secret_env: Some(variable.to_owned()),
    }
}

fn camera_with_source(main: Option<StreamSource>, sub: Option<StreamSource>) -> CameraConfig {
    CameraConfig {
        source: Some(CameraSource { main, sub }),
        ..front_camera()
    }
}

fn dialed_url(
    plan: &CameraPlan,
    camera: &CameraConfig,
    restream_address: &str,
    lookup: &impl Fn(&str) -> Option<String>,
) -> String {
    let address = stream_address(camera, restream_address, lookup).expect("the dial");
    let mut factory = plan.command_factory(PathBuf::from("/usr/bin/ffmpeg"), address);
    let args: Vec<String> = factory()
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let at = args
        .iter()
        .position(|arg| arg == "-i")
        .expect("the input flag");
    args[at + 1].clone()
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
    let address = stream_address(&front_camera(), "127.0.0.1:8554", &no_secret).expect("the dial");
    let mut factory = plan.command_factory(PathBuf::from("/usr/bin/ffmpeg"), address);
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

#[test]
fn the_plan_dials_the_sub_stream_then_main_then_the_restream_form() {
    let dir = temp_dir("dial");
    let plan = CameraPlan::new(&config(&labels_in(&dir)), &front_camera()).expect("the plan");
    let lookup = env_lookup(&[("CAM_SECRET", CANARY)]);
    let rows = [
        (
            "a camera without a source keeps today's restream address byte for byte",
            front_camera(),
            "127.0.0.1:8554",
            "rtsp://127.0.0.1:8554/front_sub",
        ),
        (
            "a camera with sub and main dials sub",
            camera_with_source(
                Some(secret_stream("cam-main", "/main", "CAM_SECRET")),
                Some(secret_stream("cam-sub", "/sub", "CAM_SECRET")),
            ),
            "",
            "rtsp://u:p%40ss%3Awor%2Fd%201@cam-sub:554/sub",
        ),
        (
            "a camera with only main dials main",
            camera_with_source(Some(plain_stream("cam-main", "/live")), None),
            "",
            "rtsp://cam-main:554/live",
        ),
    ];
    for (what, camera, restream_address, expected) in rows {
        assert_eq!(
            dialed_url(&plan, &camera, restream_address, &lookup),
            expected,
            "{what}"
        );
    }
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn two_cameras_in_one_config_dial_their_own_streams() {
    let dir = temp_dir("two-dials");
    let mut config = config(&labels_in(&dir));
    let front = camera_with_source(
        Some(secret_stream("cam-front-main", "/main", "CAM_SECRET")),
        Some(secret_stream("cam-front-sub", "/sub", "CAM_SECRET")),
    );
    let back = camera_with_source(Some(plain_stream("cam-back", "/back")), None);
    config.cameras = vec![front.clone(), back];
    let plan = CameraPlan::new(&config, &front).expect("the plan");
    let lookup = env_lookup(&[("CAM_SECRET", CANARY)]);
    let expected = [
        "rtsp://u:p%40ss%3Awor%2Fd%201@cam-front-sub:554/sub",
        "rtsp://cam-back:554/back",
    ];
    for (camera, expected) in config.cameras.iter().zip(expected) {
        assert_eq!(dialed_url(&plan, camera, "", &lookup), expected);
    }
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn an_unset_or_empty_secret_variable_refuses_the_dial_naming_camera_and_key() {
    let rows: [SecretRow; 2] = [
        ("unset", &[("OTHER_SECRET", CANARY)], "variable is not set"),
        (
            "empty",
            &[("OTHER_SECRET", CANARY), ("CAM_SECRET", "")],
            "variable is empty",
        ),
    ];
    for (what, pairs, rule) in rows {
        let lookup = env_lookup(pairs);
        let camera =
            camera_with_source(Some(secret_stream("cam-main", "/live", "CAM_SECRET")), None);
        let error = stream_address(&camera, "", &lookup).expect_err("the refusal");
        assert_eq!(
            error.to_string(),
            format!("camera front: source main: secret_env: {rule}"),
            "{what}"
        );
        assert!(!error.to_string().contains(CANARY), "{what}");
    }
}
