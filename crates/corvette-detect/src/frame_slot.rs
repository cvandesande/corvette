//! Hands one producer thread's newest frame to one consumer thread, replacing and counting any frame the consumer had not taken.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use crate::reader::Frame;

/// What a take hands the consumer: a frame, a producer-reported gap, a timeout, or closure.
#[derive(Debug)]
pub enum SlotEvent {
    Frame(Frame),
    Gap,
    Timeout,
    Closed,
}

#[derive(Debug)]
pub struct SlotSender {
    shared: Arc<Shared>,
}

impl SlotSender {
    pub fn put(&self, frame: Frame) {
        self.shared.put(frame);
    }

    pub fn gap(&self) {
        self.shared.gap();
    }

    pub fn close(&self) {
        self.shared.close();
    }
}

impl Drop for SlotSender {
    fn drop(&mut self) {
        self.shared.close();
    }
}

#[derive(Debug)]
pub struct SlotReceiver {
    shared: Arc<Shared>,
}

impl SlotReceiver {
    /// Returns the newest ready event, waiting up to `timeout` for one; closure outranks a queued frame or gap.
    #[must_use]
    pub fn take(&self, timeout: Duration) -> SlotEvent {
        self.shared.take(timeout)
    }

    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.shared.dropped()
    }

    #[must_use]
    pub fn arrived(&self) -> u64 {
        self.shared.arrived()
    }

    pub fn close(&self) {
        self.shared.close();
    }

    /// A cloneable handle that closes the slot, so a stopper needs neither half.
    #[must_use]
    pub fn closer(&self) -> SlotCloser {
        SlotCloser {
            shared: Arc::clone(&self.shared),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SlotCloser {
    shared: Arc<Shared>,
}

impl SlotCloser {
    pub fn close(&self) {
        self.shared.close();
    }
}

#[must_use]
pub fn frame_slot() -> (SlotSender, SlotReceiver) {
    let shared = Shared::new();
    (
        SlotSender {
            shared: Arc::clone(&shared),
        },
        SlotReceiver { shared },
    )
}

#[derive(Debug)]
struct State {
    frame: Option<Frame>,
    gaps: u64,
    dropped: u64,
    arrived: u64,
    closed: bool,
}

#[derive(Debug)]
struct Shared {
    state: Mutex<State>,
    notice: Condvar,
}

impl Shared {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                frame: None,
                gaps: 0,
                dropped: 0,
                arrived: 0,
                closed: false,
            }),
            notice: Condvar::new(),
        })
    }

    // A panic elsewhere poisoned the lock; the slot state is plain data, so the recovered guard stays valid.
    fn lock(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn put(&self, frame: Frame) {
        {
            let mut state = self.lock();
            if state.frame.is_some() {
                state.dropped = state.dropped.saturating_add(1);
            }
            state.frame = Some(frame);
            state.arrived = state.arrived.saturating_add(1);
        }
        self.notice.notify_one();
    }

    fn gap(&self) {
        {
            let mut state = self.lock();
            state.gaps = state.gaps.saturating_add(1);
        }
        self.notice.notify_one();
    }

    fn close(&self) {
        self.lock().closed = true;
        self.notice.notify_all();
    }

    fn take(&self, timeout: Duration) -> SlotEvent {
        let (mut state, _ready) =
            match self
                .notice
                .wait_timeout_while(self.lock(), timeout, |state| {
                    !state.closed && state.frame.is_none() && state.gaps == 0
                }) {
                Ok(ready) => ready,
                Err(poisoned) => poisoned.into_inner(),
            };
        let ready = if state.closed {
            SlotEvent::Closed
        } else if let Some(frame) = state.frame.take() {
            SlotEvent::Frame(frame)
        } else if state.gaps > 0 {
            state.gaps -= 1;
            SlotEvent::Gap
        } else {
            SlotEvent::Timeout
        };
        drop(state);
        ready
    }

    fn dropped(&self) -> u64 {
        self.lock().dropped
    }

    fn arrived(&self) -> u64 {
        self.lock().arrived
    }
}
