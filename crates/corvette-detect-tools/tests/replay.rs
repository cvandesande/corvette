use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::thread;

use corvette_detect_proto::{HEADER_LEN, Reply, Request, Row, parse_header};

fn run(args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_corvette-detect-tools"))
        .args(args)
        .output()
        .expect("the binary runs")
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("the exit code is set")
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn path_str(path: &Path) -> String {
    path.display().to_string()
}
fn with<'a>(mut base: Vec<&'a str>, extra: &[&'a str]) -> Vec<&'a str> {
    base.extend_from_slice(extra);
    base
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("corvette-replay-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&path).expect("a temporary directory");
        Self { path }
    }
    fn write(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.path.join(name);
        std::fs::write(&path, bytes).expect("a test file");
        path_str(&path)
    }
    fn path_str(&self, name: &str) -> String {
        path_str(&self.path.join(name))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn make_segment(dir: &Path) -> PathBuf {
    let path = dir.join("cam_main-20261101T120000Z.ts");
    let output = std::process::Command::new("ffmpeg")
        .args(
            "-nostdin -loglevel error -y -f lavfi -i testsrc=size=704x576:rate=5 \
             -t 3 -c:v mpeg2video"
                .split_whitespace(),
        )
        .arg(&path)
        .output()
        .expect("ffmpeg is installed; these tests must fail, not skip, without it");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "segment gen failed: {stderr}");
    path
}

const fn person_row() -> Row {
    Row {
        class_id: 0,
        score: 0.9,
        y_min: 0.25,
        x_min: 0.25,
        y_max: 0.75,
        x_max: 0.75,
    }
}

fn read_request_id(stream: &mut UnixStream) -> Option<u64> {
    let mut header = [0u8; HEADER_LEN];
    if stream.read_exact(&mut header).is_err() {
        return None;
    }
    let parsed = parse_header(&header).expect("request header parses");
    let mut frame = vec![0u8; HEADER_LEN + parsed.body_len];
    frame[..HEADER_LEN].copy_from_slice(&header);
    if stream.read_exact(&mut frame[HEADER_LEN..]).is_err() {
        return None;
    }
    let Ok(Request::InferRequest { id, .. }) = Request::decode(&frame) else {
        return None;
    };
    Some(id)
}

fn serve_worker(listener: &UnixListener) {
    let (mut stream, _) = listener.accept().expect("worker accept");
    while let Some(id) = read_request_id(&mut stream) {
        let rows = vec![person_row()];
        let bytes = Reply::Rows { id, rows }.encode().expect("reply encodes");
        stream.write_all(&bytes).expect("reply write");
    }
}

fn replay_run(socket: &str, labels: &str, segment: &str, out: &str, extra: &[&str]) -> Output {
    let mut args: Vec<&str> = vec![
        "replay", "--socket", socket, "--labels", labels, "--in", segment, "--out", out,
    ];
    args.extend_from_slice(extra);
    run(&args)
}

fn replay_with_worker(
    dir: &TempDir,
    tag: &str,
    segment: &Path,
    extra: &[&str],
) -> (Output, Vec<u8>) {
    let socket = dir.path.join(format!("{tag}.sock"));
    let listener = UnixListener::bind(&socket).expect("the fake worker listens");
    let labels = dir.write("labels.txt", b"0 person\n");
    let out = dir.path_str(&format!("{tag}.jsonl"));
    let output = thread::scope(|scope| {
        let server = scope.spawn(|| serve_worker(&listener));
        let output = replay_run(&path_str(&socket), &labels, &path_str(segment), &out, extra);
        server.join().expect("the fake worker joins");
        output
    });
    let bytes = fs::read(&out).expect("the record file was written");
    (output, bytes)
}

fn record_lines(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8(bytes.to_vec()).expect("the record is utf-8");
    text.lines().map(str::to_owned).collect()
}

fn first_object_line(frame_lines: &[String]) -> usize {
    frame_lines
        .iter()
        .position(|line| line.contains("\"id\":"))
        .expect("a frame line lists the object")
}

#[test]
fn a_default_run_writes_a_header_and_fifteen_frame_lines() {
    let dir = TempDir::new("defaults");
    let segment = make_segment(&dir.path);
    fs::write(dir.path.join("run1.jsonl"), b"stale bytes to replace").expect("a stale out file");
    let (output, bytes) = replay_with_worker(&dir, "run1", &segment, &[]);
    assert!(!bytes.starts_with(b"stale bytes"), "stale out replaced");
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    assert!(stdout_text(&output).is_empty());
    let lines = record_lines(&bytes);
    assert_eq!(lines.len(), 16, "one header line plus one line per frame");
    assert!(
        lines[0].contains("\"camera\":\"cam_main\""),
        "camera: {}",
        lines[0]
    );
    assert!(lines[0].contains("\"frame_width\":704"), "width");
    assert!(lines[0].contains("\"frame_height\":576"), "height");
    assert!(lines[0].contains("\"detect_fps\":5.0"), "fps");
    assert!(
        lines[1..]
            .iter()
            .all(|line| line.starts_with("{\"frame_time\":"))
    );
}

#[test]
fn the_camera_flag_replaces_the_name_derived_camera() {
    let dir = TempDir::new("cameraflag");
    let segment = make_segment(&dir.path);
    let (output, bytes) = replay_with_worker(&dir, "run1", &segment, &["--camera", "other"]);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    let lines = record_lines(&bytes);
    assert!(
        lines[0].contains("\"camera\":\"other\""),
        "header: {}",
        lines[0]
    );
}

#[test]
fn two_fresh_workers_over_the_same_segment_write_identical_records() {
    let dir = TempDir::new("twice");
    let segment = make_segment(&dir.path);
    let (first_output, first) = replay_with_worker(&dir, "first", &segment, &[]);
    let (second_output, second) = replay_with_worker(&dir, "second", &segment, &[]);
    assert_eq!(code(&first_output), 0, "{}", stderr_text(&first_output));
    assert_eq!(code(&second_output), 0, "{}", stderr_text(&second_output));
    assert!(!first.is_empty());
    assert_eq!(first, second);
}

#[test]
fn the_first_object_line_follows_the_fps_flag_through_the_tracker() {
    let dir = TempDir::new("fps");
    let segment = make_segment(&dir.path);
    let (output, bytes) = replay_with_worker(&dir, "five", &segment, &[]);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    let lines = record_lines(&bytes);
    assert_eq!(lines.len(), 16, "fifteen frames at five fps");
    assert_eq!(first_object_line(&lines[1..]), 2, "five fps pins index two");
    let (output, bytes) = replay_with_worker(&dir, "ten", &segment, &["--fps", "10"]);
    assert_eq!(code(&output), 0, "{}", stderr_text(&output));
    let lines = record_lines(&bytes);
    assert_eq!(lines.len(), 31, "thirty frames at ten fps");
    assert_eq!(first_object_line(&lines[1..]), 5, "ten fps pins index five");
}

#[test]
fn a_socket_without_a_listener_exits_three_leaving_an_empty_record() {
    let dir = TempDir::new("nolistener");
    let segment = make_segment(&dir.path);
    let in_text = path_str(&segment);
    let labels = dir.write("labels.txt", b"0 person\n");
    let nobody = dir.path_str("nobody.sock");
    let out = dir.path_str("record.jsonl");
    let output = replay_run(&nobody, &labels, &in_text, &out, &[]);
    assert_eq!(code(&output), 3, "{}", stderr_text(&output));
    assert!(stderr_text(&output).starts_with("error:"));
    assert!(stdout_text(&output).is_empty());
    assert_eq!(fs::metadata(&out).expect("the record exists").len(), 0);
}

#[test]
fn a_missing_input_file_exits_three() {
    let dir = TempDir::new("missingin");
    let segment = dir.path_str("cam_gone-20261101T120000Z.ts");
    let labels = dir.write("labels.txt", b"0 person\n");
    let nobody = dir.path_str("nobody.sock");
    let out = dir.path_str("record.jsonl");
    let output = replay_run(&nobody, &labels, &segment, &out, &[]);
    assert_eq!(code(&output), 3, "{}", stderr_text(&output));
    assert!(stderr_text(&output).starts_with("error:"));
}

#[test]
fn every_path_spelling_of_the_input_is_refused_as_the_record_target() {
    for spelling in ["same", "symlink", "hardlink"] {
        let dir = TempDir::new(&format!("selfout-{spelling}"));
        let input = dir.path.join("cam_self-20261101T120000Z.ts");
        fs::write(&input, b"recorded bytes").expect("an input file");
        let link = dir.path.join(format!("{spelling}.ts"));
        match spelling {
            "same" => {}
            "symlink" => std::os::unix::fs::symlink(&input, &link).expect("a symlink"),
            _ => fs::hard_link(&input, &link).expect("a hard link"),
        }
        let out_text = path_str(match spelling {
            "same" => &input,
            _ => &link,
        });
        let labels = dir.write("labels.txt", b"0 person\n");
        let nobody = dir.path_str("nobody.sock");
        let output = replay_run(&nobody, &labels, &path_str(&input), &out_text, &[]);
        let err = stderr_text(&output);
        assert_eq!(code(&output), 3, "{spelling}: {err}");
        assert!(err.contains("is the input file"), "{spelling}: {err}");
        assert_eq!(fs::read(&input).expect("after"), b"recorded bytes");
    }
}

#[test]
fn a_missing_or_unknown_flag_or_bad_value_exits_two() {
    let dir = TempDir::new("usage");
    let labels = dir.write("labels.txt", b"0 person\n");
    let socket = dir.path_str("nobody.sock");
    let segment = dir.path_str("cam_usage-20261101T120000Z.ts");
    let out = dir.path_str("record.jsonl");
    let full: Vec<&str> = vec![
        "replay", "--socket", &socket, "--labels", &labels, "--in", &segment, "--out", &out,
    ];
    let mut cases: Vec<Vec<&str>> = vec![vec![
        "replay", "--socket", &socket, "--in", &segment, "--out", &out,
    ]];
    for extra in [
        ["--bogus", "x"],
        ["--fps", "0"],
        ["--confirm", "1.5"],
        ["--confirm", "0"],
        ["--confirm", "-0.0"],
        ["--confirm", "abc"],
    ] {
        cases.push(with(full.clone(), &extra));
    }
    for case in cases {
        let output = run(&case);
        assert_eq!(code(&output), 2, "{case:?}: {}", stderr_text(&output));
        assert!(stderr_text(&output).starts_with("usage:"), "{case:?}");
    }
}

#[test]
fn a_segment_that_decodes_no_frames_exits_three_naming_the_reason() {
    let dir = TempDir::new("noframes");
    let segment = dir.path.join("cam_zero-20261101T120000Z.ts");
    fs::write(&segment, b"").expect("an empty segment file");
    let socket = dir.path.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("the fake worker listens");
    let labels = dir.write("labels.txt", b"0 person\n");
    let out = dir.path_str("record.jsonl");
    let output = thread::scope(|scope| {
        let server = scope.spawn(|| serve_worker(&listener));
        let output = replay_run(&path_str(&socket), &labels, &path_str(&segment), &out, &[]);
        server.join().expect("the fake worker joins");
        output
    });
    assert_eq!(code(&output), 3, "{}", stderr_text(&output));
    assert!(
        stderr_text(&output).contains("decoded no frames"),
        "{}",
        stderr_text(&output)
    );
}

#[test]
fn a_missing_or_unparsable_labels_file_exits_three_naming_it() {
    let dir = TempDir::new("labels");
    let segment = make_segment(&dir.path);
    let in_text = path_str(&segment);
    let socket = dir.path_str("nobody.sock");
    let out = dir.path_str("record.jsonl");
    let missing = dir.path_str("gone.txt");
    let broken = dir.write("broken.txt", b"garbage\n");
    for labels in [missing.as_str(), broken.as_str()] {
        let output = replay_run(&socket, labels, &in_text, &out, &[]);
        assert_eq!(code(&output), 3, "{labels}: {}", stderr_text(&output));
        assert!(
            stderr_text(&output).contains(labels),
            "names the file: {labels}"
        );
    }
}

#[test]
fn the_usage_text_lists_the_replay_command() {
    const LINE: &str =
        "corvette-detect-tools replay --socket PATH --labels FILE --in SEGMENT --out FILE";
    const FLAGS: &str = "replay optional: --camera --width --height --fps --deadline-ms --confirm";
    let output = run(&["replay"]);
    assert_eq!(code(&output), 2);
    let usage = stderr_text(&output);
    assert!(usage.contains(LINE), "{usage}");
    assert!(usage.contains(FLAGS), "{usage}");
}
