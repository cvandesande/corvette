use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use corvette_detect_proto::{
    ErrorCode, HEADER_LEN, ID_LEN, RAW_VALUES, Reply, Request, Row, TENSOR_BYTES, parse_header,
};
use corvette_yolo::decode::{DEFAULT_NMS_THRESHOLD, DEFAULT_SCORE_THRESHOLD};

use crate::engine::{Engine, EngineError};
use crate::fatal::{FatalReason, Terminator, fatal};

const ACCEPT_POLL: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServeEnd {
    Stopped,
    Fatal(FatalReason),
}

/// Serves one connection at a time until the stop flag ends it or an engine failure is fatal; no request runs longer than `max_deadline`.
/// # Errors
/// Returns the I/O error of setting the listener non-blocking, of starting the watchdog thread or of a failed accept.
pub fn serve<E: Engine>(
    listener: &UnixListener,
    engine: &mut E,
    terminator: &dyn Terminator,
    stop: &AtomicBool,
    max_deadline: Duration,
) -> Result<ServeEnd, io::Error> {
    listener.set_nonblocking(true)?;
    let watchdog = Watchdog::new(max_deadline);
    thread::scope(|scope| {
        thread::Builder::new()
            .name("corvette-detect-watchdog".to_string())
            .spawn_scoped(scope, || watchdog.run(terminator))?;
        let _quit = QuitOnDrop(&watchdog);
        accept_loop(listener, engine, terminator, stop, &watchdog)
    })
}

fn accept_loop<E: Engine>(
    listener: &UnixListener,
    engine: &mut E,
    terminator: &dyn Terminator,
    stop: &AtomicBool,
    watchdog: &Watchdog,
) -> Result<ServeEnd, io::Error> {
    loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(ServeEnd::Stopped);
        }
        match listener.accept() {
            Ok((stream, _addr)) => {
                if stream.set_nonblocking(false).is_ok()
                    && let ConnectionEnd::Fatal(reason) =
                        serve_connection(stream, engine, terminator, watchdog)
                {
                    break Ok(ServeEnd::Fatal(reason));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => break Err(error),
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
    watchdog: &Watchdog,
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
            Request::InferRequest {
                id,
                deadline_ms,
                tensor,
            } => match infer_with_deadline(engine, tensor, deadline_ms, watchdog) {
                Ok(raw) => Reply::Rows {
                    id,
                    rows: decoded_rows(&raw),
                },
                Err(InferFailure::ZeroDeadline) => {
                    bad_request(&stream, id);
                    continue;
                }
                Err(InferFailure::Engine(error)) => {
                    return engine_failure(&stream, id, &error, terminator);
                }
                Err(InferFailure::Overran) => return overrun(&stream, id),
            },
            Request::InferRawRequest {
                id,
                deadline_ms,
                tensor,
            } => match infer_with_deadline(engine, tensor, deadline_ms, watchdog) {
                Ok(raw) => Reply::Raw {
                    id,
                    values: raw.to_vec(),
                },
                Err(InferFailure::ZeroDeadline) => {
                    bad_request(&stream, id);
                    continue;
                }
                Err(InferFailure::Engine(error)) => {
                    return engine_failure(&stream, id, &error, terminator);
                }
                Err(InferFailure::Overran) => return overrun(&stream, id),
            },
        };
        if !write_reply(&stream, &reply) {
            return ConnectionEnd::Closed;
        }
    }
}

enum InferFailure {
    ZeroDeadline,
    Engine(EngineError),
    Overran,
}

fn infer_with_deadline<E: Engine>(
    engine: &mut E,
    tensor: &[u8; TENSOR_BYTES],
    deadline_ms: u32,
    watchdog: &Watchdog,
) -> Result<Box<[f32; RAW_VALUES]>, InferFailure> {
    if deadline_ms == 0 {
        return Err(InferFailure::ZeroDeadline);
    }
    watchdog.arm(Duration::from_millis(u64::from(deadline_ms)));
    let raw = engine.infer(tensor);
    let overran = watchdog.disarm();
    if overran {
        return Err(InferFailure::Overran);
    }
    raw.map_err(InferFailure::Engine)
}

fn decoded_rows(raw: &[f32; RAW_VALUES]) -> Vec<Row> {
    corvette_yolo::decode(raw, DEFAULT_SCORE_THRESHOLD, DEFAULT_NMS_THRESHOLD)
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

fn overrun(stream: &UnixStream, id: u64) -> ConnectionEnd {
    write_reply(
        stream,
        &Reply::Error {
            id,
            code: ErrorCode::DeadlineOverrun,
        },
    );
    ConnectionEnd::Fatal(FatalReason::DeadlineOverrun)
}

fn bad_request(stream: &UnixStream, id: u64) {
    write_reply(
        stream,
        &Reply::Error {
            id,
            code: ErrorCode::BadRequest,
        },
    );
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

#[derive(Clone, Copy)]
struct Armed {
    deadline: Instant,
    budget: Duration,
}

struct State {
    armed: Option<Armed>,
    fired: bool,
    quit: bool,
}

struct Watchdog {
    state: Mutex<State>,
    wake: Condvar,
    cap: Duration,
}

impl Watchdog {
    const fn new(cap: Duration) -> Self {
        Self {
            state: Mutex::new(State {
                armed: None,
                fired: false,
                quit: false,
            }),
            wake: Condvar::new(),
            cap,
        }
    }

    fn arm(&self, budget: Duration) {
        let budget = budget.min(self.cap);
        let Some(deadline) = Instant::now().checked_add(budget) else {
            return;
        };
        {
            let mut state = lock(&self.state);
            state.armed = Some(Armed { deadline, budget });
            state.fired = false;
        }
        self.wake.notify_all();
    }

    fn disarm(&self) -> bool {
        let mut state = lock(&self.state);
        state.armed = None;
        let fired = state.fired;
        state.fired = false;
        fired
    }

    fn quit(&self) {
        {
            let mut state = lock(&self.state);
            state.quit = true;
        }
        self.wake.notify_all();
    }

    fn run(&self, terminator: &dyn Terminator) {
        let mut state = lock(&self.state);
        loop {
            if state.quit {
                return;
            }
            match state.armed {
                Some(armed) => {
                    let now = Instant::now();
                    if now >= armed.deadline {
                        state.armed = None;
                        state.fired = true;
                        let detail = format!(
                            "request exceeded its {} ms deadline",
                            armed.budget.as_millis()
                        );
                        fatal(terminator, FatalReason::DeadlineOverrun, &detail);
                    } else {
                        state = match self.wake.wait_timeout(state, armed.deadline - now) {
                            Ok((guard, _timed_out)) => guard,
                            Err(poisoned) => PoisonError::into_inner(poisoned).0,
                        };
                    }
                }
                None => {
                    state = match self.wake.wait(state) {
                        Ok(guard) => guard,
                        Err(poisoned) => PoisonError::into_inner(poisoned),
                    };
                }
            }
        }
    }
}

struct QuitOnDrop<'a>(&'a Watchdog);

impl Drop for QuitOnDrop<'_> {
    fn drop(&mut self) {
        self.0.quit();
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => PoisonError::into_inner(poisoned),
    }
}
