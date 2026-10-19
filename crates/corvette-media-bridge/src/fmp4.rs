//! A minimal fragmented-MP4 (fMP4) muxer, video only.
//!
//! One initialization segment per camera plus one fragment per access unit.
//! It builds bytes and performs no I/O; both the WebSocket repackager and
//! the HLS packager drive it through this module's public API.

use crate::parameter_sets::RawParameterSets;
use crate::rtp_clock::VIDEO_CLOCK_RATE_HZ;
use bytes::Bytes;
use corvette_rtsp_client::depacketize::{Codec as ClientCodec, Frame as ClientFrame};

/// One NAL per frame, so these coded slice types are each a full access
/// unit; all other NAL types carry no sample data.
const H264_NAL_TYPE_SLICE_NON_IDR: u8 = 1;
const H264_NAL_TYPE_SLICE_IDR: u8 = 5;

const H265_NAL_TYPE_VCL_MAX: u8 = 31;
const H265_NAL_TYPE_IRAP_MIN: u8 = 16;
const H265_NAL_TYPE_IRAP_MAX: u8 = 23;

/// Default first-sample duration; see docs/design/api-contracts.md.
const DEFAULT_FIRST_SAMPLE_DURATION_TICKS: u32 = VIDEO_CLOCK_RATE_HZ / 30;

const TRACK_ID: u32 = 1;

#[derive(Debug, Default)]
pub struct InitSegmentTracker {
    raw: RawParameterSets,
    last_emitted: Option<EmittedParameterSets>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EmittedParameterSets {
    H264 { sps: Bytes, pps: Bytes },
    H265 { vps: Bytes, sps: Bytes, pps: Bytes },
}

impl InitSegmentTracker {
    pub fn observe(&mut self, frame: &ClientFrame) -> Option<Bytes> {
        self.raw.observe(frame);
        match frame.codec {
            ClientCodec::H264 => {
                let (sps, pps) = self.raw.h264()?;
                let current = EmittedParameterSets::H264 {
                    sps: sps.clone(),
                    pps: pps.clone(),
                };
                if self.last_emitted.as_ref() == Some(&current) {
                    return None;
                }
                let segment = boxes::init_segment_h264(sps, pps);
                self.last_emitted = Some(current);
                Some(segment)
            }
            ClientCodec::H265 => {
                let (vps, sps, pps) = self.raw.h265()?;
                let current = EmittedParameterSets::H265 {
                    vps: vps.clone(),
                    sps: sps.clone(),
                    pps: pps.clone(),
                };
                if self.last_emitted.as_ref() == Some(&current) {
                    return None;
                }
                let segment = boxes::init_segment_h265(vps, sps, pps);
                self.last_emitted = Some(current);
                Some(segment)
            }
            // No camera's audio track is published on any live-view transport
            // yet: docs/design/api-contracts.md.
            ClientCodec::Aac => None,
        }
    }
}

/// Groups the frame stream into one `moof`/`mdat` fragment per access unit.
#[derive(Debug, Default)]
pub struct Fragmenter {
    sequence_number: u32,
    previous_timestamp: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Fragment {
    pub bytes: Bytes,
    pub is_keyframe: bool,
    pub duration_ticks: u32,
}

impl Fragmenter {
    /// Returns a fragment only when the frame carries a video access unit.
    pub fn next(&mut self, frame: &ClientFrame) -> Option<Fragment> {
        let (_nal_type, is_keyframe) = classify(frame)?;

        let duration_ticks = self
            .previous_timestamp
            .map_or(DEFAULT_FIRST_SAMPLE_DURATION_TICKS, |previous| {
                frame.timestamp.wrapping_sub(previous).max(1)
            });
        self.previous_timestamp = Some(frame.timestamp);

        self.sequence_number += 1;
        let bytes = boxes::fragment(
            self.sequence_number,
            u64::from(frame.timestamp),
            duration_ticks,
            &frame.payload[super::parameter_sets::START_CODE_LEN..],
            is_keyframe,
        );
        Some(Fragment {
            bytes,
            is_keyframe,
            duration_ticks,
        })
    }
}

fn classify(frame: &ClientFrame) -> Option<(u8, bool)> {
    match frame.codec {
        ClientCodec::H264 => {
            let nal_type = crate::parameter_sets::nal_type_h264(&frame.payload)?;
            match nal_type {
                H264_NAL_TYPE_SLICE_NON_IDR => Some((nal_type, false)),
                H264_NAL_TYPE_SLICE_IDR => Some((nal_type, true)),
                _ => None,
            }
        }
        ClientCodec::H265 => {
            let nal_type = crate::parameter_sets::nal_type_h265(&frame.payload)?;
            if nal_type > H265_NAL_TYPE_VCL_MAX {
                return None;
            }
            let is_keyframe = (H265_NAL_TYPE_IRAP_MIN..=H265_NAL_TYPE_IRAP_MAX).contains(&nal_type);
            Some((nal_type, is_keyframe))
        }
        ClientCodec::Aac => None,
    }
}

mod boxes {
    //! ISO/IEC 14496-12/14496-15 box construction, byte level.

    use super::{TRACK_ID, VIDEO_CLOCK_RATE_HZ};
    use bytes::{BufMut, Bytes, BytesMut};

    fn write_box(out: &mut BytesMut, box_type: [u8; 4], body: impl FnOnce(&mut BytesMut)) {
        let start = out.len();
        out.put_u32(0); // placeholder, patched below
        out.put_slice(&box_type);
        body(out);
        let size = u32::try_from(out.len() - start)
            .expect("a single fMP4 box never approaches 4 GiB in this item's own use");
        out[start..start + 4].copy_from_slice(&size.to_be_bytes());
    }

    fn ftyp(out: &mut BytesMut) {
        write_box(out, *b"ftyp", |out| {
            out.put_slice(b"iso5"); // major_brand
            out.put_u32(0); // minor_version
            for brand in [b"iso5", b"iso6", b"isom", b"mp41"] {
                out.put_slice(brand);
            }
        });
    }

    fn mvhd(out: &mut BytesMut) {
        write_box(out, *b"mvhd", |out| {
            out.put_u8(0); // version
            out.put_slice(&[0, 0, 0]); // flags
            out.put_u32(0); // creation_time
            out.put_u32(0); // modification_time
            out.put_u32(VIDEO_CLOCK_RATE_HZ); // timescale
            out.put_u32(0); // duration: unknown, fragmented
            out.put_i32(0x0001_0000); // rate: 1.0
            out.put_i16(0x0100); // volume: 1.0
            out.put_u16(0); // reserved
            out.put_u64(0); // reserved[2]
            for value in identity_matrix() {
                out.put_i32(value);
            }
            for _ in 0..6 {
                out.put_u32(0); // pre_defined
            }
            out.put_u32(TRACK_ID + 1); // next_track_ID
        });
    }

    fn tkhd(out: &mut BytesMut, width: u16, height: u16) {
        write_box(out, *b"tkhd", |out| {
            out.put_u8(0); // version
            out.put_slice(&[0, 0, 0x07]); // flags: enabled | in_movie | in_preview
            out.put_u32(0); // creation_time
            out.put_u32(0); // modification_time
            out.put_u32(TRACK_ID);
            out.put_u32(0); // reserved
            out.put_u32(0); // duration
            out.put_u64(0); // reserved[2]
            out.put_i16(0); // layer
            out.put_i16(0); // alternate_group
            out.put_i16(0); // volume: 0 for a video track
            out.put_u16(0); // reserved
            for value in identity_matrix() {
                out.put_i32(value);
            }
            out.put_u32(u32::from(width) << 16); // width, 16.16 fixed point
            out.put_u32(u32::from(height) << 16); // height, 16.16 fixed point
        });
    }

    fn mdhd(out: &mut BytesMut) {
        write_box(out, *b"mdhd", |out| {
            out.put_u8(0); // version
            out.put_slice(&[0, 0, 0]); // flags
            out.put_u32(0); // creation_time
            out.put_u32(0); // modification_time
            out.put_u32(VIDEO_CLOCK_RATE_HZ); // timescale
            out.put_u32(0); // duration: unknown, fragmented
            out.put_u16(0x55C4); // language: "und", ISO-639-2/T packed
            out.put_u16(0); // pre_defined
        });
    }

    fn hdlr(out: &mut BytesMut) {
        write_box(out, *b"hdlr", |out| {
            out.put_u32(0); // version + flags
            out.put_u32(0); // pre_defined
            out.put_slice(b"vide"); // handler_type
            out.put_u64(0); // reserved[3] (first 8 of 12 bytes)
            out.put_u32(0); // reserved[3] (remaining 4 bytes)
            out.put_slice(b"corvette-media-bridge\0"); // name, null-terminated
        });
    }

    fn vmhd(out: &mut BytesMut) {
        write_box(out, *b"vmhd", |out| {
            out.put_u32(1); // version 0, flags = 1 (required by the spec)
            out.put_u16(0); // graphicsmode
            out.put_u16(0); // opcolor[0]
            out.put_u16(0); // opcolor[1]
            out.put_u16(0); // opcolor[2]
        });
    }

    fn dinf(out: &mut BytesMut) {
        write_box(out, *b"dinf", |out| {
            write_box(out, *b"dref", |out| {
                out.put_u32(0); // version + flags
                out.put_u32(1); // entry_count
                write_box(out, *b"url ", |out| {
                    out.put_u32(1); // version 0, flags = 1: media is in this same file
                });
            });
        });
    }

    fn stsd_avc1(out: &mut BytesMut, sps: &[u8], pps: &[u8], width: u16, height: u16) {
        write_box(out, *b"stsd", |out| {
            out.put_u32(0); // version + flags
            out.put_u32(1); // entry_count
            write_box(out, *b"avc1", |out| {
                sample_entry_header(out, width, height);
                write_box(out, *b"avcC", |out| {
                    out.put_u8(1); // configurationVersion
                    out.put_u8(sps.get(1).copied().unwrap_or(0)); // AVCProfileIndication
                    out.put_u8(sps.get(2).copied().unwrap_or(0)); // profile_compatibility
                    out.put_u8(sps.get(3).copied().unwrap_or(0)); // AVCLevelIndication
                    out.put_u8(0xFC | 0x03); // reserved(6)=1 + lengthSizeMinusOne=3 (4-byte lengths)
                    out.put_u8(0xE0 | 0x01); // reserved(3)=1 + numOfSequenceParameterSets=1
                    out.put_u16(u16::try_from(sps.len()).unwrap_or(u16::MAX));
                    out.put_slice(sps);
                    out.put_u8(1); // numOfPictureParameterSets
                    out.put_u16(u16::try_from(pps.len()).unwrap_or(u16::MAX));
                    out.put_slice(pps);
                });
            });
        });
    }

    fn stsd_hvc1(out: &mut BytesMut, vps: &[u8], sps: &[u8], pps: &[u8], width: u16, height: u16) {
        write_box(out, *b"stsd", |out| {
            out.put_u32(0); // version + flags
            out.put_u32(1); // entry_count
            write_box(out, *b"hvc1", |out| {
                sample_entry_header(out, width, height);
                write_box(out, *b"hvcC", |out| {
                    write_hvcc_body(out, vps, sps, pps);
                });
            });
        });
    }

    fn sample_entry_header(out: &mut BytesMut, width: u16, height: u16) {
        out.put_u32(0); // reserved[6] (first 4 of 6 bytes)
        out.put_u16(0); // reserved[6] (remaining 2 bytes)
        out.put_u16(1); // data_reference_index
        out.put_u16(0); // pre_defined
        out.put_u16(0); // reserved
        out.put_u32(0); // pre_defined[3] (first 4 of 12 bytes)
        out.put_u64(0); // pre_defined[3] (remaining 8 bytes)
        out.put_u16(width);
        out.put_u16(height);
        out.put_u32(0x0048_0000); // horizresolution: 72 dpi
        out.put_u32(0x0048_0000); // vertresolution: 72 dpi
        out.put_u32(0); // reserved
        out.put_u16(1); // frame_count
        out.put_slice(&[0u8; 32]); // compressorname: empty
        out.put_u16(0x0018); // depth: 24
        out.put_i16(-1); // pre_defined
    }

    fn write_hvcc_body(out: &mut BytesMut, vps: &[u8], sps: &[u8], pps: &[u8]) {
        let profile_tier_level = h265_profile_tier_level(sps);
        out.put_u8(1); // configurationVersion
        out.put_u8(profile_tier_level[0]); // general_profile_space/tier_flag/profile_idc
        out.put_slice(&profile_tier_level[1..5]); // general_profile_compatibility_flags
        out.put_slice(&profile_tier_level[5..11]); // general_constraint_indicator_flags
        out.put_u8(profile_tier_level[11]); // general_level_idc
        out.put_u16(0xF000); // reserved(4)=1 + min_spatial_segmentation_idc=0
        out.put_u8(0xFC); // reserved(6)=1 + parallelismType=0
        out.put_u8(0xFD); // reserved(6)=1 + chroma_format_idc=1 (4:2:0)
        out.put_u8(0xF8); // reserved(5)=1 + bit_depth_luma_minus8=0
        out.put_u8(0xF8); // reserved(5)=1 + bit_depth_chroma_minus8=0
        out.put_u16(0); // avgFrameRate: unspecified
        // constantFrameRate=0, numTemporalLayers=1, temporalIdNested=0,
        // lengthSizeMinusOne=3 (4-byte lengths).
        out.put_u8(0b0000_1011);
        out.put_u8(3); // numOfArrays: VPS, SPS, PPS
        for (nal_unit_type, unit) in [(32u8, vps), (33u8, sps), (34u8, pps)] {
            out.put_u8(0x80 | nal_unit_type); // array_completeness=1, reserved=0
            out.put_u16(1); // numNalus
            out.put_u16(u16::try_from(unit.len()).unwrap_or(u16::MAX));
            out.put_slice(unit);
        }
    }

    fn h265_profile_tier_level(sps: &[u8]) -> [u8; 12] {
        let mut out = [0u8; 12];
        let start = 3; // 2-byte NAL header + 1 byte of SPS-level fields
        let end = start + 12;
        if let Some(field) = sps.get(start..end) {
            out.copy_from_slice(field);
        }
        out
    }

    fn stbl(out: &mut BytesMut, sample_entry: impl FnOnce(&mut BytesMut)) {
        write_box(out, *b"stbl", |out| {
            sample_entry(out);
            write_box(out, *b"stts", |out| {
                out.put_u32(0); // version + flags
                out.put_u32(0); // entry_count: empty, fragmented
            });
            write_box(out, *b"stsc", |out| {
                out.put_u32(0);
                out.put_u32(0);
            });
            write_box(out, *b"stsz", |out| {
                out.put_u32(0); // version + flags
                out.put_u32(0); // sample_size
                out.put_u32(0); // sample_count: empty, fragmented
            });
            write_box(out, *b"stco", |out| {
                out.put_u32(0);
                out.put_u32(0); // entry_count: empty, fragmented
            });
        });
    }

    fn mvex(out: &mut BytesMut) {
        write_box(out, *b"mvex", |out| {
            write_box(out, *b"trex", |out| {
                out.put_u32(0); // version + flags
                out.put_u32(TRACK_ID);
                out.put_u32(1); // default_sample_description_index
                out.put_u32(0); // default_sample_duration: always explicit in trun
                out.put_u32(0); // default_sample_size: always explicit in trun
                out.put_u32(0); // default_sample_flags: always explicit in trun
            });
        });
    }

    const fn identity_matrix() -> [i32; 9] {
        [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000]
    }

    pub(super) fn init_segment_h264(sps: &[u8], pps: &[u8]) -> Bytes {
        let (width, height) = PLACEHOLDER_DIMENSIONS;
        let mut out = BytesMut::new();
        ftyp(&mut out);
        write_box(&mut out, *b"moov", |out| {
            mvhd(out);
            write_box(out, *b"trak", |out| {
                tkhd(out, width, height);
                write_box(out, *b"mdia", |out| {
                    mdhd(out);
                    hdlr(out);
                    write_box(out, *b"minf", |out| {
                        vmhd(out);
                        dinf(out);
                        stbl(out, |out| stsd_avc1(out, sps, pps, width, height));
                    });
                });
            });
            mvex(out);
        });
        out.freeze()
    }

    pub(super) fn init_segment_h265(vps: &[u8], sps: &[u8], pps: &[u8]) -> Bytes {
        let (width, height) = PLACEHOLDER_DIMENSIONS;
        let mut out = BytesMut::new();
        ftyp(&mut out);
        write_box(&mut out, *b"moov", |out| {
            mvhd(out);
            write_box(out, *b"trak", |out| {
                tkhd(out, width, height);
                write_box(out, *b"mdia", |out| {
                    mdhd(out);
                    hdlr(out);
                    write_box(out, *b"minf", |out| {
                        vmhd(out);
                        dinf(out);
                        stbl(out, |out| stsd_hvc1(out, vps, sps, pps, width, height));
                    });
                });
            });
            mvex(out);
        });
        out.freeze()
    }

    const PLACEHOLDER_DIMENSIONS: (u16, u16) = (1920, 1080);

    pub(super) fn fragment(
        sequence_number: u32,
        base_media_decode_time: u64,
        sample_duration: u32,
        sample_payload: &[u8],
        is_keyframe: bool,
    ) -> Bytes {
        // docs/design/api-contracts.md explains how this offset is derived.
        const DATA_OFFSET_FIELD_OFFSET: usize = 84;

        let sample_flags: u32 = if is_keyframe {
            0x0200_0000 // sample_depends_on = 2 (I-frame; does not depend on others)
        } else {
            0x0101_0000 // sample_depends_on = 1, sample_is_non_sync_sample = 1
        };
        let sample_size = u32::try_from(sample_payload.len() + 4)
            .expect("one video access unit stays well under 4 GiB");

        let mut moof = BytesMut::new();
        write_box(&mut moof, *b"moof", |out| {
            write_box(out, *b"mfhd", |out| {
                out.put_u32(0); // version + flags
                out.put_u32(sequence_number);
            });
            write_box(out, *b"traf", |out| {
                write_box(out, *b"tfhd", |out| {
                    out.put_u32(0x02_0000); // flags: default-base-is-moof
                    out.put_u32(TRACK_ID);
                });
                write_box(out, *b"tfdt", |out| {
                    out.put_u32(1 << 24); // version 1 (64-bit base_media_decode_time)
                    out.put_u64(base_media_decode_time);
                });
                write_box(out, *b"trun", |out| {
                    // data-offset | duration | size | sample-flags present.
                    out.put_u32(0x0000_0701);
                    out.put_u32(1); // sample_count
                    out.put_i32(0); // data_offset: patched in below
                    out.put_u32(sample_duration);
                    out.put_u32(sample_size);
                    out.put_u32(sample_flags);
                });
            });
        });

        let moof_len = u32::try_from(moof.len()).expect("a single moof stays well under 4 GiB");
        let data_offset = (moof_len + 8).cast_signed(); // + mdat's own 8-byte header
        moof[DATA_OFFSET_FIELD_OFFSET..DATA_OFFSET_FIELD_OFFSET + 4]
            .copy_from_slice(&data_offset.to_be_bytes());

        let mut out = moof;
        write_box(&mut out, *b"mdat", |out| {
            out.put_u32(
                u32::try_from(sample_payload.len()).expect("checked above via sample_size"),
            );
            out.put_slice(sample_payload);
        });
        out.freeze()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real H.264 SPS/PPS/IDR triple, reused from the moq-mux import tests.
    const SPS: &[u8] = &[
        0x67, 0x42, 0xc0, 0x1f, 0xda, 0x01, 0x40, 0x16, 0xe9, 0xb8, 0x08, 0x08, 0x0a, 0x00, 0x00,
        0x07, 0xd0, 0x00, 0x01, 0xd4, 0xc0, 0x80,
    ];
    const PPS: &[u8] = &[0x68, 0xce, 0x3c, 0x80];
    const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x21];
    const NON_IDR: &[u8] = &[0x41, 0x9a, 0x24, 0x6c];

    fn annex_b(nal: &[u8]) -> Bytes {
        let mut payload = Vec::with_capacity(nal.len() + 4);
        payload.extend_from_slice(&[0, 0, 0, 1]);
        payload.extend_from_slice(nal);
        Bytes::from(payload)
    }

    fn frame(codec: ClientCodec, timestamp: u32, nal: &[u8]) -> ClientFrame {
        ClientFrame {
            codec,
            timestamp,
            payload: annex_b(nal),
        }
    }

    fn read_box_header(bytes: &[u8], offset: usize) -> ([u8; 4], usize) {
        let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let mut box_type = [0u8; 4];
        box_type.copy_from_slice(&bytes[offset + 4..offset + 8]);
        (box_type, size as usize)
    }

    #[test]
    fn init_segment_starts_with_ftyp_then_moov_and_carries_the_real_sps_pps() {
        let mut tracker = InitSegmentTracker::default();
        assert!(
            tracker.observe(&frame(ClientCodec::H264, 0, SPS)).is_none(),
            "SPS alone is not a complete set"
        );
        let segment = tracker
            .observe(&frame(ClientCodec::H264, 0, PPS))
            .expect("SPS+PPS complete the set");

        let (ftyp_type, ftyp_len) = read_box_header(&segment, 0);
        assert_eq!(&ftyp_type, b"ftyp");
        let (moov_type, moov_len) = read_box_header(&segment, ftyp_len);
        assert_eq!(&moov_type, b"moov");
        assert_eq!(ftyp_len + moov_len, segment.len());

        assert!(
            segment.windows(SPS.len()).any(|window| window == SPS),
            "the init segment must carry the real SPS bytes"
        );
        assert!(
            segment.windows(PPS.len()).any(|window| window == PPS),
            "the init segment must carry the real PPS bytes"
        );
    }

    #[test]
    fn init_segment_is_reemitted_only_when_parameter_sets_actually_change() {
        let mut tracker = InitSegmentTracker::default();
        tracker.observe(&frame(ClientCodec::H264, 0, SPS));
        let first = tracker
            .observe(&frame(ClientCodec::H264, 0, PPS))
            .expect("first resolution");

        assert!(
            tracker
                .observe(&frame(ClientCodec::H264, 1000, SPS))
                .is_none(),
            "a camera's own periodic re-announcement of the SAME SPS must not re-emit"
        );

        let changed_sps = &[0x67, 0x42, 0xc0, 0x28, 0xda, 0x01];
        let second = tracker
            .observe(&frame(ClientCodec::H264, 2000, changed_sps))
            .expect("a genuinely different SPS must re-emit the init segment");
        assert_ne!(first, second);
        assert!(second.windows(changed_sps.len()).any(|w| w == changed_sps));
    }

    #[test]
    fn fragmenter_produces_no_fragment_for_parameter_set_nals() {
        let mut fragmenter = Fragmenter::default();
        assert!(fragmenter.next(&frame(ClientCodec::H264, 0, SPS)).is_none());
        assert!(fragmenter.next(&frame(ClientCodec::H264, 0, PPS)).is_none());
    }

    #[test]
    fn fragmenter_emits_one_moof_mdat_pair_per_access_unit_in_avcc_framing() {
        let mut fragmenter = Fragmenter::default();
        let fragment = fragmenter
            .next(&frame(ClientCodec::H264, 0, IDR))
            .expect("a VCL slice NAL produces one fragment");

        let (moof_type, moof_len) = read_box_header(&fragment.bytes, 0);
        assert_eq!(&moof_type, b"moof");
        let (mdat_type, mdat_len) = read_box_header(&fragment.bytes, moof_len);
        assert_eq!(&mdat_type, b"mdat");
        assert_eq!(moof_len + mdat_len, fragment.bytes.len());

        let mdat_payload = &fragment.bytes[moof_len + 8..];
        let declared_len = u32::from_be_bytes(mdat_payload[0..4].try_into().unwrap()) as usize;
        assert_eq!(declared_len, IDR.len());
        assert_eq!(&mdat_payload[4..4 + declared_len], IDR);
    }

    #[test]
    fn fragmenter_reports_the_keyframe_flag_matching_the_nal_type() {
        let mut fragmenter = Fragmenter::default();
        let idr_fragment = fragmenter
            .next(&frame(ClientCodec::H264, 0, IDR))
            .expect("a VCL slice NAL produces one fragment");
        assert!(
            idr_fragment.is_keyframe,
            "an IDR slice is a keyframe/sync sample"
        );

        let non_idr_fragment = fragmenter
            .next(&frame(ClientCodec::H264, 3000, NON_IDR))
            .expect("a VCL slice NAL produces one fragment");
        assert!(
            !non_idr_fragment.is_keyframe,
            "a non-IDR slice is not a keyframe/sync sample"
        );
    }

    #[test]
    fn fragmenter_assigns_increasing_sequence_numbers() {
        let mut fragmenter = Fragmenter::default();
        let first = fragmenter.next(&frame(ClientCodec::H264, 0, IDR)).unwrap();
        let second = fragmenter
            .next(&frame(ClientCodec::H264, 3000, NON_IDR))
            .unwrap();

        let sequence_number = |fragment: &[u8]| {
            // moof(8) + mfhd header(8) + version/flags(4) precede sequence_number.
            u32::from_be_bytes(fragment[8 + 8 + 4..8 + 8 + 8].try_into().unwrap())
        };
        assert_eq!(sequence_number(&first.bytes), 1);
        assert_eq!(sequence_number(&second.bytes), 2);
    }

    #[test]
    fn fragmenter_measures_sample_duration_from_the_real_timestamp_delta() {
        let mut fragmenter = Fragmenter::default();
        fragmenter.next(&frame(ClientCodec::H264, 0, IDR));
        let second = fragmenter
            .next(&frame(ClientCodec::H264, 3000, NON_IDR))
            .unwrap();

        assert_eq!(second.duration_ticks, 3000);

        // sample_duration sits at byte 88: trun's data_offset field ends at 84.
        let sample_duration = u32::from_be_bytes(second.bytes[88..92].try_into().unwrap());
        assert_eq!(sample_duration, 3000);
    }
}
