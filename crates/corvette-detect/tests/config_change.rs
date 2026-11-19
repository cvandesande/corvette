//! Pins which configuration changes apply live and which need a restart.

use corvette_detect::{Change, Config, classify};

fn config(text: &str) -> Config {
    Config::from_toml(text).expect("valid configuration")
}

const GLOBAL: &str = r#"worker_socket = "/run/a.sock"
labels = "/run/b.txt"
deadline_ms = 5000

"#;

const FRONT: &str = r#"[[cameras]]
name = "front"
width = 640
height = 480
fps = 5
confirm_threshold = 0.7
[cameras.area]
name = "door"
polygon = [[0.1, 0.2], [0.9, 0.2], [0.9, 0.8]]
"#;

const YARD: &str = r#"
[[cameras]]
name = "yard"
width = 704
height = 576
fps = 10
"#;

const BACK: &str = r#"
[[cameras]]
name = "back"
width = 800
height = 600
fps = 8
"#;

fn base() -> String {
    format!("{GLOBAL}{FRONT}{YARD}")
}

#[test]
fn identical_configs_change_nothing() {
    assert_eq!(
        classify(&config(&base()), &config(&base())),
        Change::default()
    );
}

#[test]
fn worker_socket_change_requires_restart() {
    let proposed = config(&base().replace("a.sock", "z.sock"));
    let change = classify(&config(&base()), &proposed);
    assert_eq!(change.restart_required, ["worker_socket"]);
    assert!(change.live.is_empty());
}

#[test]
fn labels_change_requires_restart() {
    let proposed = config(&base().replace("b.txt", "z.txt"));
    let change = classify(&config(&base()), &proposed);
    assert_eq!(change.restart_required, ["labels"]);
    assert!(change.live.is_empty());
}

#[test]
fn deadline_change_requires_restart() {
    let proposed = config(&base().replace("deadline_ms = 5000", "deadline_ms = 4000"));
    let change = classify(&config(&base()), &proposed);
    assert_eq!(change.restart_required, ["deadline_ms"]);
    assert!(change.live.is_empty());
}

#[test]
fn all_global_fields_change_in_declared_order() {
    let text = base()
        .replace("a.sock", "z.sock")
        .replace("b.txt", "z.txt")
        .replace("deadline_ms = 5000", "deadline_ms = 4000");
    let change = classify(&config(&base()), &config(&text));
    assert_eq!(
        change.restart_required,
        ["worker_socket", "labels", "deadline_ms"]
    );
    assert!(change.live.is_empty());
}

#[test]
fn camera_fps_change_is_live() {
    let proposed = config(&base().replace("fps = 5", "fps = 6"));
    let change = classify(&config(&base()), &proposed);
    assert_eq!(change.live, ["front"]);
    assert!(change.restart_required.is_empty());
}

#[test]
fn camera_area_change_is_live() {
    let proposed = config(&base().replace("[0.1, 0.2]", "[0.2, 0.3]"));
    let change = classify(&config(&base()), &proposed);
    assert_eq!(change.live, ["front"]);
    assert!(change.restart_required.is_empty());
}

#[test]
fn camera_confirm_threshold_change_is_live() {
    let proposed = config(&base().replace("confirm_threshold = 0.7", "confirm_threshold = 0.8"));
    let change = classify(&config(&base()), &proposed);
    assert_eq!(change.live, ["front"]);
    assert!(change.restart_required.is_empty());
}

#[test]
fn two_changed_cameras_live_in_proposed_order() {
    let text = base()
        .replace("fps = 5", "fps = 6")
        .replace("fps = 10", "fps = 12");
    let change = classify(&config(&base()), &config(&text));
    assert_eq!(change.live, ["front", "yard"]);
    assert!(change.restart_required.is_empty());
}

#[test]
fn removed_camera_requires_restart() {
    let change = classify(&config(&base()), &config(&format!("{GLOBAL}{FRONT}")));
    assert_eq!(change.restart_required, ["cameras[yard]"]);
    assert!(change.live.is_empty());
}

#[test]
fn added_camera_requires_restart() {
    let text = format!("{GLOBAL}{FRONT}{YARD}{BACK}");
    let change = classify(&config(&base()), &config(&text));
    assert_eq!(change.restart_required, ["cameras[back]"]);
    assert!(change.live.is_empty());
}

#[test]
fn removed_camera_is_reported_before_added_camera() {
    let change = classify(&config(&base()), &config(&format!("{GLOBAL}{FRONT}{BACK}")));
    assert_eq!(change.restart_required, ["cameras[yard]", "cameras[back]"]);
    assert!(change.live.is_empty());
}

#[test]
fn reordered_cameras_change_nothing() {
    let change = classify(&config(&base()), &config(&format!("{GLOBAL}{YARD}{FRONT}")));
    assert_eq!(change, Change::default());
}

#[test]
fn combined_change_splits_restart_and_live_paths() {
    let text = format!(
        "{}{}",
        base()
            .replace("deadline_ms = 5000", "deadline_ms = 4000")
            .replace("fps = 5", "fps = 6"),
        BACK
    );
    let change = classify(&config(&base()), &config(&text));
    assert_eq!(change.restart_required, ["deadline_ms", "cameras[back]"]);
    assert_eq!(change.live, ["front"]);
}
