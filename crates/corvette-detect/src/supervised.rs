//! A `Supervised` value runs a child process, restarts it with the backoff policy and kills it when dropped.

use std::process::ExitStatus;
use std::time::{Duration, Instant};

use tokio::process::{Child, Command};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::backoff::{Backoff, Policy};
use crate::log_event;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub starts: u64,
    pub exits: u64,
    pub spawn_failures: u64,
    pub running: bool,
    pub last_exit: Option<ExitStatus>,
}

#[derive(Debug)]
#[must_use = "dropping the guard kills the supervised child"]
pub struct Supervised {
    supervisor: JoinHandle<()>,
    status: watch::Receiver<Status>,
}

impl Supervised {
    pub fn spawn<M, S>(unit: String, policy: Policy, mut make_command: M, mut on_start: S) -> Self
    where
        M: FnMut() -> Command + Send + 'static,
        S: FnMut(&mut Child) + Send + 'static,
    {
        let (sender, receiver) = watch::channel(Status::default());
        let supervisor = tokio::spawn(async move {
            let mut backoff = Backoff::new(policy);
            loop {
                let mut command = make_command();
                // the supervisor owns this so a caller cannot forget it and leave the child running
                command.kill_on_drop(true);
                let started = Instant::now();
                let run_length = match command.spawn() {
                    Err(error) => {
                        log_event(&unit, "spawn_failed", &error);
                        sender.send_modify(|status| status.spawn_failures += 1);
                        Duration::ZERO
                    }
                    Ok(mut child) => {
                        // the hook runs before the start is reported so a piped stream is claimed first
                        on_start(&mut child);
                        sender.send_modify(|status| {
                            status.starts += 1;
                            status.running = true;
                        });
                        match child.wait().await {
                            Ok(exit) => {
                                log_event(&unit, "exit", exit);
                                sender.send_modify(|status| {
                                    status.exits += 1;
                                    status.last_exit = Some(exit);
                                    status.running = false;
                                });
                            }
                            Err(error) => {
                                log_event(&unit, "wait_failed", &error);
                                sender.send_modify(|status| status.running = false);
                            }
                        }
                        started.elapsed()
                    }
                };
                let delay = backoff.after_run(run_length);
                log_event(
                    &unit,
                    "restart",
                    format_args!("delay_ms={}", delay.as_millis()),
                );
                tokio::time::sleep(delay).await;
            }
        });
        Self {
            supervisor,
            status: receiver,
        }
    }

    #[must_use]
    pub fn status(&self) -> Status {
        *self.status.borrow()
    }

    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }
}

impl Drop for Supervised {
    fn drop(&mut self) {
        self.supervisor.abort();
    }
}
