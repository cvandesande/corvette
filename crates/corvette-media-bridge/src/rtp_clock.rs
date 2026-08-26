//! Converts a track's raw RTP timestamps into elapsed time.
//!
//! Raw RTP timestamps are `u32`, wrapping, ticking at a fixed clock rate,
//! unconverted per `corvette_rtsp_client::depacketize::Frame`'s own doc; this
//! module turns them into a monotonically non-decreasing [`Duration`]
//! relative to this stream's first observed frame.
//!
//! H.264/H.265 RTP always ticks at 90 kHz (RFC 6184/7798): the same fixed
//! rate `rtsp-restream`'s own X2 packetizer assumes when scaling a
//! `provider::Frame::timestamp` back to wire RTP
//! (`crates/rtsp-restream/src/packetize/h264.rs`'s `CLOCK_RATE_HZ`), so both
//! of this item's roles (the RTSP-restream feed and the MoQ-publish loop)
//! agree with that fixed rate for video.
//!
//! Each independent subscription (per camera per role, per DP-3) gets its own
//! `RtpClock` instance: they compute independent epochs, which is fine since
//! each feeds an entirely different downstream consumer with no shared
//! timeline requirement between them.

use std::time::Duration;

/// The clock rate every H.264/H.265 RTP payload uses, per RFC 6184 section
/// 5.1 / RFC 7798 section 4.1 ("RTP timestamp... clock rate of 90000").
pub const VIDEO_CLOCK_RATE_HZ: u32 = 90_000;

/// Tracks one track's RTP timestamp epoch and converts subsequent raw
/// timestamps into elapsed [`Duration`]s since the first one observed.
#[derive(Debug, Clone, Copy)]
pub struct RtpClock {
    clock_rate_hz: u32,
    first: Option<u32>,
}

impl RtpClock {
    /// A clock for H.264/H.265 video, per [`VIDEO_CLOCK_RATE_HZ`].
    #[must_use]
    pub const fn video() -> Self {
        Self {
            clock_rate_hz: VIDEO_CLOCK_RATE_HZ,
            first: None,
        }
    }

    /// Converts `rtp_timestamp` into the elapsed time since the first
    /// timestamp this clock ever saw, handling the RTP timestamp's own
    /// 32-bit wraparound via wrapping subtraction (a real gap of more than
    /// `u32::MAX` ticks -- about 13.25 hours at 90 kHz -- would alias, but no
    /// single reconnect-free run this project targets approaches that).
    ///
    /// The first call always returns [`Duration::ZERO`] (it seeds the
    /// epoch); every later call is relative to that seed.
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
