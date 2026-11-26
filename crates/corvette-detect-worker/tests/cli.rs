use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command as ProcessCommand, Output, Stdio};
use std::time::{Duration, Instant};

const FULL: &[&str] = &[
    "--param",
    "p",
    "--bin",
    "b",
    "--socket",
    "s",
    "--max-deadline-ms",
    "1000",
];

fn run<S: AsRef<str>>(args: &[S]) -> Output {
    let owned: Vec<String> = args.iter().map(|a| a.as_ref().to_string()).collect();
    ProcessCommand::new(env!("CARGO_BIN_EXE_corvette-detect-worker"))
        .args(&owned)
        .stdin(Stdio::null())
        .output()
        .expect("the binary runs")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn code_of(output: &Output) -> i32 {
    output.status.code().expect("the exit code is set")
}

fn strs(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_string()).collect()
}

#[test]
fn usage_errors_exit_2_and_print_the_usage_on_stderr() {
    let rows: Vec<(Vec<String>, &str)> = vec![
        (strs(&[]), "--param"),
        (strs(&["--bogus"]), "--bogus"),
        (strs(&["extra"]), "extra"),
        (with_extra(&["--socket", "t2"]), "--socket"),
        (truncated(), "--max-deadline-ms"),
        (full_at("row0", "0"), "--max-deadline-ms"),
        (full_at("rowabc", "abc"), "--max-deadline-ms"),
        (full_at("rowbig", "4294967296"), "--max-deadline-ms"),
        (without("--param"), "--param"),
        (without("--bin"), "--bin"),
        (without("--socket"), "--socket"),
        (without("--max-deadline-ms"), "--max-deadline-ms"),
        (with_extra(&["--device", "-1"]), "--device"),
        (with_extra(&["--device", "gpu"]), "--device"),
        (with_extra(&["--device", "2147483648"]), "--device"),
        (with_extra(&["--socket", "t"]), "--socket"),
    ];
    for (args, offence) in rows {
        let output = run(&args);
        assert_eq!(code_of(&output), 2, "args {args:?} must exit 2");
        assert!(stdout_of(&output).is_empty(), "args {args:?} stdout");
        let stderr = stderr_of(&output);
        assert!(stderr.contains(offence), "args {args:?} stderr {stderr}");
        assert!(stderr.contains("usage:"), "args {args:?} stderr {stderr}");
    }
}

fn truncated() -> Vec<String> {
    FULL.iter()
        .take(FULL.len() - 1)
        .map(|a| (*a).to_string())
        .collect()
}

fn full_at(tag: &str, deadline: &str) -> Vec<String> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-{tag}"));
    fs::create_dir_all(&dir).expect("tmpdir");
    let param = dir.join("p");
    let bin = dir.join("b");
    fs::write(&param, b"x").expect("param file");
    fs::write(&bin, b"x").expect("bin file");
    let mut row = vec![
        "--param".to_string(),
        path_str(&param),
        "--bin".to_string(),
        path_str(&bin),
    ];
    row.extend(
        [
            "--socket",
            dir.join("s").to_str().expect("utf-8"),
            "--max-deadline-ms",
            deadline,
        ]
        .iter()
        .map(|a| (*a).to_string()),
    );
    row
}

fn with_extra(extra: &[&str]) -> Vec<String> {
    with_extra_at(
        &FULL
            .iter()
            .map(|a| (*a).to_string())
            .collect::<Vec<String>>(),
        extra,
    )
}

fn with_extra_at(row: &[String], extra: &[&str]) -> Vec<String> {
    let mut out = row.to_vec();
    out.extend(extra.iter().map(|a| (*a).to_string()));
    out
}

fn without(flag: &str) -> Vec<String> {
    let mut out = strs(FULL);
    if let Some(at) = out.iter().position(|a| a.as_str() == flag) {
        out.drain(at..at + 2);
    }
    out
}

#[test]
fn help_prints_the_usage_and_the_exit_codes_on_stdout_and_exits_0() {
    let rows: Vec<Vec<String>> = vec![
        strs(&["--help"]),
        strs(&["--help", "--bogus"]),
        with_extra(&["--help"]),
    ];
    let mut expected = String::new();
    for args in rows {
        let output = run(&args);
        assert_eq!(code_of(&output), 0, "args {args:?}");
        let text = stdout_of(&output);
        assert!(text.contains("usage:"), "args {args:?} stdout {text}");
        assert!(
            text.contains("20") && text.contains("21"),
            "args {args:?} stdout {text}"
        );
        assert!(stderr_of(&output).is_empty(), "args {args:?} stderr");
        if expected.is_empty() {
            expected = text;
        } else {
            assert_eq!(text, expected, "args {args:?} stdout differs");
        }
    }
}

fn path_str(path: &Path) -> String {
    path.to_str().expect("utf-8").to_string()
}

#[test]
fn a_missing_model_exits_3_and_creates_no_socket() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-missing-model");
    fs::create_dir_all(&dir).expect("tmpdir");
    let socket = dir.join("serve.sock");
    let mut child = ProcessCommand::new(env!("CARGO_BIN_EXE_corvette-detect-worker"))
        .arg("--param")
        .arg(dir.join("absent.param"))
        .arg("--bin")
        .arg(dir.join("absent.bin"))
        .arg("--socket")
        .arg(&socket)
        .arg("--max-deadline-ms")
        .arg("1000")
        .arg("--device")
        .arg("cpu")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the worker starts");
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait works") {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the worker is still running after 10 s");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut stderr = Vec::new();
    child
        .stderr
        .as_mut()
        .expect("stderr piped")
        .read_to_end(&mut stderr)
        .expect("stderr reads");
    assert_eq!(status.code().expect("exit code"), 3);
    assert!(!stderr.is_empty());
    assert!(!socket.exists(), "serving socket must not exist");
}
