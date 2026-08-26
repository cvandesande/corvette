//! Builds the fixed 12-byte RTP header (RFC 3550 §5.1) common to every
//! packetization mode this module emits -- the codec-specific logic in
//! sibling modules only ever appends its own payload bytes after this
//! header. Never a CSRC list, extension header, or padding: nothing this
//! crate produces needs any of them.

/// Fields needed to build one RTP packet's fixed header.
pub(super) struct RtpHeaderFields {
    pub(super) sequence_number: u16,
    pub(super) timestamp: u32,
    pub(super) ssrc: u32,
    pub(super) marker: bool,
    pub(super) payload_type: u8,
}

/// Builds one RTP packet: the fixed 12-byte header followed by `payload`.
pub(super) fn build_packet(fields: &RtpHeaderFields, payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(12 + payload.len());
    packet.push(0x80); // V=2, P=0, X=0, CC=0
    packet.push((u8::from(fields.marker) << 7) | (fields.payload_type & 0x7F));
    packet.extend_from_slice(&fields.sequence_number.to_be_bytes());
    packet.extend_from_slice(&fields.timestamp.to_be_bytes());
    packet.extend_from_slice(&fields.ssrc.to_be_bytes());
    packet.extend_from_slice(payload);
    packet
}

/// Scales a presentation timestamp, relative to the start of the stream,
/// into the codec's own RTP clock, wrapping at 32 bits the way RFC 3550's
/// own rolling RTP timestamp does.
///
/// Integer arithmetic (nanoseconds times the clock rate, divided back down)
/// keeps this exact for every duration this crate will ever see, including
/// ones long enough that scaling through `f64` would start losing whole
/// ticks. The final mask-and-`try_from` step (rather than an `as u32`
/// truncating cast) keeps the wraparound explicit and provably infallible,
/// rather than relying on a lossy cast clippy would otherwise have to be
/// told to ignore.
pub(super) fn scale_timestamp(pts: std::time::Duration, clock_rate_hz: u32) -> u32 {
    let ticks = pts.as_nanos() * u128::from(clock_rate_hz) / 1_000_000_000;
    let wrapped = ticks & u128::from(u32::MAX);
    u32::try_from(wrapped).expect("masked to the low 32 bits of `ticks`")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn builds_a_packet_with_no_csrc_extension_or_padding() {
        let packet = build_packet(
            &RtpHeaderFields {
                sequence_number: 0x0102,
                timestamp: 0x0304_0506,
                ssrc: 0x0708_090A,
                marker: true,
                payload_type: 96,
            },
            &[0xAA, 0xBB],
        );

        assert_eq!(
            packet,
            vec![
                0x80, // V=2, P=0, X=0, CC=0
                0xE0, // M=1, PT=96
                0x01, 0x02, // sequence number
                0x03, 0x04, 0x05, 0x06, // timestamp
                0x07, 0x08, 0x09, 0x0A, // SSRC
                0xAA, 0xBB, // payload
            ]
        );
    }

    #[test]
    fn marker_bit_and_payload_type_share_the_second_byte_independently() {
        let unmarked = build_packet(
            &RtpHeaderFields {
                sequence_number: 0,
                timestamp: 0,
                ssrc: 0,
                marker: false,
                payload_type: 97,
            },
            &[],
        );
        assert_eq!(unmarked[1], 97);

        let marked = build_packet(
            &RtpHeaderFields {
                sequence_number: 0,
                timestamp: 0,
                ssrc: 0,
                marker: true,
                payload_type: 97,
            },
            &[],
        );
        assert_eq!(marked[1], 0x80 | 0x61);
    }

    #[test]
    fn scales_zero_duration_to_zero_ticks() {
        assert_eq!(scale_timestamp(Duration::ZERO, 90_000), 0);
    }

    #[test]
    fn scales_one_second_to_exactly_the_clock_rate() {
        assert_eq!(scale_timestamp(Duration::from_secs(1), 90_000), 90_000);
        assert_eq!(scale_timestamp(Duration::from_secs(1), 48_000), 48_000);
    }

    #[test]
    fn scales_a_sub_second_duration_without_rounding_to_zero() {
        // Half a second at 90 kHz is a whole number of ticks (45,000): a
        // duration well under one second must still scale correctly, not
        // be truncated away to 0.
        assert_eq!(
            scale_timestamp(Duration::from_millis(500), 90_000),
            45_000,
            "half a second at a 90 kHz clock rate is 45,000 ticks"
        );
    }

    #[test]
    fn wraps_at_32_bits_like_a_real_rtp_clock() {
        // A 1 GHz clock rate makes ticks equal nanoseconds exactly, with no
        // fractional-tick rounding to reason about, isolating the
        // wraparound behavior this test actually targets.
        let clock_rate_hz = 1_000_000_000;
        let just_under_wrap = Duration::from_nanos(u64::from(u32::MAX));
        let one_tick = Duration::from_nanos(1);

        assert_eq!(scale_timestamp(just_under_wrap, clock_rate_hz), u32::MAX);
        assert_eq!(
            scale_timestamp(just_under_wrap + one_tick, clock_rate_hz),
            0,
            "the tick after u32::MAX must wrap to 0"
        );
    }
}
