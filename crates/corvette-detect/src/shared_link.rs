//! One process-wide inference link that every camera thread queues behind.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use corvette_detect_proto::{Row, TENSOR_BYTES};

use crate::candidate_stage::Infer;
use crate::engine::EngineError;
use crate::worker_link::{LinkState, WorkerLink};

fn saturating_ms(elapsed: Duration) -> u32 {
    u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX)
}

// A panic elsewhere poisoned the lock; the link is plain data, so the recovered guard stays valid.
fn lock_link(link: &Arc<Mutex<WorkerLink>>) -> MutexGuard<'_, WorkerLink> {
    match link.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Clone, Debug)]
pub struct SharedLink {
    link: Arc<Mutex<WorkerLink>>,
}

impl SharedLink {
    #[must_use]
    pub fn new(link: WorkerLink) -> Self {
        Self {
            link: Arc::new(Mutex::new(link)),
        }
    }

    #[must_use]
    pub fn state(&self) -> LinkState {
        lock_link(&self.link).state()
    }

    #[must_use]
    pub fn infer_handle(&self) -> SharedInfer {
        SharedInfer {
            link: Arc::clone(&self.link),
            timings: LinkTimings::default(),
        }
    }
}

#[derive(Debug)]
pub struct SharedInfer {
    link: Arc<Mutex<WorkerLink>>,
    timings: LinkTimings,
}

impl SharedInfer {
    #[must_use]
    pub fn timings(&self) -> LinkTimings {
        self.timings.clone()
    }
}

impl Infer for SharedInfer {
    fn infer(&mut self, tensor: &[u8; TENSOR_BYTES]) -> Result<Vec<Row>, EngineError> {
        let waited_at = Instant::now();
        let (lock_wait, roundtrip, worker_ms, outcome) = {
            let mut link = lock_link(&self.link);
            let lock_wait = waited_at.elapsed();
            let started = Instant::now();
            let outcome = link.infer(tensor);
            let roundtrip = started.elapsed();
            let worker_ms = outcome.is_ok().then_some(link.last_worker_ms());
            drop(link);
            (lock_wait, roundtrip, worker_ms, outcome)
        };
        self.timings.record(lock_wait, roundtrip, worker_ms);
        outcome
    }
}

// Relaxed loads are enough: the three counters are independent diagnostics and nothing else synchronises through them.
#[derive(Clone, Debug, Default)]
pub struct LinkTimings {
    counters: Arc<LinkCounters>,
}

#[derive(Debug, Default)]
struct LinkCounters {
    lock_wait: AtomicU32,
    roundtrip: AtomicU32,
    worker: AtomicU32,
}

impl LinkTimings {
    fn record(&self, lock_wait: Duration, roundtrip: Duration, worker_ms: Option<u32>) {
        self.counters
            .lock_wait
            .store(saturating_ms(lock_wait), Ordering::Relaxed);
        self.counters
            .roundtrip
            .store(saturating_ms(roundtrip), Ordering::Relaxed);
        if let Some(worker_ms) = worker_ms {
            self.counters.worker.store(worker_ms, Ordering::Relaxed);
        }
    }

    #[must_use]
    pub fn lock_wait_ms(&self) -> u32 {
        self.counters.lock_wait.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn roundtrip_ms(&self) -> u32 {
        self.counters.roundtrip.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn worker_ms(&self) -> u32 {
        self.counters.worker.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use crate::backoff::DEFAULT_POLICY;

    use super::*;

    #[test]
    fn a_poisoned_link_lock_is_recovered_and_the_next_request_still_runs() {
        let socket = std::env::temp_dir().join("corvette-shared-link-poison-test.sock");
        let shared = SharedLink::new(WorkerLink::new(
            socket,
            Duration::from_secs(5),
            DEFAULT_POLICY,
        ));
        let poisoner = shared.clone();
        let crashed = thread::spawn(move || {
            let _guard = poisoner.link.lock().expect("link lock starts unpoisoned");
            panic!("dying while holding the link lock");
        });
        assert!(crashed.join().is_err(), "the poisoning thread must panic");
        let tensor_bytes = vec![0u8; TENSOR_BYTES];
        let tensor: &[u8; TENSOR_BYTES] =
            tensor_bytes.as_slice().try_into().expect("tensor length");
        let mut handle = shared.infer_handle();
        let outcome = handle.infer(tensor);
        assert!(matches!(outcome, Err(EngineError::Io(_))), "{outcome:?}");
        assert_eq!(shared.state(), LinkState::Unavailable);
    }

    #[test]
    fn milliseconds_saturate_at_the_u32_maximum() {
        assert_eq!(
            saturating_ms(Duration::from_millis(u64::from(u32::MAX) + 8)),
            u32::MAX
        );
        assert_eq!(saturating_ms(Duration::from_millis(7)), 7);
        assert_eq!(saturating_ms(Duration::from_micros(999)), 0);
    }
}
