use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use corvette_detect::{DEFAULT_POLICY, EngineError, LinkState, Policy, WorkerLink};
use corvette_detect_proto::{ErrorCode, HEADER_LEN, Reply, Row, TENSOR_BYTES, parse_header};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wlink-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn read_request_frame(stream: &mut UnixStream) -> Vec<u8> {
    let mut header_bytes = [0u8; HEADER_LEN];
    stream
        .read_exact(&mut header_bytes)
        .expect("request header");
    let header = parse_header(&header_bytes).expect("header decodes");
    let mut frame = vec![0u8; HEADER_LEN + header.body_len];
    frame[..HEADER_LEN].copy_from_slice(&header_bytes);
    stream
        .read_exact(&mut frame[HEADER_LEN..])
        .expect("request body");
    frame
}

fn write_reply(stream: &mut UnixStream, reply: &Reply) {
    let bytes = reply.encode().expect("reply encodes");
    stream.write_all(&bytes).expect("reply written");
}

fn serve_replies_once(listener: UnixListener, replies: Vec<Reply>) {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        for reply in replies {
            read_request_frame(&mut stream);
            write_reply(&mut stream, &reply);
        }
    });
}

fn serve_replies_counting(listener: UnixListener, replies: Vec<Reply>, accepts: Arc<AtomicU32>) {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        accepts.fetch_add(1, Ordering::Relaxed);
        for reply in replies {
            read_request_frame(&mut stream);
            write_reply(&mut stream, &reply);
        }
    });
}

fn serve_and_close_each_connection(listener: UnixListener, accepts: Arc<AtomicU32>) {
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            accepts.fetch_add(1, Ordering::Relaxed);
            read_request_frame(&mut stream);
            drop(stream);
        }
    });
}

const fn row(class_id: u32, score: f32) -> Row {
    Row {
        class_id,
        score,
        y_min: 0.25,
        x_min: 0.25,
        y_max: 0.75,
        x_max: 0.75,
    }
}

fn sample_tensor() -> Box<[u8; TENSOR_BYTES]> {
    let mut bytes = vec![0x5A; TENSOR_BYTES];
    bytes[7] = 0xF0;
    <Box<[u8; TENSOR_BYTES]>>::try_from(bytes.into_boxed_slice()).expect("tensor length")
}

const fn short_policy() -> Policy {
    Policy {
        initial: Duration::from_millis(40),
        max: Duration::from_millis(160),
        healthy_after: Duration::from_secs(30),
    }
}

fn wait_for_connect_attempt(
    link: &mut WorkerLink,
    tensor: &[u8; TENSOR_BYTES],
    started: Instant,
) -> Duration {
    let stop = Instant::now() + Duration::from_millis(400);
    loop {
        match link.infer(tensor) {
            Err(EngineError::Io(text)) if text.contains("until its retry") => {
                assert!(Instant::now() < stop, "the retry gate never opened");
            }
            other => {
                assert!(matches!(other, Err(EngineError::Io(_))), "{other:?}");
                return started.elapsed();
            }
        }
    }
}

#[test]
fn infer_returns_rows_and_reports_connected() {
    let dir = temp_dir("rows");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let rows = vec![row(3, 0.75), row(17, 0.5)];
    serve_replies_once(
        listener,
        vec![Reply::Rows {
            id: 0,
            rows: rows.clone(),
        }],
    );
    let mut link = WorkerLink::new(socket, Duration::from_secs(5), DEFAULT_POLICY);
    assert_eq!(link.state(), LinkState::Unavailable);
    let outcome = link.infer(&sample_tensor()).expect("rows reply");
    assert_eq!(outcome, rows);
    assert_eq!(link.state(), LinkState::Connected);
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn a_closed_connection_is_an_io_error_and_no_new_connect_until_due() {
    let dir = temp_dir("closed");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let accepts = Arc::new(AtomicU32::new(0));
    serve_and_close_each_connection(listener, Arc::clone(&accepts));
    let mut link = WorkerLink::new(socket, Duration::from_secs(5), DEFAULT_POLICY);
    let tensor = sample_tensor();
    let first = link.infer(&tensor);
    assert!(matches!(first, Err(EngineError::Io(_))), "{first:?}");
    assert_eq!(link.state(), LinkState::Unavailable);
    let second = link.infer(&tensor);
    assert!(matches!(second, Err(EngineError::Io(_))), "{second:?}");
    assert_eq!(accepts.load(Ordering::Relaxed), 1);
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn a_worker_returning_on_the_same_path_serves_again_after_the_delay() {
    let dir = temp_dir("reappear");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let gone = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        read_request_frame(&mut stream);
    });
    let mut link = WorkerLink::new(socket.clone(), Duration::from_secs(5), short_policy());
    let first = link.infer(&sample_tensor());
    assert!(matches!(first, Err(EngineError::Io(_))), "{first:?}");
    assert_eq!(link.state(), LinkState::Unavailable);
    gone.join().expect("first fake finished");
    fs::remove_file(&socket).expect("remove the stale socket");
    let listener = UnixListener::bind(&socket).expect("rebind");
    let rows = vec![row(9, 0.6)];
    serve_replies_once(
        listener,
        vec![Reply::Rows {
            id: 0,
            rows: rows.clone(),
        }],
    );
    thread::sleep(Duration::from_millis(45));
    let second = link
        .infer(&sample_tensor())
        .expect("rows after the worker came back");
    assert_eq!(second, rows);
    assert_eq!(link.state(), LinkState::Connected);
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn an_error_reply_keeps_the_connection_and_the_state_connected() {
    let dir = temp_dir("errorreply");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let accepts = Arc::new(AtomicU32::new(0));
    let replies = vec![
        Reply::Error {
            id: 0,
            code: ErrorCode::DeadlineOverrun,
        },
        Reply::Error {
            id: 1,
            code: ErrorCode::DeadlineOverrun,
        },
    ];
    serve_replies_counting(listener, replies, Arc::clone(&accepts));
    let mut link = WorkerLink::new(socket, Duration::from_secs(5), DEFAULT_POLICY);
    let tensor = sample_tensor();
    let first = link.infer(&tensor);
    assert!(
        matches!(&first, Err(EngineError::Worker(text)) if text.contains("DeadlineOverrun")),
        "{first:?}"
    );
    assert_eq!(link.state(), LinkState::Connected);
    let second = link.infer(&tensor);
    assert!(matches!(second, Err(EngineError::Worker(_))), "{second:?}");
    assert_eq!(link.state(), LinkState::Connected);
    assert_eq!(accepts.load(Ordering::Relaxed), 1);
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn missing_socket_gates_retries_at_initial_then_doubled() {
    let dir = temp_dir("gaps");
    let socket = dir.join("absent.sock");
    let mut link = WorkerLink::new(socket, Duration::from_secs(5), short_policy());
    let tensor = sample_tensor();
    let started = Instant::now();
    let first = link.infer(&tensor);
    assert!(matches!(first, Err(EngineError::Io(_))), "{first:?}");
    assert_eq!(link.state(), LinkState::Unavailable);
    let second_gap = wait_for_connect_attempt(&mut link, &tensor, started);
    assert!(
        second_gap >= Duration::from_millis(40),
        "second attempt waited {second_gap:?}"
    );
    let third_gap = wait_for_connect_attempt(&mut link, &tensor, started);
    assert!(
        third_gap >= Duration::from_millis(115),
        "third attempt waited {third_gap:?}"
    );
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn a_failure_after_a_healthy_run_waits_only_the_initial_delay() {
    let dir = temp_dir("healthyreset");
    let socket = dir.join("worker.sock");
    let policy = Policy {
        initial: Duration::from_millis(40),
        max: Duration::from_millis(160),
        healthy_after: Duration::from_millis(30),
    };
    let mut link = WorkerLink::new(socket.clone(), Duration::from_secs(5), policy);
    let tensor = sample_tensor();
    let first = link.infer(&tensor);
    assert!(matches!(first, Err(EngineError::Io(_))), "{first:?}");
    thread::sleep(Duration::from_millis(45));
    let second = link.infer(&tensor);
    assert!(matches!(second, Err(EngineError::Io(_))), "{second:?}");
    let listener = UnixListener::bind(&socket).expect("bind");
    let rows = vec![row(5, 0.5)];
    serve_replies_once(
        listener,
        vec![Reply::Rows {
            id: 0,
            rows: rows.clone(),
        }],
    );
    thread::sleep(Duration::from_millis(85));
    let served = link.infer(&tensor).expect("rows from the new worker");
    assert_eq!(served, rows);
    assert_eq!(link.state(), LinkState::Connected);
    thread::sleep(Duration::from_millis(40));
    let closed = link.infer(&tensor);
    assert!(matches!(closed, Err(EngineError::Io(_))), "{closed:?}");
    assert_eq!(link.state(), LinkState::Unavailable);
    let gap = wait_for_connect_attempt(&mut link, &tensor, Instant::now());
    assert!(
        (Duration::from_millis(35)..=Duration::from_millis(120)).contains(&gap),
        "retry waited {gap:?}, expected the reset initial delay"
    );
    fs::remove_dir_all(&dir).expect("remove temp dir");
}
