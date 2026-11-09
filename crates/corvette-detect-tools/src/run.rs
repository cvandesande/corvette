use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use corvette_detect_proto::{HEADER_LEN, RAW_VALUES, Reply, Request, TENSOR_BYTES, parse_header};

use crate::{EXIT_MATCH, Failure, args, files};

const RUN_FLAGS: &[&str] = &["socket", "in", "out", "deadline-ms"];
const MIN_DEADLINE_MS: u32 = 1;
const MAX_DEADLINE_MS: u32 = 600_000;
const DEFAULT_DEADLINE_MS: u32 = 10_000;
const TIMEOUT_MARGIN_MS: u64 = 5_000;
const VALUE_BYTES: usize = 4;
const LATENCY_HEADER: &[u8] = b"index,microseconds\n";

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let flags = args::Flags::parse(args, RUN_FLAGS).ok_or(Failure::Usage)?;
    let socket = flags.required("socket")?;
    let input = flags.required("in")?;
    let out = flags.required("out")?;
    let deadline_ms = flags.bounded(
        "deadline-ms",
        MIN_DEADLINE_MS,
        MAX_DEADLINE_MS,
        DEFAULT_DEADLINE_MS,
    )?;
    let tensors = files::tensor_set(input)?;
    let out_dir = Path::new(out);
    if files::holds_output_files(out_dir)? {
        return Err(Failure::Input(format!(
            "{out} already holds output files or a latency file"
        )));
    }
    fs::create_dir_all(out)
        .map_err(|error| Failure::Input(format!("cannot create {out}: {error}")))?;
    let mut stream = UnixStream::connect(socket)
        .map_err(|error| Failure::Input(format!("cannot connect to {socket}: {error}")))?;
    let timeout = Some(Duration::from_millis(
        u64::from(deadline_ms) + TIMEOUT_MARGIN_MS,
    ));
    stream
        .set_read_timeout(timeout)
        .map_err(|error| Failure::Input(format!("cannot set the reply timeout: {error}")))?;
    stream
        .set_write_timeout(timeout)
        .map_err(|error| Failure::Input(format!("cannot set the request timeout: {error}")))?;
    let mut cleanup = Cleanup::new(out_dir);
    let mut latencies: Vec<u64> = Vec::new();
    for (index, path) in tensors.iter().enumerate() {
        let id = u64::try_from(index)
            .map_err(|_| Failure::Input(format!("{input} holds too many tensors")))?;
        let raw = fs::read(path)
            .map_err(|error| Failure::Input(format!("cannot read {}: {error}", path.display())))?;
        let tensor = <&[u8; TENSOR_BYTES]>::try_from(raw.as_slice()).map_err(|_| {
            Failure::Input(format!("{} is shorter than a full tensor", path.display()))
        })?;
        let frame = Request::InferRawRequest {
            id,
            deadline_ms,
            tensor,
        }
        .encode();
        let started = Instant::now();
        stream
            .write_all(&frame)
            .map_err(|error| Failure::Input(format!("cannot send tensor {index}: {error}")))?;
        let reply = read_reply(&mut stream)?;
        let microseconds = u64::try_from(started.elapsed().as_micros())
            .map_err(|_| Failure::Input(String::from("a latency does not fit in microseconds")))?;
        let values = accepted_values(reply, id, index)?;
        let mut bytes = Vec::with_capacity(RAW_VALUES * VALUE_BYTES);
        for value in &values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let destination = out_dir.join(files::output_name(id));
        write_file(&destination, &bytes)?;
        cleanup.wrote(destination);
        latencies.push(microseconds);
    }
    let mut csv = Vec::with_capacity(LATENCY_HEADER.len() + latencies.len() * 16);
    csv.extend_from_slice(LATENCY_HEADER);
    for (index, microseconds) in latencies.iter().enumerate() {
        csv.extend_from_slice(format!("{index},{microseconds}\n").as_bytes());
    }
    write_file(&out_dir.join("latency.csv"), &csv)?;
    cleanup.keep();
    println!("ran {} tensors into {out}", tensors.len());
    Ok(ExitCode::from(EXIT_MATCH))
}

fn accepted_values(reply: Reply, id: u64, index: usize) -> Result<Vec<f32>, Failure> {
    match reply {
        Reply::Raw {
            id: reply_id,
            values,
        } if reply_id == id => {
            if values.len() != RAW_VALUES {
                return Err(Failure::Input(format!(
                    "tensor {index}: raw reply has {} values, expected {RAW_VALUES}",
                    values.len()
                )));
            }
            Ok(values)
        }
        Reply::Raw { id: reply_id, .. } => Err(Failure::Input(format!(
            "tensor {index}: raw reply carries id {reply_id}, expected {id}"
        ))),
        Reply::Error { code, .. } => Err(Failure::Input(format!(
            "tensor {index}: worker error {code:?}"
        ))),
        Reply::Rows { .. } => Err(Failure::Input(format!(
            "tensor {index}: reply is rows, not raw values"
        ))),
        Reply::Health { .. } => Err(Failure::Input(format!(
            "tensor {index}: reply is health, not raw values"
        ))),
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    fs::write(path, bytes)
        .map_err(|error| Failure::Input(format!("cannot write {}: {error}", path.display())))
}

fn read_reply(stream: &mut UnixStream) -> Result<Reply, Failure> {
    let mut header_bytes = [0u8; HEADER_LEN];
    stream
        .read_exact(&mut header_bytes)
        .map_err(|error| Failure::Input(format!("cannot read a reply header: {error}")))?;
    let header = parse_header(&header_bytes)
        .map_err(|error| Failure::Input(format!("reply header is not valid: {error}")))?;
    let total = HEADER_LEN
        .checked_add(header.body_len)
        .ok_or_else(|| Failure::Input(String::from("a reply frame length overflows")))?;
    let mut frame = vec![0u8; total];
    let Some((head, body)) = frame.split_at_mut_checked(HEADER_LEN) else {
        return Err(Failure::Input(String::from(
            "a reply frame is shorter than its header",
        )));
    };
    head.copy_from_slice(&header_bytes);
    stream
        .read_exact(body)
        .map_err(|error| Failure::Input(format!("cannot read a reply body: {error}")))?;
    Reply::decode(&frame)
        .map_err(|error| Failure::Input(format!("reply frame is not valid: {error}")))
}

struct Cleanup {
    written: Vec<PathBuf>,
    latency: PathBuf,
    keep: bool,
}

impl Cleanup {
    fn new(out: &Path) -> Self {
        Self {
            written: Vec::new(),
            latency: out.join("latency.csv"),
            keep: false,
        }
    }

    fn wrote(&mut self, path: PathBuf) {
        self.written.push(path);
    }

    const fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        for path in &self.written {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_file(&self.latency);
    }
}
