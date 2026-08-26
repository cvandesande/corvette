//! H.264 (RFC 6184) RTP packetization: splits each Annex-B access unit into
//! NAL units, emitting a single-NAL RTP packet for a small NAL and
//! FU-A-fragmenting a large one.

use super::annex_b::split_nal_units;
use super::rtp_header::{RtpHeaderFields, build_packet, scale_timestamp};
use super::{MAX_SINGLE_NAL_SIZE, PacketizeError};
use crate::provider::Frame;

const NAL_TYPE_FU_A: u8 = 28;

/// The RTP clock rate H.264 uses, fixed by RFC 6184 §5.1 (not negotiable
/// per-stream, matching `sdp::VIDEO_CLOCK_RATE_HZ`).
const CLOCK_RATE_HZ: u32 = 90_000;

/// Packetizes one H.264 track's Annex-B access units into RTP payloads.
///
/// Construct one instance per track with that track's negotiated RTP
/// payload type and SSRC; call [`Self::packetize`] with each of that
/// track's [`Frame`]s, in timestamp order. Sequence numbers increment
/// across every RTP packet this instance emits, including every FU-A
/// fragment, matching RFC 6184's own requirement that a fragmented NAL
/// unit's fragments carry consecutive sequence numbers.
#[derive(Debug)]
pub struct H264Packetizer {
    payload_type: u8,
    ssrc: u32,
    next_sequence_number: u16,
}

impl H264Packetizer {
    #[must_use]
    pub const fn new(payload_type: u8, ssrc: u32) -> Self {
        Self {
            payload_type,
            ssrc,
            next_sequence_number: 0,
        }
    }

    /// Packetizes one access unit into the RTP packets that carry it, in
    /// wire order. The RTP marker bit is set on the last packet of the last
    /// NAL unit in `frame`, per RFC 6184's own access-unit-boundary
    /// convention.
    ///
    /// # Errors
    ///
    /// Returns an error if `frame`'s payload contains no Annex-B start
    /// code, or declares a zero-length NAL unit.
    pub fn packetize(&mut self, frame: &Frame) -> Result<Vec<Vec<u8>>, PacketizeError> {
        let nal_units = split_nal_units(&frame.payload)?;
        let timestamp = scale_timestamp(frame.timestamp, CLOCK_RATE_HZ);
        let last_nal_index = nal_units.len() - 1;

        let mut packets = Vec::new();
        for (index, nal) in nal_units.into_iter().enumerate() {
            let is_last_nal = index == last_nal_index;
            if nal.len() <= MAX_SINGLE_NAL_SIZE {
                packets.push(self.single_nal_packet(nal, timestamp, is_last_nal));
            } else {
                self.fragment_nal(nal, timestamp, is_last_nal, &mut packets);
            }
        }
        Ok(packets)
    }

    fn single_nal_packet(&mut self, nal: &[u8], timestamp: u32, marker: bool) -> Vec<u8> {
        let packet = build_packet(&self.header_fields(timestamp, marker), nal);
        self.next_sequence_number = self.next_sequence_number.wrapping_add(1);
        packet
    }

    /// Splits `nal` into FU-A fragments (RFC 6184 §5.8), each no larger
    /// than [`MAX_SINGLE_NAL_SIZE`] including its own 2-byte FU
    /// indicator/header. `marker` is this NAL's own access-unit-boundary
    /// marker; only the fragment carrying the FU-A end bit also carries the
    /// RTP marker bit.
    fn fragment_nal(&mut self, nal: &[u8], timestamp: u32, marker: bool, packets: &mut Vec<Vec<u8>>) {
        let nal_header = nal[0];
        let fu_indicator = (nal_header & 0xE0) | NAL_TYPE_FU_A;
        let nal_type = nal_header & 0x1F;
        let fragment_payload_size = MAX_SINGLE_NAL_SIZE - 2; // FU indicator + FU header

        let fragments: Vec<&[u8]> = nal[1..].chunks(fragment_payload_size).collect();
        let last_fragment_index = fragments.len() - 1;

        for (index, chunk) in fragments.into_iter().enumerate() {
            let is_start = index == 0;
            let is_end = index == last_fragment_index;
            let fu_header = (u8::from(is_start) << 7) | (u8::from(is_end) << 6) | nal_type;

            let mut payload = Vec::with_capacity(2 + chunk.len());
            payload.push(fu_indicator);
            payload.push(fu_header);
            payload.extend_from_slice(chunk);

            let packet_marker = marker && is_end;
            packets.push(build_packet(
                &self.header_fields(timestamp, packet_marker),
                &payload,
            ));
            self.next_sequence_number = self.next_sequence_number.wrapping_add(1);
        }
    }

    const fn header_fields(&self, timestamp: u32, marker: bool) -> RtpHeaderFields {
        RtpHeaderFields {
            sequence_number: self.next_sequence_number,
            timestamp,
            ssrc: self.ssrc,
            marker,
            payload_type: self.payload_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn header(packet: &[u8]) -> (u16, u32, bool) {
        let sequence_number = u16::from_be_bytes([packet[2], packet[3]]);
        let timestamp = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
        let marker = packet[1] & 0x80 != 0;
        (sequence_number, timestamp, marker)
    }

    #[test]
    fn a_small_single_nal_frame_emits_one_marked_packet() {
        let mut packetizer = H264Packetizer::new(96, 0xDEAD_BEEF);
        let frame = Frame {
            track_index: 0,
            timestamp: Duration::from_secs(1),
            payload: [&[0, 0, 0, 1][..], &[0x65, 0xAA, 0xBB]].concat(),
        };

        let packets = packetizer.packetize(&frame).expect("packetizes");

        assert_eq!(packets.len(), 1);
        assert_eq!(&packets[0][12..], &[0x65, 0xAA, 0xBB]);
        let (sequence_number, timestamp, marker) = header(&packets[0]);
        assert_eq!(sequence_number, 0);
        assert_eq!(timestamp, 90_000);
        assert!(marker, "the only NAL in the frame ends the access unit");
    }

    #[test]
    fn multiple_small_nals_in_one_frame_each_become_their_own_single_nal_packet() {
        let mut packetizer = H264Packetizer::new(96, 1);
        let frame = Frame {
            track_index: 0,
            timestamp: Duration::ZERO,
            payload: [
                &[0, 0, 0, 1][..],
                &[0x67, 0x11, 0x22],
                &[0, 0, 0, 1],
                &[0x68, 0x33],
                &[0, 0, 0, 1],
                &[0x65, 0x44, 0x55],
            ]
            .concat(),
        };

        let packets = packetizer.packetize(&frame).expect("packetizes");

        assert_eq!(packets.len(), 3);
        assert_eq!(&packets[0][12..], &[0x67, 0x11, 0x22]);
        assert_eq!(&packets[1][12..], &[0x68, 0x33]);
        assert_eq!(&packets[2][12..], &[0x65, 0x44, 0x55]);
        assert!(!header(&packets[0]).2, "not the last NAL of the frame");
        assert!(!header(&packets[1]).2, "not the last NAL of the frame");
        assert!(header(&packets[2]).2, "the last NAL ends the access unit");

        let sequence_numbers: Vec<u16> = packets.iter().map(|p| header(p).0).collect();
        assert_eq!(sequence_numbers, vec![0, 1, 2]);
    }

    #[test]
    fn a_nal_larger_than_the_threshold_is_fu_a_fragmented() {
        let mut packetizer = H264Packetizer::new(96, 1);
        let nal_header = 0x65u8;
        let nal_payload = vec![0xABu8; MAX_SINGLE_NAL_SIZE + 100];
        let mut nal = vec![nal_header];
        nal.extend_from_slice(&nal_payload);
        let frame = Frame {
            track_index: 0,
            timestamp: Duration::ZERO,
            payload: [&[0, 0, 0, 1][..], &nal].concat(),
        };

        let packets = packetizer.packetize(&frame).expect("packetizes");

        assert!(packets.len() > 1, "must fragment: {}", packets.len());
        for packet in &packets {
            assert!(packet.len() - 12 <= MAX_SINGLE_NAL_SIZE);
        }

        let first_payload = &packets[0][12..];
        assert_eq!(first_payload[0] & 0x1F, NAL_TYPE_FU_A);
        assert_eq!(first_payload[1] & 0x80, 0x80, "S bit set on first fragment");
        assert_eq!(first_payload[1] & 0x40, 0, "E bit clear on first fragment");

        let last_payload = &packets[packets.len() - 1][12..];
        assert_eq!(last_payload[1] & 0x80, 0, "S bit clear on last fragment");
        assert_eq!(last_payload[1] & 0x40, 0x40, "E bit set on last fragment");
        assert!(
            header(&packets[packets.len() - 1]).2,
            "only the last fragment of the last NAL carries the RTP marker"
        );
        assert!(!header(&packets[0]).2);
    }

    #[test]
    fn rejects_a_payload_with_no_start_code() {
        let mut packetizer = H264Packetizer::new(96, 1);
        let frame = Frame {
            track_index: 0,
            timestamp: Duration::ZERO,
            payload: vec![0x65, 0xAA],
        };
        assert!(matches!(
            packetizer.packetize(&frame),
            Err(PacketizeError::NoStartCode)
        ));
    }
}
