use corvette_detect::{Area, Config, ConfigError, DEFAULT_PAD_PIXELS, render};

const TWO_CAMERAS: &str = r#"# Detection settings for the house, edited by hand.
worker_socket = "/run/detect.sock"
labels = "labels.txt"
deadline_ms = 10000

# the back yard camera
[[cameras]]
name = "back"
width = 704
height = 576
# detect rate
fps = 5 # five frames a second
confirm_threshold = 0.7

[cameras.area]
name = "Back-Detection-Area"
polygon = [[0.04, 0.43], [0.53, 0.25], [0.85, 0.38]]
pad_pixels = 16

# the front door camera
[[cameras]]
name = "front"
width = 640
height = 480
fps = 2
"#;

const COMMENTED: &str = r#"# Detection settings for the house, edited by hand.
worker_socket = "/run/detect.sock"
labels = "labels.txt"
deadline_ms = 10000

# the back yard camera
[[cameras]]
name = "back"
width = 704
height = 576
# detect rate
fps = 5 # five frames a second
# confirm level
confirm_threshold = 0.7

[cameras.area]
name = "Back-Detection-Area"
polygon = [[0.04, 0.43], [0.53, 0.25], [0.85, 0.38]]
pad_pixels = 16

# the front door camera
[[cameras]]
name = "front"
width = 640
height = 480
fps = 2
"#;

const NO_DEFAULTS: &str = r#"worker_socket = "/s"
labels = "l.txt"
deadline_ms = 100
[[cameras]]
name = "a"
width = 640
height = 480
fps = 5

[cameras.area]
name = "zone"
polygon = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]
"#;

const KEY_ORDER: &str = r#"worker_socket = "/s"
labels = "l.txt"
deadline_ms = 100
[[cameras]]
fps = 5
# above name
name = "a"
height = 480
width = 640
"#;

#[test]
fn fresh_render_round_trips_to_the_same_value() {
    let config = Config::from_toml(TWO_CAMERAS).expect("loads");
    let out = render(None, &config).expect("renders");
    let again = Config::from_toml(&out).expect("reloads");
    assert_eq!(again.to_value(), config.to_value());
}

#[test]
fn unchanged_render_is_byte_identical_after_one_newline() {
    let config = Config::from_toml(TWO_CAMERAS).expect("loads");
    assert_eq!(
        render(Some(TWO_CAMERAS), &config).expect("renders"),
        TWO_CAMERAS
    );
    let stripped = TWO_CAMERAS.trim_end_matches('\n');
    assert_eq!(
        render(Some(stripped), &config).expect("renders"),
        TWO_CAMERAS
    );
    let padded = format!("{TWO_CAMERAS}\n\n\n");
    assert_eq!(
        render(Some(&padded), &config).expect("renders"),
        TWO_CAMERAS
    );
    let out = render(Some(TWO_CAMERAS), &config).expect("renders");
    assert!(
        out.contains("fps = 5 # five frames a second"),
        "unchanged trailing comment lost:\n{out}"
    );
    assert!(
        out.contains("[[0.04, 0.43]"),
        "unchanged number format changed:\n{out}"
    );
}

#[test]
fn rendering_the_output_again_changes_nothing() {
    let config = Config::from_toml(TWO_CAMERAS).expect("loads");
    for first in [
        render(None, &config),
        render(Some(TWO_CAMERAS), &config),
        render(Some(TWO_CAMERAS.trim_end_matches('\n')), &config),
        render(Some(&format!("{TWO_CAMERAS}\n\n\n")), &config),
    ] {
        let out = first.expect("renders");
        assert_eq!(render(Some(&out), &config).expect("renders"), out);
    }
}

#[test]
fn rewriting_a_camera_keeps_comments_elsewhere() {
    let mut config = Config::from_toml(COMMENTED).expect("loads");
    config.cameras[0].width = 800;
    config.cameras[0].fps = 9;
    config.cameras[0].area.as_mut().expect("has area").polygon =
        vec![(0.0, 0.0), (1.0, 0.5), (0.5, 1.0)];
    config.cameras[1].area = Some(Area {
        name: "Front-Door".to_owned(),
        polygon: vec![(0.1, 0.2), (0.3, 0.4), (0.5, 0.6)],
        pad_pixels: DEFAULT_PAD_PIXELS,
    });
    let out = render(Some(COMMENTED), &config).expect("renders");
    assert_eq!(
        Config::from_toml(&out).expect("reloads").to_value(),
        config.to_value()
    );
    for comment in [
        "# Detection settings for the house, edited by hand.",
        "# the back yard camera",
        "# the front door camera",
        "# confirm level",
        "# detect rate",
    ] {
        assert!(out.contains(comment), "comment lost: {comment}\n{out}");
    }
    assert!(out.contains("width = 800"), "changed width missing:\n{out}");
    assert!(out.contains("fps = 9"), "changed fps missing:\n{out}");
    assert!(
        out.contains("1.0, 0.5"),
        "float not written with decimal point:\n{out}"
    );
    assert!(out.contains("Front-Door"), "added area missing:\n{out}");
}

#[test]
fn removing_a_camera_keeps_the_survivor_and_its_comments() {
    let mut config = Config::from_toml(TWO_CAMERAS).expect("loads");
    config.cameras.remove(0);
    let out = render(Some(TWO_CAMERAS), &config).expect("renders");
    assert!(
        !out.contains("back"),
        "removed camera still present:\n{out}"
    );
    assert!(
        out.contains("# the front door camera"),
        "survivor comment lost:\n{out}"
    );
    assert!(out.contains("name = \"front\""), "survivor lost:\n{out}");
}

#[test]
fn cameras_follow_the_submitted_order_when_one_is_added() {
    let mut config = Config::from_toml(TWO_CAMERAS).expect("loads");
    let mut side = config.cameras[0].clone();
    side.name = "side".to_owned();
    config.cameras.push(side);
    let out = render(Some(TWO_CAMERAS), &config).expect("renders");
    let back = out.find("back").expect("back present");
    let front = out.find("front").expect("front present");
    let new = out.find("side").expect("side present");
    assert!(back < front && front < new, "submitted order lost:\n{out}");
    assert!(
        out.contains("# the back yard camera"),
        "survivor comment lost:\n{out}"
    );
    assert_eq!(
        Config::from_toml(&out).expect("reloads").to_value(),
        config.to_value()
    );
}

#[test]
fn cameras_follow_the_submitted_order_when_reordered() {
    let mut config = Config::from_toml(TWO_CAMERAS).expect("loads");
    config.cameras.swap(0, 1);
    let out = render(Some(TWO_CAMERAS), &config).expect("renders");
    let front = out.find("name = \"front\"").expect("front present");
    let back = out.find("name = \"back\"").expect("back present");
    assert!(front < back, "submitted order lost:\n{out}");
    assert!(
        out.find("# the front door camera").expect("front comment") < front,
        "comment left its camera:\n{out}"
    );
    assert!(
        out.contains("# the back yard camera"),
        "survivor comment lost:\n{out}"
    );
}

#[test]
fn defaults_omitted_by_the_file_stay_omitted() {
    let config = Config::from_toml(NO_DEFAULTS).expect("loads");
    let out = render(Some(NO_DEFAULTS), &config).expect("renders");
    assert!(
        !out.contains("confirm_threshold"),
        "default materialised:\n{out}"
    );
    assert!(!out.contains("pad_pixels"), "default materialised:\n{out}");
}

#[test]
fn an_explicit_default_stays_written() {
    let config = Config::from_toml(TWO_CAMERAS).expect("loads");
    let out = render(Some(TWO_CAMERAS), &config).expect("renders");
    assert!(
        out.contains("confirm_threshold = 0.7"),
        "explicit default lost:\n{out}"
    );
    assert!(
        out.contains("pad_pixels = 16"),
        "explicit default lost:\n{out}"
    );
}

#[test]
fn camera_keys_come_out_in_declared_order_with_comments() {
    let config = Config::from_toml(KEY_ORDER).expect("loads");
    let out = render(Some(KEY_ORDER), &config).expect("renders");
    let mut last = 0;
    for key in ["name = ", "width = ", "height = ", "fps = "] {
        let found = out.find(key).expect("key present");
        assert!(found >= last, "keys not in declared order:\n{out}");
        last = found;
    }
    assert!(
        out.contains("# above name\nname = "),
        "comment lost its key:\n{out}"
    );
}

#[test]
fn unparseable_existing_text_returns_a_syntax_error() {
    let config = Config::from_toml(NO_DEFAULTS).expect("loads");
    let error = render(Some("this is = = not toml"), &config).expect_err("fails");
    assert!(
        matches!(error, ConfigError::Syntax { .. }),
        "not a syntax error: {error}"
    );
}
