use std::path::PathBuf;
use std::process::Output;

const HAND: &str = "0\t1\t0,0,702,576\n\
                    1\t0\t34,149,74,224\n\
                    2\t0\t-\n\
                    3\t0\t443,437,506,501;34,115,155,218;230,92,276,138\n\
                    4\t0\t604,253,696,576\n\
                    5\t0\t10,10,50,50;60,60,100,100\n\
                    6\t1\t34,149,74,224\n";
const DEFAULT_OUT: &str = "0\t-\n\
                           1\t0,26,320,346\n\
                           2\t-\n\
                           3\t0,6,320,326;93,0,413,320;314,256,634,576\n\
                           4\t60,0,704,644\n\
                           5\t0,0,320,320\n\
                           6\t-\n";
const SMALL_OUT: &str = "0\t-\n\
                         1\t0,112,148,260\n\
                         2\t-\n\
                         3\t0,46,240,286;203,65,303,165;410,405,538,533\n\
                         4\t60,0,704,644\n\
                         5\t0,0,180,180\n\
                         6\t-\n";
const HAND_SUMMARY: &str = "pictures 7, with regions 4, regions 6\n";

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

fn run_args<'a>(boxes: &'a str, out: &'a str) -> Vec<&'a str> {
    vec![
        "regions", "--boxes", boxes, "--width", "704", "--height", "576", "--out", out,
    ]
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "corvette-detect-regions-{label}-{}",
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

#[test]
fn hand_file_matches_the_frigate_oracle() {
    let dir = TempDir::new("oracle");
    let boxes = dir.write("hand.tsv", HAND.as_bytes());
    let out = dir.path_str("out.tsv");
    let output = run(&run_args(&boxes, &out));
    assert_eq!(code(&output), 0);
    assert_eq!(stdout_text(&output), HAND_SUMMARY);
    assert_eq!(std::fs::read_to_string(&out).unwrap(), DEFAULT_OUT);
    assert!(stderr_text(&output).is_empty());
}

#[test]
fn a_smaller_model_gives_a_smaller_floor() {
    let dir = TempDir::new("model");
    let boxes = dir.write("hand.tsv", HAND.as_bytes());
    let small = dir.path_str("small.tsv");
    let mut args = run_args(&boxes, &small);
    args.extend_from_slice(&["--model-size", "100"]);
    let output = run(&args);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout_text(&output), HAND_SUMMARY);
    assert_eq!(std::fs::read_to_string(&small).unwrap(), SMALL_OUT);
    let same = dir.path_str("same.tsv");
    let mut args = run_args(&boxes, &same);
    args.extend_from_slice(&["--model-size", "322"]);
    let output = run(&args);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout_text(&output), HAND_SUMMARY);
    assert_eq!(std::fs::read_to_string(&same).unwrap(), DEFAULT_OUT);
}

#[test]
fn squares_may_extend_beyond_the_picture() {
    let dir = TempDir::new("beyond");
    let boxes = dir.write(
        "boxes.tsv",
        b"0\t0\t513,129,638,480;0,24,403,480\n1\t0\t571,331,638,480;0,0,465,480\n",
    );
    let out = dir.path_str("out.tsv");
    let output = run(&[
        "regions", "--boxes", &boxes, "--width", "640", "--height", "480", "--out", &out,
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(
        stdout_text(&output),
        "pictures 2, with regions 2, regions 3\n"
    );
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "0\t0,0,1276,1276\n1\t0,0,960,960;320,160,640,480\n"
    );
}

#[test]
fn box_order_does_not_change_the_line() {
    let dir = TempDir::new("order");
    let boxes = dir.write(
        "boxes.tsv",
        b"0\t0\t230,92,276,138;443,437,506,501;34,115,155,218\n",
    );
    let out = dir.path_str("out.tsv");
    let output = run(&run_args(&boxes, &out));
    assert_eq!(code(&output), 0);
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "0\t0,6,320,326;93,0,413,320;314,256,634,576\n"
    );
}

#[test]
fn a_calibrating_picture_ignores_its_boxes() {
    let dir = TempDir::new("calibrating");
    let out = dir.path_str("out.tsv");
    let calibrating = dir.write("calibrating.tsv", b"0\t1\t1048577,0,1048627,40\n");
    let output = run(&run_args(&calibrating, &out));
    assert_eq!(code(&output), 0);
    assert_eq!(
        stdout_text(&output),
        "pictures 1, with regions 0, regions 0\n"
    );
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "0\t-\n");
    let detecting = dir.write("detecting.tsv", b"0\t0\t1048577,0,1048627,40\n");
    let output = run(&run_args(&detecting, &out));
    assert_eq!(code(&output), 3);
    assert!(stdout_text(&output).is_empty());
    assert!(stderr_text(&output).contains("picture 0"));
}

#[test]
fn two_runs_write_identical_output() {
    let dir = TempDir::new("twice");
    let boxes = dir.write("hand.tsv", HAND.as_bytes());
    let first = dir.path_str("first.tsv");
    let second = dir.path_str("second.tsv");
    let one = run(&run_args(&boxes, &first));
    let two = run(&run_args(&boxes, &second));
    assert_eq!(code(&one), 0);
    assert_eq!(code(&two), 0);
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap()
    );
}

#[test]
fn bad_usage_exits_2() {
    let dir = TempDir::new("usage");
    let boxes = dir.write("boxes.tsv", HAND.as_bytes());
    let out = dir.path_str("out.tsv");
    let mut cases: Vec<Vec<&str>> = vec![vec!["regions"]];
    for skip in ["--boxes", "--width", "--height", "--out"] {
        let mut case: Vec<&str> = vec!["regions"];
        for pair in run_args(&boxes, &out)[1..].chunks(2) {
            if pair[0] != skip {
                case.extend_from_slice(pair);
            }
        }
        cases.push(case);
    }
    cases.push(vec![
        "regions", "--boxes", &boxes, "--width", "704", "--height", "576", "--out", &out,
        "--bogus", "x",
    ]);
    cases.push(vec![
        "regions", "--boxes", &boxes, "--boxes", &boxes, "--width", "704", "--height", "576",
        "--out", &out,
    ]);
    cases.push(vec!["regions", "--boxes"]);
    cases.push(vec![
        "regions", "--boxes", &boxes, "--width", "0", "--height", "576", "--out", &out,
    ]);
    cases.push(vec![
        "regions", "--boxes", &boxes, "--width", "704", "--height", "abc", "--out", &out,
    ]);
    for bad_model in ["0", "8193", "abc"] {
        cases.push(vec![
            "regions",
            "--boxes",
            &boxes,
            "--width",
            "704",
            "--height",
            "576",
            "--out",
            &out,
            "--model-size",
            bad_model,
        ]);
    }
    for case in &cases {
        let output = run(case);
        assert_eq!(code(&output), 2, "{case:?}");
        assert!(stdout_text(&output).is_empty(), "{case:?}");
        let usage = stderr_text(&output);
        assert!(usage.starts_with("usage:"), "{case:?}: {usage}");
        assert!(usage.contains("regions"), "{case:?}: {usage}");
    }
}

#[test]
fn bad_input_exits_3() {
    let dir = TempDir::new("input");
    let out = dir.path_str("out.tsv");
    let cases: Vec<(&str, &str)> = vec![
        ("0\t0\n", "line 1"),
        ("0\t0\t-\textra\n", "line 1"),
        ("1\t0\t-\n", "line 1"),
        ("0\t0\t-\n0\t0\t-\n", "line 2"),
        ("0\t2\t-\n", "line 1"),
        ("0\t\t-\n", "line 1"),
        ("0\t0\t1,2,3\n", "line 1"),
        ("0\t0\t-1,0,5,5\n", "line 1"),
        ("0\t0\t+1,0,5,5\n", "line 1"),
        ("0\t0\t1,,5,5\n", "line 1"),
        ("+0\t0\t-\n", "line 1"),
        ("0\t0\tab,cd,ef,gh\n", "line 1"),
        ("0\t0\t1, 2,5,5\n", "line 1"),
        ("0\t0\t-\n\n1\t0\t-\n", "line 2"),
        ("", "line 1"),
    ];
    for (content, needle) in cases {
        let boxes = dir.write("bad.tsv", content.as_bytes());
        let output = run(&run_args(&boxes, &out));
        assert_eq!(code(&output), 3, "{content:?}");
        assert!(stdout_text(&output).is_empty(), "{content:?}");
        let error = stderr_text(&output);
        assert!(error.starts_with("error: "), "{content:?}: {error}");
        assert!(error.contains(needle), "{content:?}: {error}");
    }
    let missing = dir.path_str("missing.tsv");
    let output = run(&run_args(&missing, &out));
    assert_eq!(code(&output), 3);
    assert!(stdout_text(&output).is_empty());
    assert!(stderr_text(&output).contains("missing.tsv"));
    let bad_out = dir.path_str("no-such-dir/out.tsv");
    let good = dir.write("good.tsv", HAND.as_bytes());
    let output = run(&run_args(&good, &bad_out));
    assert_eq!(code(&output), 3);
    assert!(stdout_text(&output).is_empty());
    assert!(stderr_text(&output).contains("no-such-dir"));
    let last = dir.write("last.tsv", b"0\t0\t-\n1\t0\t10,10,50,50");
    let output = run(&run_args(&last, &out));
    assert_eq!(code(&output), 0);
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "0\t-\n1\t0,0,320,320\n"
    );
}

#[test]
fn squares_that_share_a_corner_sort_by_the_far_corner() {
    let dir = TempDir::new("corner");
    let out = dir.path_str("out.tsv");
    let forward = dir.write("forward.tsv", b"0\t0\t0,0,704,576;61,11,62,12\n");
    let output = run(&run_args(&forward, &out));
    assert_eq!(code(&output), 0);
    assert_eq!(
        stdout_text(&output),
        "pictures 1, with regions 1, regions 2\n"
    );
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "0\t0,0,320,320;0,0,1408,1408\n"
    );
    assert!(stderr_text(&output).is_empty());
    let reversed = dir.write("reversed.tsv", b"0\t0\t61,11,62,12;0,0,704,576\n");
    let output = run(&run_args(&reversed, &out));
    assert_eq!(code(&output), 0);
    assert_eq!(
        stdout_text(&output),
        "pictures 1, with regions 1, regions 2\n"
    );
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "0\t0,0,320,320;0,0,1408,1408\n"
    );
    assert!(stderr_text(&output).is_empty());
}

#[test]
fn trailing_blank_lines_are_refused() {
    let dir = TempDir::new("trailing");
    let out = dir.path_str("out.tsv");
    let cases: Vec<(&str, &str)> = vec![("0\t0\t-\n\n", "line 2"), ("\n", "line 1")];
    for (content, needle) in cases {
        let boxes = dir.write("blank.tsv", content.as_bytes());
        let output = run(&run_args(&boxes, &out));
        assert_eq!(code(&output), 3, "{content:?}");
        assert!(stdout_text(&output).is_empty(), "{content:?}");
        let error = stderr_text(&output);
        assert!(error.contains(needle), "{content:?}: {error}");
        assert!(
            error.contains("expected 3 tab-separated fields"),
            "{content:?}: {error}"
        );
    }
}

#[test]
fn coordinates_above_i32_are_refused_at_their_line() {
    let dir = TempDir::new("i32");
    let out = dir.path_str("out.tsv");
    let too_big = dir.write("too_big.tsv", b"0\t0\t2147483648,0,5,5\n");
    let output = run(&run_args(&too_big, &out));
    assert_eq!(code(&output), 3);
    assert!(stdout_text(&output).is_empty());
    let error = stderr_text(&output);
    assert!(error.contains("line 1"), "{error}");
    assert!(error.contains("out of range"), "{error}");
    assert!(!error.contains("picture"), "{error}");
    let last_i32 = dir.write("last_i32.tsv", b"0\t0\t2147483647,0,5,5\n");
    let output = run(&run_args(&last_i32, &out));
    assert_eq!(code(&output), 3);
    assert!(stdout_text(&output).is_empty());
    let error = stderr_text(&output);
    assert!(error.contains("picture 0"), "{error}");
    assert!(error.contains("outside the supported range"), "{error}");
    assert!(!error.contains("out of range"), "{error}");
}
