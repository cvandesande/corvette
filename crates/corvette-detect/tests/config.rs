use std::path::PathBuf;
use std::time::Duration;

use corvette_detect::{
    AREA_KEYS, CAMERA_KEYS, CONFIG_KEYS, CONFIRM_MAX, CONFIRM_MIN_EXCLUSIVE, Config,
    DEFAULT_CONFIRM_THRESHOLD, DEFAULT_PAD_PIXELS, DEFAULT_STREAM_PATH, DEFAULT_STREAM_PORT,
    MAX_DEADLINE_MS, MAX_EDGE, MAX_FPS, MAX_PAD_PIXELS, MAX_POLYGON_COORD, MAX_STREAM_PORT,
    MIN_CAMERAS, MIN_DEADLINE_MS, MIN_EDGE, MIN_FPS, MIN_POLYGON_COORD, MIN_POLYGON_POINTS,
    MIN_STREAM_PORT, POINT_COORDINATES, REQUIRED_AREA_KEYS, REQUIRED_CAMERA_KEYS,
    REQUIRED_CONFIG_KEYS, REQUIRED_STREAM_KEYS, SOURCE_KEYS, STREAM_KEYS,
};
use toml_edit::DocumentMut;

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

const MINIMAL_AREA: &str = r#"worker_socket = "/s"
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

fn camera_with(fields: &str) -> String {
    format!(
        r#"worker_socket = "/s"
labels = "l.txt"
deadline_ms = 100
[[cameras]]
name = "a"
{fields}"#
    )
}

fn camera_with_area(area_extra: &str) -> String {
    format!(
        r#"worker_socket = "/s"
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
{area_extra}"#
    )
}

fn camera_area_polygon(polygon: &str) -> String {
    format!(
        r#"worker_socket = "/s"
labels = "l.txt"
deadline_ms = 100
[[cameras]]
name = "a"
width = 640
height = 480
fps = 5

[cameras.area]
name = "zone"
polygon = {polygon}
"#
    )
}

fn doc_deadline(deadline_ms: u32) -> String {
    format!(
        r#"worker_socket = "/s"
labels = "l.txt"
deadline_ms = {deadline_ms}
[[cameras]]
name = "a"
width = 640
height = 480
fps = 5
"#
    )
}

#[test]
fn assigning_one_value_keeps_comments_and_camera_order() {
    let mut doc = TWO_CAMERAS.parse::<DocumentMut>().expect("fixture parses");
    doc["cameras"][0]["fps"] = toml_edit::value(6i64);
    let out = doc.to_string();
    assert!(
        out.contains("# Detection settings for the house, edited by hand."),
        "leading comment lost:\n{out}"
    );
    assert!(
        out.contains("# the back yard camera"),
        "comment above camera header lost:\n{out}"
    );
    assert!(
        out.contains("# detect rate"),
        "comment above the changed key lost:\n{out}"
    );
    assert!(
        out.contains("# the front door camera"),
        "comment on the other camera lost:\n{out}"
    );
    assert!(out.contains("fps = 6"), "changed value not written:\n{out}");
    assert!(
        !out.contains("five frames a second"),
        "trailing comment on the changed value unexpectedly kept:\n{out}"
    );
    let back = out.find("back").expect("back camera present");
    let front = out.find("front").expect("front camera present");
    assert!(back < front, "camera order changed:\n{out}");
}

#[test]
fn complete_file_loads_every_field() {
    let config = Config::from_toml(TWO_CAMERAS).expect("loads");
    assert_eq!(config.worker_socket, PathBuf::from("/run/detect.sock"));
    assert_eq!(config.labels, PathBuf::from("labels.txt"));
    assert_eq!(config.deadline, Duration::from_secs(10));
    assert_eq!(config.cameras.len(), 2);
    let back = &config.cameras[0];
    assert_eq!(back.name, "back");
    assert_eq!(back.width, 704);
    assert_eq!(back.height, 576);
    assert_eq!(back.fps, 5);
    assert_eq!(back.confirm_threshold.to_bits(), 0.7_f64.to_bits());
    let area = back.area.clone().expect("area present");
    assert_eq!(area.name, "Back-Detection-Area");
    assert_eq!(area.pad_pixels, 16);
    assert_eq!(area.polygon, vec![(0.04, 0.43), (0.53, 0.25), (0.85, 0.38)]);
    let front = &config.cameras[1];
    assert_eq!(front.name, "front");
    assert_eq!(front.fps, 2);
    assert_eq!(front.confirm_threshold.to_bits(), 0.7_f64.to_bits());
    assert!(front.area.is_none());
}

#[test]
fn absent_optional_fields_take_their_defaults() {
    let config = Config::from_toml(MINIMAL_AREA).expect("loads");
    assert_eq!(
        config.cameras[0].confirm_threshold.to_bits(),
        DEFAULT_CONFIRM_THRESHOLD.to_bits()
    );
    assert_eq!(
        config.cameras[0].area.clone().expect("area").pad_pixels,
        DEFAULT_PAD_PIXELS
    );
    assert_eq!(DEFAULT_CONFIRM_THRESHOLD.to_bits(), 0.7_f64.to_bits());
    assert_eq!(DEFAULT_PAD_PIXELS, 16);
}

#[test]
fn missing_worker_socket_is_refused() {
    let text = MINIMAL_AREA.replace("worker_socket = \"/s\"\n", "");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(error.to_string(), "worker_socket: missing");
}

#[test]
fn missing_labels_is_refused() {
    let text = MINIMAL_AREA.replace("labels = \"l.txt\"\n", "");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(error.to_string(), "labels: missing");
}

#[test]
fn missing_deadline_ms_is_refused() {
    let text = MINIMAL_AREA.replace("deadline_ms = 100\n", "");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(error.to_string(), "deadline_ms: missing");
}

#[test]
fn missing_cameras_is_refused() {
    let text = "worker_socket = \"/s\"\nlabels = \"l.txt\"\ndeadline_ms = 100\n";
    let error = Config::from_toml(text).expect_err("refused");
    assert_eq!(error.to_string(), "cameras: missing");
}

#[test]
fn missing_camera_name_is_refused() {
    let text = camera_with("width = 640\nheight = 480\nfps = 5\n").replace("name = \"a\"\n", "");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(error.to_string(), "cameras[0].name: missing");
}

#[test]
fn missing_camera_width_is_refused() {
    let text = camera_with("height = 480\nfps = 5\n");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(error.to_string(), "cameras[0].width: missing");
}

#[test]
fn missing_camera_height_is_refused() {
    let text = camera_with("width = 640\nfps = 5\n");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(error.to_string(), "cameras[0].height: missing");
}

#[test]
fn missing_camera_fps_is_refused() {
    let text = camera_with("width = 640\nheight = 480\n");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(error.to_string(), "cameras[0].fps: missing");
}

#[test]
fn missing_area_name_is_refused() {
    let text = MINIMAL_AREA.replace("name = \"zone\"\n", "");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(error.to_string(), "cameras[0].area.name: missing");
}

#[test]
fn missing_area_polygon_is_refused() {
    let text = MINIMAL_AREA.replace("polygon = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]\n", "");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(error.to_string(), "cameras[0].area.polygon: missing");
}

#[test]
fn fps_edges_load_and_values_outside_the_bounds_are_refused() {
    for fps in [MIN_FPS, MAX_FPS] {
        let text = camera_with(&format!("width = 640\nheight = 480\nfps = {fps}\n"));
        let config = Config::from_toml(&text).expect("inside bound");
        assert_eq!(config.cameras[0].fps, fps);
    }
    let text = camera_with("width = 640\nheight = 480\nfps = 0\n");
    let error = Config::from_toml(&text).expect_err("below bound");
    assert_eq!(error.to_string(), "cameras[0].fps: 0 is below 1");
    let text = camera_with("width = 640\nheight = 480\nfps = 61\n");
    let error = Config::from_toml(&text).expect_err("above bound");
    assert_eq!(error.to_string(), "cameras[0].fps: 61 is above 60");
}

#[test]
fn pad_pixels_edges_load_and_values_outside_the_bounds_are_refused() {
    for pad in [0u32, 200] {
        let text = camera_with_area(&format!("pad_pixels = {pad}\n"));
        let config = Config::from_toml(&text).expect("inside bound");
        assert_eq!(
            config.cameras[0].area.clone().expect("area").pad_pixels,
            pad
        );
    }
    let error =
        Config::from_toml(&camera_with_area("pad_pixels = 201\n")).expect_err("above bound");
    assert_eq!(
        error.to_string(),
        "cameras[0].area.pad_pixels: 201 is above 200"
    );
}

#[test]
fn every_other_bound_accepts_edges_and_refuses_outside_values() {
    for edge in [MIN_EDGE, MAX_EDGE] {
        let text = camera_with(&format!("width = {edge}\nheight = 480\nfps = 5\n"));
        let config = Config::from_toml(&text).expect("width edge inside");
        assert_eq!(config.cameras[0].width, edge);
        let text = camera_with(&format!("width = 640\nheight = {edge}\nfps = 5\n"));
        let config = Config::from_toml(&text).expect("height edge inside");
        assert_eq!(config.cameras[0].height, edge);
    }
    for value in [MIN_DEADLINE_MS, MAX_DEADLINE_MS] {
        Config::from_toml(&doc_deadline(value)).expect("deadline edge inside");
    }
    Config::from_toml(&camera_area_polygon("[[0.0, 0.0], [1.0, 0.5], [0.0, 1.0]]"))
        .expect("polygon coordinate edges inside");
    let text = camera_with("width = 640\nheight = 480\nfps = 5\nconfirm_threshold = 1\n");
    let config = Config::from_toml(&text).expect("confirm top edge inside");
    assert_eq!(
        config.cameras[0].confirm_threshold.to_bits(),
        1.0_f64.to_bits()
    );
    let cases: &[(&str, &str)] = &[
        (
            &camera_with("width = 15\nheight = 480\nfps = 5\n"),
            "cameras[0].width: 15 is below 16",
        ),
        (
            &camera_with("width = 8193\nheight = 480\nfps = 5\n"),
            "cameras[0].width: 8193 is above 8192",
        ),
        (
            &camera_with("width = 640\nheight = 15\nfps = 5\n"),
            "cameras[0].height: 15 is below 16",
        ),
        (
            &camera_with("width = 640\nheight = 8193\nfps = 5\n"),
            "cameras[0].height: 8193 is above 8192",
        ),
        (&doc_deadline(0), "deadline_ms: 0 is below 1"),
        (
            &doc_deadline(600_001),
            "deadline_ms: 600001 is above 600000",
        ),
        (
            &camera_with("width = 640\nheight = 480\nfps = 5\nconfirm_threshold = 0\n"),
            "cameras[0].confirm_threshold: 0 is not above 0",
        ),
        (
            &camera_with("width = 640\nheight = 480\nfps = 5\nconfirm_threshold = 1.5\n"),
            "cameras[0].confirm_threshold: 1.5 is above 1",
        ),
        (
            &camera_area_polygon("[[0.0, 0.0], [1.0, 0.0]]"),
            "cameras[0].area.polygon: expected at least 3 points, found 2",
        ),
        (
            &camera_area_polygon("[[0.0, 0.0], [1.5, 0.0], [0.0, 1.0]]"),
            "cameras[0].area.polygon[1]: 1.5 is above 1",
        ),
        (
            &camera_area_polygon("[[0.0, 0.0], [-0.1, 0.0], [0.0, 1.0]]"),
            "cameras[0].area.polygon[1]: -0.1 is below 0",
        ),
        (
            &camera_area_polygon("[[0.0, 0.0], [0.5], [0.0, 1.0]]"),
            "cameras[0].area.polygon[1]: expected 2 numbers, found 1",
        ),
    ];
    for (text, want) in cases {
        let error = Config::from_toml(text).expect_err("refused");
        assert_eq!(error.to_string(), *want);
    }
}

#[test]
fn unknown_key_at_top_level_is_refused() {
    let text = MINIMAL_AREA.replace("deadline_ms = 100", "deadline_ms = 100\nextra = 1");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(error.to_string(), "extra: unknown key");
}

#[test]
fn unknown_key_inside_a_camera_is_refused() {
    let text = MINIMAL_AREA.replace("fps = 5\n", "fps = 5\nextra = 1\n");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(error.to_string(), "cameras[0].extra: unknown key");
}

#[test]
fn unknown_key_inside_an_area_is_refused() {
    let text = MINIMAL_AREA.replace("name = \"zone\"", "name = \"zone\"\nextra = 1");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(error.to_string(), "cameras[0].area.extra: unknown key");
}

#[test]
fn url_key_is_refused_as_unknown() {
    let text = MINIMAL_AREA.replace("fps = 5\n", "fps = 5\nurl = \"rtsp://cam/stream\"\n");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(error.to_string(), "cameras[0].url: unknown key");
}

#[test]
fn duplicate_camera_names_are_refused() {
    let text = TWO_CAMERAS.replace("name = \"front\"", "name = \"back\"");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(
        error.to_string(),
        "cameras[1].name: duplicate camera name \"back\""
    );
}

#[test]
fn camera_name_with_unallowed_character_is_refused() {
    // Each row is the camera's name as TOML basic-string text; \n and \" are TOML
    // escapes, so the parsed name holds a newline or a quote respectively.
    let cases = [
        r#"name = "a\"b""#,
        r#"name = "a\nb""#,
        r#"name = "a b""#,
        r#"name = "é""#,
        r#"name = "a.b""#,
    ];
    for replacement in cases {
        let text = MINIMAL_AREA.replace("name = \"a\"", replacement);
        let shown = Config::from_toml(&text).expect_err("refused").to_string();
        assert_eq!(
            shown,
            "cameras[0].name: name may hold only letters, digits, _ and -"
        );
        assert!(
            !shown.contains('"'),
            "a quote forged into the error: {shown}"
        );
        assert!(
            !shown.contains('\n'),
            "a newline forged into the error: {shown}"
        );
    }
}

#[test]
fn camera_name_with_allowed_characters_loads() {
    for name in ["front", "back_2", "cam-3", "A1"] {
        let text = MINIMAL_AREA.replace("name = \"a\"", &format!("name = \"{name}\""));
        Config::from_toml(&text).unwrap_or_else(|error| panic!("name {name}: {error}"));
    }
}

#[test]
fn zero_cameras_is_refused() {
    let text = "worker_socket = \"/s\"\nlabels = \"l.txt\"\ndeadline_ms = 100\ncameras = []\n";
    let error = Config::from_toml(text).expect_err("refused");
    assert_eq!(
        error.to_string(),
        "cameras: expected at least 1 camera, found 0"
    );
}

#[test]
fn toml_syntax_error_names_the_line_and_column() {
    let error =
        Config::from_toml("worker_socket = \"/s\"\nlabels \"l.txt\"\n").expect_err("refused");
    let text = error.to_string();
    assert!(
        text.contains("toml line 2 column"),
        "no position in: {text}"
    );
}

#[test]
fn wrong_type_is_refused_naming_found_type() {
    let text = camera_with("width = 640\nheight = 480\nfps = \"5\"\n");
    let error = Config::from_toml(&text).expect_err("refused");
    assert_eq!(
        error.to_string(),
        "cameras[0].fps: expected an integer, found a string"
    );
}

#[test]
fn to_value_from_value_returns_an_equal_config() {
    let original = Config::from_toml(TWO_CAMERAS).expect("loads");
    let round = Config::from_value(&original.to_value()).expect("round trip loads");
    assert_eq!(round.worker_socket, original.worker_socket);
    assert_eq!(round.labels, original.labels);
    assert_eq!(round.deadline, original.deadline);
    assert_eq!(round.cameras.len(), original.cameras.len());
    for (kept, source) in round.cameras.iter().zip(&original.cameras) {
        assert_eq!(kept.name, source.name);
        assert_eq!(kept.width, source.width);
        assert_eq!(kept.height, source.height);
        assert_eq!(kept.fps, source.fps);
        assert_eq!(
            kept.confirm_threshold.to_bits(),
            source.confirm_threshold.to_bits()
        );
        match (&kept.area, &source.area) {
            (Some(kept), Some(source)) => {
                assert_eq!(kept.name, source.name);
                assert_eq!(kept.pad_pixels, source.pad_pixels);
                assert_eq!(kept.polygon, source.polygon);
            }
            (None, None) => {}
            _ => panic!("area presence differs on camera {}", kept.name),
        }
    }
}

#[test]
fn three_errors_read_as_path_then_rule() {
    let above = TWO_CAMERAS.replace("fps = 2", "fps = 61");
    let error = Config::from_toml(&above).expect_err("refused");
    assert_eq!(error.to_string(), "cameras[1].fps: 61 is above 60");
    let missing = camera_with("width = 640\nheight = 480\nfps = 5\n").replace("name = \"a\"\n", "");
    let error = Config::from_toml(&missing).expect_err("refused");
    assert_eq!(error.to_string(), "cameras[0].name: missing");
    let url = camera_with("url = \"x\"\nwidth = 640\nheight = 480\nfps = 5\n");
    let error = Config::from_toml(&url).expect_err("refused");
    assert_eq!(error.to_string(), "cameras[0].url: unknown key");
}

fn sorted<'a>(keys: &'a [&'a str]) -> Vec<&'a str> {
    let mut keys = keys.to_vec();
    keys.sort_unstable();
    keys
}

fn schema_names(object: &serde_json::Map<String, serde_json::Value>) -> Vec<&str> {
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys
}

fn schema_strings(list: &serde_json::Value) -> Vec<&str> {
    list.as_array()
        .expect("required list")
        .iter()
        .map(|value| value.as_str().expect("a required key"))
        .collect()
}

#[test]
fn schema_agrees_with_the_loader_bounds_and_lists() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/design/detection-config.schema.json"
    );
    let text = std::fs::read_to_string(path).expect("schema file present");
    let schema: serde_json::Value = serde_json::from_str(&text).expect("schema parses");
    let number =
        |value: &serde_json::Value| value.as_f64().expect("a number in the schema").to_bits();
    assert_eq!(
        schema_names(schema["properties"].as_object().expect("root properties")),
        sorted(CONFIG_KEYS)
    );
    assert_eq!(schema_strings(&schema["required"]), REQUIRED_CONFIG_KEYS);
    assert_eq!(
        number(&schema["properties"]["deadline_ms"]["minimum"]),
        f64::from(MIN_DEADLINE_MS).to_bits()
    );
    assert_eq!(
        number(&schema["properties"]["deadline_ms"]["maximum"]),
        f64::from(MAX_DEADLINE_MS).to_bits()
    );
    assert_eq!(
        number(&schema["properties"]["cameras"]["minItems"]),
        f64::from(u32::try_from(MIN_CAMERAS).expect("fits")).to_bits()
    );
    let camera = &schema["properties"]["cameras"]["items"];
    assert_eq!(
        schema_names(camera["properties"].as_object().expect("camera properties")),
        sorted(CAMERA_KEYS)
    );
    assert_eq!(schema_strings(&camera["required"]), REQUIRED_CAMERA_KEYS);
    for key in ["width", "height"] {
        assert_eq!(
            number(&camera["properties"][key]["minimum"]),
            f64::from(MIN_EDGE).to_bits()
        );
        assert_eq!(
            number(&camera["properties"][key]["maximum"]),
            f64::from(MAX_EDGE).to_bits()
        );
    }
    assert_eq!(
        number(&camera["properties"]["fps"]["minimum"]),
        f64::from(MIN_FPS).to_bits()
    );
    assert_eq!(
        number(&camera["properties"]["fps"]["maximum"]),
        f64::from(MAX_FPS).to_bits()
    );
    assert_eq!(
        number(&camera["properties"]["confirm_threshold"]["exclusiveMinimum"]),
        CONFIRM_MIN_EXCLUSIVE.to_bits()
    );
    assert_eq!(
        number(&camera["properties"]["confirm_threshold"]["maximum"]),
        CONFIRM_MAX.to_bits()
    );
    assert_eq!(
        number(&camera["properties"]["confirm_threshold"]["default"]),
        DEFAULT_CONFIRM_THRESHOLD.to_bits()
    );
    let area = &camera["properties"]["area"];
    assert_eq!(
        schema_names(area["properties"].as_object().expect("area properties")),
        sorted(AREA_KEYS)
    );
    assert_eq!(schema_strings(&area["required"]), REQUIRED_AREA_KEYS);
    assert_eq!(
        number(&area["properties"]["pad_pixels"]["minimum"]),
        f64::from(0).to_bits()
    );
    assert_eq!(
        number(&area["properties"]["pad_pixels"]["maximum"]),
        f64::from(MAX_PAD_PIXELS).to_bits()
    );
    assert_eq!(
        number(&area["properties"]["pad_pixels"]["default"]),
        f64::from(DEFAULT_PAD_PIXELS).to_bits()
    );
    assert_eq!(
        number(&area["properties"]["polygon"]["minItems"]),
        f64::from(u32::try_from(MIN_POLYGON_POINTS).expect("fits")).to_bits()
    );
    let point = &area["properties"]["polygon"]["items"];
    assert_eq!(
        number(&point["minItems"]),
        f64::from(u32::try_from(POINT_COORDINATES).expect("fits")).to_bits()
    );
    assert_eq!(
        number(&point["maxItems"]),
        f64::from(u32::try_from(POINT_COORDINATES).expect("fits")).to_bits()
    );
    assert_eq!(
        number(&point["items"]["minimum"]),
        MIN_POLYGON_COORD.to_bits()
    );
    assert_eq!(
        number(&point["items"]["maximum"]),
        MAX_POLYGON_COORD.to_bits()
    );
}

#[test]
fn schema_agrees_with_the_source_and_stream_keys() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/design/detection-config.schema.json"
    );
    let text = std::fs::read_to_string(path).expect("schema file present");
    let schema: serde_json::Value = serde_json::from_str(&text).expect("schema parses");
    let number =
        |value: &serde_json::Value| value.as_f64().expect("a number in the schema").to_bits();
    let camera = &schema["properties"]["cameras"]["items"];
    let source = &camera["properties"]["source"];
    assert_eq!(
        schema_names(source["properties"].as_object().expect("source properties")),
        sorted(SOURCE_KEYS)
    );
    let stream = &schema["$defs"]["stream"];
    assert_eq!(
        schema_names(stream["properties"].as_object().expect("stream properties")),
        sorted(STREAM_KEYS)
    );
    assert_eq!(schema_strings(&stream["required"]), REQUIRED_STREAM_KEYS);
    let port = &stream["properties"]["port"];
    assert_eq!(
        number(&port["minimum"]),
        f64::from(MIN_STREAM_PORT).to_bits()
    );
    assert_eq!(
        number(&port["maximum"]),
        f64::from(MAX_STREAM_PORT).to_bits()
    );
    assert_eq!(
        number(&port["default"]),
        f64::from(DEFAULT_STREAM_PORT).to_bits()
    );
    assert_eq!(
        stream["properties"]["path"]["default"].as_str(),
        Some(DEFAULT_STREAM_PATH)
    );
}

const TWO_AREAS: &str = r#"[[cameras]]
[cameras.area]
name = "zone"
polygon = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]
[[cameras]]
[cameras.area]
name = "zone2"
polygon = [[0.5, 0.5], [0.8, 0.5], [0.5, 0.8]]
"#;
#[test]
fn conversion_errors_name_the_key_once_in_every_array_of_tables_position() {
    let first = TWO_AREAS.replace(
        "polygon = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]",
        "polygon = [[0.0, 0.0], [nan, 0.0], [0.0, 1.0]]",
    );
    let error = Config::from_toml(&first).expect_err("refused");
    assert_eq!(
        error.to_string(),
        "cameras[0].area.polygon[1][0]: not a finite number"
    );
    let second = TWO_AREAS.replace(
        "polygon = [[0.5, 0.5], [0.8, 0.5], [0.5, 0.8]]",
        "polygon = [[nan, 0.0], [0.8, 0.5], [0.5, 0.8]]",
    );
    let error = Config::from_toml(&second).expect_err("refused");
    assert_eq!(
        error.to_string(),
        "cameras[1].area.polygon[0][0]: not a finite number"
    );
    let error = Config::from_toml("[[extra]]\nwhen = 1979-05-27T07:32:00Z\n").expect_err("refused");
    assert_eq!(
        error.to_string(),
        "extra[0].when: expected a plain value, found datetime"
    );
}

#[test]
fn to_value_saturates_a_deadline_to_u64_max_milliseconds() {
    let huge = Config {
        worker_socket: PathBuf::from("/s"),
        labels: PathBuf::from("l.txt"),
        deadline: Duration::MAX,
        cameras: Vec::new(),
    };
    assert_eq!(huge.to_value()["deadline_ms"].as_u64(), Some(u64::MAX));
}

#[test]
fn syntax_error_after_a_multibyte_character_names_the_line() {
    let error = Config::from_toml("worker_socket = \"/s\"\nhéllo world\n").expect_err("refused");
    let text = error.to_string();
    assert!(
        text.contains("toml line 2 column"),
        "no position in: {text}"
    );
}

fn one_camera(extra_fields: &serde_json::Value) -> serde_json::Value {
    let mut camera = serde_json::json!({"name": "a", "width": 640, "height": 480, "fps": 5});
    if let (Some(camera), Some(extra)) = (camera.as_object_mut(), extra_fields.as_object()) {
        camera.extend(extra.clone());
    }
    serde_json::json!({
        "worker_socket": "/s",
        "labels": "l.txt",
        "deadline_ms": 100,
        "cameras": [camera],
    })
}

#[test]
fn validate_all_lists_three_independent_faults_in_document_order() {
    let value = serde_json::json!({
        "worker_socket": "/s",
        "labels": "l.txt",
        "deadline_ms": 0,
        "cameras": [
            {"name": "a", "width": 640, "height": 480, "fps": 5},
            {"name": "b", "width": 640, "height": 480, "fps": 100},
            {"name": "a", "width": 640, "height": 480, "fps": 5},
        ],
    });
    let violations = Config::validate_all(&value).expect_err("three faults");
    let listed: Vec<(&str, &str)> = violations
        .iter()
        .map(|violation| (violation.path.as_str(), violation.rule.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            ("deadline_ms", "0 is below 1"),
            ("cameras[1].fps", "100 is above 60"),
            ("cameras[2].name", "duplicate camera name \"a\""),
        ]
    );
}

#[test]
fn validate_all_lists_every_bad_field_of_one_camera() {
    let value = one_camera(&serde_json::json!({"width": 15, "height": 8193}));
    let violations = Config::validate_all(&value).expect_err("two faults");
    let listed: Vec<(&str, &str)> = violations
        .iter()
        .map(|violation| (violation.path.as_str(), violation.rule.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            ("cameras[0].width", "15 is below 16"),
            ("cameras[0].height", "8193 is above 8192"),
        ]
    );
}

#[test]
fn validate_all_reports_each_missing_required_key_in_key_order() {
    let value = serde_json::json!({
        "worker_socket": "/s",
        "cameras": [{"name": "a", "height": 480}],
    });
    let violations = Config::validate_all(&value).expect_err("four missing keys");
    let listed: Vec<(&str, &str)> = violations
        .iter()
        .map(|violation| (violation.path.as_str(), violation.rule.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            ("labels", "missing"),
            ("deadline_ms", "missing"),
            ("cameras[0].width", "missing"),
            ("cameras[0].fps", "missing"),
        ]
    );
}

#[test]
fn validate_all_fills_found_and_allowed_for_ranges_and_unknown_keys() {
    let range = Config::validate_all(&one_camera(&serde_json::json!({"fps": 100})))
        .expect_err("fps out of range");
    assert_eq!(range[0].found, Some(serde_json::json!(100)));
    assert_eq!(
        range[0].allowed,
        Some(serde_json::json!({"min": 1, "max": 60}))
    );
    let unknown = Config::validate_all(&one_camera(&serde_json::json!({"extra": 7})))
        .expect_err("unknown key");
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].rule, "unknown key");
    assert_eq!(unknown[0].found, Some(serde_json::json!(7)));
    let names: Vec<serde_json::Value> = CAMERA_KEYS
        .iter()
        .map(|key| serde_json::json!(*key))
        .collect();
    assert_eq!(unknown[0].allowed, Some(serde_json::Value::Array(names)));
    let nested = Config::validate_all(&one_camera(&serde_json::json!({"extra": [7]})))
        .expect_err("unknown array value");
    assert_eq!(nested[0].rule, "unknown key");
    assert_eq!(nested[0].found, None);
    let missing = Config::validate_all(&serde_json::json!({
        "worker_socket": "/s",
        "labels": "l.txt",
        "cameras": [{"name": "a", "width": 640, "height": 480, "fps": 5}],
    }))
    .expect_err("missing deadline");
    assert_eq!(missing[0].rule, "missing");
    assert_eq!(missing[0].found, None);
    assert_eq!(missing[0].allowed, None);
}

#[test]
fn validate_all_reports_a_wrong_type_cameras_as_one_violation() {
    let value = serde_json::json!({
        "worker_socket": "/s",
        "labels": "l.txt",
        "deadline_ms": 100,
        "cameras": "not-an-array",
    });
    let violations = Config::validate_all(&value).expect_err("wrong type");
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].path, "cameras");
    assert_eq!(violations[0].rule, "expected an array, found a string");
}

#[test]
fn from_value_maps_the_first_violation_of_a_multi_fault_document() {
    let value = serde_json::json!({
        "worker_socket": "/s",
        "labels": "l.txt",
        "deadline_ms": 0,
        "cameras": [
            {"name": "a", "width": 640, "height": 480, "fps": 5},
            {"name": "b", "width": 640, "height": 480, "fps": 100},
            {"name": "a", "width": 640, "height": 480, "fps": 5},
        ],
    });
    let error = Config::from_value(&value).expect_err("refused");
    assert_eq!(error.to_string(), "deadline_ms: 0 is below 1");
}

const MAIN_FULL: &str = r#"[cameras.source.main]
host = "cam.example"
port = 554
path = "/live"
username = "op"
secret_env = "CAM_SECRET"
"#;

const SUB_FULL: &str = r#"[cameras.source.sub]
host = "cam-sub.example"
port = 8554
path = "/sub"
username = "sub-op"
secret_env = "CAM_SUB_SECRET"
"#;

const SECRET_CANARY: &str = "p@ss:wor/d 1";

fn camera_source(streams: &str) -> String {
    format!(
        r#"worker_socket = "/s"
labels = "l.txt"
deadline_ms = 100
[[cameras]]
name = "front"
width = 640
height = 480
fps = 5

{streams}"#
    )
}

#[test]
fn source_tables_parse_whole_partial_and_with_defaults() {
    let both = Config::from_toml(&camera_source(&format!("{MAIN_FULL}\n{SUB_FULL}")))
        .expect("a camera with both streams is accepted");
    let source = both.cameras[0].source.as_ref().expect("source parsed");
    let main = source.main.as_ref().expect("main parsed");
    assert_eq!(main.host, "cam.example");
    assert_eq!(main.port, 554);
    assert_eq!(main.path, "/live");
    assert_eq!(main.username, "op");
    assert_eq!(main.secret_env.as_deref(), Some("CAM_SECRET"));
    let sub = source.sub.as_ref().expect("sub parsed");
    assert_eq!(sub.host, "cam-sub.example");
    assert_eq!(sub.port, 8554);
    assert_eq!(sub.secret_env.as_deref(), Some("CAM_SUB_SECRET"));
    let main_only = Config::from_toml(&camera_source(MAIN_FULL)).expect("main only");
    let source = main_only.cameras[0].source.as_ref().expect("source parsed");
    assert!(source.main.is_some() && source.sub.is_none());
    let sub_only = Config::from_toml(&camera_source(SUB_FULL)).expect("sub only");
    let source = sub_only.cameras[0].source.as_ref().expect("source parsed");
    assert!(source.main.is_none() && source.sub.is_some());
    let sparse = Config::from_toml(&camera_source(
        "[cameras.source.main]\nhost = \"cam.example\"\n",
    ))
    .expect("a stream of one key is accepted");
    let main = sparse.cameras[0]
        .source
        .as_ref()
        .expect("source parsed")
        .main
        .as_ref()
        .expect("main parsed");
    assert_eq!(main.port, DEFAULT_STREAM_PORT);
    assert_eq!(main.path, DEFAULT_STREAM_PATH);
    assert_eq!(main.username, "");
    assert_eq!(main.secret_env, None);
}

#[test]
fn source_refusals_name_the_camera_key_and_rule_and_never_a_credential() {
    let with_canary = |key: &str| {
        MAIN_FULL.replace(
            "host = \"cam.example\"",
            &format!("{key} = \"{SECRET_CANARY}\"\nhost = \"cam.example\""),
        )
    };
    let cases = [
        (
            "[cameras.source]\n".to_owned(),
            "camera front: source: expected main or sub",
        ),
        (
            MAIN_FULL.replace("port = 554", "port = 0"),
            "camera front: source main: port: 0 is below 1",
        ),
        (
            MAIN_FULL.replace("port = 554", "port = 65536"),
            "camera front: source main: port: 65536 is above 65535",
        ),
        (
            MAIN_FULL.replace("path = \"/live\"", "path = \"live\""),
            "camera front: source main: path: expected a path starting with /",
        ),
        (
            MAIN_FULL.replace("host = \"cam.example\"", "host = \"cam.example/live\""),
            "camera front: source main: host: expected a host without /",
        ),
        (
            MAIN_FULL.replace("host = \"cam.example\"", "host = \"\""),
            "camera front: source main: host: empty",
        ),
        (
            with_canary("password"),
            "camera front: source main: password: unknown key",
        ),
        (
            with_canary("secret_file"),
            "camera front: source main: secret_file: unknown key",
        ),
    ];
    for (streams, want) in cases {
        let refusal = Config::from_toml(&camera_source(&streams)).expect_err("refused");
        assert_eq!(refusal.to_string(), want);
        assert!(
            !refusal.to_string().contains(SECRET_CANARY),
            "the refusal echoed the canary: {refusal}"
        );
    }
}

#[test]
fn a_source_refusal_without_a_usable_camera_name_names_the_indexed_path() {
    let value = serde_json::json!({
        "worker_socket": "/s",
        "labels": "l.txt",
        "deadline_ms": 100,
        "cameras": [{"width": 640, "height": 480, "fps": 5, "source": {"main": {"host": ""}}}],
    });
    let violations = Config::validate_all(&value).expect_err("refused");
    let listed: Vec<(&str, &str)> = violations
        .iter()
        .map(|violation| (violation.path.as_str(), violation.rule.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            ("cameras[0].name", "missing"),
            ("cameras[0]: source main: host", "empty"),
        ]
    );
}

#[test]
fn the_direct_source_sample_loads_with_placeholders_only() {
    let text = include_str!("fixtures/detection-direct.toml");
    let sample = Config::from_toml(text).expect("sample accepted");
    assert_eq!(sample.cameras.len(), 2);
    let names: Vec<&str> = sample
        .cameras
        .iter()
        .map(|camera| camera.name.as_str())
        .collect();
    assert_eq!(names, ["front", "back"]);
    let front = sample.cameras[0].source.as_ref().expect("front source");
    let main = front.main.as_ref().expect("front main");
    assert_eq!(main.host, "HOST_PLACEHOLDER");
    assert_eq!(main.port, 554);
    assert_eq!(main.path, "/PATH_PLACEHOLDER");
    assert_eq!(main.username, "USERNAME_PLACEHOLDER");
    assert_eq!(main.secret_env.as_deref(), Some("CORVETTE_CAM_FRONT"));
    let back = sample.cameras[1].source.as_ref().expect("back source");
    let sub = back.sub.as_ref().expect("back sub");
    assert_eq!(sub.secret_env.as_deref(), Some("CORVETTE_CAM_BACK"));
    assert!(
        !text.contains(SECRET_CANARY),
        "no credential value belongs in the sample"
    );
}

#[test]
fn a_camera_without_the_enabled_key_defaults_to_enabled() {
    let config =
        Config::from_toml(&camera_with("width = 640\nheight = 480\nfps = 5\n")).expect("loads");
    assert!(config.cameras[0].enabled, "absent means enabled");
}

#[test]
fn a_camera_marked_disabled_in_the_file_starts_disabled() {
    let config = Config::from_toml(&camera_with(
        "width = 640\nheight = 480\nfps = 5\nenabled = false\n",
    ))
    .expect("loads");
    assert!(!config.cameras[0].enabled);
    let value = config.to_value();
    let camera = &value["cameras"].as_array().expect("cameras")[0];
    assert_eq!(
        camera["enabled"],
        serde_json::Value::Bool(false),
        "false stays in the route view"
    );
}

#[test]
fn an_enabled_flag_that_is_not_a_boolean_is_refused() {
    let error = Config::from_toml(&camera_with(
        "width = 640\nheight = 480\nfps = 5\nenabled = \"off\"\n",
    ))
    .expect_err("refused");
    assert!(format!("{error:?}").contains("enabled"), "{error:?}");
}
