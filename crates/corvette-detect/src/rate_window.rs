//! Samples two running frame totals on a one-second window and publishes whole tenths of a frame per second.

use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(1);
const NANOS_PER_SEC: u128 = 1_000_000_000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rates {
    pub fps_in_tenths: u32,
    pub fps_processed_tenths: u32,
}

#[derive(Debug)]
pub struct RateWindow {
    window_start: Instant,
    last_processed: u64,
    last_arrived: u64,
    published: Rates,
}

impl RateWindow {
    #[must_use]
    pub fn new(start: Instant, frames_processed: u64, frames_arrived: u64) -> Self {
        Self {
            window_start: start,
            last_processed: frames_processed,
            last_arrived: frames_arrived,
            published: Rates::default(),
        }
    }

    /// Feeds the running totals in; once the window is a second old its rates complete and a new window starts at `now`.
    pub fn observe(&mut self, now: Instant, frames_processed: u64, frames_arrived: u64) {
        let Some(elapsed) = now.checked_duration_since(self.window_start) else {
            return;
        };
        if elapsed < WINDOW {
            return;
        }
        self.published = Rates {
            fps_in_tenths: tenths(frames_arrived.saturating_sub(self.last_arrived), elapsed),
            fps_processed_tenths: tenths(
                frames_processed.saturating_sub(self.last_processed),
                elapsed,
            ),
        };
        self.window_start = now;
        self.last_processed = frames_processed;
        self.last_arrived = frames_arrived;
    }

    #[must_use]
    pub const fn rates(&self) -> Rates {
        self.published
    }
}

// Whole tenths rounded to nearest in fixed point: scale by 10 and add half an elapsed second before dividing.
fn tenths(delta: u64, elapsed: Duration) -> u32 {
    let elapsed_nanos = elapsed.as_nanos();
    let scaled = u128::from(delta) * 10 * NANOS_PER_SEC;
    let rounded = (scaled + elapsed_nanos / 2) / elapsed_nanos;
    u32::try_from(rounded).unwrap_or(u32::MAX)
}
