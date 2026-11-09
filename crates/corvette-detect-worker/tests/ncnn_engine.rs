use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use corvette_detect_proto::{RAW_VALUES, Reply, Request, Row, TENSOR_BYTES};
use corvette_detect_worker::{Engine, NcnnEngine, ServeEnd, Terminator, serve};
use corvette_ncnn::Device;

const WRONG_PARAM: &str = concat!(
    "7767517\n",
    "4 4\n",
    "Input in0 0 1 in0 0=640 1=640 2=3\n",
    "Flatten flat 1 1 in0 flat0\n",
    "Crop crop 1 1 flat0 crop0 0=0 3=100\n",
    "Reshape out 1 1 crop0 out0 0=100 1=1 2=1\n",
);

static ZERO_TENSOR: [u8; TENSOR_BYTES] = [0u8; TENSOR_BYTES];

static ONES: [f32; RAW_VALUES] = [1.0f32; RAW_VALUES];

const PARAM: &str = concat!(
    "7767517\n",
    "4 4\n",
    "Input in0 0 1 in0 0=640 1=640 2=3\n",
    "Flatten flat 1 1 in0 flat0\n",
    "Crop crop 1 1 flat0 crop0 0=0 3=420000\n",
    "Reshape out 1 1 crop0 out0 0=8400 1=50 2=1\n",
);

fn write_net(tag: &str, param: &str) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(tag);
    fs::create_dir_all(&dir).unwrap();
    let param_path = dir.join("net.param");
    let bin_path = dir.join("net.bin");
    fs::write(&param_path, param).unwrap();
    fs::write(&bin_path, b"").unwrap();
    (param_path, bin_path)
}

fn patterned_tensor() -> Vec<u8> {
    (0..TENSOR_BYTES)
        .map(|k| u8::try_from((k * 7 + 3) % 256).unwrap())
        .collect()
}

fn planar_normalised(bytes: &[u8]) -> Vec<f32> {
    let plane = 640 * 640;
    (0..RAW_VALUES)
        .map(|i| f32::from(bytes[(i % plane) * 3 + i / plane]) / 255.0)
        .collect()
}

#[test]
fn the_engine_returns_the_first_420000_normalised_values() {
    let (param, bin) = write_net("engine-normalised", PARAM);
    let mut engine = NcnnEngine::load(&param, &bin, Device::Cpu).unwrap();
    let tensor = patterned_tensor();
    let reference = planar_normalised(&tensor);
    let tensor = <&[u8; TENSOR_BYTES]>::try_from(tensor.as_slice()).unwrap();
    let raw = engine.infer(tensor).unwrap();
    assert_eq!(raw.len(), RAW_VALUES);
    for (i, (got, want)) in raw.iter().zip(&reference).enumerate() {
        assert_eq!(got.to_bits(), want.to_bits(), "value {i} differs");
    }
}

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
    Some(Reply::decode(&read_frame(stream)?).unwrap())
}

fn mapped_rows(raw: &[f32; RAW_VALUES]) -> Vec<Row> {
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
fn the_worker_answers_infer_and_infer_raw_with_the_real_engine_over_a_socket() {
    let (param, bin) = write_net("worker-socket", PARAM);
    let mut engine = NcnnEngine::load(&param, &bin, Device::Cpu).unwrap();
    let path = socket_path("ncnnsock");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let pattern = patterned_tensor();
    let raw_reference = planar_normalised(&pattern);
    let ones = vec![255u8; TENSOR_BYTES];
    let ones_reference = mapped_rows(&ONES);
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let mut stream = connect(&path);
            let tensor = <&[u8; TENSOR_BYTES]>::try_from(pattern.as_slice()).unwrap();
            let frame = Request::InferRawRequest {
                id: 33,
                deadline_ms: 0,
                tensor,
            }
            .encode();
            stream.write_all(&frame).unwrap();
            let reply = read_reply(&mut stream).unwrap();
            let Reply::Raw { id, values: got } = reply else {
                panic!("expected a Raw reply, got {reply:?}");
            };
            assert_eq!(id, 33);
            assert_eq!(got.len(), RAW_VALUES);
            for (i, (got, want)) in got.iter().zip(&raw_reference).enumerate() {
                assert_eq!(got.to_bits(), want.to_bits(), "value {i} differs");
            }
            let tensor = <&[u8; TENSOR_BYTES]>::try_from(ones.as_slice()).unwrap();
            let frame = Request::InferRequest {
                id: 32,
                deadline_ms: 0,
                tensor,
            }
            .encode();
            stream.write_all(&frame).unwrap();
            let reply = read_reply(&mut stream).unwrap();
            let Reply::Rows { id, rows } = reply else {
                panic!("expected a Rows reply, got {reply:?}");
            };
            assert_eq!(id, 32);
            assert!(
                !rows.is_empty(),
                "an all-ones tensor yields at least one row"
            );
            assert_eq!(rows, ones_reference);
            let frame = Request::HealthRequest {
                id: 31,
                deadline_ms: 0,
            }
            .encode();
            stream.write_all(&frame).unwrap();
            assert_eq!(read_reply(&mut stream), Some(Reply::Health { id: 31 }));
        });
        let end = serve(&listener, &mut engine, &recorder, &stop);
        assert_eq!(end.unwrap(), ServeEnd::Stopped);
    });
    assert_eq!(recorder.codes(), Vec::<i32>::new());
}

#[test]
fn a_net_with_the_wrong_output_size_is_an_engine_error() {
    let (param, bin) = write_net("wrong-size", WRONG_PARAM);
    let mut engine = NcnnEngine::load(&param, &bin, Device::Cpu).unwrap();
    let result = engine.infer(&ZERO_TENSOR);
    let Err(error) = result else {
        panic!("expected an engine error for the 100-value net");
    };
    assert!(
        error.reason.contains("100"),
        "the reason names the actual length: {error}"
    );
    assert!(
        error.reason.contains("420000"),
        "the reason names the expected length: {error}"
    );
}
