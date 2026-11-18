// The fixtures are hand-written synthetic data; no replay runner exists yet,
// so the record files were written by hand against the replay record file contract.
use std::path::PathBuf;
use std::process::Output;

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/compare");
const HEADER: &str = "{\"schema\": 1, \"camera\": \"cam1\"}";

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
            "corvette-detect-compare-{label}-{}",
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
    fn copy_fixture(&self, name: &str) -> String {
        let text = std::fs::read_to_string(format!("{FIXTURE_DIR}/{name}")).expect("the fixture");
        self.write(name, text.as_bytes())
    }
    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.path.join(name)).expect("the report file")
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

fn key(events: &[String]) -> String {
    format!("{{\"schema\": 1, \"events\": [{}]}}", events.join(", "))
}

fn event(id: &str, start: usize, end: usize) -> String {
    format!(
        "{{\"id\": \"{id}\", \"camera\": \"cam1\", \"label\": \"person\", \"start\": {start}, \"end\": {end}}}"
    )
}

fn record(frames: &[String]) -> String {
    let mut text = format!("{HEADER}\n");
    for frame in frames {
        text.push_str(frame);
        text.push('\n');
    }
    text
}

fn frame(time: usize, objects: &str, ended: &str) -> String {
    format!("{{\"frame_time\": {time}, \"objects\": [{objects}], \"ended\": [{ended}]}}")
}

fn object(id: &str, first_seen: usize) -> String {
    format!(
        "{{\"id\": \"{id}\", \"label\": \"person\", \"confirmed\": true, \"first_seen\": {first_seen}}}"
    )
}

fn keyev(inner: &str) -> String {
    format!("{{\"schema\": 1, \"events\": [{inner}]}}")
}
fn rec(line: &str) -> String {
    format!("{HEADER}\n{line}")
}
fn fobj(inner: &str) -> String {
    rec(&format!("{{\"frame_time\": 1, \"objects\": [{inner}]}}"))
}

fn compare_to(key_path: &str, records: &[&str], out: &str) -> Output {
    let mut args: Vec<&str> = vec!["compare-events", "--events", key_path];
    for record in records {
        args.push("--record");
        args.push(record);
    }
    args.extend_from_slice(&["--out", out]);
    run(&args)
}

#[test]
fn the_full_fixture_report_matches_the_expected_text() {
    let dir = TempDir::new("full");
    let key = dir.copy_fixture("key.json");
    let record_a = dir.copy_fixture("record-a.jsonl");
    let record_b = dir.copy_fixture("record-b.jsonl");
    let out = dir.path_str("report.txt");
    let output = compare_to(&key, &[&record_a, &record_b], &out);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let want = format!(
        "compare-events report\n\
         matching is greedy and deterministic\n\
         events read 3\n\
         tracks read 4\n\
         events matched 2\n\
         events without a track 1\n\
         tracks without an event 2\n\
         events matched 2/3\n\
         tracks matched 2/4\n\
         start offsets count 2 median 10 p95 20 min 0 max 20\n\
         end offsets count 1 median 0 p95 0 min 0 max 0\n\
         zones identical 1 missed 1 extra 1\n\
         unmatched events:\n\
         e3 cam1 car 12 open -\n\
         unmatched tracks:\n\
         {record_a} cam1 person 7 11 20 front frames=11..20\n\
         {record_a} cam1 person 9 26 open - frames=30..30\n"
    );
    assert_eq!(dir.read("report.txt"), want);
}

#[test]
fn rerunning_with_the_same_inputs_writes_identical_bytes() {
    let dir = TempDir::new("rerun");
    let key = dir.copy_fixture("key.json");
    let record_a = dir.copy_fixture("record-a.jsonl");
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key, &[&record_a], &out)), 0);
    let before = std::fs::read(&out).expect("the first report");
    assert_eq!(code(&compare_to(&key, &[&record_a], &out)), 0);
    assert_eq!(before, std::fs::read(&out).expect("the second report"));
}

#[test]
fn a_matching_track_needs_the_same_label_as_the_event() {
    let dir = TempDir::new("label");
    let body = key(&[event("e", 0, 10)]).replace("person", "car");
    let key_path = dir.write("key.json", body.as_bytes());
    let record_path = dir.write(
        "record.jsonl",
        record(&[frame(5, &object("t", 1), "\"t\"")]).as_bytes(),
    );
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    let report = dir.read("report.txt");
    assert!(report.contains("events read 1\ntracks read 1\nevents matched 0\nevents without a track 1\ntracks without an event 1\n"));
    assert!(report.contains(&format!("{record_path} cam1 person t 1 5 - frames=5..5")));
}

#[test]
fn an_empty_key_and_a_header_only_record_report_zeroes_and_na() {
    let dir = TempDir::new("empty");
    let key_path = dir.write("key.json", b"{\"schema\": 1, \"events\": []}");
    let record_path = dir.write("record.jsonl", format!("{HEADER}\n").as_bytes());
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    assert_eq!(
        dir.read("report.txt"),
        "compare-events report\nmatching is greedy and deterministic\nevents read 0\ntracks read 0\nevents matched 0\nevents without a track 0\ntracks without an event 0\nevents matched 0/0\ntracks matched 0/0\nstart offsets count 0 median n/a p95 n/a min n/a max n/a\nend offsets count 0 median n/a p95 n/a min n/a max n/a\nzones identical 0 missed 0 extra 0\nunmatched events:\nunmatched tracks:\n"
    );
}

fn start_offsets_line(count: usize) -> String {
    let dir = TempDir::new(&format!("stats{count}"));
    let mut events: Vec<String> = Vec::new();
    let mut frames: Vec<String> = Vec::new();
    for index in 0..count {
        let base = 100 * index;
        let id = format!("t{index}");
        events.push(event(&format!("e{index}"), base, base + 50));
        frames.push(frame(
            base + 60,
            &object(&id, base + index + 1),
            &format!("\"{id}\""),
        ));
    }
    let key_path = dir.write("key.json", key(&events).as_bytes());
    let record_path = dir.write("record.jsonl", record(&frames).as_bytes());
    let out = dir.path_str("report.txt");
    let output = compare_to(&key_path, &[&record_path], &out);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    dir.read("report.txt")
        .lines()
        .find(|line| line.starts_with("start offsets"))
        .expect("the start offsets line")
        .to_string()
}

#[test]
fn p95_ranks_and_medians_follow_the_nearest_rank_rules() {
    let cases: [(usize, &str); 5] = [
        (1, "start offsets count 1 median 1 p95 1 min 1 max 1"),
        (2, "start offsets count 2 median 1.5 p95 2 min 1 max 2"),
        (3, "start offsets count 3 median 2 p95 3 min 1 max 3"),
        (20, "start offsets count 20 median 10.5 p95 19 min 1 max 20"),
        (21, "start offsets count 21 median 11 p95 20 min 1 max 21"),
    ];
    for (count, want) in cases {
        assert_eq!(start_offsets_line(count), want, "count {count}");
    }
}

#[test]
fn an_equal_tie_goes_to_the_record_passed_first() {
    let dir = TempDir::new("recordtie");
    let key_path = dir.write("key.json", key(&[event("e", 0, 1000)]).as_bytes());
    let first = dir.write(
        "first.jsonl",
        record(&[frame(600, &object("x", 500), "\"x\"")]).as_bytes(),
    );
    let second = dir.write(
        "second.jsonl",
        record(&[frame(600, &object("x", 500), "\"x\"")]).as_bytes(),
    );
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&first, &second], &out)), 0);
    let tied = " cam1 person x 500 600 - frames=600..600";
    assert!(dir.read("report.txt").contains(&format!("{second}{tied}")));
    assert_eq!(code(&compare_to(&key_path, &[&second, &first], &out)), 0);
    assert!(dir.read("report.txt").contains(&format!("{first}{tied}")));
}

#[test]
fn an_equal_tie_goes_to_the_smaller_id_bytes() {
    let dir = TempDir::new("idtie");
    let key_path = dir.write("key.json", key(&[event("e", 0, 1000)]).as_bytes());
    let both = format!("{}, {}", object("m", 500), object("a", 500));
    let frame_text = frame(600, &both, "\"m\", \"a\"");
    let record_path = dir.write("record.jsonl", record(&[frame_text]).as_bytes());
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    let report = dir.read("report.txt");
    assert!(report.contains("cam1 person m 500 600 - frames=600..600"));
    assert!(!report.contains("cam1 person a 500 600 - frames=600..600"));
}

#[test]
fn an_ended_id_never_seen_in_objects_is_ignored() {
    let dir = TempDir::new("ghostended");
    let key_path = dir.write("key.json", key(&[event("e", 0, 10)]).as_bytes());
    let frame_text = format!(
        "{{\"frame_time\": 5, \"objects\": [{}], \"ended\": [\"ghost\"]}}",
        object("t", 1)
    );
    let record_path = dir.write("record.jsonl", record(&[frame_text]).as_bytes());
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    assert!(
        dir.read("report.txt")
            .contains("tracks read 1\nevents matched 1\n")
    );
}

#[test]
fn a_missing_or_duplicated_flag_is_a_usage_error() {
    let dir = TempDir::new("usage");
    let key = dir.copy_fixture("key.json");
    let record = dir.copy_fixture("record-a.jsonl");
    let out = dir.path_str("report.txt");
    let cases: Vec<Vec<&str>> = vec![
        vec!["compare-events"],
        vec!["compare-events", "--events", &key, "--out", &out],
        vec!["compare-events", "--record", &record, "--out", &out],
        vec!["compare-events", "--events", &key, "--record", &record],
        vec![
            "compare-events",
            "--events",
            &key,
            "--events",
            &key,
            "--record",
            &record,
            "--out",
            &out,
        ],
        vec![
            "compare-events",
            "--bogus",
            "x",
            "--events",
            &key,
            "--record",
            &record,
            "--out",
            &out,
        ],
        vec![
            "compare-events",
            "--events",
            &key,
            "--record",
            &record,
            "--record",
            "--out",
            &out,
        ],
    ];
    for case in cases {
        let output = run(&case);
        assert_eq!(code(&output), 2, "{case:?}: {}", stderr_text(&output));
        assert!(stderr_text(&output).starts_with("usage: corvette-detect-tools"));
    }
}

#[test]
fn malformed_inputs_exit_three_naming_the_path_or_index() {
    let dir = TempDir::new("errors");
    let out = dir.path_str("report.txt");
    let key_ok = dir.write("kkey.json", key(&[event("e", 0, 10)]).as_bytes());
    let record_ok = dir.write(
        "krecord.jsonl",
        record(&[frame(600, &object("t", 500), "\"t\"")]).as_bytes(),
    );
    let cases: [(&str, &str); 23] = [
        ("k0", "nope"),
        ("k1", "{\"schema\": 2, \"events\": []}"),
        ("k2", "[]"),
        ("k3", "{\"schema\": 1}"),
        (
            "k4",
            &keyev("{\"camera\": \"c\", \"label\": \"l\", \"start\": 1}"),
        ),
        (
            "k5",
            &keyev("{\"id\":\"a\",\"camera\":\"c\",\"label\":\"l\",\"start\":\"x\"}"),
        ),
        (
            "k6",
            &keyev("{\"id\":\"a\",\"camera\":\"c\",\"label\":\"l\",\"start\":1,\"end\":\"x\"}"),
        ),
        (
            "k7",
            &keyev("{\"id\":\"a\",\"camera\":\"c\",\"label\":\"l\",\"start\":1,\"zones\":5}"),
        ),
        ("r0", ""),
        ("r1", "{\"schema\": 2, \"camera\": \"cam1\"}\n"),
        ("r2", "{\"schema\": 1}"),
        ("r3", &rec("nope")),
        ("r4", &rec("[1]")),
        ("r5", &rec("{\"objects\": []}")),
        ("r6", &rec("{\"frame_time\": 1, \"objects\": {}}")),
        ("r7", &rec("{\"frame_time\": 1, \"ended\": \"x\"}")),
        ("r8", &rec("{\"frame_time\": 1, \"ended\": [5]}")),
        (
            "r9",
            &fobj("{\"confirmed\":true,\"label\":\"p\",\"first_seen\":1}"),
        ),
        (
            "r10",
            &fobj("{\"id\":\"a\",\"confirmed\":true,\"first_seen\":1}"),
        ),
        (
            "r11",
            &fobj("{\"id\":\"a\",\"label\":\"p\",\"confirmed\":true}"),
        ),
        (
            "r12",
            &fobj("{\"id\":\"a\",\"label\":\"p\",\"confirmed\":true,\"first_seen\":\"x\"}"),
        ),
        (
            "r13",
            &fobj("{\"id\":\"a\",\"label\":\"p\",\"first_seen\":1}"),
        ),
        (
            "r14",
            &fobj("{\"id\":\"a\",\"label\":\"p\",\"confirmed\":\"yes\",\"first_seen\":1}"),
        ),
    ];
    let messages: [&str; 23] = [
        "{P}: key is not valid JSON",
        "{P}: schema is not 1",
        "{P}: schema is not 1",
        "{P}: events is not an array",
        "event 0: id is missing",
        "event 0: start is not a number",
        "event 0: end is not a number or null",
        "event 0: zones are not an array",
        "{P}: line 1: no header line",
        "{P}: line 1: schema is not 1",
        "{P}: line 1: camera is missing",
        "{P}: line 2: frame is not valid JSON",
        "{P}: line 2: frame is not a JSON object",
        "{P}: line 2: frame_time is missing",
        "{P}: line 2: objects is not an array",
        "{P}: line 2: ended is not an array",
        "{P}: line 2: ended is not an array of strings",
        "{P}: line 2: object 0: id is missing",
        "{P}: line 2: object 0: label is missing",
        "{P}: line 2: object 0: first_seen is missing",
        "{P}: line 2: object 0: first_seen is not a number",
        "{P}: line 2: object 0: confirmed is missing",
        "{P}: line 2: object 0: confirmed is not a boolean",
    ];
    for ((index, (label, body)), message) in cases.iter().enumerate().zip(messages) {
        let path = dir.write(label, body.as_bytes());
        let (key_arg, record_arg) = if label.starts_with('k') {
            (path.as_str(), record_ok.as_str())
        } else {
            (key_ok.as_str(), path.as_str())
        };
        let output = compare_to(key_arg, &[record_arg], &out);
        assert_eq!(code(&output), 3, "{label}: {}", stderr_text(&output));
        let want = format!("error: {}\n", message.replace("{P}", &path));
        assert_eq!(stderr_text(&output), want, "case {index} {label}");
    }
}

#[test]
fn an_unreadable_input_file_is_an_input_error() {
    let dir = TempDir::new("unreadable");
    let key = dir.copy_fixture("key.json");
    let missing_key = dir.path_str("missing.json");
    let missing_record = dir.path_str("missing.jsonl");
    let out = dir.path_str("report.txt");
    let output = compare_to(&missing_key, &[&key], &out);
    assert_eq!(code(&output), 3);
    assert!(stderr_text(&output).starts_with(&format!("error: cannot read {missing_key}")));
    let output = compare_to(&key, &[&missing_record], &out);
    assert_eq!(code(&output), 3);
    assert!(stderr_text(&output).starts_with(&format!("error: cannot read {missing_record}")));
}

#[test]
fn zone_agreement_counts_each_matched_pair_once_in_each_direction() {
    let dir = TempDir::new("zonestally");
    let kev = |id: &str, start: usize, end: usize, zones: &str| {
        format!(
            "{{\"id\": \"{id}\", \"camera\": \"cam1\", \"label\": \"person\", \"start\": {start}, \"end\": {end}, \"zones\": [{zones}]}}"
        )
    };
    let tobj = |id: &str, first: usize, zones: &str| {
        format!(
            "{{\"id\": \"{id}\", \"label\": \"person\", \"confirmed\": true, \"first_seen\": {first}, \"zones_entered\": [{zones}]}}"
        )
    };
    let events = [
        kev("e1", 0, 20, "\"a\""),
        kev("e2", 30, 50, "\"b\""),
        kev("e3", 60, 80, ""),
    ];
    let key_path = dir.write("key.json", key(&events).as_bytes());
    let frames = [
        frame(15, &tobj("t1", 10, ""), "\"t1\""),
        frame(45, &tobj("t2", 40, ""), "\"t2\""),
        frame(75, &tobj("t3", 70, "\"c\""), "\"t3\""),
    ];
    let record_path = dir.write("record.jsonl", record(&frames).as_bytes());
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    assert!(
        dir.read("report.txt")
            .contains("zones identical 0 missed 2 extra 1")
    );
}

#[test]
fn a_pair_with_two_lacking_zones_counts_once_as_missed() {
    let dir = TempDir::new("pairzones");
    let key_json = "{\"schema\": 1, \"events\": [{\"id\": \"e\", \"camera\": \"cam1\", \"label\": \"person\", \"start\": 0, \"end\": 100, \"zones\": [\"a\", \"b\"]}]}";
    let key_path = dir.write("key.json", key_json.as_bytes());
    let tobj = "{\"id\": \"t\", \"label\": \"person\", \"confirmed\": true, \"first_seen\": 1, \"zones_entered\": []}";
    let record_path = dir.write(
        "record.jsonl",
        record(&[frame(50, tobj, "\"t\"")]).as_bytes(),
    );
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    assert!(
        dir.read("report.txt")
            .contains("zones identical 0 missed 1 extra 0")
    );
}

#[test]
fn a_track_only_matches_an_event_on_the_same_camera() {
    let dir = TempDir::new("camera");
    let key_path = dir.write("key.json", key(&[event("e", 0, 10)]).as_bytes());
    let header = "{\"schema\": 1, \"camera\": \"cam2\"}";
    let frame_text = frame(5, &object("t", 1), "\"t\"");
    let record_path = dir.write(
        "record.jsonl",
        format!("{header}\n{frame_text}\n").as_bytes(),
    );
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    assert!(
        dir.read("report.txt")
            .contains("events read 1\ntracks read 1\nevents matched 0\nevents without a track 1\ntracks without an event 1\n")
    );
}

#[test]
fn an_event_ending_exactly_at_a_track_start_does_not_match() {
    let dir = TempDir::new("touching");
    let key_path = dir.write("key.json", key(&[event("e", 0, 10)]).as_bytes());
    let record_path = dir.write(
        "record.jsonl",
        record(&[frame(20, &object("t", 10), "\"t\"")]).as_bytes(),
    );
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    assert!(
        dir.read("report.txt")
            .contains("events read 1\ntracks read 1\nevents matched 0\nevents without a track 1\ntracks without an event 1\n")
    );
}

#[test]
fn one_track_is_taken_by_the_first_event_only() {
    let dir = TempDir::new("secondclaim");
    let events = [event("e1", 0, 100), event("e2", 40, 80)];
    let key_path = dir.write("key.json", key(&events).as_bytes());
    let record_path = dir.write(
        "record.jsonl",
        record(&[frame(100, &object("t", 0), "\"t\"")]).as_bytes(),
    );
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    let report = dir.read("report.txt");
    assert!(report.contains("events read 2\ntracks read 1\nevents matched 1\nevents without a track 1\ntracks without an event 0\n"));
    assert!(report.contains("unmatched events:\ne2 cam1 person 40 80 -\n"));
}

#[test]
fn an_open_track_end_is_excluded_from_end_offsets() {
    let dir = TempDir::new("openend");
    let key_path = dir.write("key.json", key(&[event("e", 0, 10)]).as_bytes());
    let record_path = dir.write(
        "record.jsonl",
        record(&[frame(5, &object("t", 1), "")]).as_bytes(),
    );
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    let report = dir.read("report.txt");
    assert!(report.contains("events matched 1\n"));
    assert!(report.contains("start offsets count 1 median 1 p95 1 min 1 max 1\nend offsets count 0 median n/a p95 n/a min n/a max n/a\n"));
}

#[test]
fn a_track_label_and_zones_come_from_its_last_frame() {
    let dir = TempDir::new("lastframe");
    let key_json = "{\"schema\": 1, \"events\": [{\"id\": \"e\", \"camera\": \"cam1\", \"label\": \"person\", \"start\": 0, \"end\": 100, \"zones\": [\"a\"]}]}";
    let key_path = dir.write("key.json", key_json.as_bytes());
    let first = "{\"id\": \"t\", \"label\": \"car\", \"confirmed\": true, \"first_seen\": 1, \"zones_entered\": []}";
    let last = "{\"id\": \"t\", \"label\": \"person\", \"confirmed\": true, \"first_seen\": 1, \"zones_entered\": [\"a\"]}";
    let frames = [frame(5, first, ""), frame(6, last, "")];
    let record_path = dir.write("record.jsonl", record(&frames).as_bytes());
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    let report = dir.read("report.txt");
    assert!(report.contains("events read 1\ntracks read 1\nevents matched 1\nevents without a track 0\ntracks without an event 0\n"));
    assert!(report.contains("zones identical 1 missed 0 extra 0"));
}

#[test]
fn an_unconfirmed_object_creates_no_track() {
    let dir = TempDir::new("unconfirmed");
    let key_path = dir.write("key.json", key(&[event("e", 0, 10)]).as_bytes());
    let unconfirmed =
        "{\"id\": \"t\", \"label\": \"person\", \"confirmed\": false, \"first_seen\": 1}";
    let record_path = dir.write(
        "record.jsonl",
        record(&[frame(5, unconfirmed, "")]).as_bytes(),
    );
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    assert!(
        dir.read("report.txt")
            .contains("events read 1\ntracks read 0\nevents matched 0\nevents without a track 1\ntracks without an event 0\n")
    );
}

#[test]
fn a_track_starts_at_its_first_confirmed_frame() {
    let dir = TempDir::new("confirmstart");
    let key_path = dir.write("key.json", key(&[]).as_bytes());
    let early = "{\"id\": \"t\", \"label\": \"person\", \"confirmed\": false, \"first_seen\": 2}";
    let later = "{\"id\": \"t\", \"label\": \"person\", \"confirmed\": true, \"first_seen\": 2}";
    let frames = [
        frame(3, early, ""),
        frame(7, later, ""),
        frame(9, early, ""),
    ];
    let record_path = dir.write("record.jsonl", record(&frames).as_bytes());
    let out = dir.path_str("report.txt");
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    let report = dir.read("report.txt");
    assert!(report.contains("tracks read 1\nevents matched 0\n"));
    assert!(report.contains(&format!(
        "{record_path} cam1 person t 2 open - frames=7..9\n"
    )));
}

fn zoned_object(id: &str, first: usize, zones: &str) -> String {
    format!(
        "{{\"id\": \"{id}\", \"label\": \"person\", \"confirmed\": true, \"first_seen\": {first}, \"zones_entered\": [{zones}]}}"
    )
}

fn compare_with_filters(key_path: &str, record: &str, out: &str, filter: &[&str]) -> Output {
    let mut args: Vec<&str> = vec!["compare-events", "--events", key_path, "--record", record];
    args.extend_from_slice(filter);
    args.extend_from_slice(&["--out", out]);
    run(&args)
}

#[test]
fn zoned_only_drops_tracks_that_never_entered_a_zone() {
    let dir = TempDir::new("zonedonly");
    let key_path = dir.write(
        "key.json",
        key(&[event("e1", 0, 100), event("e2", 200, 300)]).as_bytes(),
    );
    let frames = [
        frame(10, &zoned_object("t1", 0, ""), ""),
        frame(210, &zoned_object("t2", 200, "\"a\""), ""),
    ];
    let record_path = dir.write("record.jsonl", record(&frames).as_bytes());
    let out = dir.path_str("report.txt");
    let output = compare_with_filters(&key_path, &record_path, &out, &["--zoned-only", "yes"]);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    let report = dir.read("report.txt");
    assert!(
        report.contains(
            "tracks read 2\ntracks kept 1\nevents matched 1\nevents without a track 1\ntracks without an event 0\nevents matched 1/2\ntracks matched 1/1\n"
        ),
        "{report}"
    );
    assert!(report.contains("unmatched events:\ne1 cam1 person 0 100 -\n"));
    assert_eq!(code(&compare_to(&key_path, &[&record_path], &out)), 0);
    let unfiltered = dir.read("report.txt");
    assert!(unfiltered.contains("tracks read 2\nevents matched 2\n"));
    assert!(!unfiltered.contains("tracks kept"));
}

#[test]
fn min_seconds_keeps_tracks_whose_span_is_at_least_the_value() {
    let dir = TempDir::new("minsec");
    let key_path = dir.write("key.json", key(&[]).as_bytes());
    let frames = [
        frame(5, &object("s1", 5), ""),
        frame(6, &object("s1", 5), ""),
        frame(10, &object("s2", 10), ""),
        frame(13, &object("s2", 10), ""),
        frame(20, &object("s3", 20), ""),
        frame(22, &object("s3", 20), ""),
    ];
    let record_path = dir.write("record.jsonl", record(&frames).as_bytes());
    let out = dir.path_str("report.txt");
    let output = compare_with_filters(&key_path, &record_path, &out, &["--min-seconds", "2"]);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    let report = dir.read("report.txt");
    assert!(
        report.contains(
            "tracks read 3\ntracks kept 2\nevents matched 0\nevents without a track 0\ntracks without an event 2\n"
        ),
        "{report}"
    );
    assert!(report.contains(&format!("{record_path} cam1 person s2 10")));
    assert!(report.contains(&format!("{record_path} cam1 person s3 20")));
    assert!(!report.contains("cam1 person s1"));
}

#[test]
fn invalid_filter_flag_values_are_usage_errors() {
    let dir = TempDir::new("badfilter");
    let key_path = dir.write("key.json", key(&[]).as_bytes());
    let record_path = dir.write(
        "record.jsonl",
        record(&[frame(5, &object("t", 1), "")]).as_bytes(),
    );
    let out = dir.path_str("report.txt");
    let cases: [(&str, &str); 4] = [
        ("--min-seconds", "-1"),
        ("--min-seconds", "abc"),
        ("--min-seconds", "NaN"),
        ("--zoned-only", "maybe"),
    ];
    for (flag, value) in cases {
        let output = compare_with_filters(&key_path, &record_path, &out, &[flag, value]);
        assert_eq!(code(&output), 2, "{flag} {value}: {}", stderr_text(&output));
    }
}

#[test]
fn zone_and_duration_filters_apply_together() {
    let dir = TempDir::new("bothfilters");
    let key_path = dir.write("key.json", key(&[]).as_bytes());
    let frames = [
        frame(0, &zoned_object("zl", 0, "\"a\""), ""),
        frame(10, &zoned_object("zl", 0, "\"a\""), ""),
        frame(20, &zoned_object("zs", 20, "\"a\""), ""),
        frame(21, &zoned_object("zs", 20, "\"a\""), ""),
        frame(30, &zoned_object("nl", 30, ""), ""),
        frame(40, &zoned_object("nl", 30, ""), ""),
        frame(50, &zoned_object("ns", 50, ""), ""),
        frame(51, &zoned_object("ns", 50, ""), ""),
    ];
    let record_path = dir.write("record.jsonl", record(&frames).as_bytes());
    let out = dir.path_str("report.txt");
    let output = compare_with_filters(
        &key_path,
        &record_path,
        &out,
        &["--zoned-only", "yes", "--min-seconds", "5"],
    );
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    let report = dir.read("report.txt");
    assert!(
        report.contains(
            "tracks read 4\ntracks kept 1\nevents matched 0\nevents without a track 0\ntracks without an event 1\n"
        ),
        "{report}"
    );
    assert!(report.contains(&format!("{record_path} cam1 person zl 0")));
    for dropped in ["zs", "nl", "ns"] {
        assert!(!report.contains(&format!("person {dropped}")));
    }
}

#[test]
fn min_seconds_measures_the_confirmed_span_not_the_first_seen() {
    let dir = TempDir::new("confirmedspan");
    let key_path = dir.write("key.json", key(&[]).as_bytes());
    let unconfirmed =
        "{\"id\": \"t\", \"label\": \"person\", \"confirmed\": false, \"first_seen\": 2}";
    let confirmed =
        "{\"id\": \"t\", \"label\": \"person\", \"confirmed\": true, \"first_seen\": 2}";
    let frames = [
        frame(3, unconfirmed, ""),
        frame(7, confirmed, ""),
        frame(9, unconfirmed, ""),
    ];
    let record_path = dir.write("record.jsonl", record(&frames).as_bytes());
    let out = dir.path_str("report.txt");
    let output = compare_with_filters(&key_path, &record_path, &out, &["--min-seconds", "3"]);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    let dropped = dir.read("report.txt");
    assert!(
        dropped.contains(
            "tracks read 1\ntracks kept 0\nevents matched 0\nevents without a track 0\ntracks without an event 0\n"
        ),
        "{dropped}"
    );
    assert!(!dropped.contains("cam1 person t "));
    let output = compare_with_filters(&key_path, &record_path, &out, &["--min-seconds", "2"]);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    let kept = dir.read("report.txt");
    assert!(
        kept.contains(
            "tracks read 1\ntracks kept 1\nevents matched 0\nevents without a track 0\ntracks without an event 1\n"
        ),
        "{kept}"
    );
    assert!(kept.contains(&format!(
        "{record_path} cam1 person t 2 open - frames=7..9\n"
    )));
}
