use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use corvette_detect_proto::{
    ErrorCode, HEADER_LEN, MAX_BODY, RAW_VALUES, Reply, Request, Row, TENSOR_BYTES,
};
use corvette_detect_worker::{
    EXIT_DEADLINE_OVERRUN, EXIT_DEVICE_LOST, Engine, EngineError, FatalReason, ServeEnd,
    Terminator, fatal, serve,
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

struct Scripted {
    queue: Mutex<VecDeque<Result<Box<[f32; RAW_VALUES]>, EngineError>>>,
    calls: AtomicUsize,
}

impl Scripted {
    fn new(results: Vec<Result<Box<[f32; RAW_VALUES]>, EngineError>>) -> Self {
        Self {
            queue: Mutex::new(results.into()),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Engine for Scripted {
    fn infer(&mut self, _rgb: &[u8; TENSOR_BYTES]) -> Result<Box<[f32; RAW_VALUES]>, EngineError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the scripted engine has an answer for every call")
    }
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

fn request_frame(request: &Request<'_>) -> Vec<u8> {
    request.encode()
}

fn boxed(values: Vec<f32>) -> Box<[f32; RAW_VALUES]> {
    values.into_boxed_slice().try_into().unwrap()
}

static ZERO_TENSOR: [u8; TENSOR_BYTES] = [0u8; TENSOR_BYTES];

fn mapped_rows(raw: &[f32; corvette_yolo::decode::RAW_LEN]) -> Vec<Row> {
    corvette_yolo::decode(raw, 0.4, 0.4)
        .into_iter()
        .map(|detection| Row {
            class_id: detection.class_id,
            score: detection.score,
            y_min: detection.y_min,
            x_min: detection.x_min,
            y_max: detection.y_max,
            x_max: detection.x_max,
        })
        .collect()
}

#[test]
fn a_health_request_is_answered_with_its_id_and_never_reaches_the_engine() {
    let path = socket_path("health");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let mut engine = Scripted::new(Vec::new());
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            let frame = request_frame(&Request::HealthRequest {
                id: 7,
                deadline_ms: 0,
            });
            stream.write_all(&frame).unwrap();
            assert_eq!(read_reply(&mut stream), Some(Reply::Health { id: 7 }));
        });
        let end = serve(&listener, &mut engine, &recorder, &stop);
        assert_eq!(end.unwrap(), ServeEnd::Stopped);
    });
    assert_eq!(engine.calls(), 0);
    assert_eq!(recorder.codes(), Vec::<i32>::new());
}

#[test]
fn an_infer_request_returns_the_decoded_rows() {
    let path = socket_path("rows");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let mut raw = vec![0.0f32; RAW_VALUES];
    raw[5] = 320.0;
    raw[8400 + 5] = 160.0;
    raw[2 * 8400 + 5] = 160.0;
    raw[3 * 8400 + 5] = 80.0;
    raw[11 * 8400 + 5] = 0.9;
    let mut engine = Scripted::new(vec![Ok(boxed(raw.clone()))]);
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            let tensor = &ZERO_TENSOR;
            let frame = request_frame(&Request::InferRequest {
                id: 3,
                deadline_ms: 10_000,
                tensor,
            });
            stream.write_all(&frame).unwrap();
            let raw_ref =
                <&[f32; corvette_yolo::decode::RAW_LEN]>::try_from(raw.as_slice()).unwrap();
            let expected_rows = mapped_rows(raw_ref);
            let reply = read_reply(&mut stream).unwrap();
            let Reply::Rows { id, rows } = reply else {
                panic!("expected a Rows reply, got {reply:?}");
            };
            assert_eq!(id, 3);
            assert_eq!(rows.len(), 1);
            let row = &rows[0];
            assert_eq!(row.class_id, 7);
            assert_eq!(row.score.to_bits(), 0.9f32.to_bits());
            assert_eq!(row.y_min.to_bits(), 0.1875f32.to_bits());
            assert_eq!(row.x_min.to_bits(), 0.375f32.to_bits());
            assert_eq!(row.y_max.to_bits(), 0.3125f32.to_bits());
            assert_eq!(row.x_max.to_bits(), 0.625f32.to_bits());
            assert_eq!(rows, expected_rows);
        });
        let end = serve(&listener, &mut engine, &recorder, &stop);
        assert_eq!(end.unwrap(), ServeEnd::Stopped);
    });
    assert_eq!(recorder.codes(), Vec::<i32>::new());
}

#[test]
fn an_infer_raw_request_returns_the_exact_values() {
    let path = socket_path("raw");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let values: Vec<f32> = (0..RAW_VALUES)
        .map(|i| f32::from(u8::try_from(i % 97).unwrap()) * 0.25)
        .collect();
    let mut engine = Scripted::new(vec![Ok(boxed(values.clone()))]);
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            let tensor = &ZERO_TENSOR;
            let frame = request_frame(&Request::InferRawRequest {
                id: 9,
                deadline_ms: 10_000,
                tensor,
            });
            stream.write_all(&frame).unwrap();
            let reply = read_reply(&mut stream).unwrap();
            let Reply::Raw { id, values: got } = reply else {
                panic!("expected a Raw reply, got {reply:?}");
            };
            assert_eq!(id, 9);
            assert_eq!(got.len(), RAW_VALUES);
            for (i, (got, want)) in got.iter().zip(&values).enumerate() {
                assert_eq!(got.to_bits(), want.to_bits(), "value {i} differs");
            }
        });
        let end = serve(&listener, &mut engine, &recorder, &stop);
        assert_eq!(end.unwrap(), ServeEnd::Stopped);
    });
    assert_eq!(recorder.codes(), Vec::<i32>::new());
}

#[test]
fn an_engine_failure_answers_device_lost_and_ends_the_server() {
    let path = socket_path("devlost");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let mut engine = Scripted::new(vec![
        Ok(boxed(vec![0.0; RAW_VALUES])),
        Err(EngineError {
            reason: "the device went away".to_string(),
        }),
    ]);
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            let tensor = &ZERO_TENSOR;
            for id in [11u64, 12] {
                let frame = request_frame(&Request::InferRequest {
                    id,
                    deadline_ms: 10_000,
                    tensor,
                });
                stream.write_all(&frame).unwrap();
            }
            let first = read_reply(&mut stream).unwrap();
            assert!(matches!(first, Reply::Rows { id: 11, .. }));
            assert_eq!(
                read_reply(&mut stream),
                Some(Reply::Error {
                    id: 12,
                    code: ErrorCode::DeviceLost
                })
            );
            assert!(is_closed(&mut stream));
        });
        let end = serve(&listener, &mut engine, &recorder, &stop);
        assert_eq!(end.unwrap(), ServeEnd::Fatal(FatalReason::DeviceLost));
        assert_eq!(recorder.codes(), vec![EXIT_DEVICE_LOST]);
    });
    assert_eq!(engine.calls(), 2);
    assert_eq!(EXIT_DEVICE_LOST, 20);
}

#[test]
fn a_decode_failure_answers_bad_request_and_the_connection_survives() {
    let path = socket_path("badkind");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let mut engine = Scripted::new(Vec::new());
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            let mut frame = vec![1u8, 0x83];
            frame.extend_from_slice(&8u32.to_le_bytes());
            frame.extend_from_slice(&5u64.to_le_bytes());
            stream.write_all(&frame).unwrap();
            assert_eq!(
                read_reply(&mut stream),
                Some(Reply::Error {
                    id: 5,
                    code: ErrorCode::BadRequest
                })
            );
            let health = request_frame(&Request::HealthRequest {
                id: 6,
                deadline_ms: 0,
            });
            stream.write_all(&health).unwrap();
            assert_eq!(read_reply(&mut stream), Some(Reply::Health { id: 6 }));
        });
        let end = serve(&listener, &mut engine, &recorder, &stop);
        assert_eq!(end.unwrap(), ServeEnd::Stopped);
    });
    assert_eq!(engine.calls(), 0);
    assert_eq!(recorder.codes(), Vec::<i32>::new());
}

#[test]
fn a_bad_header_answers_bad_request_and_closes_the_connection() {
    let path = socket_path("badhdr");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let mut engine = Scripted::new(Vec::new());
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            stream.write_all(&[2u8, 0x01, 0u8, 0u8, 0u8, 0u8]).unwrap();
            assert_eq!(
                read_reply(&mut stream),
                Some(Reply::Error {
                    id: 0,
                    code: ErrorCode::BadRequest
                })
            );
            assert!(is_closed(&mut stream));
            drop(stream);
            let mut stream = connect(&path);
            let oversize = u32::try_from(MAX_BODY + 1).unwrap().to_le_bytes();
            stream
                .write_all(&[
                    1u8,
                    0x01,
                    oversize[0],
                    oversize[1],
                    oversize[2],
                    oversize[3],
                ])
                .unwrap();
            assert_eq!(
                read_reply(&mut stream),
                Some(Reply::Error {
                    id: 0,
                    code: ErrorCode::BadRequest
                })
            );
            assert!(is_closed(&mut stream));
            drop(stream);
            let mut stream = connect(&path);
            let health = request_frame(&Request::HealthRequest {
                id: 8,
                deadline_ms: 0,
            });
            stream.write_all(&health).unwrap();
            assert_eq!(read_reply(&mut stream), Some(Reply::Health { id: 8 }));
        });
        let end = serve(&listener, &mut engine, &recorder, &stop);
        assert_eq!(end.unwrap(), ServeEnd::Stopped);
    });
    assert_eq!(recorder.codes(), Vec::<i32>::new());
}

#[test]
fn connections_are_served_one_after_another() {
    let path = socket_path("serial");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let mut engine = Scripted::new(Vec::new());
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            for id in [21u64, 22] {
                let mut stream = connect(&path);
                let health = request_frame(&Request::HealthRequest { id, deadline_ms: 0 });
                stream.write_all(&health).unwrap();
                assert_eq!(read_reply(&mut stream), Some(Reply::Health { id }));
            }
        });
        let end = serve(&listener, &mut engine, &recorder, &stop);
        assert_eq!(end.unwrap(), ServeEnd::Stopped);
    });
    assert_eq!(recorder.codes(), Vec::<i32>::new());
}

#[test]
fn a_stop_request_ends_an_idle_server() {
    let path = socket_path("stopidle");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(true);
    let recorder = Recorder::new();
    let mut engine = Scripted::new(Vec::new());
    let end = serve(&listener, &mut engine, &recorder, &stop);
    assert_eq!(end.unwrap(), ServeEnd::Stopped);
    assert_eq!(recorder.codes(), Vec::<i32>::new());
    assert_eq!(engine.calls(), 0);
}

#[test]
fn fatal_reports_each_reasons_exit_code_to_the_terminator() {
    let lost = Recorder::new();
    fatal(&lost, FatalReason::DeviceLost, "one device fault");
    assert_eq!(lost.codes(), vec![EXIT_DEVICE_LOST]);
    let overrun = Recorder::new();
    fatal(&overrun, FatalReason::DeadlineOverrun, "one deadline fault");
    assert_eq!(overrun.codes(), vec![EXIT_DEADLINE_OVERRUN]);
    assert_eq!(EXIT_DEADLINE_OVERRUN, 21);
    assert_eq!(FatalReason::DeviceLost.exit_code(), EXIT_DEVICE_LOST);
}
