//! H.265 (RFC 7798) RTP depacketization: single-NAL, FU fragmented, and AP
//! aggregated packets, reassembled into Annex-B NAL units.

use super::rtp_header::parse as parse_rtp_header;
use super::{Codec, DepacketizeError, Frame, annex_b_frame};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

const NAL_TYPE_AP: u8 = 48;
const NAL_TYPE_FU: u8 = 49;

/// One track's SDP `sprop-vps`/`sprop-sps`/`sprop-pps` `a=fmtp` values (RFC
/// 7798 §7.1), each a comma-separated list of base64-encoded NAL units.
///
/// Grouped into one type rather than three adjacent `&str` parameters so a
/// caller cannot accidentally pass them in the wrong order.
#[derive(Debug, Clone, Copy)]
pub struct SpropParameterSets<'a> {
    pub vps: &'a str,
    pub sps: &'a str,
    pub pps: &'a str,
}

/// Reassembles one H.265 track's RTP payloads into Annex-B NAL units.
///
/// Constructed from that track's [`SpropParameterSets`]; call
/// [`Self::depacketize`] with each RTP payload in arrival order. Holds
/// the in-progress state of at most one FU fragmentation at a time -- RFC
/// 7798 never interleaves fragments of different NAL units on one track.
#[derive(Debug)]
pub struct H265Depacketizer {
    pending_parameter_sets: Option<Vec<bytes::Bytes>>,
    fragment: Option<Fragment>,
}

#[derive(Debug)]
struct Fragment {
    data: Vec<u8>,
    timestamp: u32,
}

impl H265Depacketizer {
    /// Builds a depacketizer whose stream begins with the VPS, SPS, and PPS
    /// NAL units decoded from `sprop`, emitted in that order.
    ///
    /// # Errors
    ///
    /// Returns an error if any comma-separated part does not decode as
    /// base64.
    pub fn new(sprop: SpropParameterSets<'_>) -> Result<Self, DepacketizeError> {
        Ok(Self {
            pending_parameter_sets: Some(decode_all_parameter_sets(sprop)?),
            fragment: None,
        })
    }

    /// Re-decodes `sprop` and schedules the VPS/SPS/PPS NAL units to be
    /// emitted before the next produced frame, per D-9's "insert at stream
    /// start and on any renegotiation" requirement.
    ///
    /// # Errors
    ///
    /// Returns an error if any comma-separated part does not decode as
    /// base64.
    pub fn renegotiate(&mut self, sprop: SpropParameterSets<'_>) -> Result<(), DepacketizeError> {
        self.pending_parameter_sets = Some(decode_all_parameter_sets(sprop)?);
        Ok(())
    }

    /// Depacketizes one RTP payload, returning the zero or more Annex-B
    /// frames it completed.
    ///
    /// # Errors
    ///
    /// Returns an error if the RTP header is malformed, the payload is too
    /// short to hold an H.265 NAL header, an AP packet's declared sizes run
    /// past its payload, an FU continuation arrives with no fragment in
    /// progress, or the payload declares an H.265 NAL unit type this
    /// depacketizer does not implement (anything other than single-NAL, FU,
    /// or AP).
    pub fn depacketize(&mut self, rtp_packet: &[u8]) -> Result<Vec<Frame>, DepacketizeError> {
        let header = parse_rtp_header(rtp_packet)?;
        let mut frames = self.take_pending_parameter_sets(header.timestamp);

        if header.payload.len() < 2 {
            return Err(DepacketizeError::EmptyPayload);
        }
        let nal_type = (header.payload[0] >> 1) & 0x3F;
        match nal_type {
            NAL_TYPE_AP => split_ap(header.payload, header.timestamp, &mut frames)?,
            NAL_TYPE_FU => {
                if let Some(frame) = self.reassemble_fu(header.payload, header.timestamp)? {
                    frames.push(frame);
                }
            }
            0..=40 => frames.push(annex_b_frame(Codec::H265, header.timestamp, header.payload)),
            other => return Err(DepacketizeError::UnsupportedNalUnitType(other)),
        }
        Ok(frames)
    }

    fn take_pending_parameter_sets(&mut self, timestamp: u32) -> Vec<Frame> {
        self.pending_parameter_sets
            .take()
            .into_iter()
            .flatten()
            .map(|nal| annex_b_frame(Codec::H265, timestamp, &nal))
            .collect()
    }

    fn reassemble_fu(
        &mut self,
        payload: &[u8],
        timestamp: u32,
    ) -> Result<Option<Frame>, DepacketizeError> {
        if payload.len() < 3 {
            return Err(DepacketizeError::TruncatedFragmentationUnit);
        }
        let payload_header = [payload[0], payload[1]];
        let fu_header = payload[2];
        let is_start = fu_header & 0x80 != 0;
        let is_end = fu_header & 0x40 != 0;
        let fu_type = fu_header & 0x3F;
        // Reconstructed byte0: forbidden_zero_bit and layer-id-high bit come
        // from the FU payload header unchanged; nal_unit_type is replaced by
        // the FU header's own type field.
        let reconstructed_byte0 = (payload_header[0] & 0b1000_0001) | (fu_type << 1);
        let fragment_data = &payload[3..];

        if is_start {
            let mut data = Vec::with_capacity(fragment_data.len() + 2);
            data.push(reconstructed_byte0);
            data.push(payload_header[1]);
            data.extend_from_slice(fragment_data);
            self.fragment = Some(Fragment { data, timestamp });
        } else {
            let fragment = self
                .fragment
                .as_mut()
                .ok_or(DepacketizeError::FragmentWithoutStart)?;
            fragment.data.extend_from_slice(fragment_data);
        }

        if is_end {
            let fragment = self
                .fragment
                .take()
                .ok_or(DepacketizeError::FragmentWithoutStart)?;
            Ok(Some(annex_b_frame(
                Codec::H265,
                fragment.timestamp,
                &fragment.data,
            )))
        } else {
            Ok(None)
        }
    }
}

fn decode_all_parameter_sets(
    sprop: SpropParameterSets<'_>,
) -> Result<Vec<bytes::Bytes>, DepacketizeError> {
    let mut sets = decode_parameter_sets(sprop.vps, "sprop-vps")?;
    sets.extend(decode_parameter_sets(sprop.sps, "sprop-sps")?);
    sets.extend(decode_parameter_sets(sprop.pps, "sprop-pps")?);
    Ok(sets)
}

fn decode_parameter_sets(
    value: &str,
    field: &'static str,
) -> Result<Vec<bytes::Bytes>, DepacketizeError> {
    value
        .split(',')
        .map(|part| {
            BASE64
                .decode(part.trim())
                .map(bytes::Bytes::from)
                .map_err(|source| DepacketizeError::Base64 { field, source })
        })
        .collect()
}

fn split_ap(
    payload: &[u8],
    timestamp: u32,
    frames: &mut Vec<Frame>,
) -> Result<(), DepacketizeError> {
    let mut offset = 2; // skip the 2-byte AP NAL header
    while offset < payload.len() {
        let size_bytes = payload
            .get(offset..offset + 2)
            .ok_or(DepacketizeError::TruncatedAggregationUnit)?;
        let size = usize::from(u16::from_be_bytes([size_bytes[0], size_bytes[1]]));
        offset += 2;
        let nal = payload
            .get(offset..offset + size)
            .ok_or(DepacketizeError::TruncatedAggregationUnit)?;
        frames.push(annex_b_frame(Codec::H265, timestamp, nal));
        offset += size;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtp_packet(timestamp: u32, payload: &[u8]) -> Vec<u8> {
        let mut packet = vec![0x80, 0xE0, 0, 1];
        packet.extend_from_slice(&timestamp.to_be_bytes());
        packet.extend_from_slice(&[0, 0, 0, 0]); // SSRC
        packet.extend_from_slice(payload);
        packet
    }

    /// Owned base64 `sprop-*` values, so a test can hand out a borrowed
    /// [`SpropParameterSets`] without the base64 `String`s being dropped
    /// first.
    struct FabricatedSprop {
        vps: String,
        sps: String,
        pps: String,
    }

    impl FabricatedSprop {
        fn as_sprop(&self) -> SpropParameterSets<'_> {
            SpropParameterSets {
                vps: &self.vps,
                sps: &self.sps,
                pps: &self.pps,
            }
        }
    }

    /// Fabricated VPS/SPS/PPS bytes plus their base64 `sprop-*` values,
    /// computed rather than hand-encoded so expectations stay obviously
    /// correct.
    fn sprop_parameter_sets() -> (FabricatedSprop, Vec<u8>, Vec<u8>, Vec<u8>) {
        let vps = vec![0x40, 0x01, 0x0C];
        let sps = vec![0x42, 0x01, 0x01];
        let pps = vec![0x44, 0x01, 0xC1];
        let sprop = FabricatedSprop {
            vps: BASE64.encode(&vps),
            sps: BASE64.encode(&sps),
            pps: BASE64.encode(&pps),
        };
        (sprop, vps, sps, pps)
    }

    #[test]
    fn single_nal_packet_emits_one_annex_b_frame_with_vps_sps_pps_first() {
        let (sprop, vps, sps, pps) = sprop_parameter_sets();
        let mut depacketizer = H265Depacketizer::new(sprop.as_sprop()).expect("decodes");

        let nal = [0x26, 0x01, 0xDE, 0xAD]; // type 19 (IDR_W_RADL), single NAL
        let frames = depacketizer
            .depacketize(&rtp_packet(1000, &nal))
            .expect("depacketizes");

        assert_eq!(frames.len(), 4, "VPS, SPS, PPS, then the frame itself");
        assert_eq!(
            frames[0].payload.as_ref(),
            [&[0, 0, 0, 1][..], &vps].concat()
        );
        assert_eq!(
            frames[1].payload.as_ref(),
            [&[0, 0, 0, 1][..], &sps].concat()
        );
        assert_eq!(
            frames[2].payload.as_ref(),
            [&[0, 0, 0, 1][..], &pps].concat()
        );
        assert_eq!(
            frames[3].payload.as_ref(),
            [&[0, 0, 0, 1][..], &nal[..]].concat()
        );
        assert!(frames.iter().all(|frame| frame.codec == Codec::H265));
    }

    #[test]
    fn three_fragment_fu_sequence_reassembles_into_one_nal_byte_identical_to_the_source() {
        let (sprop, ..) = sprop_parameter_sets();
        let mut depacketizer = H265Depacketizer::new(sprop.as_sprop()).expect("decodes");
        depacketizer
            .depacketize(&rtp_packet(1000, &[0x26, 0x01, 0x00]))
            .expect("consumes the initial parameter-set emission");

        // Original NAL header [0x26, 0x01]: type=19, layer-id=0, tid=1.
        let nal_payload = b"ABCDEFGHIJ";
        let fu_payload_header = [0x62u8, 0x01u8]; // type replaced with 49 (FU)

        let start = depacketizer
            .depacketize(&rtp_packet(
                2000,
                &[
                    fu_payload_header[0],
                    fu_payload_header[1],
                    0x80 | 0x13, // S=1, FuType=19 (0x13)
                    nal_payload[0],
                    nal_payload[1],
                    nal_payload[2],
                ],
            ))
            .expect("depacketizes");
        assert!(start.is_empty(), "start fragment alone completes nothing");

        let middle = depacketizer
            .depacketize(&rtp_packet(
                2000,
                &[
                    fu_payload_header[0],
                    fu_payload_header[1],
                    19,
                    nal_payload[3],
                    nal_payload[4],
                    nal_payload[5],
                ],
            ))
            .expect("depacketizes");
        assert!(middle.is_empty(), "middle fragment alone completes nothing");

        let end = depacketizer
            .depacketize(&rtp_packet(
                2000,
                &[
                    fu_payload_header[0],
                    fu_payload_header[1],
                    0x40 | 0x13, // E=1, FuType=19 (0x13)
                    nal_payload[6],
                    nal_payload[7],
                    nal_payload[8],
                    nal_payload[9],
                ],
            ))
            .expect("depacketizes");

        assert_eq!(end.len(), 1);
        let mut expected_nal = vec![0x26, 0x01];
        expected_nal.extend_from_slice(nal_payload);
        assert_eq!(
            end[0].payload.as_ref(),
            [&[0, 0, 0, 1][..], &expected_nal[..]].concat()
        );
    }

    #[test]
    fn two_nal_ap_packet_splits_into_two_separate_annex_b_frames() {
        let (sprop, ..) = sprop_parameter_sets();
        let mut depacketizer = H265Depacketizer::new(sprop.as_sprop()).expect("decodes");
        depacketizer
            .depacketize(&rtp_packet(1000, &[0x26, 0x01, 0x00]))
            .expect("consumes the initial parameter-set emission");

        let nal_a = [0x40, 0x01, 0x11]; // VPS-shaped
        let nal_b = [0x42, 0x01, 0x22]; // SPS-shaped
        let mut ap_payload = vec![0x60, 0x01]; // AP NAL header (type 48)
        ap_payload.extend_from_slice(&u16::try_from(nal_a.len()).unwrap().to_be_bytes());
        ap_payload.extend_from_slice(&nal_a);
        ap_payload.extend_from_slice(&u16::try_from(nal_b.len()).unwrap().to_be_bytes());
        ap_payload.extend_from_slice(&nal_b);

        let frames = depacketizer
            .depacketize(&rtp_packet(3000, &ap_payload))
            .expect("depacketizes");

        assert_eq!(frames.len(), 2);
        assert_eq!(
            frames[0].payload.as_ref(),
            [&[0, 0, 0, 1][..], &nal_a[..]].concat()
        );
        assert_eq!(
            frames[1].payload.as_ref(),
            [&[0, 0, 0, 1][..], &nal_b[..]].concat()
        );
    }

    #[test]
    fn every_emitted_frame_begins_with_a_start_code_never_a_length_prefix() {
        // INV-1's own unit-test enforcement point. Covers all three
        // packetization modes -- single-NAL, AP, and FU -- since a
        // regression could plausibly hit only one of their code paths.
        let (sprop, ..) = sprop_parameter_sets();
        let mut depacketizer = H265Depacketizer::new(sprop.as_sprop()).expect("decodes");
        let mut frames = Vec::new();

        frames.extend(
            depacketizer
                .depacketize(&rtp_packet(1000, &[0x26, 0x01, 0xAA]))
                .expect("depacketizes a single-NAL packet"),
        );

        let mut ap_payload = vec![0x60, 0x01]; // AP NAL header (type 48)
        let nal_a = [0x40, 0x01, 0x11];
        ap_payload.extend_from_slice(&u16::try_from(nal_a.len()).unwrap().to_be_bytes());
        ap_payload.extend_from_slice(&nal_a);
        frames.extend(
            depacketizer
                .depacketize(&rtp_packet(2000, &ap_payload))
                .expect("depacketizes an AP packet"),
        );

        let fu_payload_header = [0x62u8, 0x01u8]; // type replaced with 49 (FU)
        frames.extend(
            depacketizer
                .depacketize(&rtp_packet(
                    3000,
                    &[
                        fu_payload_header[0],
                        fu_payload_header[1],
                        0x80 | 0x13,
                        0xAA,
                    ],
                ))
                .expect("depacketizes an FU start fragment"),
        );
        frames.extend(
            depacketizer
                .depacketize(&rtp_packet(
                    3000,
                    &[
                        fu_payload_header[0],
                        fu_payload_header[1],
                        0x40 | 0x13,
                        0xBB,
                    ],
                ))
                .expect("depacketizes an FU end fragment"),
        );

        assert!(!frames.is_empty());
        for frame in &frames {
            assert!(
                frame.payload.starts_with(&[0, 0, 0, 1]),
                "frame payload must begin with a 4-byte Annex-B start code, got {:?}",
                &frame.payload[..4.min(frame.payload.len())]
            );
        }
    }

    #[test]
    fn fu_continuation_without_a_start_fragment_is_an_error() {
        let (sprop, ..) = sprop_parameter_sets();
        let mut depacketizer = H265Depacketizer::new(sprop.as_sprop()).expect("decodes");
        depacketizer
            .depacketize(&rtp_packet(1000, &[0x26, 0x01, 0x00]))
            .expect("consumes the initial parameter-set emission");

        let result = depacketizer.depacketize(&rtp_packet(2000, &[0x62, 0x01, 19, 1, 2]));

        assert!(matches!(
            result,
            Err(DepacketizeError::FragmentWithoutStart)
        ));
    }

    #[test]
    fn an_unsupported_nal_unit_type_is_rejected() {
        let (sprop, ..) = sprop_parameter_sets();
        let mut depacketizer = H265Depacketizer::new(sprop.as_sprop()).expect("decodes");
        depacketizer
            .depacketize(&rtp_packet(1000, &[0x26, 0x01, 0x00]))
            .expect("consumes the initial parameter-set emission");

        // Type 50 (PACI) is reserved/unimplemented per this crate's scope.
        let result = depacketizer.depacketize(&rtp_packet(2000, &[50 << 1, 0x01]));

        assert!(matches!(
            result,
            Err(DepacketizeError::UnsupportedNalUnitType(50))
        ));
    }
}
