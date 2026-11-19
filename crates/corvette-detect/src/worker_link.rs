//! Keeps one worker connection and reconnects it only when the back-off says it is due.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use corvette_detect_proto::{Row, TENSOR_BYTES};

use crate::backoff::{Backoff, Policy};
use crate::engine::{EngineError, WorkerClient};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkState {
    Connected,
    Unavailable,
}

#[derive(Debug)]
pub struct WorkerLink {
    path: PathBuf,
    deadline: Duration,
    backoff: Backoff,
    client: Option<WorkerClient>,
    connected_at: Option<Instant>,
    retry_at: Instant,
}

impl WorkerLink {
    #[must_use]
    pub fn new(path: PathBuf, deadline: Duration, policy: Policy) -> Self {
        Self {
            path,
            deadline,
            backoff: Backoff::new(policy),
            client: None,
            connected_at: None,
            retry_at: Instant::now(),
        }
    }

    #[must_use]
    pub const fn state(&self) -> LinkState {
        if self.client.is_some() {
            LinkState::Connected
        } else {
            LinkState::Unavailable
        }
    }

    /// # Errors
    /// `Io` when gated, connecting or exchanging fails; `Worker` on an error reply.
    pub fn infer(&mut self, tensor: &[u8; TENSOR_BYTES]) -> Result<Vec<Row>, EngineError> {
        let mut client = if let Some(client) = self.client.take() {
            client
        } else {
            let now = Instant::now();
            if now < self.retry_at {
                return Err(EngineError::Io(format!(
                    "worker at {} is unavailable until its retry",
                    self.path.display()
                )));
            }
            match WorkerClient::connect(&self.path, self.deadline) {
                Ok(client) => {
                    self.connected_at = Some(now);
                    client
                }
                Err(error) => {
                    self.retry_at = now + self.backoff.after_run(Duration::ZERO);
                    return Err(error);
                }
            }
        };
        let outcome = client.infer(tensor);
        if matches!(&outcome, Ok(_) | Err(EngineError::Worker(_))) {
            self.client = Some(client);
        } else {
            let now = Instant::now();
            let ran_for = self
                .connected_at
                .take()
                .map_or(Duration::ZERO, |at| now - at);
            self.retry_at = now + self.backoff.after_run(ran_for);
        }
        outcome
    }
}
