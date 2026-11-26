//! The process-wide one-shot stop signal and the shutdown drain that finalizes recording.

use crate::RecordUnit;
use crate::RunningCamera;
use crate::record_role::RecordingRuntime;
use crate::supervise::log_event;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// Finalization budget after a stop signal: a quarter of the kubelet default 30 s grace.
pub const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(8);

#[derive(Debug, PartialEq, Eq)]
pub enum BridgeExit {
    Served,
    Stopped,
    DeadlineExceeded,
}

#[derive(Clone)]
pub struct Shutdown(watch::Receiver<bool>);

impl std::fmt::Debug for Shutdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shutdown")
            .field("triggered", &self.is_triggered())
            .finish()
    }
}

pub struct ShutdownTrigger(watch::Sender<bool>);

impl std::fmt::Debug for ShutdownTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShutdownTrigger").finish_non_exhaustive()
    }
}

impl Shutdown {
    #[must_use]
    pub fn channel() -> (ShutdownTrigger, Self) {
        let (sender, receiver) = watch::channel(false);
        (ShutdownTrigger(sender), Self(receiver))
    }

    #[must_use]
    pub fn is_triggered(&self) -> bool {
        *self.0.borrow()
    }

    pub async fn wait(&self) {
        let mut receiver = self.0.clone();
        while !self.is_triggered() {
            if receiver.changed().await.is_err() {
                std::future::pending::<()>().await; // trigger gone: no stop can arrive; keep serving
            }
        }
    }
}

impl ShutdownTrigger {
    pub fn trigger(&self) {
        let _ = self.0.send(true); // an error means every receiver is gone: nothing left to stop
    }
}

/// The clean-stop body: each camera's record role is awaited until it flips settled,
/// then dropping the runtime releases the last index-row senders so the writer thread
/// commits and joins. The caller bounds this with [`SHUTDOWN_DEADLINE`].
pub(crate) async fn drain_for_stop(
    cameras: Vec<RunningCamera>,
    recording: Option<Arc<RecordingRuntime>>,
    events: Option<tokio::task::JoinHandle<()>>,
) {
    for camera in cameras {
        let RunningCamera {
            client,
            restream_feed,
            fmp4_repackage,
            hls_segment,
            record,
        } = camera;
        drop((restream_feed, fmp4_repackage, hls_segment)); // these roles hold no files on disk
        if let Some(RecordUnit {
            supervised,
            mut settled,
        }) = record
        {
            if settled.wait_for(|done| *done).await.is_err() {
                log_event(
                    "record",
                    "settle-lost",
                    "record role ended without finalizing",
                );
            }
            drop(supervised); // aborts the worker parked after finalizing (no-op if it returned)
        }
        drop(client);
    }
    if let Some(task) = events
        && let Err(join_error) = task.await
    {
        log_event("events", "consumer-join", join_error);
    }
    let Some(runtime) = recording else {
        return;
    };
    let Some(runtime) = Arc::into_inner(runtime) else {
        log_event(
            "index",
            "runtime-still-shared",
            "a task still holds the recording runtime; writer join skipped",
        );
        return;
    };
    let writer = runtime.into_writer();
    match tokio::task::spawn_blocking(move || writer.join()).await {
        Ok(Ok(_index)) => {}
        Ok(Err(_panic)) => log_event("index", "writer-panic", "index writer thread panicked"),
        Err(join_error) => log_event("index", "writer-join", join_error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervise::spawn_supervised_with;
    use corvette_rtsp_client::client::{CameraConfig, Client};
    use corvette_rtsp_client::session::Credentials;
    use std::net::IpAddr;

    fn dialing_nothing() -> Arc<Client> {
        let config = CameraConfig::new(
            "stuck",
            IpAddr::from([127, 0, 0, 1]),
            1,
            "/",
            Credentials::new(String::new(), String::new()),
        );
        Arc::new(Client::new(config))
    }

    fn hang() -> std::future::Pending<()> {
        std::future::pending()
    }

    #[tokio::test]
    async fn firing_the_trigger_resolves_waiters_and_flags_the_state() {
        let (trigger, stop) = Shutdown::channel();
        assert!(!stop.is_triggered());
        let waiter_stop = stop.clone();
        let waiter = tokio::spawn(async move { waiter_stop.wait().await });
        trigger.trigger();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("the waiter wakes on the trigger")
            .expect("the waiter does not panic");
        assert!(stop.is_triggered());
        stop.wait().await;
    }

    #[tokio::test]
    async fn a_record_role_that_never_settles_runs_out_the_deadline() {
        let (trigger, _stop) = Shutdown::channel();
        let (_settled_tx, settled_rx) = watch::channel(false);
        let camera = RunningCamera {
            client: dialing_nothing(),
            restream_feed: spawn_supervised_with("idle".to_string(), hang),
            fmp4_repackage: spawn_supervised_with("idle/fmp4".to_string(), hang),
            hls_segment: spawn_supervised_with("idle/hls".to_string(), hang),
            record: Some(RecordUnit {
                supervised: spawn_supervised_with("stuck/record".to_string(), hang),
                settled: settled_rx,
            }),
        };
        trigger.trigger();
        let drained = tokio::time::timeout(
            Duration::from_millis(100),
            drain_for_stop(vec![camera], None, None),
        )
        .await;
        assert!(
            drained.is_err(),
            "an unsettled record role must keep the drain from finishing"
        );
    }
}
