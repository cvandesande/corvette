//! Fabricated RTP packets and RTSP interleaved-frame ($-prefixed) framing.
//!
//! Content need not be decodable video: only structurally valid RTP framing
//! matters here, since this crate's own future depacketizer is what a later
//! item tests against these bytes, not a real decoder.

const RTP_VERSION_2_NO_PADDING_NO_EXTENSION: u8 = 0x80;
const MARKER_SET_PAYLOAD_TYPE_96: u8 = 0xE0;

/// Builds one RTP packet carrying a single fabricated H.264 NAL unit
/// (reported as an IDR slice so a real parser treats it as a keyframe).
pub(super) fn fabricate_h264_packet(
    sequence_number: u16,
    timestamp: u32,
    ssrc: u32,
    frame_index: u64,
) -> Vec<u8> {
    let mut packet = Vec::with_capacity(12 + 1 + 8);
    packet.push(RTP_VERSION_2_NO_PADDING_NO_EXTENSION);
    packet.push(MARKER_SET_PAYLOAD_TYPE_96);
    packet.extend_from_slice(&sequence_number.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&ssrc.to_be_bytes());
    // NAL header: forbidden_zero_bit=0, nal_ref_idc=3, nal_unit_type=5 (IDR slice).
    packet.push(0x65);
    packet.extend_from_slice(&frame_index.to_be_bytes());
    packet
}

/// Wraps `payload` in an RTSP `$`-prefixed interleaved frame: channel byte
/// plus a big-endian 16-bit length, per RFC 2326 section 10.12.
///
/// # Panics
///
/// Panics if `payload` is longer than [`u16::MAX`] bytes; every payload this
/// mock produces is a small fabricated packet, far under that bound.
pub(super) fn interleave(channel: u8, payload: &[u8]) -> Vec<u8> {
    let len = u16::try_from(payload.len()).expect("fabricated RTP payload fits in a u16 length");
    let mut framed = Vec::with_capacity(4 + payload.len());
    framed.push(b'$');
    framed.push(channel);
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(payload);
    framed
}
