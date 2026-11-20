use std::fmt;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use corvette_detect_proto::{HEADER_LEN, Reply, Request, Row, TENSOR_BYTES, parse_header};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Labels {
    names: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LabelError {
    NotANumber {
        line: usize,
        text: String,
    },
    IndexMismatch {
        line: usize,
        expected: u32,
        found: u32,
    },
    MissingName {
        line: usize,
    },
}

impl fmt::Display for LabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotANumber { line, text } => {
                write!(
                    f,
                    "label line {line}: index {text:?} is not a decimal number"
                )
            }
            Self::IndexMismatch {
                line,
                expected,
                found,
            } => {
                write!(
                    f,
                    "label line {line}: expected index {expected}, found {found}"
                )
            }
            Self::MissingName { line } => write!(f, "label line {line}: the name is empty"),
        }
    }
}

impl std::error::Error for LabelError {}

impl Labels {
    /// # Errors
    /// `NotANumber`, `IndexMismatch` or `MissingName`; each names the 1-based faulty line.
    pub fn parse(text: &str) -> Result<Self, LabelError> {
        let mut names: Vec<String> = Vec::new();
        let mut expected: u32 = 0;
        for (number, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let line_number = number + 1;
            let (index_text, name) = line.split_once(' ').unwrap_or((line, ""));
            let Ok(found) = index_text.parse::<u32>() else {
                return Err(LabelError::NotANumber {
                    line: line_number,
                    text: index_text.to_owned(),
                });
            };
            if found != expected {
                return Err(LabelError::IndexMismatch {
                    line: line_number,
                    expected,
                    found,
                });
            }
            let name = name.trim_end();
            if name.is_empty() {
                return Err(LabelError::MissingName { line: line_number });
            }
            names.push(name.to_owned());
            expected += 1;
        }
        Ok(Self { names })
    }

    #[must_use]
    pub fn name(&self, class_id: u32) -> Option<&str> {
        let index = usize::try_from(class_id).ok()?;
        self.names.get(index).map(String::as_str)
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.names.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineError {
    Io(String),
    Worker(String),
    BadFrame(String),
    IdMismatch { sent: u64, received: u64 },
    UnexpectedReply,
    ZeroDeadline,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(text) => write!(f, "worker socket: {text}"),
            Self::Worker(message) => write!(f, "worker error reply: {message}"),
            Self::BadFrame(text) => write!(f, "worker reply frame invalid: {text}"),
            Self::IdMismatch { sent, received } => {
                write!(f, "worker reply carries id {received}, expected {sent}")
            }
            Self::UnexpectedReply => {
                write!(f, "worker replied with a kind this client cannot use")
            }
            Self::ZeroDeadline => write!(f, "the deadline must be longer than zero"),
        }
    }
}

impl std::error::Error for EngineError {}

#[derive(Debug)]
pub struct WorkerClient {
    stream: UnixStream,
    request_deadline_ms: u32,
    next_id: u64,
    last_worker_ms: u32,
}

impl WorkerClient {
    /// # Errors
    /// `ZeroDeadline` for a zero deadline; `Io` when the socket cannot be opened or timed.
    pub fn connect(path: &Path, deadline: Duration) -> Result<Self, EngineError> {
        if deadline.is_zero() {
            return Err(EngineError::ZeroDeadline);
        }
        let stream = UnixStream::connect(path).map_err(|error| io_failure("connect", &error))?;
        let timeout = Some(deadline);
        stream
            .set_read_timeout(timeout)
            .map_err(|error| io_failure("set the read timeout", &error))?;
        stream
            .set_write_timeout(timeout)
            .map_err(|error| io_failure("set the write timeout", &error))?;
        let request_deadline_ms = u32::try_from(deadline.as_millis()).unwrap_or(u32::MAX);
        Ok(Self {
            stream,
            request_deadline_ms,
            next_id: 0,
            last_worker_ms: 0,
        })
    }

    /// # Errors
    /// `Io` on socket failure or timeout, any error spends the client; `Worker` on an error reply.
    pub fn infer(&mut self, tensor: &[u8; TENSOR_BYTES]) -> Result<Vec<Row>, EngineError> {
        let id = self.next_id;
        self.next_id += 1;
        let frame = Request::InferRequest {
            id,
            deadline_ms: self.request_deadline_ms,
            tensor,
        }
        .encode();
        self.stream
            .write_all(&frame)
            .map_err(|error| io_failure("send the request", &error))?;
        let reply = read_reply(&mut self.stream)?;
        let received = match reply {
            Reply::Rows { id, .. }
            | Reply::Raw { id, .. }
            | Reply::Health { id }
            | Reply::Error { id, .. } => id,
        };
        if received != id {
            return Err(EngineError::IdMismatch { sent: id, received });
        }
        match reply {
            Reply::Rows {
                rows, worker_ms, ..
            } => {
                self.last_worker_ms = worker_ms;
                Ok(rows)
            }
            Reply::Error { code, .. } => Err(EngineError::Worker(format!("{code:?}"))),
            Reply::Raw { .. } | Reply::Health { .. } => Err(EngineError::UnexpectedReply),
        }
    }

    #[must_use]
    pub const fn last_worker_ms(&self) -> u32 {
        self.last_worker_ms
    }
}

fn io_failure(operation: &str, error: &std::io::Error) -> EngineError {
    EngineError::Io(format!("{operation} failed: {error}"))
}

fn read_reply(stream: &mut UnixStream) -> Result<Reply, EngineError> {
    let mut header_bytes = [0u8; HEADER_LEN];
    stream
        .read_exact(&mut header_bytes)
        .map_err(|error| io_failure("read the reply header", &error))?;
    let header =
        parse_header(&header_bytes).map_err(|error| EngineError::BadFrame(error.to_string()))?;
    let total = HEADER_LEN
        .checked_add(header.body_len)
        .ok_or_else(|| EngineError::BadFrame(String::from("the reply length overflows")))?;
    let mut frame = vec![0u8; total];
    let (head, body) = frame.split_at_mut_checked(HEADER_LEN).ok_or_else(|| {
        EngineError::BadFrame(String::from("the reply is shorter than its header"))
    })?;
    head.copy_from_slice(&header_bytes);
    stream
        .read_exact(body)
        .map_err(|error| io_failure("read the reply body", &error))?;
    Reply::decode(&frame).map_err(|error| EngineError::BadFrame(error.to_string()))
}
