//! Stamps frame numbers with times, from the wall clock or from a fixed start and frame rate.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::ffmpeg::FrameSpec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameClock {
    mode: Mode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Live,
    Replay { start: Duration, fps: u32 },
}

impl FrameClock {
    #[must_use]
    pub const fn live() -> Self {
        Self { mode: Mode::Live }
    }

    #[must_use]
    pub const fn replay(start: Duration, spec: FrameSpec) -> Self {
        Self {
            mode: Mode::Replay {
                start,
                fps: spec.fps(),
            },
        }
    }

    #[must_use]
    pub fn stamp(self, index: u64) -> Duration {
        match self.mode {
            Mode::Live => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO),
            Mode::Replay { start, fps } => {
                let rate = u64::from(fps);
                let seconds = index / rate;
                let nanos = (index % rate) * 1_000_000_000 / rate;
                let nanos = u32::try_from(nanos).unwrap_or(0);
                start.saturating_add(Duration::new(seconds, nanos))
            }
        }
    }
}
