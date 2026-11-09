use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Output, Stdio};
use std::time::{Duration, Instant};

const FULL: &[&str] = &[
    "--param",
    "p",
    "--bin",
    "b",
    "--socket",
    "s",
    "--health-socket",
    "h",
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
        (without("--health-socket"), "--health-socket"),
        (without("--max-deadline-ms"), "--max-deadline-ms"),
        (with_extra(&["--device", "-1"]), "--device"),
        (with_extra(&["--device", "gpu"]), "--device"),
        (with_extra(&["--device", "2147483648"]), "--device"),
        (with_extra(&["--socket", "t"]), "--socket"),
        (strs(&["--probe"]), "--health-socket"),
        (
            strs(&["--probe", "--health-socket", "h", "--socket", "s"]),
            "--socket",
        ),
        (
            strs(&["--probe", "--health-socket", "h", "--health-socket", "h"]),
            "--health-socket",
        ),
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
            "--health-socket",
            dir.join("h").to_str().expect("utf-8"),
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

fn socket_path(tag: &str) -> PathBuf {
    fs::create_dir_all(env!("CARGO_TARGET_TMPDIR")).expect("tmpdir");
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-{tag}.sock"));
    assert!(
        path.as_os_str().as_encoded_bytes().len() < 90,
        "socket path too long"
    );
    let _ = fs::remove_file(&path);
    path
}

fn spawn_probe(path: &Path) -> Child {
    ProcessCommand::new(env!("CARGO_BIN_EXE_corvette-detect-worker"))
        .args(["--probe", "--health-socket"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the probe starts")
}

fn wait_child(child: &mut Child, bound: u64) -> Output {
    let deadline = Instant::now() + Duration::from_secs(bound);
    loop {
        if let Some(status) = child.try_wait().expect("wait works") {
            let stdout = child.stdout.take();
            let stderr_pipe = child.stderr.take();
            return Output {
                status,
                stdout: stdout.map_or_else(Vec::new, |mut p| {
                    let mut v = Vec::new();
                    p.read_to_end(&mut v).expect("stdout");
                    v
                }),
                stderr: stderr_pipe.map_or_else(Vec::new, |mut p| {
                    let mut v = Vec::new();
                    p.read_to_end(&mut v).expect("stderr");
                    v
                }),
            };
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the probe did not finish within {bound} s");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn accept_bounded(listener: &UnixListener) -> UnixStream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
                assert!(Instant::now() <= deadline, "no probe connection within 5 s");
            }
            Err(error) => panic!("accept failed: {error}"),
        }
    }
}

#[test]
fn the_probe_exits_0_on_a_correct_health_answer() {
    let path = socket_path("good");
    let listener = UnixListener::bind(&path).expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let mut child = spawn_probe(&path);
    let mut server = accept_bounded(&listener);
    let mut request = [0u8; 18];
    server.read_exact(&mut request).expect("request reads");
    assert_eq!(&request[..6], &[1, 3, 12, 0, 0, 0]);
    let mut reply = vec![1, 0x83, 8, 0, 0, 0];
    reply.extend_from_slice(&request[6..14]);
    server.write_all(&reply).expect("reply writes");
    drop(server);
    let output = wait_child(&mut child, 5);
    assert_eq!(code_of(&output), 0);
    assert!(stdout_of(&output).is_empty());
}

#[test]
fn the_probe_exits_1_when_nothing_answers() {
    let missing = socket_path("missing");
    let output = run(&["--probe", "--health-socket", &path_str(&missing)]);
    assert_eq!(code_of(&output), 1);
    assert!(!stderr_of(&output).is_empty());

    let stale = socket_path("stale");
    drop(UnixListener::bind(&stale).expect("bind"));
    let output = run(&["--probe", "--health-socket", &path_str(&stale)]);
    assert_eq!(code_of(&output), 1);
    assert!(!stderr_of(&output).is_empty());

    let file = socket_path("regular");
    fs::write(&file, b"x").expect("regular file");
    let output = run(&["--probe", "--health-socket", &path_str(&file)]);
    assert_eq!(code_of(&output), 1);
    assert!(!stderr_of(&output).is_empty());
}

fn path_str(path: &Path) -> String {
    path.to_str().expect("utf-8").to_string()
}

#[test]
fn the_probe_exits_1_on_a_wrong_answer() {
    let cases: Vec<Vec<u8>> = vec![
        vec![1u8, 0x83, 8, 0, 0, 0],
        vec![1, 0x8F, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3],
        vec![0xFF; 14],
        vec![],
    ];
    for (index, answer) in cases.iter().enumerate() {
        let path = socket_path(&format!("wrong-{index}"));
        let listener = UnixListener::bind(&path).expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let mut child = spawn_probe(&path);
        let mut server = accept_bounded(&listener);
        let mut request = [0u8; 18];
        server.read_exact(&mut request).expect("request reads");
        let mut reply = answer.clone();
        if reply.len() == 6 {
            let mut id: Vec<u8> = request[6..14].to_vec();
            id[0] = id[0].wrapping_add(1);
            reply.extend_from_slice(&id);
        }
        let _ = server.write_all(&reply);
        drop(server);
        let output = wait_child(&mut child, 5);
        assert_eq!(code_of(&output), 1, "case {index}");
    }
}

#[test]
fn the_probe_gives_up_on_a_server_that_never_answers() {
    let path = socket_path("silent");
    let listener = UnixListener::bind(&path).expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let mut child = spawn_probe(&path);
    let started = Instant::now();
    let server = accept_bounded(&listener);
    let output = wait_child(&mut child, 5);
    let elapsed = started.elapsed();
    assert_eq!(code_of(&output), 1);
    assert!(elapsed < Duration::from_millis(4500), "elapsed {elapsed:?}");
    drop(server);
}

#[test]
fn a_missing_model_exits_3_and_creates_no_socket() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-missing-model");
    fs::create_dir_all(&dir).expect("tmpdir");
    let socket = dir.join("serve.sock");
    let health = dir.join("health.sock");
    let mut child = ProcessCommand::new(env!("CARGO_BIN_EXE_corvette-detect-worker"))
        .arg("--param")
        .arg(dir.join("absent.param"))
        .arg("--bin")
        .arg(dir.join("absent.bin"))
        .arg("--socket")
        .arg(&socket)
        .arg("--health-socket")
        .arg(&health)
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
    assert!(!health.exists(), "health socket must not exist");
}
