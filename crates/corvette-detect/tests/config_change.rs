//! Pins the restart-required paths a classification answers; every change is restart-required.

use corvette_detect::{Config, classify};

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
    assert!(classify(&config(&base()), &config(&base())).is_empty());
}

#[test]
fn worker_socket_change_requires_restart() {
    let proposed = config(&base().replace("a.sock", "z.sock"));
    assert_eq!(classify(&config(&base()), &proposed), ["worker_socket"]);
}

#[test]
fn labels_change_requires_restart() {
    let proposed = config(&base().replace("b.txt", "z.txt"));
    assert_eq!(classify(&config(&base()), &proposed), ["labels"]);
}

#[test]
fn deadline_change_requires_restart() {
    let proposed = config(&base().replace("deadline_ms = 5000", "deadline_ms = 4000"));
    assert_eq!(classify(&config(&base()), &proposed), ["deadline_ms"]);
}

#[test]
fn all_global_fields_change_in_declared_order() {
    let text = base()
        .replace("a.sock", "z.sock")
        .replace("b.txt", "z.txt")
        .replace("deadline_ms = 5000", "deadline_ms = 4000");
    assert_eq!(
        classify(&config(&base()), &config(&text)),
        ["worker_socket", "labels", "deadline_ms"]
    );
}

#[test]
fn camera_fps_change_requires_restart() {
    let proposed = config(&base().replace("fps = 5", "fps = 6"));
    assert_eq!(classify(&config(&base()), &proposed), ["cameras[front]"]);
}

#[test]
fn camera_area_change_requires_restart() {
    let proposed = config(&base().replace("[0.1, 0.2]", "[0.2, 0.3]"));
    assert_eq!(classify(&config(&base()), &proposed), ["cameras[front]"]);
}

#[test]
fn camera_confirm_threshold_change_requires_restart() {
    let proposed = config(&base().replace("confirm_threshold = 0.7", "confirm_threshold = 0.8"));
    assert_eq!(classify(&config(&base()), &proposed), ["cameras[front]"]);
}

#[test]
fn two_changed_cameras_in_proposed_order() {
    let text = base()
        .replace("fps = 5", "fps = 6")
        .replace("fps = 10", "fps = 12");
    assert_eq!(
        classify(&config(&base()), &config(&text)),
        ["cameras[front]", "cameras[yard]"]
    );
}

#[test]
fn removed_camera_requires_restart() {
    assert_eq!(
        classify(&config(&base()), &config(&format!("{GLOBAL}{FRONT}"))),
        ["cameras[yard]"]
    );
}

#[test]
fn added_camera_requires_restart() {
    let text = format!("{GLOBAL}{FRONT}{YARD}{BACK}");
    assert_eq!(
        classify(&config(&base()), &config(&text)),
        ["cameras[back]"]
    );
}

#[test]
fn removed_camera_is_reported_before_added_camera() {
    assert_eq!(
        classify(&config(&base()), &config(&format!("{GLOBAL}{FRONT}{BACK}"))),
        ["cameras[yard]", "cameras[back]"]
    );
}

#[test]
fn reordered_cameras_change_nothing() {
    assert!(classify(&config(&base()), &config(&format!("{GLOBAL}{YARD}{FRONT}"))).is_empty());
}

#[test]
fn changed_removed_and_added_cameras_pinned_order() {
    let text = format!("{}{}{}", GLOBAL, FRONT.replace("fps = 5", "fps = 6"), BACK);
    assert_eq!(
        classify(&config(&base()), &config(&text)),
        ["cameras[yard]", "cameras[front]", "cameras[back]"]
    );
}

#[test]
fn global_change_precedes_camera_change() {
    let text = base()
        .replace("deadline_ms = 5000", "deadline_ms = 4000")
        .replace("fps = 5", "fps = 6");
    assert_eq!(
        classify(&config(&base()), &config(&text)),
        ["deadline_ms", "cameras[front]"]
    );
}
