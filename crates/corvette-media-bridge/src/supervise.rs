use std::future::Future;
use tokio::task::JoinHandle;

/// Spawns `make_worker()` as a task, respawning it on panic or return; dropping the returned guard stops both.
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

/// One structured log line on stderr; this workspace has no logging framework to add.
pub fn log_event(unit: &str, event: &str, detail: impl std::fmt::Display) {
    eprintln!("corvette-media-bridge unit=\"{unit}\" event={event} detail=\"{detail}\"");
}

#[derive(Debug)]
pub struct Supervised {
    supervisor: JoinHandle<()>,
}

impl Supervised {
    /// True only after the guard was dropped: a live supervisor respawns its worker forever.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.supervisor.is_finished()
    }
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
