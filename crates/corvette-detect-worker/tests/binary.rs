use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use corvette_detect_proto::{ErrorCode, RAW_VALUES, Reply, Request, TENSOR_BYTES};

const WRONG_PARAM: &str = concat!(
    "7767517\n",
    "4 4\n",
    "Input in0 0 1 in0 0=640 1=640 2=3\n",
    "Flatten flat 1 1 in0 flat0\n",
    "Crop crop 1 1 flat0 crop0 0=0 3=100\n",
    "Reshape out 1 1 crop0 out0 0=100 1=1 2=1\n",
);

const PARAM: &str = concat!(
    "7767517\n",
    "4 4\n",
    "Input in0 0 1 in0 0=640 1=640 2=3\n",
    "Flatten flat 1 1 in0 flat0\n",
    "Crop crop 1 1 flat0 crop0 0=0 3=420000\n",
    "Reshape out 1 1 crop0 out0 0=8400 1=50 2=1\n",
);

static ZERO_TENSOR: [u8; TENSOR_BYTES] = [0u8; TENSOR_BYTES];

struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn write_net(tag: &str, param: &str) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(tag);
    fs::create_dir_all(&dir).unwrap();
    let param_path = dir.join("net.param");
    let bin_path = dir.join("net.bin");
    fs::write(&param_path, param).unwrap();
    fs::write(&bin_path, b"").unwrap();
    (param_path, bin_path)
}

fn socket_path(tag: &str) -> PathBuf {
    fs::create_dir_all(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cw-{tag}.sock"));
    assert!(
        path.as_os_str().as_encoded_bytes().len() < 90,
        "socket path too long"
    );
    let _ = fs::remove_file(&path);
    path
}

fn start(tag: &str, param: &str, max_deadline_ms: &str, socket: &Path, health: &Path) -> Worker {
    let (param_path, bin_path) = write_net(&format!("net-{tag}"), param);
    let child = ProcessCommand::new(env!("CARGO_BIN_EXE_corvette-detect-worker"))
        .arg("--param")
        .arg(&param_path)
        .arg("--bin")
        .arg(&bin_path)
        .arg("--socket")
        .arg(socket)
        .arg("--health-socket")
        .arg(health)
        .arg("--max-deadline-ms")
        .arg(max_deadline_ms)
        .arg("--device")
        .arg("cpu")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the worker starts");
    Worker(child)
}

fn probe_exit(health: &Path) -> Option<i32> {
    ProcessCommand::new(env!("CARGO_BIN_EXE_corvette-detect-worker"))
        .arg("--probe")
        .arg("--health-socket")
        .arg(health)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .expect("the probe runs")
        .status
        .code()
}

fn wait_ready(health: &Path) {
    for _ in 0..100 {
        if probe_exit(health) == Some(0) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("no ready probe within 10 s on {}", health.display());
}

fn wait_exit(worker: &mut Worker, seconds: u64) -> Option<i32> {
    let limit = Instant::now() + Duration::from_secs(seconds);
    loop {
        if let Some(status) = worker.0.try_wait().expect("wait works") {
            return status.code();
        }
        if Instant::now() > limit {
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn connect(path: &Path) -> UnixStream {
    try_connect(path).expect("connect")
}

fn try_connect(path: &Path) -> Option<UnixStream> {
    let stream = UnixStream::connect(path).ok()?;
    let timeout = Some(Duration::from_secs(10));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return None;
    }
    Some(stream)
}

fn read_frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut header = [0u8; 6];
    if stream.read_exact(&mut header).is_err() {
        return None;
    }
    let declared = usize::try_from(u32::from_le_bytes(header[2..6].try_into().unwrap())).unwrap();
    let mut frame = vec![0u8; 6 + declared];
    frame[..6].copy_from_slice(&header);
    stream.read_exact(&mut frame[6..]).ok()?;
    Some(frame)
}

fn read_reply(stream: &mut UnixStream) -> Option<Reply> {
    Some(Reply::decode(&read_frame(stream)?).expect("decodable reply"))
}

fn send(stream: &mut UnixStream, request: &Request) {
    stream.write_all(&request.encode()).expect("request writes");
}

#[test]
fn the_worker_serves_a_request_and_answers_the_probe_while_the_connection_stays_open() {
    let socket = socket_path("bin-serve-s");
    let health = socket_path("bin-serve-h");
    let mut worker = start("serve", PARAM, "10000", &socket, &health);
    wait_ready(&health);
    let mut stream = connect(&socket);
    send(
        &mut stream,
        &Request::InferRawRequest {
            id: 7,
            deadline_ms: 10_000,
            tensor: &ZERO_TENSOR,
        },
    );
    let reply = read_reply(&mut stream).expect("raw reply");
    let Reply::Raw { id, values } = reply else {
        panic!("expected a Raw reply, got {reply:?}");
    };
    assert_eq!(id, 7);
    assert_eq!(values.len(), RAW_VALUES);
    assert!(
        values.iter().all(|value| *value == 0.0),
        "a zero tensor yields zero values"
    );
    assert_eq!(
        probe_exit(&health),
        Some(0),
        "the probe answers while the serving connection stays open"
    );
    send(
        &mut stream,
        &Request::InferRequest {
            id: 8,
            deadline_ms: 10_000,
            tensor: &ZERO_TENSOR,
        },
    );
    let reply = read_reply(&mut stream).expect("rows reply");
    let Reply::Rows { id, rows } = reply else {
        panic!("expected a Rows reply, got {reply:?}");
    };
    assert_eq!(id, 8);
    assert!(rows.is_empty(), "a zero tensor yields no rows: {rows:?}");
    assert_eq!(probe_exit(&health), Some(0));
    worker.0.kill().expect("kill");
    worker.0.wait().expect("wait");
    assert_eq!(
        probe_exit(&health),
        Some(1),
        "the socket files remain but nothing listens"
    );
}

#[test]
fn a_failing_engine_ends_the_process_with_exit_code_20() {
    let socket = socket_path("bin-wrong-s");
    let health = socket_path("bin-wrong-h");
    let mut worker = start("wrong", WRONG_PARAM, "10000", &socket, &health);
    wait_ready(&health);
    let mut stream = connect(&socket);
    send(
        &mut stream,
        &Request::InferRawRequest {
            id: 9,
            deadline_ms: 10_000,
            tensor: &ZERO_TENSOR,
        },
    );
    let reply = read_reply(&mut stream).expect("error reply");
    assert_eq!(
        reply,
        Reply::Error {
            id: 9,
            code: ErrorCode::DeviceLost
        }
    );
    assert_eq!(wait_exit(&mut worker, 10), Some(20));
}

#[test]
fn a_request_that_outlives_the_cap_ends_the_process_with_exit_code_21() {
    let socket = socket_path("bin-cap-s");
    let health = socket_path("bin-cap-h");
    let mut worker = start("cap", PARAM, "1", &socket, &health);
    wait_ready(&health);
    for _ in 0..50 {
        if worker.0.try_wait().expect("wait works").is_some() {
            break;
        }
        let Some(mut stream) = try_connect(&socket) else {
            break;
        };
        send(
            &mut stream,
            &Request::InferRawRequest {
                id: 10,
                deadline_ms: 10_000,
                tensor: &ZERO_TENSOR,
            },
        );
        let _ = read_frame(&mut stream);
    }
    assert_eq!(wait_exit(&mut worker, 10), Some(21));
}

#[test]
fn a_stale_socket_file_is_replaced() {
    let socket = socket_path("bin-stale-s");
    let health = socket_path("bin-stale-h");
    drop(UnixListener::bind(&socket).expect("stale serving socket"));
    drop(UnixListener::bind(&health).expect("stale health socket"));
    let _worker = start("stale", PARAM, "10000", &socket, &health);
    wait_ready(&health);
    let mut stream = connect(&socket);
    send(
        &mut stream,
        &Request::InferRawRequest {
            id: 11,
            deadline_ms: 10_000,
            tensor: &ZERO_TENSOR,
        },
    );
    let reply = read_reply(&mut stream).expect("raw reply");
    let Reply::Raw { id, values } = reply else {
        panic!("expected a Raw reply, got {reply:?}");
    };
    assert_eq!(id, 11);
    assert_eq!(values.len(), RAW_VALUES);
}

#[test]
fn a_regular_file_at_a_socket_path_is_refused_and_left_alone() {
    let socket = socket_path("bin-refuse-a-s");
    let health = socket_path("bin-refuse-a-h");
    fs::write(&socket, b"keep").expect("regular file at the serving path");
    let mut worker = start("refuse-a", PARAM, "10000", &socket, &health);
    assert_eq!(wait_exit(&mut worker, 10), Some(3));
    assert_eq!(
        fs::read(&socket).expect("serving path readable"),
        b"keep",
        "the file is left alone"
    );
    let socket = socket_path("bin-refuse-b-s");
    let health = socket_path("bin-refuse-b-h");
    fs::write(&health, b"keep").expect("regular file at the health path");
    let mut worker = start("refuse-b", PARAM, "10000", &socket, &health);
    assert_eq!(wait_exit(&mut worker, 10), Some(3));
    assert_eq!(
        fs::read(&health).expect("health path readable"),
        b"keep",
        "the file is left alone"
    );
}
