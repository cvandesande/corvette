use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use corvette_detect_proto::{ErrorCode, HEADER_LEN, ID_LEN, Reply, Request, Row, parse_header};
use corvette_yolo::decode::{DEFAULT_NMS_THRESHOLD, DEFAULT_SCORE_THRESHOLD};

use crate::engine::{Engine, EngineError};
use crate::fatal::{FatalReason, Terminator, fatal};

const ACCEPT_POLL: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServeEnd {
    Stopped,
    Fatal(FatalReason),
}

/// Serves one connection at a time until the stop flag ends it or an engine failure is fatal.
/// # Errors
/// Returns the I/O error of setting the listener non-blocking or of a failed accept.
pub fn serve<E: Engine>(
    listener: &UnixListener,
    engine: &mut E,
    terminator: &dyn Terminator,
    stop: &AtomicBool,
) -> Result<ServeEnd, io::Error> {
    listener.set_nonblocking(true)?;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(ServeEnd::Stopped);
        }
        match listener.accept() {
            Ok((stream, _addr)) => {
                if stream.set_nonblocking(false).is_ok()
                    && let ConnectionEnd::Fatal(reason) =
                        serve_connection(stream, engine, terminator)
                {
                    return Ok(ServeEnd::Fatal(reason));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ConnectionEnd {
    Closed,
    Fatal(FatalReason),
}

fn serve_connection<E: Engine>(
    mut stream: UnixStream,
    engine: &mut E,
    terminator: &dyn Terminator,
) -> ConnectionEnd {
    loop {
        let mut header_bytes = [0u8; HEADER_LEN];
        if stream.read_exact(&mut header_bytes).is_err() {
            return ConnectionEnd::Closed;
        }
        let Ok(header) = parse_header(&header_bytes) else {
            write_reply(
                &stream,
                &Reply::Error {
                    id: 0,
                    code: ErrorCode::BadRequest,
                },
            );
            return ConnectionEnd::Closed;
        };
        let mut frame = vec![0u8; HEADER_LEN + header.body_len];
        let Some((header_slot, body)) = frame.split_at_mut_checked(HEADER_LEN) else {
            return ConnectionEnd::Closed;
        };
        header_slot.copy_from_slice(&header_bytes);
        if stream.read_exact(body).is_err() {
            return ConnectionEnd::Closed;
        }
        let Ok(request) = Request::decode(&frame) else {
            let id = frame
                .get(HEADER_LEN..)
                .and_then(|body| body.first_chunk::<ID_LEN>())
                .map_or(0, |bytes| u64::from_le_bytes(*bytes));
            write_reply(
                &stream,
                &Reply::Error {
                    id,
                    code: ErrorCode::BadRequest,
                },
            );
            continue;
        };
        let reply = match request {
            Request::HealthRequest { id, .. } => Reply::Health { id },
            Request::InferRequest { id, tensor, .. } => match engine.infer(tensor) {
                Ok(raw) => Reply::Rows {
                    id,
                    rows: corvette_yolo::decode(
                        &raw,
                        DEFAULT_SCORE_THRESHOLD,
                        DEFAULT_NMS_THRESHOLD,
                    )
                    .into_iter()
                    .map(|detection| Row {
                        class_id: detection.class_id,
                        score: detection.score,
                        y_min: detection.y_min,
                        x_min: detection.x_min,
                        y_max: detection.y_max,
                        x_max: detection.x_max,
                    })
                    .collect(),
                },
                Err(error) => {
                    return engine_failure(&stream, id, &error, terminator);
                }
            },
            Request::InferRawRequest { id, tensor, .. } => match engine.infer(tensor) {
                Ok(raw) => Reply::Raw {
                    id,
                    values: raw.to_vec(),
                },
                Err(error) => {
                    return engine_failure(&stream, id, &error, terminator);
                }
            },
        };
        if !write_reply(&stream, &reply) {
            return ConnectionEnd::Closed;
        }
    }
}

fn engine_failure(
    stream: &UnixStream,
    id: u64,
    error: &EngineError,
    terminator: &dyn Terminator,
) -> ConnectionEnd {
    write_reply(
        stream,
        &Reply::Error {
            id,
            code: ErrorCode::DeviceLost,
        },
    );
    fatal(terminator, FatalReason::DeviceLost, &error.reason);
    ConnectionEnd::Fatal(FatalReason::DeviceLost)
}

fn write_reply(mut stream: &UnixStream, reply: &Reply) -> bool {
    reply
        .encode()
        .is_ok_and(|bytes| stream.write_all(&bytes).is_ok())
}
