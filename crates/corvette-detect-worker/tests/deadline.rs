use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use corvette_detect_proto::{ErrorCode, HEADER_LEN, RAW_VALUES, Reply, Request, TENSOR_BYTES};
use corvette_detect_worker::{
    EXIT_DEADLINE_OVERRUN, Engine, EngineError, FatalReason, ServeEnd, Terminator, serve,
};

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
    calls: AtomicUsize,
}

impl Gated {
    const fn new(gate: Receiver<()>) -> Self {
        Self {
            gate,
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Engine for Gated {
    fn infer(&mut self, _rgb: &[u8; TENSOR_BYTES]) -> Result<Box<[f32; RAW_VALUES]>, EngineError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = self.gate.recv_timeout(Duration::from_secs(10));
        Ok(zero_raw())
    }
}

struct Quick {
    calls: AtomicUsize,
}

impl Quick {
    const fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Engine for Quick {
    fn infer(&mut self, _rgb: &[u8; TENSOR_BYTES]) -> Result<Box<[f32; RAW_VALUES]>, EngineError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(zero_raw())
    }
}

struct Panicking;

impl Engine for Panicking {
    fn infer(&mut self, _rgb: &[u8; TENSOR_BYTES]) -> Result<Box<[f32; RAW_VALUES]>, EngineError> {
        panic!("engine panic");
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

fn read_reply(stream: &mut UnixStream) -> Option<Reply> {
    Some(Reply::decode(&read_frame(stream)?).unwrap())
}

fn wait_for_codes(recorder: &Recorder) -> Vec<i32> {
    for _ in 0..2000 {
        let codes = recorder.codes();
        if !codes.is_empty() {
            return codes;
        }
        thread::sleep(Duration::from_millis(5));
    }
    recorder.codes()
}

static ZERO_TENSOR: [u8; TENSOR_BYTES] = [0u8; TENSOR_BYTES];

const MAX_DEADLINE: Duration = Duration::from_mins(1);

#[test]
fn a_request_that_outlives_its_deadline_ends_the_server_with_the_deadline_code() {
    let path = socket_path("dl-overrun");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut engine = Gated::new(receiver);
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            let tensor = &ZERO_TENSOR;
            stream
                .write_all(
                    &Request::InferRequest {
                        id: 11,
                        deadline_ms: 50,
                        tensor,
                    }
                    .encode(),
                )
                .unwrap();
            let codes = wait_for_codes(&recorder);
            sender.send(()).unwrap();
            assert_eq!(codes, vec![EXIT_DEADLINE_OVERRUN]);
            assert_eq!(
                read_reply(&mut stream),
                Some(Reply::Error {
                    id: 11,
                    code: ErrorCode::DeadlineOverrun
                })
            );
            assert!(is_closed(&mut stream));
        });
        let end = serve(&listener, &mut engine, &recorder, &stop, MAX_DEADLINE);
        assert_eq!(end.unwrap(), ServeEnd::Fatal(FatalReason::DeadlineOverrun));
    });
    assert_eq!(recorder.codes(), vec![EXIT_DEADLINE_OVERRUN]);
    assert_eq!(engine.calls(), 1);
}

#[test]
fn a_cap_shorter_than_the_request_deadline_ends_the_server_at_the_cap() {
    let path = socket_path("dl-cap");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut engine = Gated::new(receiver);
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            let tensor = &ZERO_TENSOR;
            stream
                .write_all(
                    &Request::InferRequest {
                        id: 13,
                        deadline_ms: 60_000,
                        tensor,
                    }
                    .encode(),
                )
                .unwrap();
            let codes = wait_for_codes(&recorder);
            sender.send(()).unwrap();
            assert_eq!(codes, vec![EXIT_DEADLINE_OVERRUN]);
            assert_eq!(
                read_reply(&mut stream),
                Some(Reply::Error {
                    id: 13,
                    code: ErrorCode::DeadlineOverrun
                })
            );
            assert!(is_closed(&mut stream));
        });
        let end = serve(
            &listener,
            &mut engine,
            &recorder,
            &stop,
            Duration::from_millis(50),
        );
        assert_eq!(end.unwrap(), ServeEnd::Fatal(FatalReason::DeadlineOverrun));
    });
    assert_eq!(recorder.codes(), vec![EXIT_DEADLINE_OVERRUN]);
    assert_eq!(engine.calls(), 1);
}

#[test]
fn an_infer_raw_request_that_outlives_its_deadline_does_the_same() {
    let path = socket_path("dl-overraw");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut engine = Gated::new(receiver);
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            let tensor = &ZERO_TENSOR;
            stream
                .write_all(
                    &Request::InferRawRequest {
                        id: 12,
                        deadline_ms: 50,
                        tensor,
                    }
                    .encode(),
                )
                .unwrap();
            let codes = wait_for_codes(&recorder);
            sender.send(()).unwrap();
            assert_eq!(codes, vec![EXIT_DEADLINE_OVERRUN]);
            assert_eq!(
                read_reply(&mut stream),
                Some(Reply::Error {
                    id: 12,
                    code: ErrorCode::DeadlineOverrun
                })
            );
            assert!(is_closed(&mut stream));
        });
        let end = serve(&listener, &mut engine, &recorder, &stop, MAX_DEADLINE);
        assert_eq!(end.unwrap(), ServeEnd::Fatal(FatalReason::DeadlineOverrun));
    });
    assert_eq!(recorder.codes(), vec![EXIT_DEADLINE_OVERRUN]);
    assert_eq!(engine.calls(), 1);
}

#[test]
fn requests_within_their_deadline_are_served_and_the_watchdog_stays_quiet() {
    let path = socket_path("dl-within");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let mut engine = Quick::new();
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            let tensor = &ZERO_TENSOR;
            for id in [1u64, 2, 3] {
                stream
                    .write_all(
                        &Request::InferRequest {
                            id,
                            deadline_ms: 10_000,
                            tensor,
                        }
                        .encode(),
                    )
                    .unwrap();
                let reply = read_reply(&mut stream).unwrap();
                let Reply::Rows { id: got, rows } = reply else {
                    panic!("expected a Rows reply, got {reply:?}");
                };
                assert_eq!(got, id);
                assert!(rows.is_empty());
            }
            stream
                .write_all(
                    &Request::InferRawRequest {
                        id: 4,
                        deadline_ms: 10_000,
                        tensor,
                    }
                    .encode(),
                )
                .unwrap();
            let reply = read_reply(&mut stream).unwrap();
            let Reply::Raw { id, values } = reply else {
                panic!("expected a Raw reply, got {reply:?}");
            };
            assert_eq!(id, 4);
            assert_eq!(values.len(), RAW_VALUES);
            assert!(
                values
                    .iter()
                    .all(|value| value.to_bits() == 0.0f32.to_bits())
            );
        });
        let end = serve(&listener, &mut engine, &recorder, &stop, MAX_DEADLINE);
        assert_eq!(end.unwrap(), ServeEnd::Stopped);
    });
    assert_eq!(recorder.codes(), Vec::<i32>::new());
    assert_eq!(engine.calls(), 4);
}

#[test]
fn a_finished_request_is_not_cut_off_by_its_old_deadline() {
    let path = socket_path("dl-olddl");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let mut engine = Quick::new();
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            let tensor = &ZERO_TENSOR;
            stream
                .write_all(
                    &Request::InferRequest {
                        id: 1,
                        deadline_ms: 100,
                        tensor,
                    }
                    .encode(),
                )
                .unwrap();
            let reply = read_reply(&mut stream).unwrap();
            assert!(matches!(reply, Reply::Rows { id: 1, .. }));
            thread::sleep(Duration::from_millis(400));
            stream
                .write_all(
                    &Request::HealthRequest {
                        id: 2,
                        deadline_ms: 0,
                    }
                    .encode(),
                )
                .unwrap();
            assert_eq!(read_reply(&mut stream), Some(Reply::Health { id: 2 }));
        });
        let end = serve(&listener, &mut engine, &recorder, &stop, MAX_DEADLINE);
        assert_eq!(end.unwrap(), ServeEnd::Stopped);
    });
    assert_eq!(recorder.codes(), Vec::<i32>::new());
}

#[test]
fn a_zero_deadline_infer_or_infer_raw_request_answers_bad_request_without_reaching_the_engine() {
    let path = socket_path("dl-zero");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let mut engine = Quick::new();
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            let tensor = &ZERO_TENSOR;
            stream
                .write_all(
                    &Request::InferRequest {
                        id: 21,
                        deadline_ms: 0,
                        tensor,
                    }
                    .encode(),
                )
                .unwrap();
            assert_eq!(
                read_reply(&mut stream),
                Some(Reply::Error {
                    id: 21,
                    code: ErrorCode::BadRequest
                })
            );
            stream
                .write_all(
                    &Request::InferRawRequest {
                        id: 22,
                        deadline_ms: 0,
                        tensor,
                    }
                    .encode(),
                )
                .unwrap();
            assert_eq!(
                read_reply(&mut stream),
                Some(Reply::Error {
                    id: 22,
                    code: ErrorCode::BadRequest
                })
            );
            stream
                .write_all(
                    &Request::HealthRequest {
                        id: 23,
                        deadline_ms: 0,
                    }
                    .encode(),
                )
                .unwrap();
            assert_eq!(read_reply(&mut stream), Some(Reply::Health { id: 23 }));
        });
        let end = serve(&listener, &mut engine, &recorder, &stop, MAX_DEADLINE);
        assert_eq!(end.unwrap(), ServeEnd::Stopped);
    });
    assert_eq!(engine.calls(), 0);
    assert_eq!(recorder.codes(), Vec::<i32>::new());
}

#[test]
fn a_panic_in_the_engine_ends_serve_instead_of_hanging_it() {
    let path = socket_path("dl-panic");
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let stop = AtomicBool::new(false);
        let recorder = Recorder::new();
        serve(&listener, &mut Panicking, &recorder, &stop, MAX_DEADLINE)
    });
    let mut stream = connect(&path);
    stream
        .write_all(
            &Request::InferRequest {
                id: 31,
                deadline_ms: 10_000,
                tensor: &ZERO_TENSOR,
            }
            .encode(),
        )
        .unwrap();
    for _ in 0..2000 {
        if server.is_finished() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(server.is_finished());
    assert!(server.join().is_err());
}
