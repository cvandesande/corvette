// The fixture is synthetic: no real response has been captured because this machine
// has no cluster access; replace it with a redacted real sample when one exists.
use std::path::PathBuf;
use std::process::Output;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/events_sample.json"
);
const DOC_OPEN: &str = "{\"schema\": 1, \"source\": \"frigate-events\", \"events\": [";
const DOC_END: &str = "]}\n";
const EV_A: &str = "{\"id\": \"1000.5-aaaaaa\", \"camera\": \"front\", \"label\": \"person\", \
    \"start\": 1000.5, \"end\": 1010.5, \"zones\": [\"porch\", \"drive\"], \"top_score\": 0.87, \
    \"score\": 0.81, \"box\": [0.1, 0.2, 0.3, 0.4], \"has_clip\": true}";
const EV_B: &str = "{\"id\": \"1001.0-bbbbbb\", \"camera\": \"back\", \"label\": \"truck\", \
    \"start\": 1001.0, \"end\": null, \"zones\": [\"yard\"], \"top_score\": 0.55, \"score\": 0.5, \
    \"box\": [0.5, 0.5, 0.6, 0.7], \"has_clip\": false}";
const EV_C: &str = "{\"id\": \"1002.0-cccccc\", \"camera\": \"front\", \"label\": \"dog\", \
    \"start\": 1002.0, \"end\": 1005.0, \"zones\": [\"porch\"], \"top_score\": null, \
    \"score\": null, \"box\": null, \"has_clip\": true}";
const EV_D: &str = "{\"id\": \"1003.0-dddddd\", \"camera\": \"back\", \"label\": \"cat\", \
    \"start\": 1003.0, \"end\": 1004.0, \"zones\": [\"yard\"], \"top_score\": null, \
    \"score\": 0.4, \"box\": [0.0, 0.1, 0.2, 0.3], \"has_clip\": true}";
const EV_E: &str = "{\"id\": \"1004.0-eeeeee\", \"camera\": \"front\", \"label\": \"person\", \
    \"start\": 1004.0, \"end\": 1006.0, \"zones\": [], \"top_score\": 0.3, \"score\": 0.3, \
    \"box\": [0.9, 0.9, 1.0, 1.0], \"has_clip\": false}";
const EV_F: &str = "{\"id\": \"1005.0-ffffff\", \"camera\": \"garage\", \"label\": \"person\", \
    \"start\": 1005.0, \"end\": 1007.5, \"zones\": [\"bay\"], \"top_score\": null, \
    \"score\": null, \"box\": null, \"has_clip\": false}";
const EV_Z: &str = "{\"id\": \"1005.0-zzzzzz\", \"camera\": \"back\", \"label\": \"car\", \
    \"start\": 1005.0, \"end\": 1007.0, \"zones\": [\"driveway\"], \"top_score\": 0.61, \
    \"score\": 0.6, \"box\": [0.5, 0.25, 0.75, 0.9], \"has_clip\": true}";
const EV_G: &str = "{\"id\": \"1006.0-gggggg\", \"camera\": \"front\", \"label\": \"père \\\"x\\\"\", \
    \"start\": 1006.0, \"end\": 1008.0, \"zones\": [\"kitchen\"], \"top_score\": 0.25, \
    \"score\": 0.25, \"box\": [0.0, 0.0, 0.0, 0.0], \"has_clip\": true}";
const EMPTY_DOC: &str = "{\"schema\": 1, \"source\": \"frigate-events\", \"events\": []}\n";

fn full_doc() -> String {
    format!("{DOC_OPEN}{EV_A}, {EV_B}, {EV_C}, {EV_D}, {EV_E}, {EV_F}, {EV_Z}, {EV_G}{DOC_END}")
}

fn run(args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_corvette-detect-tools"))
        .args(args)
        .output()
        .expect("the binary runs")
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("the exit code is set")
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "corvette-detect-export-{label}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("a temporary directory");
        Self { path }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.path.join(name);
        std::fs::write(&path, bytes).expect("a test file");
        path.display().to_string()
    }

    fn path_str(&self, name: &str) -> String {
        self.path.join(name).display().to_string()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn export_to(out: &str, extra: &[&str]) -> String {
    let mut args = vec!["export-events", "--in", FIXTURE, "--out", out];
    args.extend_from_slice(extra);
    let output = run(&args);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    std::fs::read_to_string(out).unwrap()
}

#[test]
fn the_whole_fixture_writes_the_answer_key_in_key_and_time_order() {
    let dir = TempDir::new("full");
    let out = dir.path_str("out.json");
    assert_eq!(export_to(&out, &[]), full_doc());
}

#[test]
fn the_camera_filter_keeps_only_the_named_camera() {
    let dir = TempDir::new("camera");
    let out = dir.path_str("out.json");
    let want = format!("{DOC_OPEN}{EV_A}, {EV_C}, {EV_E}, {EV_G}{DOC_END}");
    assert_eq!(export_to(&out, &["--camera", "front"]), want);
}

#[test]
fn the_camera_filter_requires_the_whole_camera_name() {
    let dir = TempDir::new("cameraprefix");
    let out = dir.path_str("out.json");
    assert_eq!(export_to(&out, &["--camera", "fro"]), EMPTY_DOC);
}

#[test]
fn the_label_filter_keeps_only_the_named_label() {
    let dir = TempDir::new("label");
    let out = dir.path_str("out.json");
    let want = format!("{DOC_OPEN}{EV_Z}{DOC_END}");
    assert_eq!(export_to(&out, &["--label", "car"]), want);
}

#[test]
fn the_label_filter_requires_the_exact_case_of_a_label() {
    let dir = TempDir::new("labelcase");
    let out = dir.path_str("out.json");
    assert_eq!(export_to(&out, &["--label", "CAR"]), EMPTY_DOC);
}

#[test]
fn camera_and_label_filters_keep_only_events_matching_both() {
    let dir = TempDir::new("both");
    let out = dir.path_str("out.json");
    let want = format!("{DOC_OPEN}{EV_A}, {EV_E}{DOC_END}");
    assert_eq!(
        export_to(&out, &["--camera", "front", "--label", "person"]),
        want
    );
    let out2 = dir.path_str("out2.json");
    assert_eq!(export_to(&out2, &["--camera", "nosuch-camera"]), EMPTY_DOC);
}

#[test]
fn an_in_progress_event_writes_a_null_end_and_a_finished_event_keeps_its_end() {
    let dir = TempDir::new("inprogress");
    let out = dir.path_str("out.json");
    let text = export_to(&out, &["--camera", "back"]);
    assert!(text.contains("\"end\": null"), "{text}");
    assert!(text.contains("\"end\": 1007.0"), "{text}");
    let out2 = dir.path_str("out2.json");
    assert_eq!(
        export_to(&out2, &["--label", "truck"]),
        format!("{DOC_OPEN}{EV_B}{DOC_END}")
    );
}

#[test]
fn legacy_top_level_score_and_box_columns_are_never_read() {
    let dir = TempDir::new("legacy");
    let out = dir.path_str("out.json");
    let text = export_to(&out, &[]);
    assert!(text.contains("\"top_score\": 0.87"), "{text}");
    assert!(
        !text.contains("0.11"),
        "the legacy top_score leaked: {text}"
    );
    assert!(!text.contains("0.99"), "the legacy score leaked: {text}");
    assert!(
        !text.contains("[0.0, 0.0, 1.0, 1.0]"),
        "the legacy box leaked: {text}"
    );
}

#[test]
fn events_with_equal_starts_are_ordered_by_id_byte_order() {
    let dir = TempDir::new("sort");
    let out = dir.path_str("out.json");
    let text = export_to(&out, &[]);
    let first = text.find("1005.0-ffffff").unwrap();
    let second = text.find("1005.0-zzzzzz").unwrap();
    assert!(first < second, "{text}");
}

#[test]
fn rows_are_ordered_by_start_time_before_id_byte_order() {
    let dir = TempDir::new("sortstart");
    let input = dir.write(
        "starts.json",
        b"[{\"id\": \"1000.5-b\", \"camera\": \"c\", \"label\": \"l\", \
\"start_time\": 1000.5}, {\"id\": \"99.5-a\", \"camera\": \"c\", \"label\": \"l\", \
\"start_time\": 99.5}]",
    );
    let out = dir.path_str("out.json");
    let output = run(&["export-events", "--in", &input, "--out", &out]);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    let text = std::fs::read_to_string(&out).unwrap();
    // The id byte order puts 1000.5-b first, so only a start sort yields this order.
    let first = text.find("99.5-a").unwrap();
    let second = text.find("1000.5-b").unwrap();
    assert!(first < second, "{text}");
}

#[test]
fn a_non_ascii_label_with_a_quote_survives_json_escaping() {
    let dir = TempDir::new("escape");
    let out = dir.path_str("out.json");
    let text = export_to(&out, &[]);
    assert!(text.contains(r#""label": "père \"x\"""#), "{text}");
}

#[test]
fn malformed_input_exits_3_naming_the_event_index_and_key() {
    let dir = TempDir::new("errors");
    let base = "{\"id\": \"a\", \"camera\": \"c\", \"label\": \"l\", \"start_time\": 1";
    let cases: Vec<(String, String)> = vec![
        (
            String::from("{\"a\": 1}"),
            String::from("input is not a JSON array"),
        ),
        (String::from("[5]"), String::from("event 0: id is missing")),
        (
            String::from("[{\"camera\": \"c\", \"label\": \"l\", \"start_time\": 1}]"),
            String::from("event 0: id is missing"),
        ),
        (
            String::from("[{\"id\": \"a\", \"label\": \"l\", \"start_time\": 1}]"),
            String::from("event 0: camera is missing"),
        ),
        (
            String::from("[{\"id\": \"a\", \"camera\": \"c\", \"start_time\": 1}]"),
            String::from("event 0: label is missing"),
        ),
        (
            String::from("[{\"id\": \"a\", \"camera\": \"c\", \"label\": \"l\"}]"),
            String::from("event 0: start_time is missing"),
        ),
        (
            String::from("[{\"id\": 7, \"camera\": \"c\", \"label\": \"l\", \"start_time\": 1}]"),
            String::from("event 0: id is not a string"),
        ),
        (
            String::from("[{\"id\": \"a\", \"camera\": 7, \"label\": \"l\", \"start_time\": 1}]"),
            String::from("event 0: camera is not a string"),
        ),
        (
            String::from("[{\"id\": \"a\", \"camera\": \"c\", \"label\": 7, \"start_time\": 1}]"),
            String::from("event 0: label is not a string"),
        ),
        (
            String::from(
                "[{\"id\": \"a\", \"camera\": \"c\", \"label\": \"l\", \"start_time\": \"x\"}]",
            ),
            String::from("event 0: start_time is not a number"),
        ),
        (
            format!("[{base}, \"end_time\": \"x\"}}]"),
            String::from("event 0: end_time is not a number or null"),
        ),
        (
            format!("[{base}, \"zones\": \"porch\"}}]"),
            String::from("event 0: zones is not an array"),
        ),
        (
            format!("[{base}, \"zones\": [\"porch\", 7]}}]"),
            String::from("event 0: zones is not an array of strings"),
        ),
        (
            format!("[{base}, \"data\": {{\"box\": [1, 2, 3]}}}}]"),
            String::from("event 0: box is not an array of four numbers"),
        ),
        (
            format!("[{base}, \"data\": {{\"box\": 5}}}}]"),
            String::from("event 0: box is not an array of four numbers"),
        ),
        (
            format!("[{base}, \"data\": {{\"box\": [1, 2, 3, \"x\"]}}}}]"),
            String::from("event 0: box is not an array of four numbers"),
        ),
        (
            format!("[{base}}}, {{\"camera\": \"c\"}}]"),
            String::from("event 1: id is missing"),
        ),
    ];
    let out = dir.path_str("out.json");
    for (input, message) in &cases {
        let path = dir.write("bad.json", input.as_bytes());
        let output = run(&["export-events", "--in", &path, "--out", &out]);
        assert_eq!(code(&output), 3, "{input}");
        assert_eq!(
            stderr_text(&output),
            format!("error: {message}\n"),
            "{input}"
        );
    }
    let broken = dir.write("broken.json", b"{\"a\": ");
    let output = run(&["export-events", "--in", &broken, "--out", &out]);
    assert_eq!(code(&output), 3);
    assert!(stderr_text(&output).starts_with("error: input is not valid JSON"));
}

#[test]
fn an_empty_array_writes_an_empty_event_list() {
    let dir = TempDir::new("empty");
    let input = dir.write("empty.json", b"[]");
    let out = dir.path_str("out.json");
    let output = run(&["export-events", "--in", &input, "--out", &out]);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    assert_eq!(std::fs::read_to_string(&out).unwrap(), EMPTY_DOC);
}

#[test]
fn unknown_keys_are_ignored_and_absent_optional_fields_degrade() {
    let dir = TempDir::new("mini");
    let input = dir.write(
        "mini.json",
        b"[{\"id\":\"1\",\"camera\":\"c\",\"label\":\"l\",\"start_time\":7,\"note\":\"x\",\
\"extra\":{\"a\":[1]}}]",
    );
    let out = dir.path_str("out.json");
    let output = run(&["export-events", "--in", &input, "--out", &out]);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    let want = format!(
        "{DOC_OPEN}{{\"id\": \"1\", \"camera\": \"c\", \"label\": \"l\", \"start\": 7.0, \
\"end\": null, \"zones\": [], \"top_score\": null, \"score\": null, \"box\": null, \
\"has_clip\": false}}{DOC_END}"
    );
    assert_eq!(std::fs::read_to_string(&out).unwrap(), want);
}

#[test]
fn the_output_ends_with_exactly_one_newline() {
    let dir = TempDir::new("newline");
    let out = dir.path_str("out.json");
    let _ = export_to(&out, &[]);
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(*bytes.last().unwrap(), b'\n');
    assert_ne!(bytes[bytes.len() - 2], b'\n');
}

#[test]
fn two_runs_write_identical_bytes() {
    let dir = TempDir::new("twice");
    let first = dir.path_str("first.json");
    let second = dir.path_str("second.json");
    assert_eq!(export_to(&first, &[]), export_to(&second, &[]));
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap()
    );
}
