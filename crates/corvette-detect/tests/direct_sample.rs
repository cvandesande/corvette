//! Acceptance over the tracked direct-source sample: loader, dial and refusal texts.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use corvette_detect::{CameraConfig, CameraPlan, Config, stream_address};

const FIXTURE: &str = include_str!("fixtures/detection-direct.toml");
const CANARY: &str = "p@ss:wor/d 1";
const ENCODED_CANARY: &str = "p%40ss%3Awor%2Fd%201";
const HOST: &str = "HOST_PLACEHOLDER";
const STREAM_PATH: &str = "/PATH_PLACEHOLDER";
const USERNAME: &str = "USERNAME_PLACEHOLDER";
const CAMERAS: [(&str, &str); 2] = [
    ("front", "CORVETTE_CAM_FRONT"),
    ("back", "CORVETTE_CAM_BACK"),
];

// Detect fields copied from the window's detection.toml; the test reads no path under .agents.
const WINDOW_SIZE: (u32, u32, u32) = (704, 576, 5);
const WINDOW_CONFIRM_BITS: u64 = 0.7_f64.to_bits();
const FRONT_POLYGON: &[(f64, f64)] = &[
    (0.002, 0.618),
    (0.0, 0.754),
    (0.042, 0.959),
    (0.056, 1.0),
    (0.831, 1.0),
    (0.914, 0.289),
    (0.555, 0.108),
];
const BACK_POLYGON: &[(f64, f64)] = &[
    (0.041, 0.427),
    (0.533, 0.246),
    (0.846, 0.381),
    (1.0, 0.483),
    (1.0, 1.0),
    (0.131, 0.998),
];

fn sub_table(variable: &str, path: &str) -> String {
    format!(
        "[cameras.source.sub]\n\
         host = \"{HOST}\"\n\
         port = 554\n\
         path = \"{path}\"\n\
         username = \"{USERNAME}\"\n\
         secret_env = \"{variable}\"\n"
    )
}

fn plan_for(sample: &Config) -> (PathBuf, CameraPlan) {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let mut name = format!("cdirect-{}", std::process::id());
    name.push_str(&COUNTER.fetch_add(1, Ordering::Relaxed).to_string());
    let dir = std::env::temp_dir().join(name);
    fs::create_dir_all(&dir).expect("temp dir");
    fs::write(dir.join("labels.txt"), "0 person\n").expect("write labels");
    let plan_config = Config {
        worker_socket: PathBuf::from("/no/such/dir/direct.sock"),
        labels: dir.join("labels.txt"),
        deadline: sample.deadline,
        cameras: sample.cameras.clone(),
    };
    let plan = CameraPlan::new(&plan_config, &sample.cameras[0]).expect("the plan");
    (dir, plan)
}

fn dialed_url(plan: &CameraPlan, camera: &CameraConfig, variable: &str) -> String {
    let lookup = |name: &str| (name == variable).then(|| CANARY.to_owned());
    let address = stream_address(camera, "", &lookup).expect("the dial");
    let mut factory = plan.command_factory(PathBuf::from("/usr/bin/ffmpeg"), &address);
    let args: Vec<String> = factory()
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let at = args.iter().position(|arg| arg == "-i").unwrap();
    args[at + 1].clone()
}

#[test]
fn the_sample_loads_both_cameras_with_both_streams_and_each_secret_variable() {
    let sample = Config::from_toml(FIXTURE).expect("sample accepted");
    assert_eq!(sample.cameras.len(), CAMERAS.len());
    for (camera, (name, variable)) in sample.cameras.iter().zip(CAMERAS) {
        assert_eq!(camera.name, name);
        let source = camera.source.as_ref().expect("source");
        let streams = [
            ("main", source.main.as_ref().expect("main")),
            ("sub", source.sub.as_ref().expect("sub")),
        ];
        for (kind, stream) in streams {
            assert_eq!(stream.host, HOST, "{name} {kind} host");
            assert_eq!(stream.port, 554, "{name} {kind} port");
            assert_eq!(stream.path, STREAM_PATH, "{name} {kind} path");
            assert_eq!(stream.username, USERNAME, "{name} {kind} username");
            assert_eq!(
                stream.secret_env.as_deref(),
                Some(variable),
                "{name} {kind} secret_env"
            );
        }
    }
}

#[test]
fn the_sample_dials_the_sub_stream_of_each_camera_with_the_encoded_secret() {
    let sample = Config::from_toml(FIXTURE).expect("sample accepted");
    let (dir, plan) = plan_for(&sample);
    let expected = format!("rtsp://{USERNAME}:{ENCODED_CANARY}@{HOST}:554{STREAM_PATH}");
    for (camera, (name, variable)) in sample.cameras.iter().zip(CAMERAS) {
        assert_eq!(camera.name, name);
        let url = dialed_url(&plan, camera, variable);
        assert_eq!(url, expected);
        assert!(url.contains(STREAM_PATH) && url.contains(ENCODED_CANARY));
        assert!(!url.contains(CANARY), "the raw secret reached {url}");
    }
    let probe_paths = [
        ("CORVETTE_CAM_FRONT", "/front-sub-probe"),
        ("CORVETTE_CAM_BACK", "/back-sub-probe"),
    ];
    let mut text = FIXTURE.to_owned();
    for (variable, probe) in probe_paths {
        text = text.replace(
            &sub_table(variable, STREAM_PATH),
            &sub_table(variable, probe),
        );
    }
    let probed_sample = Config::from_toml(&text).expect("probed sample accepted");
    for (index, camera) in probed_sample.cameras.iter().enumerate() {
        let (variable, probe) = probe_paths[index];
        let url = dialed_url(&plan, camera, variable);
        assert!(url.contains(probe), "{probe} missing from {url}");
        assert!(!url.contains(STREAM_PATH), "the main path reached {url}");
    }
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn the_sample_without_sub_tables_loads_and_dials_main() {
    let mut text = FIXTURE.to_owned();
    for (_, variable) in CAMERAS {
        text = text.replace(&format!("\n{}", sub_table(variable, STREAM_PATH)), "");
    }
    let sample = Config::from_toml(&text).expect("a main-only camera is accepted");
    let (dir, plan) = plan_for(&sample);
    let expected = format!("rtsp://{USERNAME}:{ENCODED_CANARY}@{HOST}:554{STREAM_PATH}");
    for (camera, (name, variable)) in sample.cameras.iter().zip(CAMERAS) {
        let source = camera.source.as_ref().expect("source");
        assert!(
            source.main.is_some() && source.sub.is_none(),
            "{name} keeps only main"
        );
        assert_eq!(dialed_url(&plan, camera, variable), expected);
    }
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn the_sample_cameras_match_the_window_sample_detect_fields() {
    let sample = Config::from_toml(FIXTURE).expect("sample accepted");
    assert_eq!(
        sample.labels,
        PathBuf::from("/config/model_cache/yolov9t-640-20260715-labelmap.txt")
    );
    assert_eq!(sample.cameras.len(), CAMERAS.len());
    for (camera, (name, _)) in sample.cameras.iter().zip(CAMERAS) {
        assert_eq!(camera.name, name);
        assert_eq!((camera.width, camera.height, camera.fps), WINDOW_SIZE);
        assert_eq!(camera.confirm_threshold.to_bits(), WINDOW_CONFIRM_BITS);
    }
    let front = sample.cameras[0].area.as_ref().expect("front area");
    assert_eq!(front.name, "Front-Detection-Area");
    assert_eq!(front.polygon, FRONT_POLYGON);
    let back = sample.cameras[1].area.as_ref().expect("back area");
    assert_eq!(back.name, "Back-Detection-Area");
    assert_eq!(back.polygon, BACK_POLYGON);
}

#[test]
fn broken_copies_of_the_sample_refuse_naming_camera_and_key_and_hide_no_value() {
    let rows = [
        (
            FIXTURE.replace("secret_env", "secret_file"),
            "camera front: source main: secret_file: unknown key",
        ),
        (
            FIXTURE.replace(&format!("host = \"{HOST}\"\n"), ""),
            "camera front: source main: host: missing",
        ),
    ];
    for (text, want) in rows {
        let refusal = Config::from_toml(&text).expect_err("refused");
        let line = refusal.to_string();
        assert_eq!(line, want);
        assert!(!line.contains(CANARY) && !line.contains(HOST) && !line.contains(USERNAME));
    }
}
