use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use corvette_detect_proto::{ErrorCode, HEADER_LEN, RAW_VALUES, Reply, Request, TENSOR_BYTES};
use corvette_detect_worker::{NcnnEngine, ServeEnd, Terminator, serve};
use corvette_ncnn::Device;

const PLANE: usize = 640 * 640;

const PARAM: &str = concat!(
    "7767517\n",
    "4 4\n",
    "Input in0 0 1 in0 0=640 1=640 2=3\n",
    "Flatten flat 1 1 in0 flat0\n",
    "Crop crop 1 1 flat0 crop0 0=0 3=420000\n",
    "Reshape out 1 1 crop0 out0 0=8400 1=50 2=1\n",
);

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

fn directory(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(tag);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the directory is created");
    dir
}

fn socket_path(tag: &str) -> PathBuf {
    fs::create_dir_all(env!("CARGO_TARGET_TMPDIR")).expect("the tmp directory exists");
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("rt-{tag}.sock"));
    assert!(
        path.as_os_str().as_encoded_bytes().len() < 90,
        "socket path too long"
    );
    let _ = fs::remove_file(&path);
    path
}

fn text_of(path: &Path) -> String {
    path.to_str().expect("a utf-8 path").to_string()
}

fn patterned_tensor(tensor: usize) -> Vec<u8> {
    (0..TENSOR_BYTES)
        .map(|j| u8::try_from((j * 7 + 3 + tensor * 13) % 256).expect("a residue below 256"))
        .collect()
}

fn fake_value(tensor: usize, index: usize) -> f32 {
    let small = u16::try_from(tensor * 1000 + (index % 97)).expect("a small reply value");
    f32::from(small)
}

fn read_frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut header = [0u8; HEADER_LEN];
    if stream.read_exact(&mut header).is_err() {
        return None;
    }
    let declared = usize::try_from(u32::from_le_bytes(
        header[2..6].try_into().expect("four header bytes"),
    ))
    .expect("a frame length");
    let mut frame = vec![0u8; HEADER_LEN + declared];
    frame[..HEADER_LEN].copy_from_slice(&header);
    stream.read_exact(&mut frame[HEADER_LEN..]).ok()?;
    Some(frame)
}

fn read_request_id(stream: &mut UnixStream) -> u64 {
    let frame = read_frame(stream).expect("a request frame");
    match Request::decode(&frame) {
        Ok(Request::InferRawRequest { id, .. }) => id,
        Ok(other) => panic!("expected a raw request, got {other:?}"),
        Err(error) => panic!("a decodable request: {error}"),
    }
}

fn read_request_recorded(stream: &mut UnixStream) -> (u64, u32, Vec<u8>) {
    let frame = read_frame(stream).expect("a request frame");
    match Request::decode(&frame) {
        Ok(Request::InferRawRequest {
            id,
            deadline_ms,
            tensor,
        }) => (id, deadline_ms, tensor.to_vec()),
        Ok(other) => panic!("expected a raw request, got {other:?}"),
        Err(error) => panic!("a decodable request: {error}"),
    }
}

fn send_reply(stream: &mut UnixStream, reply: &Reply) {
    let bytes = reply.encode().expect("an encoded reply");
    stream.write_all(&bytes).expect("a written reply");
}

fn read_values(path: &Path) -> Vec<f32> {
    fs::read(path)
        .expect("a value file")
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four bytes")))
        .collect()
}

#[test]
fn run_sends_each_tensor_in_order_and_writes_the_replies_and_latencies() {
    let inputs = directory("run-order-in");
    let out = directory("run-order-out");
    for tensor in 0..3usize {
        fs::write(
            inputs.join(format!("tensor-0000{tensor}.rgb")),
            patterned_tensor(tensor),
        )
        .expect("a tensor file");
    }
    let sock = socket_path("run-order");
    let listener = UnixListener::bind(&sock).expect("the fake worker listens");
    let in_text = text_of(&inputs);
    let out_text = text_of(&out);
    let sock_text = text_of(&sock);
    thread::scope(|scope| {
        let client = scope.spawn(|| {
            run(&[
                "run",
                "--socket",
                &sock_text,
                "--in",
                &in_text,
                "--out",
                &out_text,
                "--deadline-ms",
                "1234",
            ])
        });
        let fake = scope.spawn(|| {
            let (mut stream, _) = listener.accept().expect("one connection");
            let mut seen: Vec<(u64, u32, Vec<u8>)> = Vec::new();
            for tensor in 0..3usize {
                let (id, deadline_ms, body) = read_request_recorded(&mut stream);
                seen.push((id, deadline_ms, body));
                let values: Vec<f32> = (0..RAW_VALUES).map(|i| fake_value(tensor, i)).collect();
                send_reply(&mut stream, &Reply::Raw { id, values });
            }
            drop(stream);
            listener
                .set_nonblocking(true)
                .expect("a non-blocking listener");
            (seen, listener.accept().is_ok())
        });
        let output = client.join().expect("the client finished");
        let (seen, extra_connection) = fake.join().expect("the fake worker finished");
        assert_eq!(code(&output), 0);
        assert_eq!(
            stdout_text(&output),
            format!("ran 3 tensors into {out_text}\n")
        );
        assert_eq!(seen.len(), 3);
        for (tensor, (id, deadline_ms, body)) in seen.iter().enumerate() {
            assert_eq!(*id, u64::try_from(tensor).expect("a small index"));
            assert_eq!(*deadline_ms, 1234);
            assert_eq!(*body, patterned_tensor(tensor));
        }
        assert!(!extra_connection);
        for tensor in 0..3usize {
            let mut expected: Vec<u8> = Vec::with_capacity(RAW_VALUES * 4);
            for index in 0..RAW_VALUES {
                expected.extend_from_slice(&fake_value(tensor, index).to_le_bytes());
            }
            let got =
                fs::read(out.join(format!("output-0000{tensor}.f32"))).expect("an output file");
            assert_eq!(got.len(), RAW_VALUES * 4);
            assert_eq!(got, expected);
        }
        let latency_text = fs::read_to_string(out.join("latency.csv")).expect("the latency file");
        let lines: Vec<&str> = latency_text.lines().collect();
        assert_eq!(lines.first().copied(), Some("index,microseconds"));
        assert_eq!(lines.len(), 4);
        for (index, line) in lines[1..].iter().enumerate() {
            let (index_text, microseconds_text) = line.split_once(',').expect("a latency row");
            assert_eq!(index_text, index.to_string());
            let microseconds: u64 = microseconds_text.parse().expect("a microsecond count");
            assert!(microseconds > 0);
            assert!(microseconds < 10_000_000);
        }
        let mut held: Vec<String> = fs::read_dir(&out)
            .expect("the listing")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        held.sort();
        assert_eq!(
            held,
            vec![
                "latency.csv",
                "output-00000.f32",
                "output-00001.f32",
                "output-00002.f32"
            ]
        );
    });
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

fn write_net(tag: &str, param: &str) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(tag);
    fs::create_dir_all(&dir).expect("the net directory");
    let param_path = dir.join("net.param");
    let bin_path = dir.join("net.bin");
    fs::write(&param_path, param).expect("the param file");
    fs::write(&bin_path, b"").expect("the bin file");
    (param_path, bin_path)
}

fn planar_normalised(bytes: &[u8]) -> Vec<f32> {
    (0..RAW_VALUES)
        .map(|i| f32::from(bytes[(i % PLANE) * 3 + i / PLANE]) / 255.0)
        .collect()
}

#[test]
fn the_chain_tensors_run_compare_works_against_the_worker_engine() {
    let inputs = directory("run-chain-in");
    let out = directory("run-chain-out");
    let synthetic = run(&[
        "tensors",
        "synthetic",
        "--out",
        &text_of(&inputs),
        "--count",
        "3",
        "--seed",
        "5",
    ]);
    assert_eq!(code(&synthetic), 0);
    let (param, bin) = write_net("run-chain-net", PARAM);
    let mut engine = NcnnEngine::load(&param, &bin, Device::Cpu).expect("the cpu engine loads");
    let sock = socket_path("run-chain");
    let listener = UnixListener::bind(&sock).expect("the worker listens");
    let stop = AtomicBool::new(false);
    let recorder = Recorder::new();
    let in_text = text_of(&inputs);
    let out_text = text_of(&out);
    let sock_text = text_of(&sock);
    thread::scope(|scope| {
        scope.spawn(|| {
            let _guard = StopGuard(&stop);
            let output = run(&[
                "run", "--socket", &sock_text, "--in", &in_text, "--out", &out_text,
            ]);
            assert_eq!(code(&output), 0);
            assert_eq!(
                stdout_text(&output),
                format!("ran 3 tensors into {out_text}\n")
            );
        });
        let end = serve(
            &listener,
            &mut engine,
            &recorder,
            &stop,
            Duration::from_mins(1),
        );
        assert_eq!(end.expect("the serve loop ends"), ServeEnd::Stopped);
    });
    assert_eq!(recorder.codes(), Vec::<i32>::new());
    for tensor in 0..3usize {
        let bytes =
            fs::read(inputs.join(format!("tensor-0000{tensor}.rgb"))).expect("a tensor file");
        let got = read_values(&out.join(format!("output-0000{tensor}.f32")));
        let want = planar_normalised(&bytes);
        assert_eq!(got.len(), RAW_VALUES);
        for (index, (got, want)) in got.iter().zip(&want).enumerate() {
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "tensor {tensor} value {index} differs"
            );
        }
    }
    let same = run(&["compare", "--a", &out_text, "--b", &out_text]);
    assert_eq!(code(&same), 0);
    let same_text = stdout_text(&same);
    assert!(same_text.contains("max_abs_error 0\nfirst_difference none\n"));
    assert!(same_text.contains("latency a count 3 p50_us"));
    let out2 = directory("run-chain-out2");
    for entry in fs::read_dir(&out).expect("the listing") {
        let entry = entry.expect("an entry");
        fs::copy(entry.path(), out2.join(entry.file_name())).expect("a copied file");
    }
    let path2 = out2.join("output-00001.f32");
    let mut bytes = fs::read(&path2).expect("the second output file");
    let x = f32::from_le_bytes(bytes[20..24].try_into().expect("four bytes"));
    let y = x + 0.5;
    bytes[20..24].copy_from_slice(&y.to_le_bytes());
    fs::write(&path2, &bytes).expect("the rewritten output file");
    let differ = run(&["compare", "--a", &out_text, "--b", &text_of(&out2)]);
    assert_eq!(code(&differ), 1);
    let max_error = (f64::from(y) - f64::from(x)).abs();
    let differ_text = stdout_text(&differ);
    assert!(
        differ_text.contains(&format!("max_abs_error {max_error}\n")),
        "{differ_text}"
    );
    assert!(
        differ_text.contains(&format!("first_difference tensor 1 index 5 a {x} b {y}\n")),
        "{differ_text}"
    );
}

#[test]
fn a_failing_worker_ends_the_run_and_leaves_no_output_behind() {
    for case in [
        "error-reply",
        "rows-reply",
        "wrong-id",
        "cut-frame",
        "closed",
        "timeout",
    ] {
        failure_case(case);
    }
}

fn script_second_reply(stream: &mut UnixStream, case: &str) {
    let zeros = vec![0.0f32; RAW_VALUES];
    match case {
        "error-reply" => {
            read_request_id(stream);
            send_reply(
                stream,
                &Reply::Error {
                    id: 1,
                    code: ErrorCode::DeviceLost,
                },
            );
        }
        "rows-reply" => {
            read_request_id(stream);
            send_reply(
                stream,
                &Reply::Rows {
                    id: 1,
                    rows: Vec::new(),
                },
            );
        }
        "wrong-id" => {
            while read_frame(stream).is_some() {
                send_reply(
                    stream,
                    &Reply::Raw {
                        id: 7,
                        values: zeros.clone(),
                    },
                );
            }
        }
        "cut-frame" => {
            read_request_id(stream);
            let frame = Reply::Raw {
                id: 1,
                values: zeros,
            }
            .encode()
            .expect("an encoded frame");
            stream
                .write_all(&frame[..HEADER_LEN])
                .expect("a written header");
        }
        "closed" => {
            read_request_id(stream);
        }
        _ => {}
    }
}

fn failure_case(case: &str) {
    let inputs = directory(&format!("run-fail-{case}-in"));
    let out = directory(&format!("run-fail-{case}-out"));
    for tensor in 0..3usize {
        fs::write(
            inputs.join(format!("tensor-0000{tensor}.rgb")),
            patterned_tensor(tensor),
        )
        .expect("a tensor file");
    }
    let sock = socket_path(&format!("run-fail-{case}"));
    let listener = UnixListener::bind(&sock).expect("the fake worker listens");
    let in_text = text_of(&inputs);
    let out_text = text_of(&out);
    let sock_text = text_of(&sock);
    let deadline = if case == "timeout" { "1" } else { "10000" };
    thread::scope(|scope| {
        let client = scope.spawn(|| {
            run(&[
                "run",
                "--socket",
                &sock_text,
                "--in",
                &in_text,
                "--out",
                &out_text,
                "--deadline-ms",
                deadline,
            ])
        });
        let fake = scope.spawn(move || {
            let (mut stream, _) = listener.accept().expect("one connection");
            let id = read_request_id(&mut stream);
            send_reply(
                &mut stream,
                &Reply::Raw {
                    id,
                    values: vec![0.0f32; RAW_VALUES],
                },
            );
            script_second_reply(&mut stream, case);
            while read_frame(&mut stream).is_some() {}
        });
        let output = client.join().expect("the client finished");
        fake.join().expect("the fake worker finished");
        assert_eq!(code(&output), 3, "{case}");
        let error_text = stderr_text(&output);
        let lines: Vec<&str> = error_text.lines().collect();
        assert_eq!(lines.len(), 1, "{case}: {error_text}");
        assert!(lines[0].starts_with("error:"), "{case}: {error_text}");
        if case == "error-reply" {
            let first = lines[0];
            assert!(first.contains("tensor 1"), "names the tensor: {first}");
            assert!(first.contains("DeviceLost"), "names the code: {first}");
        }
        assert!(stdout_text(&output).is_empty(), "{case}");
        for entry in fs::read_dir(&out).expect("the listing") {
            let name = entry.expect("an entry").file_name();
            let text = name.to_string_lossy();
            assert!(
                !(text.starts_with("output-") && text.ends_with(".f32")) && text != "latency.csv",
                "{case} left {text} behind"
            );
        }
    });
}

#[test]
fn unusable_inputs_are_refused_before_anything_is_sent() {
    for case in [
        "no-tensors",
        "gap",
        "short-file",
        "out-holds-output",
        "out-holds-latency",
    ] {
        refusal_case(case);
    }
    missing_socket_case();
    extra_file_contrast();
}

fn refusal_case(case: &str) {
    let good = patterned_tensor(0);
    {
        let inputs = directory(&format!("run-refuse-{case}-in"));
        let out = directory(&format!("run-refuse-{case}-out"));
        let mut expected_held = 0usize;
        match case {
            "gap" => {
                fs::write(inputs.join("tensor-00000.rgb"), &good).expect("a tensor file");
                fs::write(inputs.join("tensor-00002.rgb"), &good).expect("a tensor file");
            }
            "short-file" => {
                fs::write(inputs.join("tensor-00000.rgb"), &good[..TENSOR_BYTES - 1])
                    .expect("a short tensor file");
            }
            "out-holds-output" => {
                fs::write(inputs.join("tensor-00000.rgb"), &good).expect("a tensor file");
                fs::write(out.join("output-00000.f32"), "keep").expect("the kept file");
                expected_held = 1;
            }
            "out-holds-latency" => {
                fs::write(inputs.join("tensor-00000.rgb"), &good).expect("a tensor file");
                fs::write(out.join("latency.csv"), "index,microseconds\n0,1\n")
                    .expect("the latency file");
                expected_held = 1;
            }
            _ => {}
        }
        let sock = socket_path(&format!("run-refuse-{case}"));
        let listener = UnixListener::bind(&sock).expect("the fake worker listens");
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");
        let output = run(&[
            "run",
            "--socket",
            &text_of(&sock),
            "--in",
            &text_of(&inputs),
            "--out",
            &text_of(&out),
        ]);
        assert_eq!(code(&output), 3, "{case}");
        assert_eq!(stderr_text(&output).lines().count(), 1, "{case}");
        assert!(stderr_text(&output).starts_with("error:"), "{case}");
        assert!(stdout_text(&output).is_empty(), "{case}");
        assert!(listener.accept().is_err(), "{case} connected");
        let held = fs::read_dir(&out).expect("the listing").count();
        assert_eq!(held, expected_held, "{case}");
        if case == "out-holds-output" {
            assert_eq!(
                fs::read(out.join("output-00000.f32")).expect("the kept file"),
                b"keep"
            );
        }
    }
}

fn missing_socket_case() {
    let good = patterned_tensor(0);
    let inputs = directory("run-refuse-missing-socket-in");
    let out = directory("run-refuse-missing-socket-out");
    fs::write(inputs.join("tensor-00000.rgb"), &good).expect("a tensor file");
    let missing = socket_path("run-refuse-gone");
    let output = run(&[
        "run",
        "--socket",
        &text_of(&missing),
        "--in",
        &text_of(&inputs),
        "--out",
        &text_of(&out),
    ]);
    assert_eq!(code(&output), 3);
    assert!(stderr_text(&output).starts_with("error:"));
    assert!(stdout_text(&output).is_empty());
    assert_eq!(fs::read_dir(&out).expect("the listing").count(), 0);
}

fn extra_file_contrast() {
    let good = patterned_tensor(0);
    let inputs = directory("run-extra-file-in");
    let out = directory("run-extra-file-out");
    fs::write(inputs.join("tensor-00000.rgb"), &good).expect("a tensor file");
    fs::write(inputs.join("tensor-00001.rgb.bak"), b"not a tensor file").expect("the extra file");
    let sock = socket_path("run-extra-file");
    let listener = UnixListener::bind(&sock).expect("the fake worker listens");
    let out_text = text_of(&out);
    let sock_text = text_of(&sock);
    thread::scope(|scope| {
        let client = scope.spawn(|| {
            run(&[
                "run",
                "--socket",
                &sock_text,
                "--in",
                &text_of(&inputs),
                "--out",
                &out_text,
            ])
        });
        let fake = scope.spawn(move || {
            let (mut stream, _) = listener.accept().expect("one connection");
            let id = read_request_id(&mut stream);
            send_reply(
                &mut stream,
                &Reply::Raw {
                    id,
                    values: vec![0.0f32; RAW_VALUES],
                },
            );
            while read_frame(&mut stream).is_some() {}
        });
        let output = client.join().expect("the client finished");
        fake.join().expect("the fake worker finished");
        assert_eq!(code(&output), 0);
        assert_eq!(
            stdout_text(&output),
            format!("ran 1 tensors into {out_text}\n")
        );
    });
}
