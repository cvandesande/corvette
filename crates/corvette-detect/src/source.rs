//! Runs a command under the supervisor and returns its decoded output one whole frame per call.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::process::{Child, ChildStdout, Command};
use tokio::sync::{mpsc, watch};

use crate::backoff::Policy;
use crate::clock::FrameClock;
use crate::ffmpeg::FrameSpec;
use crate::reader::{Frame, FrameReader, ReadError};
use crate::supervised::{Status, Supervised};

pub const STREAM_QUEUE_CAPACITY: usize = 1;

#[derive(Debug)]
pub enum SourceError {
    Ended,
    Read(ReadError),
    SpawnFailed { failures: u64 },
    Stopped,
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ended => f.write_str("the frame stream ended"),
            Self::Read(error) => write!(f, "{error}"),
            Self::SpawnFailed { failures } => {
                write!(
                    f,
                    "the frame command failed to start ({failures} failures so far)"
                )
            }
            Self::Stopped => f.write_str("the frame source stopped"),
        }
    }
}

impl std::error::Error for SourceError {}

#[derive(Debug)]
pub struct FrameSource {
    supervisor: Supervised,
    streams: mpsc::Receiver<ChildStdout>,
    status: watch::Receiver<Status>,
    dropped_streams: Arc<AtomicU64>,
    spec: FrameSpec,
    clock: FrameClock,
    reader: Option<FrameReader<ChildStdout>>,
    reported_failures: u64,
}

async fn read_next(
    reader: &mut Option<FrameReader<ChildStdout>>,
) -> Result<Option<Frame>, ReadError> {
    match reader {
        Some(reader) => reader.next_frame().await,
        None => std::future::pending().await,
    }
}

impl FrameSource {
    pub fn spawn<M>(
        unit: String,
        policy: Policy,
        spec: FrameSpec,
        clock: FrameClock,
        make_command: M,
    ) -> Self
    where
        M: FnMut() -> Command + Send + 'static,
    {
        let (sender, streams) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let dropped_streams = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&dropped_streams);
        let supervisor =
            Supervised::spawn(
                unit,
                policy,
                make_command,
                move |child: &mut Child| match child.stdout.take() {
                    Some(stdout) => {
                        if sender.try_send(stdout).is_err() {
                            counter.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    None => {
                        counter.fetch_add(1, Ordering::Relaxed);
                    }
                },
            );
        let status = supervisor.subscribe();
        Self {
            supervisor,
            streams,
            status,
            dropped_streams,
            spec,
            clock,
            reader: None,
            reported_failures: 0,
        }
    }

    /// # Errors
    /// Returns a [`SourceError`] when a child stream ends or fails, the command cannot start, or the supervisor has stopped.
    pub async fn next_frame(&mut self) -> Result<Frame, SourceError> {
        loop {
            let failures = self.status.borrow().spawn_failures;
            if failures > self.reported_failures {
                self.reported_failures = failures;
                return Err(SourceError::SpawnFailed { failures });
            }
            let reader = &mut self.reader;
            let streams = &mut self.streams;
            let status = &mut self.status;
            tokio::select! {
                biased;
                result = read_next(reader), if reader.is_some() => {
                    match result {
                        Ok(Some(frame)) => return Ok(frame),
                        Ok(None) => {
                            *reader = None;
                            return Err(SourceError::Ended);
                        }
                        Err(error) => {
                            *reader = None;
                            return Err(SourceError::Read(error));
                        }
                    }
                }
                stream = streams.recv(), if reader.is_none() => {
                    match stream {
                        Some(stdout) => {
                            *reader = Some(FrameReader::new(stdout, self.spec, self.clock));
                        }
                        None => return Err(SourceError::Stopped),
                    }
                }
                changed = status.changed() => {
                    if changed.is_err() {
                        return Err(SourceError::Stopped);
                    }
                }
            }
        }
    }

    #[must_use]
    pub fn status(&self) -> Status {
        self.supervisor.status()
    }

    #[must_use]
    pub fn dropped_streams(&self) -> u64 {
        self.dropped_streams.load(Ordering::Relaxed)
    }
}
