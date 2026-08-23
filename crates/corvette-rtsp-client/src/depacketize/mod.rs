//! Turns raw per-track RTP payloads ([`crate::session::RtpPacket`]) into
//! codec access units a later item hands to `moq_mux`.
//!
//! Output is always Annex-B-framed H.264/H.265 (start-code-prefixed NAL
//! units, parameter sets in-band) or raw, non-ADTS AAC access units --
//! never a container format (MPEG-TS, fMP4, or otherwise). See
//! `docs/design/api-contracts.md` for why: this matches what downstream
//! `moq_mux::codec::{h264,h265,aac}`'s `Import` layer expects directly, with
//! no demuxing step in between.
//!
//! Each depacketizer here is a pure, synchronous transform scoped to one
//! track: construct it from that track's SDP-derived parameters, then feed
//! it RTP payloads one at a time. It performs no I/O and owns no task --
//! session and per-camera orchestration concerns belong to [`crate::session`]
//! and a later item, not here.

mod aac;
mod h264;
mod h265;
mod rtp_header;

pub use aac::AacDepacketizer;
pub use h264::H264Depacketizer;
pub use h265::{H265Depacketizer, SpropParameterSets};

use rtp_header::RtpHeaderError;

/// The codec a [`Frame`]'s payload is encoded with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    H265,
    Aac,
}

/// One codec access unit produced by a depacketizer.
///
/// `payload` is `bytes::Bytes` so that broadcasting one frame to multiple
/// subscribers (a later item's job) is a cheap refcounted clone rather than
/// a copy. An H.264/H.265 `payload` is Annex-B: one or more
/// start-code-prefixed NAL units. An AAC `payload` is a single raw,
/// non-ADTS access unit. `timestamp` is the RTP timestamp the access unit's
/// own packet(s) declared, in the track's own clock rate, unconverted.
///
/// This type carries no track or camera identity: each depacketizer
/// instance is scoped to one track, so a caller publishing a `Frame`
/// already knows which track and camera produced it by which depacketizer
/// instance it called -- a redundant identity field here would only
/// duplicate information the caller already holds.
#[derive(Debug, Clone)]
pub struct Frame {
    pub codec: Codec,
    pub timestamp: u32,
    pub payload: bytes::Bytes,
}

/// Errors depacketizing an RTP payload into one or more [`Frame`]s.
#[derive(Debug)]
pub enum DepacketizeError {
    /// The RTP packet failed to parse (too short, or an unsupported RTP
    /// version); see the message for detail (kept as text rather than an
    /// internal type this crate does not want to commit to as public API).
    Rtp(String),
    /// A `sprop-*` SDP parameter did not decode as base64.
    Base64 {
        field: &'static str,
        source: base64::DecodeError,
    },
    /// The RTP payload was empty (or too short to hold a NAL/AU header)
    /// after the RTP header.
    EmptyPayload,
    /// A STAP-A/AP aggregation packet's declared NAL size ran past the end
    /// of the payload.
    TruncatedAggregationUnit,
    /// An FU-A/FU fragment carried a payload too short to hold its own FU
    /// header.
    TruncatedFragmentationUnit,
    /// A non-start FU-A/FU fragment arrived with no fragment in progress.
    FragmentWithoutStart,
    /// An H.264/H.265 NAL unit type this depacketizer does not implement
    /// (only single-NAL, FU-A/FU, and STAP-A/AP are in scope).
    UnsupportedNalUnitType(u8),
    /// An AAC AU-header section was truncated, or declared an access unit
    /// longer than the remaining payload.
    TruncatedAccessUnit,
}

impl std::fmt::Display for DepacketizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rtp(message) => write!(f, "{message}"),
            Self::Base64 { field, source } => {
                write!(f, "SDP {field} did not decode as base64: {source}")
            }
            Self::EmptyPayload => write!(f, "RTP payload was empty after the RTP header"),
            Self::TruncatedAggregationUnit => write!(
                f,
                "aggregation packet's declared NAL size exceeds its payload"
            ),
            Self::TruncatedFragmentationUnit => write!(
                f,
                "fragmentation unit payload is too short for its own FU header"
            ),
            Self::FragmentWithoutStart => write!(
                f,
                "fragmentation unit continuation arrived with no start fragment in progress"
            ),
            Self::UnsupportedNalUnitType(nal_type) => {
                write!(
                    f,
                    "NAL unit type {nal_type} is not a supported packetization mode"
                )
            }
            Self::TruncatedAccessUnit => write!(
                f,
                "AAC AU-header section is truncated or declares a size longer than the remaining payload"
            ),
        }
    }
}

impl std::error::Error for DepacketizeError {}

impl From<RtpHeaderError> for DepacketizeError {
    fn from(err: RtpHeaderError) -> Self {
        Self::Rtp(err.to_string())
    }
}

/// Prefixes `nal` with a 4-byte Annex-B start code (`00 00 00 01`) and wraps
/// it as a [`Frame`]. Every H.264/H.265 frame this crate emits is built
/// through this one function -- INV-1's actual enforcement point, since
/// there is no other place in this crate's source that could instead write
/// a length prefix.
fn annex_b_frame(codec: Codec, timestamp: u32, nal: &[u8]) -> Frame {
    let mut payload = bytes::BytesMut::with_capacity(nal.len() + 4);
    payload.extend_from_slice(&[0, 0, 0, 1]);
    payload.extend_from_slice(nal);
    Frame {
        codec,
        timestamp,
        payload: payload.freeze(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annex_b_frame_always_begins_with_a_four_byte_start_code() {
        let frame = annex_b_frame(Codec::H264, 1000, &[0x65, 0xAA, 0xBB]);
        assert_eq!(frame.payload.as_ref(), &[0, 0, 0, 1, 0x65, 0xAA, 0xBB]);
    }
}
