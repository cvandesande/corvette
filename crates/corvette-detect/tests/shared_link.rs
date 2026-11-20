use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use corvette_detect::{
    DEFAULT_POLICY, EngineError, Infer, LinkState, LinkTimings, SharedLink, WorkerLink,
};
use corvette_detect_proto::{HEADER_LEN, Reply, Request, Row, TENSOR_BYTES, parse_header};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "shlink-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn read_request_id(stream: &mut UnixStream) -> Option<u64> {
    let mut header_bytes = [0u8; HEADER_LEN];
    if stream.read_exact(&mut header_bytes).is_err() {
        return None;
    }
    let header = parse_header(&header_bytes).expect("header decodes");
    let mut body = vec![0u8; header.body_len];
    if stream.read_exact(&mut body).is_err() {
        return None;
    }
    let mut frame = header_bytes.to_vec();
    frame.extend_from_slice(&body);
    match Request::decode(&frame) {
        Ok(Request::InferRequest { id, .. }) => Some(id),
        _ => None,
    }
}

fn serve_one_connection(listener: UnixListener, delay: Duration, accepts: Arc<AtomicU32>) {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        accepts.fetch_add(1, Ordering::Relaxed);
        while let Some(id) = read_request_id(&mut stream) {
            if !delay.is_zero() {
                thread::sleep(delay);
            }
            let reply = Reply::Rows {
                id,
                worker_ms: 1,
                rows: vec![row(3, 0.75)],
            };
            let bytes = reply.encode().expect("reply encodes");
            if stream.write_all(&bytes).is_err() {
                return;
            }
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

#[test]
fn two_handles_answer_every_request_over_one_connection() {
    let dir = temp_dir("shared");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let accepts = Arc::new(AtomicU32::new(0));
    serve_one_connection(listener, Duration::ZERO, Arc::clone(&accepts));
    let shared = SharedLink::new(WorkerLink::new(
        socket,
        Duration::from_secs(5),
        DEFAULT_POLICY,
    ));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let mut handle = shared.infer_handle();
            let timings = handle.timings();
            thread::spawn(move || {
                let mut answered = 0;
                for _ in 0..3 {
                    if handle.infer(&sample_tensor()).is_ok() {
                        answered += 1;
                    }
                }
                (answered, timings)
            })
        })
        .collect();
    let mut answered_total = 0;
    let mut timings = Vec::new();
    for handle in handles {
        let (answered, handle_timings) = handle.join().expect("thread joins");
        answered_total += answered;
        timings.push(handle_timings);
    }
    assert_eq!(answered_total, 6);
    assert_eq!(accepts.load(Ordering::Relaxed), 1);
    assert_eq!(shared.state(), LinkState::Connected);
    for timing in &timings {
        assert!(
            timing.lock_wait_ms() < 150,
            "lock wait {:?}",
            timing.lock_wait_ms()
        );
        assert!(
            timing.roundtrip_ms() < 150,
            "round trip {:?}",
            timing.roundtrip_ms()
        );
    }
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn the_waiting_handle_measures_its_lock_wait_outside_its_roundtrip() {
    let dir = temp_dir("delay");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let accepts = Arc::new(AtomicU32::new(0));
    serve_one_connection(listener, Duration::from_millis(60), Arc::clone(&accepts));
    let shared = SharedLink::new(WorkerLink::new(
        socket,
        Duration::from_secs(5),
        DEFAULT_POLICY,
    ));
    let start = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let mut handle = shared.infer_handle();
            let timings = handle.timings();
            let start = Arc::clone(&start);
            thread::spawn(move || {
                start.wait();
                let answered = handle.infer(&sample_tensor()).is_ok();
                (answered, timings)
            })
        })
        .collect();
    let mut timings: Vec<LinkTimings> = Vec::new();
    for handle in handles {
        let (answered, handle_timings) = handle.join().expect("thread joins");
        assert!(answered, "a shared-link request failed");
        timings.push(handle_timings);
    }
    let waiting: Vec<&LinkTimings> = timings.iter().filter(|t| t.lock_wait_ms() >= 30).collect();
    assert!(
        !waiting.is_empty(),
        "no handle waited on the lock: {:?}",
        timings
            .iter()
            .map(|t| (t.lock_wait_ms(), t.roundtrip_ms()))
            .collect::<Vec<_>>()
    );
    let free: Vec<&LinkTimings> = timings.iter().filter(|t| t.lock_wait_ms() < 15).collect();
    assert!(!free.is_empty(), "no handle held the lock uncontended");
    for timing in &waiting {
        assert!(
            timing.roundtrip_ms() < timing.lock_wait_ms() + 150,
            "roundtrip {} should exclude a lock wait of {}",
            timing.roundtrip_ms(),
            timing.lock_wait_ms()
        );
    }
    for timing in &free {
        assert!(
            timing.roundtrip_ms() >= 55,
            "an uncontended round trip should cover the worker's delay, got {}",
            timing.roundtrip_ms()
        );
        for waiting in &waiting {
            assert!(
                waiting.roundtrip_ms() < timing.roundtrip_ms() + 30,
                "the waiting round trip {} includes its lock wait of {}",
                waiting.roundtrip_ms(),
                waiting.lock_wait_ms()
            );
        }
    }
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn a_panicking_handle_leaves_another_handle_working() {
    let dir = temp_dir("panic");
    let socket = dir.join("absent.sock");
    let shared = SharedLink::new(WorkerLink::new(
        socket,
        Duration::from_secs(5),
        DEFAULT_POLICY,
    ));
    let crashing = shared.clone();
    let crashed = thread::spawn(move || {
        let mut handle = crashing.infer_handle();
        let outcome = handle.infer(&sample_tensor());
        assert!(matches!(outcome, Err(EngineError::Io(_))), "{outcome:?}");
        panic!("the camera thread died while holding its handle");
    });
    assert!(
        crashed.join().is_err(),
        "the test thread must have panicked"
    );
    let mut other = shared.infer_handle();
    let outcome = other.infer(&sample_tensor());
    assert!(matches!(outcome, Err(EngineError::Io(_))), "{outcome:?}");
    assert_eq!(shared.state(), LinkState::Unavailable);
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn state_is_unavailable_before_and_connected_after_a_request() {
    let dir = temp_dir("state");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let accepts = Arc::new(AtomicU32::new(0));
    serve_one_connection(listener, Duration::ZERO, Arc::clone(&accepts));
    let shared = SharedLink::new(WorkerLink::new(
        socket,
        Duration::from_secs(5),
        DEFAULT_POLICY,
    ));
    assert_eq!(shared.state(), LinkState::Unavailable);
    let mut handle = shared.infer_handle();
    let outcome = handle.infer(&sample_tensor()).expect("rows reply");
    assert_eq!(outcome, vec![row(3, 0.75)]);
    assert_eq!(shared.state(), LinkState::Connected);
    fs::remove_dir_all(&dir).expect("remove temp dir");
}
