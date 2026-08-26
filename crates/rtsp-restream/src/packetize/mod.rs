//! Turns [`crate::provider::Frame`]s -- Annex-B H.264/H.265 access units and
//! raw AAC access units -- into the RTP payloads issue #12's own X3 item
//! sends over a `PLAY`ed connection's interleaved channel.
//!
//! This is the mirror image of issue #18's own `depacketize` module: where
//! that crate turns a camera's RTP payloads into these same Annex-B/raw-AAC
//! shapes, this module turns them back into RTP payloads for a downstream
//! consumer that expects exactly the RTP wire format a real camera would
//! have sent.
//!
//! Each packetizer here is a pure, synchronous transform scoped to one
//! track: construct it once per track with that track's RTP payload type
//! and SSRC, then feed it that track's [`crate::provider::Frame`]s in
//! timestamp order. It performs no I/O and owns no task -- an embedder's
//! async I/O layer (issue #12's own X3 item) owns delivery.

mod aac;
mod annex_b;
mod h264;
mod h265;
mod rtp_header;

pub use aac::AacPacketizer;
pub use h264::H264Packetizer;
pub use h265::H265Packetizer;

/// The largest RTP payload this module ever emits before an H.264/H.265
/// packetizer switches from a single-NAL packet to FU-A/FU fragmentation.
///
/// There is no real MTU limit over this crate's own TCP-interleaved
/// transport (`rtsp_types`/`SETUP` never negotiates UDP -- see
/// `session::handle_setup`), but a conservative fixed size keeps this
/// packetizer's output uniform with what a UDP-capable RTP receiver would
/// also need to tolerate: a standard 1500-byte Ethernet frame, minus a
/// 20-byte IPv4 header, an 8-byte UDP header, and a 12-byte RTP header,
/// leaves 1460 bytes of payload headroom. 1400 keeps roughly 60 bytes of
/// margin under that for whatever additional encapsulation a real path
/// might add (a VLAN tag, IPv6's 20-byte-larger header, a tunnel) without
/// this crate having to reason about which one applies. The same value
/// applies to both H.264 and H.265 for uniformity across every track this
/// crate packetizes.
pub(super) const MAX_SINGLE_NAL_SIZE: usize = 1400;

/// Errors packetizing a [`crate::provider::Frame`] into RTP payloads.
#[derive(Debug)]
pub enum PacketizeError {
    /// An H.264/H.265 frame's payload contained no Annex-B start code at
    /// all.
    NoStartCode,
    /// An H.264/H.265 frame's payload declared a NAL unit of zero length --
    /// a start code immediately followed by another start code, or by the
    /// end of the payload. Never valid Annex-B.
    EmptyNalUnit,
    /// An AAC frame's payload was longer than this crate's fixed AU-header
    /// `sizelength` (13 bits, matching the `a=fmtp` template
    /// `sdp::aac_fmtp` already advertises) can declare: at most 8191 bytes.
    AacAccessUnitTooLarge { size: usize, max: usize },
}

impl std::fmt::Display for PacketizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoStartCode => write!(f, "Annex-B frame payload contains no start code"),
            Self::EmptyNalUnit => {
                write!(f, "Annex-B frame payload declares a zero-length NAL unit")
            }
            Self::AacAccessUnitTooLarge { size, max } => write!(
                f,
                "AAC access unit is {size} bytes, longer than the {max}-byte maximum this \
                 crate's fixed AU-header sizelength can declare"
            ),
        }
    }
}

impl std::error::Error for PacketizeError {}
