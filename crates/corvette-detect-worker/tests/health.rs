use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use corvette_detect_proto::{HEADER_LEN, MAX_BODY, RAW_VALUES, Reply, Request, TENSOR_BYTES};
use corvette_detect_worker::{Engine, EngineError, ServeEnd, Terminator, serve, serve_health};

const HEALTH_REQUEST: [u8; 18] = [1, 3, 12, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1, 5, 0, 0, 0];
const HEALTH_REPLY: [u8; 14] = [1, 0x83, 8, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1];
const REFUSAL: [u8; 15] = [1, 0x8F, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3];
static ZERO_TENSOR: [u8; TENSOR_BYTES] = [0u8; TENSOR_BYTES];

const MAX_DEADLINE: Duration = Duration::from_mins(1);

struct Recorder(Mutex<Vec<i32>>);

impl Recorder {
    const fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    fn codes(&self) -> Vec<i32> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Terminator for Recorder {
    fn terminate(&self, code: i32) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(code);
    }
}

struct Gated {
    gate: Receiver<()>,
    entered: Sender<()>,
}

impl Gated {
    const fn new(gate: Receiver<()>, entered: Sender<()>) -> Self {
        Self { gate, entered }
    }
}

impl Engine for Gated {
    fn infer(&mut self, _rgb: &[u8; TENSOR_BYTES]) -> Result<Box<[f32; RAW_VALUES]>, EngineError> {
        let _ = self.entered.send(());
        let _ = self.gate.recv_timeout(Duration::from_secs(10));
        Ok(zero_raw())
    }
}

fn zero_raw() -> Box<[f32; RAW_VALUES]> {
    vec![0.0f32; RAW_VALUES]
        .into_boxed_slice()
        .try_into()
        .unwrap()
}

struct StopGuard<'a>(&'a AtomicBool);

impl Drop for StopGuard<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
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

fn connect(path: &Path) -> UnixStream {
    let stream = UnixStream::connect(path).unwrap();
    let timeout = Some(Duration::from_secs(10));
    stream.set_read_timeout(timeout).unwrap();
    stream.set_write_timeout(timeout).unwrap();
    stream
}

fn read_frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut header = [0u8; HEADER_LEN];
    if stream.read_exact(&mut header).is_err() {
        return None;
    }
    let declared = u32::from_le_bytes(header[2..6].try_into().unwrap());
    let declared = usize::try_from(declared).unwrap();
    let mut frame = vec![0u8; HEADER_LEN + declared];
    frame[..HEADER_LEN].copy_from_slice(&header);
    stream.read_exact(&mut frame[HEADER_LEN..]).ok()?;
    Some(frame)
}

fn is_closed(stream: &mut UnixStream) -> bool {
    let mut byte = [0u8; 1];
    match stream.read(&mut byte) {
        Ok(read) => read == 0,
        Err(error) => error.kind() == std::io::ErrorKind::ConnectionReset,
    }
}

fn health_frame(id: u64, deadline_ms: u32) -> Vec<u8> {
    let mut frame = vec![1, 3, 12, 0, 0, 0];
    frame.extend_from_slice(&id.to_le_bytes());
    frame.extend_from_slice(&deadline_ms.to_le_bytes());
    frame
}

fn health_reply(id: u64) -> Vec<u8> {
    let mut frame = vec![1, 0x83, 8, 0, 0, 0];
    frame.extend_from_slice(&id.to_le_bytes());
    frame
}

fn header_frame(kind: u8, declared: u32) -> Vec<u8> {
    let mut frame = vec![1, kind];
    frame.extend_from_slice(&declared.to_le_bytes());
    frame
}

#[test]
fn a_health_request_is_answered_with_its_own_id() {
    let path = socket_path("h-answer");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut first = connect(&path);
            first.write_all(&HEALTH_REQUEST).unwrap();
            assert_eq!(read_frame(&mut first).unwrap(), HEALTH_REPLY);
            drop(first);
            let mut second = connect(&path);
            second
                .write_all(&health_frame(0xFFFF_FFFF_FFFF_FFFF, u32::MAX))
                .unwrap();
            assert_eq!(
                read_frame(&mut second).unwrap(),
                health_reply(0xFFFF_FFFF_FFFF_FFFF)
            );
        });
        serve_health(&listener, &stop, Duration::from_secs(5)).unwrap();
    });
}

#[test]
fn several_health_requests_on_one_connection_are_each_answered() {
    let path = socket_path("h-seq");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            for id in [1u64, 2, 3] {
                stream.write_all(&health_frame(id, 7)).unwrap();
                assert_eq!(read_frame(&mut stream).unwrap(), health_reply(id));
            }
        });
        serve_health(&listener, &stop, Duration::from_secs(5)).unwrap();
    });
}

#[test]
fn a_health_request_is_answered_while_the_serving_loop_is_inside_a_slow_engine_call() {
    let infer_path = socket_path("h-slow-inf");
    let health_path = socket_path("h-slow-hlt");
    let infer_listener = UnixListener::bind(&infer_path).unwrap();
    let health_listener = UnixListener::bind(&health_path).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let recorder = Arc::new(Recorder::new());
    let (gate_sender, gate_receiver) = std::sync::mpsc::channel();
    let (entered_sender, entered_receiver) = std::sync::mpsc::channel();
    let mut engine = Gated::new(gate_receiver, entered_sender);
    let client_stop = Arc::clone(&stop);
    thread::scope(|scope| {
        scope.spawn(move || {
            let _guard = StopGuard(&client_stop);
            let mut infer_stream = connect(&infer_path);
            infer_stream
                .write_all(
                    &Request::InferRequest {
                        id: 5,
                        deadline_ms: 20_000,
                        tensor: &ZERO_TENSOR,
                    }
                    .encode(),
                )
                .unwrap();
            entered_receiver
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            let health_request = [1, 3, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0];
            let expected_reply = [1, 0x83, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            let mut health_stream = connect(&health_path);
            let start = Instant::now();
            health_stream.write_all(&health_request).unwrap();
            let reply = read_frame(&mut health_stream).unwrap();
            let elapsed = start.elapsed();
            assert_eq!(reply, expected_reply);
            assert!(elapsed < Duration::from_secs(2));
            gate_sender.send(()).unwrap();
            let infer_reply = read_frame(&mut infer_stream).unwrap();
            assert_eq!(
                Reply::decode(&infer_reply).unwrap(),
                Reply::Rows {
                    id: 5,
                    rows: Vec::new()
                }
            );
        });
        scope.spawn(|| {
            serve_health(&health_listener, &stop, Duration::from_secs(5)).unwrap();
        });
        let end = serve(
            &infer_listener,
            &mut engine,
            &*recorder,
            &stop,
            MAX_DEADLINE,
        )
        .unwrap();
        assert_eq!(end, ServeEnd::Stopped);
    });
    assert_eq!(recorder.codes(), Vec::<i32>::new());
}

#[test]
fn anything_but_a_health_request_is_refused_and_the_connection_closed() {
    let path = socket_path("h-refuse");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let mut frames: Vec<Vec<u8>> = vec![vec![2, 3, 0, 0, 0, 0]];
    for kind in [0x01u8, 0x02, 0x81, 0x82, 0x83, 0x8F, 0x7F] {
        let mut frame = header_frame(kind, 12);
        frame.extend_from_slice(&[0u8; 12]);
        frames.push(frame);
    }
    frames.push(header_frame(3, u32::MAX));
    frames.push(header_frame(3, u32::try_from(MAX_BODY).unwrap() + 1));
    frames.push(header_frame(0x01, 1_228_812));
    let mut thirteen = header_frame(3, 13);
    thirteen.extend_from_slice(&[0u8; 13]);
    frames.push(thirteen);
    let mut eleven = header_frame(3, 11);
    eleven.extend_from_slice(&[0u8; 11]);
    frames.push(eleven);
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            for frame in &frames {
                let mut stream = connect(&path);
                stream.write_all(frame).unwrap();
                assert_eq!(read_frame(&mut stream).unwrap(), REFUSAL);
                assert!(is_closed(&mut stream));
            }
        });
        serve_health(&listener, &stop, Duration::from_secs(1)).unwrap();
    });
}

#[test]
fn a_client_that_stalls_is_cut_off_and_the_next_client_is_served() {
    let path = socket_path("h-stall");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            for stall in [
                &[][..],
                &[1, 3, 12][..],
                &[1, 3, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0][..],
            ] {
                let mut stalled = connect(&path);
                if !stall.is_empty() {
                    stalled.write_all(stall).unwrap();
                }
                let mut healthy = connect(&path);
                let start = Instant::now();
                healthy.write_all(&health_frame(0, 5)).unwrap();
                let reply = read_frame(&mut healthy).unwrap();
                assert!(start.elapsed() < Duration::from_secs(3));
                assert_eq!(reply, health_reply(0));
                assert!(is_closed(&mut stalled));
            }
        });
        serve_health(&listener, &stop, Duration::from_millis(200)).unwrap();
    });
}

#[test]
fn a_client_that_closes_early_does_not_end_the_loop() {
    let path = socket_path("h-early");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut partial = connect(&path);
            partial
                .write_all(&[1, 3, 12, 0, 0, 0, 0, 0, 0, 0, 0])
                .unwrap();
            drop(partial);
            let mut served = connect(&path);
            served.write_all(&health_frame(0, 5)).unwrap();
            assert_eq!(read_frame(&mut served).unwrap(), health_reply(0));
            drop(served);
            let opener = connect(&path);
            drop(opener);
            let mut again = connect(&path);
            again.write_all(&health_frame(0, 5)).unwrap();
            assert_eq!(read_frame(&mut again).unwrap(), health_reply(0));
        });
        serve_health(&listener, &stop, Duration::from_secs(1)).unwrap();
    });
}

#[test]
fn the_stop_flag_ends_the_loop() {
    let path = socket_path("h-stop");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let server = thread::spawn(move || serve_health(&listener, &flag, Duration::from_millis(200)));
    stop.store(true, Ordering::SeqCst);
    for _ in 0..1000 {
        if server.is_finished() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(server.is_finished());
    assert!(matches!(server.join(), Ok(Ok(()))));
}
