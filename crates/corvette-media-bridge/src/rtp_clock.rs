use std::time::Duration;

/// The clock rate every H.264/H.265 RTP payload uses, per RFC 6184 section
/// 5.1 / RFC 7798 section 4.1 ("RTP timestamp... clock rate of 90000").
pub const VIDEO_CLOCK_RATE_HZ: u32 = 90_000;

#[derive(Debug, Clone, Copy)]
pub struct RtpClock {
    clock_rate_hz: u32,
    first: Option<u32>,
}

impl RtpClock {
    #[must_use]
    pub const fn video() -> Self {
        Self {
            clock_rate_hz: VIDEO_CLOCK_RATE_HZ,
            first: None,
        }
    }

    /// A gap past the whole 32-bit range (~13.25 h at 90 kHz) aliases via wrapping subtraction.
    pub fn elapsed(&mut self, rtp_timestamp: u32) -> Duration {
        let first = *self.first.get_or_insert(rtp_timestamp);
        let ticks = rtp_timestamp.wrapping_sub(first);
        let nanos = (u64::from(ticks) * 1_000_000_000) / u64::from(self.clock_rate_hz);
        Duration::from_nanos(nanos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_call_seeds_the_epoch_at_zero() {
        let mut clock = RtpClock::video();
        assert_eq!(clock.elapsed(12_345), Duration::ZERO);
    }

    #[test]
    fn scales_one_second_of_ticks_at_the_video_clock_rate() {
        let mut clock = RtpClock::video();
        clock.elapsed(0);
        assert_eq!(clock.elapsed(90_000), Duration::from_secs(1));
    }

    #[test]
    fn handles_a_wraparound_past_u32_max() {
        let mut clock = RtpClock::video();
        let seed = u32::MAX - 44_999;
        clock.elapsed(seed); // seeds the epoch
        let after_wrap = seed.wrapping_add(90_000); // one second of ticks past the seed, wrapping past u32::MAX
        assert_eq!(clock.elapsed(after_wrap), Duration::from_secs(1));
    }
}
