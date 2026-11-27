//! Turns per-track RTP payloads into Annex-B H.264/H.265 or raw non-ADTS AAC access units, never a container.

mod aac;
mod h264;
mod h265;
mod rtp_header;

pub use aac::AacDepacketizer;
pub use h264::H264Depacketizer;
pub use h265::{H265Depacketizer, SpropParameterSets};

use rtp_header::RtpHeaderError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    H265,
    Aac,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub codec: Codec,
    /// The RTP timestamp the access unit's packets declared, in the track's own clock rate, unconverted.
    pub timestamp: u32,
    pub payload: bytes::Bytes,
}

#[derive(Debug)]
pub enum DepacketizeError {
    Rtp(String),
    Base64 {
        field: &'static str,
        source: base64::DecodeError,
    },
    EmptyPayload,
    TruncatedAggregationUnit,
    TruncatedFragmentationUnit,
    FragmentWithoutStart,
    UnsupportedNalUnitType(u8),
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

/// The crate's one Annex-B framing point: every emitted H.264/H.265 frame gets its start code here.
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
