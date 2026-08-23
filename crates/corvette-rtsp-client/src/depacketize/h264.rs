//! H.264 (RFC 6184) RTP depacketization: single-NAL, FU-A fragmented, and
//! STAP-A aggregated packets, reassembled into Annex-B NAL units.

use super::rtp_header::parse as parse_rtp_header;
use super::{Codec, DepacketizeError, Frame, annex_b_frame};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

const NAL_TYPE_STAP_A: u8 = 24;
const NAL_TYPE_FU_A: u8 = 28;

/// Reassembles one H.264 track's RTP payloads into Annex-B NAL units.
///
/// Constructed from that track's SDP `sprop-parameter-sets`; call
/// [`Self::depacketize`] with each RTP payload in arrival order. Holds the
/// in-progress state of at most one FU-A fragmentation at a time -- RFC 6184
/// never interleaves fragments of different NAL units on one track.
#[derive(Debug)]
pub struct H264Depacketizer {
    pending_parameter_sets: Option<Vec<bytes::Bytes>>,
    fragment: Option<Fragment>,
}

#[derive(Debug)]
struct Fragment {
    data: Vec<u8>,
    timestamp: u32,
}

impl H264Depacketizer {
    /// Builds a depacketizer whose stream begins with the SPS/PPS NAL units
    /// decoded from `sprop_parameter_sets` (the SDP `a=fmtp` value: a
    /// comma-separated list of base64-encoded NAL units, per RFC 6184
    /// §8.1).
    ///
    /// # Errors
    ///
    /// Returns an error if any comma-separated part does not decode as
    /// base64.
    pub fn new(sprop_parameter_sets: &str) -> Result<Self, DepacketizeError> {
        Ok(Self {
            pending_parameter_sets: Some(decode_parameter_sets(sprop_parameter_sets)?),
            fragment: None,
        })
    }

    /// Re-decodes `sprop_parameter_sets` and schedules the new SPS/PPS NAL
    /// units to be emitted before the next produced frame, per D-9's
    /// "insert at stream start and on any renegotiation" requirement.
    ///
    /// # Errors
    ///
    /// Returns an error if any comma-separated part does not decode as
    /// base64.
    pub fn renegotiate(&mut self, sprop_parameter_sets: &str) -> Result<(), DepacketizeError> {
        self.pending_parameter_sets = Some(decode_parameter_sets(sprop_parameter_sets)?);
        Ok(())
    }

    /// Depacketizes one RTP payload, returning the zero or more Annex-B
    /// frames it completed. A single-NAL or STAP-A packet completes
    /// immediately; a non-final FU-A fragment returns an empty vector while
    /// its NAL unit is reassembled.
    ///
    /// # Errors
    ///
    /// Returns an error if the RTP header is malformed, the payload is
    /// empty, a STAP-A packet's declared sizes run past its payload, an
    /// FU-A continuation arrives with no fragment in progress, or the
    /// payload declares an H.264 NAL unit type this depacketizer does not
    /// implement (anything other than single-NAL, FU-A, or STAP-A).
    pub fn depacketize(&mut self, rtp_packet: &[u8]) -> Result<Vec<Frame>, DepacketizeError> {
        let header = parse_rtp_header(rtp_packet)?;
        let mut frames = self.take_pending_parameter_sets(header.timestamp);

        let nal_header = *header
            .payload
            .first()
            .ok_or(DepacketizeError::EmptyPayload)?;
        let nal_type = nal_header & 0x1F;
        match nal_type {
            1..=23 => frames.push(annex_b_frame(Codec::H264, header.timestamp, header.payload)),
            NAL_TYPE_STAP_A => split_stap_a(header.payload, header.timestamp, &mut frames)?,
            NAL_TYPE_FU_A => {
                if let Some(frame) = self.reassemble_fu_a(header.payload, header.timestamp)? {
                    frames.push(frame);
                }
            }
            other => return Err(DepacketizeError::UnsupportedNalUnitType(other)),
        }
        Ok(frames)
    }

    fn take_pending_parameter_sets(&mut self, timestamp: u32) -> Vec<Frame> {
        self.pending_parameter_sets
            .take()
            .into_iter()
            .flatten()
            .map(|nal| annex_b_frame(Codec::H264, timestamp, &nal))
            .collect()
    }

    fn reassemble_fu_a(
        &mut self,
        payload: &[u8],
        timestamp: u32,
    ) -> Result<Option<Frame>, DepacketizeError> {
        if payload.len() < 2 {
            return Err(DepacketizeError::TruncatedFragmentationUnit);
        }
        let indicator = payload[0];
        let fu_header = payload[1];
        let is_start = fu_header & 0x80 != 0;
        let is_end = fu_header & 0x40 != 0;
        let reconstructed_nal_header = (indicator & 0xE0) | (fu_header & 0x1F);
        let fragment_data = &payload[2..];

        if is_start {
            let mut data = Vec::with_capacity(fragment_data.len() + 1);
            data.push(reconstructed_nal_header);
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
                Codec::H264,
                fragment.timestamp,
                &fragment.data,
            )))
        } else {
            Ok(None)
        }
    }
}

fn decode_parameter_sets(
    sprop_parameter_sets: &str,
) -> Result<Vec<bytes::Bytes>, DepacketizeError> {
    sprop_parameter_sets
        .split(',')
        .map(|part| {
            BASE64
                .decode(part.trim())
                .map(bytes::Bytes::from)
                .map_err(|source| DepacketizeError::Base64 {
                    field: "sprop-parameter-sets",
                    source,
                })
        })
        .collect()
}

fn split_stap_a(
    payload: &[u8],
    timestamp: u32,
    frames: &mut Vec<Frame>,
) -> Result<(), DepacketizeError> {
    let mut offset = 1; // skip the STAP-A NAL header byte itself
    while offset < payload.len() {
        let size_bytes = payload
            .get(offset..offset + 2)
            .ok_or(DepacketizeError::TruncatedAggregationUnit)?;
        let size = usize::from(u16::from_be_bytes([size_bytes[0], size_bytes[1]]));
        offset += 2;
        let nal = payload
            .get(offset..offset + size)
            .ok_or(DepacketizeError::TruncatedAggregationUnit)?;
        frames.push(annex_b_frame(Codec::H264, timestamp, nal));
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

    /// Builds a `sprop-parameter-sets` string from fabricated SPS/PPS bytes,
    /// returning it alongside the raw bytes so tests can assert the exact
    /// Annex-B framing expected -- without hand-computing base64.
    fn sprop_parameter_sets() -> (String, Vec<u8>, Vec<u8>) {
        let sps = vec![0x67, 0x42, 0x00, 0x1E];
        let pps = vec![0x68, 0xCE, 0x3C, 0x80];
        let sprop = format!("{},{}", BASE64.encode(&sps), BASE64.encode(&pps));
        (sprop, sps, pps)
    }

    #[test]
    fn single_nal_packet_emits_one_annex_b_frame_with_parameter_sets_first() {
        let (sprop, sps, pps) = sprop_parameter_sets();
        let mut depacketizer = H264Depacketizer::new(&sprop).expect("decodes");
        let nal = [0x65, 0xAA, 0xBB, 0xCC]; // IDR slice NAL, type 5

        let frames = depacketizer
            .depacketize(&rtp_packet(1000, &nal))
            .expect("depacketizes");

        assert_eq!(frames.len(), 3, "SPS, then PPS, then the frame itself");
        assert_eq!(
            frames[0].payload.as_ref(),
            [&[0, 0, 0, 1][..], &sps].concat()
        );
        assert_eq!(
            frames[1].payload.as_ref(),
            [&[0, 0, 0, 1][..], &pps].concat()
        );
        assert_eq!(
            frames[2].payload.as_ref(),
            [&[0, 0, 0, 1][..], &nal[..]].concat()
        );
        assert!(frames.iter().all(|frame| frame.codec == Codec::H264));
        assert!(frames.iter().all(|frame| frame.timestamp == 1000));
    }

    #[test]
    fn parameter_sets_are_emitted_once_not_on_every_packet() {
        let (sprop, _sps, _pps) = sprop_parameter_sets();
        let mut depacketizer = H264Depacketizer::new(&sprop).expect("decodes");
        depacketizer
            .depacketize(&rtp_packet(1000, &[0x65, 1]))
            .expect("depacketizes");

        let frames = depacketizer
            .depacketize(&rtp_packet(1040, &[0x65, 2]))
            .expect("depacketizes");

        assert_eq!(
            frames.len(),
            1,
            "no repeat parameter sets on the second packet"
        );
    }

    #[test]
    fn renegotiate_re_emits_parameter_sets_before_the_next_frame() {
        let (sprop, _sps, _pps) = sprop_parameter_sets();
        let mut depacketizer = H264Depacketizer::new(&sprop).expect("decodes");
        depacketizer
            .depacketize(&rtp_packet(1000, &[0x65, 1]))
            .expect("depacketizes");

        depacketizer.renegotiate(&sprop).expect("decodes again");
        let frames = depacketizer
            .depacketize(&rtp_packet(1040, &[0x65, 2]))
            .expect("depacketizes");

        assert_eq!(
            frames.len(),
            3,
            "parameter sets are reinserted after renegotiation"
        );
    }

    #[test]
    fn three_fragment_fu_a_sequence_reassembles_into_one_nal_byte_identical_to_the_source() {
        let (sprop, _sps, _pps) = sprop_parameter_sets();
        let mut depacketizer = H264Depacketizer::new(&sprop).expect("decodes");
        depacketizer
            .depacketize(&rtp_packet(2000, &[0x65]))
            .expect("consumes the initial parameter-set emission");

        // A fabricated NAL unit: header 0x65 (nal_ref_idc=3, type=5 IDR),
        // payload "ABCDEFGHIJ" (10 bytes), split into three FU-A fragments.
        let original_nal_header = 0x65u8;
        let nal_payload = b"ABCDEFGHIJ";
        let fu_indicator = 0x60 | 0x1C; // NRI preserved from the original header, type=FU-A (28)
        let start_fu_header = 0x80 | (original_nal_header & 0x1F);
        let middle_fu_header = original_nal_header & 0x1F;
        let end_fu_header = 0x40 | (original_nal_header & 0x1F);

        let start = depacketizer
            .depacketize(&rtp_packet(
                3000,
                &[
                    fu_indicator,
                    start_fu_header,
                    nal_payload[0],
                    nal_payload[1],
                    nal_payload[2],
                ],
            ))
            .expect("depacketizes");
        assert!(start.is_empty(), "start fragment alone completes nothing");

        let middle = depacketizer
            .depacketize(&rtp_packet(
                3000,
                &[
                    fu_indicator,
                    middle_fu_header,
                    nal_payload[3],
                    nal_payload[4],
                    nal_payload[5],
                ],
            ))
            .expect("depacketizes");
        assert!(middle.is_empty(), "middle fragment alone completes nothing");

        let end = depacketizer
            .depacketize(&rtp_packet(
                3000,
                &[
                    fu_indicator,
                    end_fu_header,
                    nal_payload[6],
                    nal_payload[7],
                    nal_payload[8],
                    nal_payload[9],
                ],
            ))
            .expect("depacketizes");

        assert_eq!(end.len(), 1);
        let mut expected_nal = vec![original_nal_header];
        expected_nal.extend_from_slice(nal_payload);
        assert_eq!(
            end[0].payload.as_ref(),
            [&[0, 0, 0, 1][..], &expected_nal[..]].concat()
        );
        assert_eq!(end[0].timestamp, 3000);
    }

    #[test]
    fn two_nal_stap_a_packet_splits_into_two_separate_annex_b_frames() {
        let (sprop, _sps, _pps) = sprop_parameter_sets();
        let mut depacketizer = H264Depacketizer::new(&sprop).expect("decodes");
        depacketizer
            .depacketize(&rtp_packet(4000, &[0x65]))
            .expect("consumes the initial parameter-set emission");

        let nal_a = [0x67, 0x11, 0x22]; // SPS-shaped
        let nal_b = [0x68, 0x33]; // PPS-shaped
        let mut stap_payload = vec![NAL_TYPE_STAP_A];
        stap_payload.extend_from_slice(&u16::try_from(nal_a.len()).unwrap().to_be_bytes());
        stap_payload.extend_from_slice(&nal_a);
        stap_payload.extend_from_slice(&u16::try_from(nal_b.len()).unwrap().to_be_bytes());
        stap_payload.extend_from_slice(&nal_b);

        let frames = depacketizer
            .depacketize(&rtp_packet(5000, &stap_payload))
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
        // packetization modes -- single-NAL, STAP-A, and FU-A -- since a
        // regression could plausibly hit only one of their code paths.
        let (sprop, _sps, _pps) = sprop_parameter_sets();
        let mut depacketizer = H264Depacketizer::new(&sprop).expect("decodes");
        let mut frames = Vec::new();

        frames.extend(
            depacketizer
                .depacketize(&rtp_packet(1000, &[0x65, 0xAA, 0xBB]))
                .expect("depacketizes a single-NAL packet"),
        );

        let mut stap_payload = vec![NAL_TYPE_STAP_A];
        let nal_a = [0x67, 0x11];
        stap_payload.extend_from_slice(&u16::try_from(nal_a.len()).unwrap().to_be_bytes());
        stap_payload.extend_from_slice(&nal_a);
        frames.extend(
            depacketizer
                .depacketize(&rtp_packet(2000, &stap_payload))
                .expect("depacketizes a STAP-A packet"),
        );

        let fu_indicator = 0x60 | 0x1C; // type=FU-A (28)
        frames.extend(
            depacketizer
                .depacketize(&rtp_packet(3000, &[fu_indicator, 0x80 | 0x05, 0xAA]))
                .expect("depacketizes an FU-A start fragment"),
        );
        frames.extend(
            depacketizer
                .depacketize(&rtp_packet(3000, &[fu_indicator, 0x40 | 0x05, 0xBB]))
                .expect("depacketizes an FU-A end fragment"),
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
    fn fu_a_continuation_without_a_start_fragment_is_an_error() {
        let (sprop, _sps, _pps) = sprop_parameter_sets();
        let mut depacketizer = H264Depacketizer::new(&sprop).expect("decodes");
        depacketizer
            .depacketize(&rtp_packet(1000, &[0x65]))
            .expect("consumes the initial parameter-set emission");

        let middle_fu_header = 0x05; // no S/E bits set
        let result = depacketizer.depacketize(&rtp_packet(2000, &[0x3C, middle_fu_header, 1, 2]));

        assert!(matches!(
            result,
            Err(DepacketizeError::FragmentWithoutStart)
        ));
    }

    #[test]
    fn an_unsupported_nal_unit_type_is_rejected() {
        let (sprop, _sps, _pps) = sprop_parameter_sets();
        let mut depacketizer = H264Depacketizer::new(&sprop).expect("decodes");
        depacketizer
            .depacketize(&rtp_packet(1000, &[0x65]))
            .expect("consumes the initial parameter-set emission");

        let result = depacketizer.depacketize(&rtp_packet(2000, &[29])); // FU-B, out of scope

        assert!(matches!(
            result,
            Err(DepacketizeError::UnsupportedNalUnitType(29))
        ));
    }
}
