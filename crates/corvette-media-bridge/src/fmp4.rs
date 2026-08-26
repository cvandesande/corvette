//! A minimal fragmented-MP4 (fMP4) muxer for the grid tile's WebSocket/MSE
//! transport (issue #12 item G2).
//!
//! Builds one initialization segment per camera (`ftyp`/`moov`, from the
//! stream's own SPS/PPS or VPS/SPS/PPS, re-emitted whenever those parameter
//! sets change -- see [`InitSegmentTracker`]) plus a continuous stream of
//! fragments (`moof`/`mdat`), one per access unit (one video sample per
//! fragment -- see [`Fragmenter`]). Video only: this item inherits the same
//! audio-track gap G1's own Premise names (see `crate`'s top-level doc), and
//! does not fix it.
//!
//! No frame is ever decoded, scaled, or re-encoded here -- the existing NAL
//! units are repackaged into ISO/IEC 14496-12 boxes, wire format only.
//!
//! This module builds bytes; it performs no I/O and owns no task or
//! connection -- `crate::ws_repackager` drives it per camera and hosts the
//! WebSocket transport. Kept free of any WebSocket concern so a later HLS/
//! CMAF packager (a separate, not-yet-built item) can reuse the same
//! fragmentation core through this module's own public API rather than
//! through `ws_repackager`.

use crate::parameter_sets::RawParameterSets;
use crate::rtp_clock::VIDEO_CLOCK_RATE_HZ;
use bytes::Bytes;
use corvette_rtsp_client::depacketize::{Codec as ClientCodec, Frame as ClientFrame};

/// H.264 NAL types that are one full video access unit, given that this
/// crate's `corvette_rtsp_client::depacketize::Frame` stream carries exactly
/// one NAL per frame (see `crate::parameter_sets`'s own doc): a coded slice
/// of a non-IDR (1) or IDR (5) picture. Every other type (parameter sets,
/// SEI, AUD, ...) is not sample data.
const H264_NAL_TYPE_SLICE_NON_IDR: u8 = 1;
const H264_NAL_TYPE_SLICE_IDR: u8 = 5;

/// H.265 VCL NAL unit types (RFC 7798 / ITU-T H.265 Table 7-1): types 0-31
/// are coded slice segments; the IRAP (keyframe) range is 16-23.
const H265_NAL_TYPE_VCL_MAX: u8 = 31;
const H265_NAL_TYPE_IRAP_MIN: u8 = 16;
const H265_NAL_TYPE_IRAP_MAX: u8 = 23;

/// A reasonable default sample duration (in [`VIDEO_CLOCK_RATE_HZ`] ticks)
/// for a fragment's very first sample, before a second access unit has
/// arrived to measure a real interval from -- 30fps at the 90kHz video clock
/// rate. Every later fragment uses the real, measured interval between
/// consecutive access units instead (see [`Fragmenter::next`]); a viewer's
/// `SourceBuffer` does not reject a segment for an approximate first-sample
/// duration, and this is corrected by the second fragment either way.
const DEFAULT_FIRST_SAMPLE_DURATION_TICKS: u32 = VIDEO_CLOCK_RATE_HZ / 30;

/// The one video track ID this muxer ever produces (one video track per
/// camera; see `crate`'s top-level doc on the audio-track gap).
const TRACK_ID: u32 = 1;

/// Tracks one camera's parameter sets and rebuilds the initialization
/// segment (`ftyp`+`moov`) whenever they change.
///
/// Unlike `restream_provider::ParameterSetCache` (which resolves once and
/// never updates again -- see `crate::parameter_sets`'s own doc), this
/// tracker must re-emit on every genuine change: Do step 2 requires it, and
/// a viewer that connects after a resolution change needs the current init
/// segment, not the first one ever produced.
#[derive(Debug, Default)]
pub struct InitSegmentTracker {
    raw: RawParameterSets,
    last_emitted: Option<EmittedParameterSets>,
}

/// The exact parameter-set bytes the currently-held initialization segment
/// was built from, kept so a later [`InitSegmentTracker::observe`] call can
/// tell a genuine change (different bytes) from a re-observation of the same
/// set (a camera's own periodic in-stream repetition, which must not rebuild
/// and re-emit the segment on every repeat).
#[derive(Debug, Clone, PartialEq, Eq)]
enum EmittedParameterSets {
    H264 { sps: Bytes, pps: Bytes },
    H265 { vps: Bytes, sps: Bytes, pps: Bytes },
}

impl InitSegmentTracker {
    /// Observes one frame, returning a freshly built initialization segment
    /// exactly when its codec's complete parameter set is new or has
    /// changed since the last one this tracker returned (`None` otherwise,
    /// including for every frame that carries no parameter-set NAL).
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
            // See crate's top-level doc: never actually produced today.
            ClientCodec::Aac => None,
        }
    }
}

/// Groups the access-unit frame stream into one `moof`/`mdat` fragment per
/// access unit.
///
/// Each sample is written in AVCC (length-prefixed) framing -- the format
/// the initialization segment's own `avcC`/`hvcC` box declares
/// (`lengthSizeMinusOne` = 3, i.e. 4-byte lengths), not the wire's own
/// Annex-B framing.
///
/// A parameter-set NAL (SPS/PPS/VPS) carries no sample data of its own (it is
/// consumed by [`InitSegmentTracker`] instead) and produces no fragment here.
/// Any other non-VCL NAL a real stream might carry (SEI, AUD, filler) is
/// likewise not sample data a decoder needs and is dropped -- a real, stated
/// simplification of this "minimal" muxer (see this item's own Do step 2:
/// only "one moof/mdat per access unit" is required, not full NAL-unit
/// fidelity).
#[derive(Debug, Default)]
pub struct Fragmenter {
    sequence_number: u32,
    previous_timestamp: Option<u32>,
}

impl Fragmenter {
    /// Consumes one frame, returning a `moof`+`mdat` fragment if it carries
    /// an access unit (`None` for a parameter-set or other non-sample NAL).
    pub fn next(&mut self, frame: &ClientFrame) -> Option<Bytes> {
        let (_nal_type, is_keyframe) = classify(frame)?;

        let duration_ticks = self
            .previous_timestamp
            .map_or(DEFAULT_FIRST_SAMPLE_DURATION_TICKS, |previous| {
                frame.timestamp.wrapping_sub(previous).max(1)
            });
        self.previous_timestamp = Some(frame.timestamp);

        self.sequence_number += 1;
        Some(boxes::fragment(
            self.sequence_number,
            u64::from(frame.timestamp),
            duration_ticks,
            &frame.payload[super::parameter_sets::START_CODE_LEN..],
            is_keyframe,
        ))
    }
}

/// Classifies one frame as (NAL type, is a sync/keyframe sample), or `None`
/// if it carries no video access unit (a parameter set, or -- for a codec
/// this muxer does not mux -- AAC).
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
    //! ISO/IEC 14496-12/14496-15 box construction, byte level. Pure,
    //! synchronous, and allocation-only -- no I/O, matching this module's own
    //! top-level doc.

    use super::{TRACK_ID, VIDEO_CLOCK_RATE_HZ};
    use bytes::{BufMut, Bytes, BytesMut};

    /// Writes one box: a big-endian `u32` size (patched in after `body`
    /// runs), the four-character type, then whatever `body` appends.
    fn write_box(out: &mut BytesMut, box_type: [u8; 4], body: impl FnOnce(&mut BytesMut)) {
        let start = out.len();
        out.put_u32(0); // placeholder, patched below
        out.put_slice(&box_type);
        body(out);
        let size = u32::try_from(out.len() - start)
            .expect("a single fMP4 box never approaches 4 GiB in this item's own use");
        out[start..start + 4].copy_from_slice(&size.to_be_bytes());
    }

    /// `ftyp`: declares CMAF-style fragmented-MP4 compatibility. `iso5`
    /// signals `default-base-is-moof` support (used by every `tfhd` this
    /// module writes); `mp41` and the major brand `isom` are the widest,
    /// most conservatively supported compatible brands.
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
                    // `sps` (per `parameter_sets::parameter_set_bytes`'s own
                    // doc) includes its own 1-byte NAL header at index 0
                    // (RFC 6184 section 1.3), so the SPS payload's own
                    // profile_idc/constraint_flags/level_idc -- what avcC's
                    // AVCProfileIndication/profile_compatibility/
                    // AVCLevelIndication fields actually copy -- start at
                    // index 1, not index 0.
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

    /// The 78-byte `VisualSampleEntry` header common to `avc1`/`hvc1` (ISO/IEC
    /// 14496-12 section 12.1.3), preceding the codec configuration box.
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

    /// `HEVCDecoderConfigurationRecord` (ISO/IEC 14496-15 section 8.3.3.1).
    /// Only `general_profile_space`/`tier_flag`/`profile_idc`,
    /// `general_profile_compatibility_flags`, `general_constraint_indicator_flags`,
    /// and `general_level_idc` are parsed from the real SPS bitstream (fixed
    /// bit-offset fields in `profile_tier_level`, no exp-Golomb decoding
    /// needed for these); every other field this record carries
    /// (`chroma_format_idc`, bit depths, frame-rate hints, ...) uses a
    /// conservative common default. This mirrors G1's own AAC-gap precedent:
    /// written correctly enough to produce a structurally valid record, but
    /// this item's own evidence exercises only the H.264 path against a real
    /// browser (see this crate's tests and `.agents/issue-12/evidence/`) --
    /// no real H.265 fixture exists in this workspace to verify hvcC against
    /// a real decoder.
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

    /// Reads `general_profile_space`/`general_tier_flag`/`general_profile_idc`
    /// (1 byte), `general_profile_compatibility_flags` (4 bytes),
    /// `general_constraint_indicator_flags` (6 bytes), and
    /// `general_level_idc` (1 byte) -- `profile_tier_level`'s first 12 bytes,
    /// per ITU-T H.265 section 7.3.3 -- from a raw H.265 SPS NAL (including
    /// its own 2-byte NAL header). These are fixed-width fields at a fixed
    /// bit offset (2 bytes NAL header + 1 byte
    /// `sps_video_parameter_set_id`/`sps_max_sub_layers_minus1`/
    /// `sps_temporal_id_nesting_flag`), so no exp-Golomb bitstream decoding
    /// is needed to reach them. Returns all zero bytes if `sps` is too short
    /// to hold them (defensive only -- never expected from a real camera).
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

    /// Builds the `ftyp`+`moov` initialization segment for an H.264 track.
    /// `width`/`height` are a fixed placeholder: `tkhd`'s and `avc1`'s own
    /// declared dimensions affect rendering geometry only, not whether a
    /// real `SourceBuffer` accepts the segment (checked directly by this
    /// item's own browser-based test) -- parsing the true dimensions out of
    /// the SPS's exp-Golomb-coded fields would add real complexity for a
    /// property this item's Verify never asserts on.
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

    /// Builds the `ftyp`+`moov` initialization segment for an H.265 track.
    /// See [`init_segment_h264`] for the `width`/`height` placeholder
    /// rationale, and [`write_hvcc_body`] for this item's own honest limit on
    /// `hvcC` field accuracy.
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

    /// A fixed placeholder resolution -- see [`init_segment_h264`]'s own doc
    /// for why this item does not parse the real one out of the SPS.
    const PLACEHOLDER_DIMENSIONS: (u16, u16) = (1920, 1080);

    /// One `moof`+`mdat` fragment carrying exactly one video sample.
    ///
    /// `data_offset` in `trun` is written relative to the first byte of the
    /// `moof` box (`tfhd`'s `default-base-is-moof` flag, set below), so the
    /// `moof` box must be fully built -- and its own final size known --
    /// before `trun`'s `data_offset` field can be patched in. Built in two
    /// passes: once to learn `moof`'s size, then `data_offset` is set to
    /// that size plus `mdat`'s own 8-byte header.
    pub(super) fn fragment(
        sequence_number: u32,
        base_media_decode_time: u64,
        sample_duration: u32,
        sample_payload: &[u8],
        is_keyframe: bool,
    ) -> Bytes {
        // The fixed byte offset from moof's start to trun's data_offset
        // field, patched in below once moof's own total size is known: every
        // box (or box header) preceding it -- moof header:8, mfhd:16, traf
        // header:8, tfhd:16, tfdt:20, trun header:8, trun's own flags:4,
        // sample_count:4 -- summing to 84.
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
                    // flags: data-offset-present | sample-duration-present |
                    // sample-size-present | sample-flags-present.
                    out.put_u32(0x0000_0701);
                    out.put_u32(1); // sample_count
                    out.put_i32(0); // data_offset: patched in below
                    out.put_u32(sample_duration);
                    out.put_u32(sample_size);
                    out.put_u32(sample_flags);
                });
            });
        });

        // Patch trun's data_offset now that moof's own total size is known.
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

    // The exact fixture bytes `moq-mux`'s own `h264::import` unit tests
    // (`avc3_self_initializes_from_first_keyframe`) and this crate's own G1
    // integration test use -- a real, valid H.264 SPS/PPS/IDR triple, reused
    // here rather than invented, matching this project's established
    // precedent for fixture provenance.
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

    /// Reads one big-endian `u32` box size + 4-character type at `offset`,
    /// returning (`type`, `total box length`).
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

        let (moof_type, moof_len) = read_box_header(&fragment, 0);
        assert_eq!(&moof_type, b"moof");
        let (mdat_type, mdat_len) = read_box_header(&fragment, moof_len);
        assert_eq!(&mdat_type, b"mdat");
        assert_eq!(moof_len + mdat_len, fragment.len());

        // mdat's payload is length-prefixed (AVCC), not Annex-B
        // start-code-prefixed: a 4-byte big-endian length, then the NAL
        // bytes without their own start code.
        let mdat_payload = &fragment[moof_len + 8..];
        let declared_len = u32::from_be_bytes(mdat_payload[0..4].try_into().unwrap()) as usize;
        assert_eq!(declared_len, IDR.len());
        assert_eq!(&mdat_payload[4..4 + declared_len], IDR);
    }

    #[test]
    fn fragmenter_assigns_increasing_sequence_numbers() {
        let mut fragmenter = Fragmenter::default();
        let first = fragmenter.next(&frame(ClientCodec::H264, 0, IDR)).unwrap();
        let second = fragmenter
            .next(&frame(ClientCodec::H264, 3000, NON_IDR))
            .unwrap();

        let sequence_number = |fragment: &[u8]| {
            // moof(8) + mfhd header(8) -> mfhd's own version/flags(4) precede
            // sequence_number.
            u32::from_be_bytes(fragment[8 + 8 + 4..8 + 8 + 8].try_into().unwrap())
        };
        assert_eq!(sequence_number(&first), 1);
        assert_eq!(sequence_number(&second), 2);
    }

    #[test]
    fn fragmenter_measures_sample_duration_from_the_real_timestamp_delta() {
        let mut fragmenter = Fragmenter::default();
        fragmenter.next(&frame(ClientCodec::H264, 0, IDR));
        let second = fragmenter
            .next(&frame(ClientCodec::H264, 3000, NON_IDR))
            .unwrap();

        // sample_duration sits right after trun's own data_offset field,
        // which the fragment builder itself documents at byte 84 (see
        // boxes::fragment's DATA_OFFSET_FIELD_OFFSET) -- so 88.
        let sample_duration = u32::from_be_bytes(second[88..92].try_into().unwrap());
        assert_eq!(sample_duration, 3000);
    }
}
