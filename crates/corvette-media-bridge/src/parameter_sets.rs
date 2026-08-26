//! Shared Annex-B parameter-set NAL scanning, extracted from
//! `restream_provider`'s own G1 implementation.
//!
//! `restream_provider` already solved the one genuinely reusable problem this
//! module exists for: since neither `corvette_rtsp_client::Client`'s public
//! API nor `rtsp_restream`'s own `StreamProvider::describe` contract gives a
//! parameter-set source other than the frame stream itself, this crate's
//! fMP4 muxer (`crate::fmp4`, issue #12 item G2) needs the same SPS/PPS (or
//! VPS/SPS/PPS for H.265) scanning `restream_provider` already implemented,
//! rather than a second, duplicated implementation.
//!
//! This module only accumulates the latest-seen bytes for each parameter-set
//! NAL type -- it has no opinion on when a caller should consider the
//! resulting set "resolved" or "changed". `restream_provider`'s own
//! `ParameterSetCache` resolves once and never updates again (unchanged
//! behavior from before this extraction); `crate::fmp4`'s tracker instead
//! re-emits an initialization segment on every genuine byte-level change (per
//! this item's own Do step 2). Each caller layers that policy on top of the
//! same accumulator here.

use bytes::Bytes;
use corvette_rtsp_client::depacketize::{Codec as ClientCodec, Frame as ClientFrame};

/// The Annex-B start code every `Frame` payload begins with: four bytes, so a
/// NAL's own header byte(s) start at index 4.
///
/// Confirmed by direct source read of `depacketize::annex_b_frame`. Each
/// `Frame` carries exactly one NAL unit (confirmed by direct source
/// read of `depacketize::h264`/`h265`'s own `depacketize`: an RTP
/// aggregation packet is already split into one `Frame` per NAL before this
/// module ever sees it), so reading only the first NAL's header below is
/// complete, not a simplification.
pub const START_CODE_LEN: usize = 4;

pub const H264_NAL_TYPE_SPS: u8 = 7;
pub const H264_NAL_TYPE_PPS: u8 = 8;
pub const H265_NAL_TYPE_VPS: u8 = 32;
pub const H265_NAL_TYPE_SPS: u8 = 33;
pub const H265_NAL_TYPE_PPS: u8 = 34;

/// Accumulates the latest parameter-set NAL bytes a camera's frame stream has
/// carried in-band, per codec.
///
/// A real camera repeats its parameter sets periodically (or a depacketizer
/// seeds them from `sprop-*` SDP attributes), so scanning the frame stream is
/// the only avenue available -- see this module's own top-level doc.
#[derive(Debug, Default, Clone)]
pub struct RawParameterSets {
    h264_sps: Option<Bytes>,
    h264_pps: Option<Bytes>,
    h265_vps: Option<Bytes>,
    h265_sps: Option<Bytes>,
    h265_pps: Option<Bytes>,
}

impl RawParameterSets {
    /// Scans one frame for a parameter-set NAL, updating the cached bytes for
    /// its codec if it carries one. A non-parameter-set NAL (a slice, SEI,
    /// AUD, ...) leaves this cache unchanged.
    pub fn observe(&mut self, frame: &ClientFrame) {
        match frame.codec {
            ClientCodec::H264 => {
                if let Some(nal_type) = nal_type_h264(&frame.payload) {
                    match nal_type {
                        H264_NAL_TYPE_SPS => {
                            self.h264_sps = Some(parameter_set_bytes(&frame.payload));
                        }
                        H264_NAL_TYPE_PPS => {
                            self.h264_pps = Some(parameter_set_bytes(&frame.payload));
                        }
                        _ => {}
                    }
                }
            }
            ClientCodec::H265 => {
                if let Some(nal_type) = nal_type_h265(&frame.payload) {
                    match nal_type {
                        H265_NAL_TYPE_VPS => {
                            self.h265_vps = Some(parameter_set_bytes(&frame.payload));
                        }
                        H265_NAL_TYPE_SPS => {
                            self.h265_sps = Some(parameter_set_bytes(&frame.payload));
                        }
                        H265_NAL_TYPE_PPS => {
                            self.h265_pps = Some(parameter_set_bytes(&frame.payload));
                        }
                        _ => {}
                    }
                }
            }
            // Never actually produced today -- see this crate's top-level
            // doc and G1's own Premise -- and has no in-band parameter set to
            // scan for even if it were (its config is an AudioSpecificConfig,
            // not a NAL).
            ClientCodec::Aac => {}
        }
    }

    /// The current complete H.264 set (SPS, PPS), or `None` until both have
    /// been observed at least once.
    #[must_use]
    pub fn h264(&self) -> Option<(&Bytes, &Bytes)> {
        Some((self.h264_sps.as_ref()?, self.h264_pps.as_ref()?))
    }

    /// The current complete H.265 set (VPS, SPS, PPS), or `None` until all
    /// three have been observed at least once.
    #[must_use]
    pub fn h265(&self) -> Option<(&Bytes, &Bytes, &Bytes)> {
        Some((
            self.h265_vps.as_ref()?,
            self.h265_sps.as_ref()?,
            self.h265_pps.as_ref()?,
        ))
    }
}

/// The NAL header byte's low 5 bits (H.264, RFC 6184 section 1.3), read past
/// the frame's own 4-byte Annex-B start code.
///
/// `None` for a payload too short to hold one -- never produced by
/// `corvette-rtsp-client`'s own depacketizer, which always emits at least a
/// start code plus one NAL byte, but checked defensively since this module
/// does not control that invariant.
#[must_use]
pub fn nal_type_h264(payload: &[u8]) -> Option<u8> {
    payload.get(START_CODE_LEN).map(|header| header & 0x1F)
}

/// The NAL header's type field (H.265, RFC 7798 section 1.1.4: bits 1-6 of
/// the first header byte), read the same way as [`nal_type_h264`].
#[must_use]
pub fn nal_type_h265(payload: &[u8]) -> Option<u8> {
    payload
        .get(START_CODE_LEN)
        .map(|header| (header >> 1) & 0x3F)
}

/// The full NAL unit (including its own header byte(s)), without the 4-byte
/// Annex-B start code.
#[must_use]
pub fn parameter_set_bytes(payload: &Bytes) -> Bytes {
    payload.slice(START_CODE_LEN..)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn annex_b(nal: &[u8]) -> Bytes {
        let mut payload = Vec::with_capacity(nal.len() + START_CODE_LEN);
        payload.extend_from_slice(&[0, 0, 0, 1]);
        payload.extend_from_slice(nal);
        Bytes::from(payload)
    }

    fn frame(codec: ClientCodec, nal: &[u8]) -> ClientFrame {
        ClientFrame {
            codec,
            timestamp: 0,
            payload: annex_b(nal),
        }
    }

    #[test]
    fn h264_is_none_until_both_sps_and_pps_are_seen() {
        let mut sets = RawParameterSets::default();
        let sps = [0x67, 0x42, 0xc0, 0x1f];
        let pps = [0x68, 0xce, 0x3c, 0x80];

        sets.observe(&frame(ClientCodec::H264, &sps));
        assert!(sets.h264().is_none(), "SPS alone is not a complete set");

        sets.observe(&frame(ClientCodec::H264, &pps));
        let (sps_out, pps_out) = sets.h264().expect("SPS+PPS complete the set");
        assert_eq!(sps_out, &sps[..]);
        assert_eq!(pps_out, &pps[..]);
    }

    #[test]
    fn h264_tracks_a_later_parameter_set_change() {
        let mut sets = RawParameterSets::default();
        let sps_v1 = [0x67, 0x42, 0xc0, 0x1f];
        let pps = [0x68, 0xce, 0x3c, 0x80];
        let sps_v2 = [0x67, 0x42, 0xc0, 0x28];

        sets.observe(&frame(ClientCodec::H264, &sps_v1));
        sets.observe(&frame(ClientCodec::H264, &pps));
        sets.observe(&frame(ClientCodec::H264, &sps_v2));

        let (sps_out, _) = sets.h264().expect("still complete");
        assert_eq!(
            sps_out,
            &sps_v2[..],
            "a later SPS NAL replaces the cached one"
        );
    }

    #[test]
    fn h265_is_none_until_vps_sps_and_pps_are_all_seen() {
        let mut sets = RawParameterSets::default();
        sets.observe(&frame(ClientCodec::H265, &[0x40, 0x01]));
        assert!(sets.h265().is_none());
        sets.observe(&frame(ClientCodec::H265, &[0x42, 0x01]));
        assert!(sets.h265().is_none());
        sets.observe(&frame(ClientCodec::H265, &[0x44, 0x01]));
        assert!(sets.h265().is_some());
    }

    #[test]
    fn a_non_parameter_set_nal_leaves_the_cache_unchanged() {
        let mut sets = RawParameterSets::default();
        sets.observe(&frame(ClientCodec::H264, &[0x65, 0x88])); // IDR slice
        assert!(sets.h264().is_none());
    }
}
