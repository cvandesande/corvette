//! Restart-on-panic supervision for one camera's one role (INV-5(b), DP-3).
//!
//! Matches `corvette-rtsp-client`'s own established convention
//! (`crates/corvette-rtsp-client/src/client/task.rs`'s
//! `spawn_supervised_with`) shape-for-shape. That function is a private
//! implementation detail of issue #18's own crate (not part of its public
//! API, and this item does not modify that crate), so this is a fresh
//! implementation of the same pattern, not a shared or copied dependency.
//!
//! Each camera's RTSP-restream-feeding task and its MoQ-publish task each get
//! their own call to [`spawn_supervised_with`], so one role's panic for one
//! camera restarts only that same task -- never a sibling role for the same
//! camera, never another camera's task, matching INV-5.

use std::future::Future;
use tokio::task::JoinHandle;

/// Spawns `make_worker()`'s output as its own task and respawns it whenever
/// it ends.
///
/// That is whether by panic or (unexpectedly, since a real worker here runs
/// forever) by returning -- so a panic inside it is caught by the runtime as
/// a [`tokio::task::JoinError`] at this function's own boundary rather than
/// propagating to the caller or to any other supervised task.
///
/// Returns the supervisor's own [`JoinHandle`]; dropping the returned
/// [`Supervised`] guard aborts both the supervisor and whichever worker
/// attempt is currently running.
pub fn spawn_supervised_with<F, Fut>(name: String, make_worker: F) -> Supervised
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let supervisor = tokio::spawn(async move {
        loop {
            let worker = tokio::spawn(make_worker());
            match worker.await {
                Err(join_error) if join_error.is_cancelled() => return,
                Err(join_error) => log_event(&name, "panic", &join_error),
                Ok(()) => log_event(
                    &name,
                    "panic",
                    "worker returned without panicking; restarting anyway",
                ),
            }
        }
    });
    Supervised { supervisor }
}

/// Writes one structured log line, matching `corvette-rtsp-client`'s own
/// `eprintln!`-based convention: no metrics/logging-framework dependency
/// exists in this workspace, and this item does not add one.
pub fn log_event(unit: &str, event: &str, detail: impl std::fmt::Display) {
    eprintln!("corvette-media-bridge unit=\"{unit}\" event={event} detail=\"{detail}\"");
}

/// Owns a supervised task; dropping it stops the supervisor (and whichever
/// worker attempt is in flight), the same lifecycle
/// `corvette_rtsp_client::Client`'s own `Drop` gives its supervisor.
#[derive(Debug)]
pub struct Supervised {
    supervisor: JoinHandle<()>,
}

impl Drop for Supervised {
    fn drop(&mut self) {
        self.supervisor.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    use std::time::Duration;

    /// A stand-in worker: increments `ticks` on a steady interval, panicking
    /// on its `panic_after`th tick if that count is nonzero (used once, then
    /// never again). Mirrors `corvette-rtsp-client`'s own equivalent test
    /// double (`client/task.rs`'s `instrumented_worker`).
    async fn instrumented_worker(ticks: Arc<AtomicU32>, panic_after: Arc<AtomicUsize>) {
        loop {
            tokio::time::sleep(Duration::from_millis(5)).await;
            let remaining = panic_after.load(Ordering::SeqCst);
            if remaining == 1 {
                panic_after.store(0, Ordering::SeqCst);
                panic!(
                    "deliberately injected panic for corvette-media-bridge's own supervision test"
                );
            } else if remaining > 1 {
                panic_after.store(remaining - 1, Ordering::SeqCst);
            }
            ticks.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// INV-5(b): a panicking supervised worker recovers (resumes progress)
    /// without affecting a sibling's own progress (INV-5(a), which holds
    /// unconditionally under Tokio regardless, and is exercised here too
    /// simply as an incidental double-check, not as this test's own proof of
    /// (a)).
    #[tokio::test]
    async fn a_panicking_supervised_worker_recovers_without_affecting_a_sibling() {
        let panicking_ticks = Arc::new(AtomicU32::new(0));
        let panic_after = Arc::new(AtomicUsize::new(20));
        let _panicking = spawn_supervised_with("panicking".to_string(), {
            let panicking_ticks = Arc::clone(&panicking_ticks);
            let panic_after = Arc::clone(&panic_after);
            move || instrumented_worker(Arc::clone(&panicking_ticks), Arc::clone(&panic_after))
        });

        let healthy_ticks = Arc::new(AtomicU32::new(0));
        let _healthy = spawn_supervised_with("healthy".to_string(), {
            let healthy_ticks = Arc::clone(&healthy_ticks);
            move || instrumented_worker(Arc::clone(&healthy_ticks), Arc::new(AtomicUsize::new(0)))
        });

        tokio::time::sleep(Duration::from_millis(400)).await;

        let panicking_after_recovery = panicking_ticks.load(Ordering::SeqCst);
        let healthy_after = healthy_ticks.load(Ordering::SeqCst);

        assert!(
            panicking_after_recovery > 20,
            "the panicking worker's own supervisor must restart it and let it resume ticking, got {panicking_after_recovery} ticks"
        );
        assert!(
            healthy_after > 20,
            "a sibling worker that never panicked must keep ticking throughout, got {healthy_after} ticks"
        );
    }
}
