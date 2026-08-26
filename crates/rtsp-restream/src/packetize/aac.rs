//! AAC (RFC 3640, `mpeg4-generic`) RTP packetization: wraps each raw access
//! unit with the AU-header framing `mpeg4-generic` mode requires, one
//! access unit per RTP packet -- matching [`crate::provider::Frame`]'s own
//! shape, where a single AAC frame is already exactly one access unit.

use super::PacketizeError;
use super::rtp_header::{RtpHeaderFields, build_packet, scale_timestamp};
use crate::provider::Frame;

/// This crate's fixed choice of RFC 3640 `aac-hbr` AU-header bit widths --
/// matching `sdp::aac_fmtp`'s own `sizelength=13;indexlength=3;
/// indexdeltalength=3` template exactly, so a real client parses this
/// packetizer's output the same way it parses the SDP that describes it.
/// One access unit per RTP packet needs only the size field (its index is
/// always 0); `indexdeltalength` is never used because this packetizer
/// never aggregates more than one access unit into a single packet.
const SIZE_LENGTH_BITS: u32 = 13;
const MAX_ACCESS_UNIT_SIZE: usize = (1 << SIZE_LENGTH_BITS) - 1; // 8191 bytes
const AU_HEADERS_LENGTH_BITS: u16 = 16; // one 16-bit AU-header: 13-bit size + 3-bit index

/// Packetizes one AAC track's raw access units into RTP payloads.
///
/// Construct one instance per track with that track's negotiated RTP
/// payload type, SSRC, and sample rate (AAC's RTP clock rate is its own
/// sample rate -- RFC 3640 does not resample to a fixed rate, unlike
/// H.264/H.265's fixed 90 kHz); call [`Self::packetize`] with each of that
/// track's [`Frame`]s, in timestamp order.
#[derive(Debug)]
pub struct AacPacketizer {
    payload_type: u8,
    ssrc: u32,
    clock_rate_hz: u32,
    next_sequence_number: u16,
}

impl AacPacketizer {
    #[must_use]
    pub const fn new(payload_type: u8, ssrc: u32, sample_rate_hz: u32) -> Self {
        Self {
            payload_type,
            ssrc,
            clock_rate_hz: sample_rate_hz,
            next_sequence_number: 0,
        }
    }

    /// Packetizes one raw access unit into the single RTP packet that
    /// carries it. The RTP marker bit is always set: every packet this
    /// packetizer emits already carries one complete, unfragmented access
    /// unit, so every packet is an access-unit boundary.
    ///
    /// # Errors
    ///
    /// Returns [`PacketizeError::AacAccessUnitTooLarge`] if `frame`'s
    /// payload is longer than 8191 bytes -- the largest size this crate's
    /// fixed 13-bit AU-header `sizelength` can declare.
    ///
    /// # Panics
    ///
    /// Never panics: the one `expect` below is unreachable because the
    /// length check just above it already rejects every input that would
    /// make it fail.
    pub fn packetize(&mut self, frame: &Frame) -> Result<Vec<u8>, PacketizeError> {
        let access_unit = &frame.payload;
        if access_unit.len() > MAX_ACCESS_UNIT_SIZE {
            return Err(PacketizeError::AacAccessUnitTooLarge {
                size: access_unit.len(),
                max: MAX_ACCESS_UNIT_SIZE,
            });
        }
        let size = u16::try_from(access_unit.len())
            .expect("checked above: access_unit.len() <= MAX_ACCESS_UNIT_SIZE < u16::MAX");
        let au_header: u16 = size << 3; // index = 0, in the low 3 bits

        let mut payload = Vec::with_capacity(4 + access_unit.len());
        payload.extend_from_slice(&AU_HEADERS_LENGTH_BITS.to_be_bytes());
        payload.extend_from_slice(&au_header.to_be_bytes());
        payload.extend_from_slice(access_unit);

        let timestamp = scale_timestamp(frame.timestamp, self.clock_rate_hz);
        let packet = build_packet(
            &RtpHeaderFields {
                sequence_number: self.next_sequence_number,
                timestamp,
                ssrc: self.ssrc,
                marker: true,
                payload_type: self.payload_type,
            },
            &payload,
        );
        self.next_sequence_number = self.next_sequence_number.wrapping_add(1);
        Ok(packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn wraps_an_access_unit_in_a_single_au_header() {
        let mut packetizer = AacPacketizer::new(97, 0xDEAD_BEEF, 48_000);
        let access_unit = vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        let frame = Frame {
            track_index: 1,
            timestamp: Duration::from_secs(1),
            payload: access_unit.clone(),
        };

        let packet = packetizer.packetize(&frame).expect("packetizes");

        let payload = &packet[12..];
        assert_eq!(
            u16::from_be_bytes([payload[0], payload[1]]),
            16,
            "AU-headers-length is 16 bits: one 13-bit size + 3-bit index"
        );
        let au_header = u16::from_be_bytes([payload[2], payload[3]]);
        assert_eq!(usize::from(au_header >> 3), access_unit.len());
        assert_eq!(au_header & 0b111, 0, "index is 0");
        assert_eq!(&payload[4..], &access_unit[..]);

        assert_eq!(packet[1] & 0x80, 0x80, "marker bit is always set");
        let timestamp = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
        assert_eq!(timestamp, 48_000, "one second at a 48 kHz clock rate");
    }

    #[test]
    fn sequence_numbers_increment_across_successive_access_units() {
        let mut packetizer = AacPacketizer::new(97, 1, 16_000);
        let frame = |payload: Vec<u8>| Frame {
            track_index: 0,
            timestamp: Duration::ZERO,
            payload,
        };

        let first = packetizer
            .packetize(&frame(vec![1, 2]))
            .expect("packetizes");
        let second = packetizer
            .packetize(&frame(vec![3, 4]))
            .expect("packetizes");

        assert_eq!(u16::from_be_bytes([first[2], first[3]]), 0);
        assert_eq!(u16::from_be_bytes([second[2], second[3]]), 1);
    }

    #[test]
    fn rejects_an_access_unit_longer_than_8191_bytes() {
        let mut packetizer = AacPacketizer::new(97, 1, 48_000);
        let frame = Frame {
            track_index: 0,
            timestamp: Duration::ZERO,
            payload: vec![0u8; MAX_ACCESS_UNIT_SIZE + 1],
        };

        let result = packetizer.packetize(&frame);

        assert!(matches!(
            result,
            Err(PacketizeError::AacAccessUnitTooLarge {
                size,
                max
            }) if size == MAX_ACCESS_UNIT_SIZE + 1 && max == MAX_ACCESS_UNIT_SIZE
        ));
    }

    #[test]
    fn accepts_an_access_unit_at_exactly_the_8191_byte_limit() {
        let mut packetizer = AacPacketizer::new(97, 1, 48_000);
        let frame = Frame {
            track_index: 0,
            timestamp: Duration::ZERO,
            payload: vec![0u8; MAX_ACCESS_UNIT_SIZE],
        };

        assert!(packetizer.packetize(&frame).is_ok());
    }
}
