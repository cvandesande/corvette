//! Builds a `DESCRIBE` response's SDP body from a [`StreamInfo`]
//! (`RESEARCH-rtsp-restream-server.md` F-13's per-codec fmtp shapes for
//! H.264/H.265/AAC).

use crate::provider::{StreamInfo, TrackInfo};
use base64::Engine as _;
use sdp_types::{Connection, Media, MediaType, Origin, Session, TransportProto};
use std::net::Ipv4Addr;

/// The RTP clock rate H.264 and H.265 both use, fixed by RFC 6184 §5.1 /
/// RFC 7798 §5.1 (not negotiable per-stream, unlike AAC's sample rate).
const VIDEO_CLOCK_RATE_HZ: u32 = 90_000;

/// The first RTP payload type this crate assigns. RFC 3551 §3 reserves
/// 96-127 for dynamic per-session assignment; nothing here needs to match a
/// caller's own numbering since the payload type only has to agree between
/// this crate's own `DESCRIBE` and `SETUP`/`PLAY` handling, both in
/// `session.rs`.
const FIRST_DYNAMIC_PAYLOAD_TYPE: u8 = 96;

/// The RTP payload type this crate assigns to the track at `index` in a
/// stream's declared track order -- the same numbering [`build_sdp`] embeds
/// in the `DESCRIBE` response's `m=`/`a=rtpmap` lines. An embedder's own
/// async I/O layer (issue #12 item X3) calls this to build a packetizer
/// with the exact payload type value the client already learned from
/// `DESCRIBE`, without duplicating this crate's own numbering scheme.
///
/// # Panics
///
/// Panics if `index` is 32 or higher, matching [`build_sdp`]'s own limit.
#[must_use]
pub(crate) fn payload_type_for_track(index: usize) -> u8 {
    FIRST_DYNAMIC_PAYLOAD_TYPE
        .checked_add(u8::try_from(index).expect("fewer than 32 tracks per stream"))
        .expect("fewer than 32 tracks per stream")
}

/// Builds the SDP body a `DESCRIBE` response returns for `stream`.
///
/// Each track is assigned a payload type starting at 96, in declaration
/// order, and an `a=control:trackID=<index>` attribute using the same
/// `trackID=<index>` convention `session::parse_track_index` parses back
/// out of a `SETUP` request's URI.
///
/// # Panics
///
/// Panics if `stream` has more than 32 tracks (payload types would run
/// past the dynamic range) or if an H.264/H.265 parameter set is too short
/// to contain the bytes its fmtp template needs -- both indicate a caller
/// bug upstream of this crate, not malformed network input.
#[must_use]
pub fn build_sdp(stream: &StreamInfo) -> Vec<u8> {
    let origin = Origin::with_ip_addr("0", 0, Ipv4Addr::UNSPECIFIED);
    let mut session = Session::new(origin, stream.name.as_str());
    session.connection = Some(Connection::from_ip_addr(Ipv4Addr::UNSPECIFIED));

    for (index, track) in stream.tracks.iter().enumerate() {
        let payload_type = payload_type_for_track(index);
        let mut media = build_media(payload_type, track);
        media.add_attribute_with_value("control", format!("trackID={index}"));
        session.medias.push(media);
    }

    let mut body = Vec::new();
    session
        .write(&mut body)
        .expect("writing SDP to an in-memory Vec<u8> never fails");
    body
}

fn build_media(payload_type: u8, track: &TrackInfo) -> Media {
    match track {
        TrackInfo::H264 { sps, pps } => {
            let mut media = Media::new(
                MediaType::Video,
                0,
                TransportProto::RtpAvp,
                payload_type.to_string(),
            );
            media.add_attribute_with_value(
                "rtpmap",
                format!("{payload_type} H264/{VIDEO_CLOCK_RATE_HZ}"),
            );
            media.add_attribute_with_value("fmtp", h264_fmtp(payload_type, sps, pps));
            media
        }
        TrackInfo::H265 { vps, sps, pps } => {
            let mut media = Media::new(
                MediaType::Video,
                0,
                TransportProto::RtpAvp,
                payload_type.to_string(),
            );
            media.add_attribute_with_value(
                "rtpmap",
                format!("{payload_type} H265/{VIDEO_CLOCK_RATE_HZ}"),
            );
            media.add_attribute_with_value("fmtp", h265_fmtp(payload_type, vps, sps, pps));
            media
        }
        TrackInfo::Aac {
            audio_specific_config,
            sample_rate,
            channels,
        } => {
            let mut media = Media::new(
                MediaType::Audio,
                0,
                TransportProto::RtpAvp,
                payload_type.to_string(),
            );
            media.add_attribute_with_value(
                "rtpmap",
                format!("{payload_type} mpeg4-generic/{sample_rate}/{channels}"),
            );
            media.add_attribute_with_value("fmtp", aac_fmtp(payload_type, audio_specific_config));
            media
        }
    }
}

/// `packetization-mode=1;profile-level-id=<hex 3-byte>;sprop-parameter-sets=<b64
/// SPS>,<b64 PPS>` (RFC 6184).
fn h264_fmtp(payload_type: u8, sps: &[u8], pps: &[u8]) -> String {
    assert!(
        sps.len() >= 4,
        "H.264 SPS must be at least 4 bytes (NAL header + profile_idc + \
         constraint_flags + level_idc); got {} bytes",
        sps.len()
    );
    let profile_level_id = to_hex(&sps[1..4]);
    format!(
        "{payload_type} packetization-mode=1;profile-level-id={profile_level_id};\
         sprop-parameter-sets={},{}",
        base64_encode(sps),
        base64_encode(pps),
    )
}

/// `sprop-vps=<b64>;sprop-sps=<b64>;sprop-pps=<b64>` (RFC 7798-style,
/// minimal form -- no `profile-level-id`).
fn h265_fmtp(payload_type: u8, vps: &[u8], sps: &[u8], pps: &[u8]) -> String {
    format!(
        "{payload_type} sprop-vps={};sprop-sps={};sprop-pps={}",
        base64_encode(vps),
        base64_encode(sps),
        base64_encode(pps),
    )
}

/// `streamtype=5;profile-level-id=1;mode=AAC-hbr;sizelength=13;indexlength=3;
/// indexdeltalength=3;config=<hex AudioSpecificConfig>` (RFC 3640-style
/// MPEG4-GENERIC).
fn aac_fmtp(payload_type: u8, audio_specific_config: &[u8]) -> String {
    format!(
        "{payload_type} streamtype=5;profile-level-id=1;mode=AAC-hbr;sizelength=13;\
         indexlength=3;indexdeltalength=3;config={}",
        to_hex(audio_specific_config),
    )
}

fn base64_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(String::new(), |mut hex, byte| {
        write!(hex, "{byte:02X}").expect("writing to a String never fails");
        hex
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sdp_as_str(stream: &StreamInfo) -> String {
        String::from_utf8(build_sdp(stream)).expect("SDP body is ASCII")
    }

    #[test]
    fn h264_fmtp_matches_rfc_6184_template_shape() {
        let stream = StreamInfo {
            name: "camera1".to_string(),
            tracks: vec![TrackInfo::H264 {
                sps: vec![0x67, 0x42, 0x00, 0x1f, 0xab, 0xcd],
                pps: vec![0x68, 0xce, 0x3c, 0x80],
            }],
        };
        let sdp = sdp_as_str(&stream);

        assert!(sdp.contains("m=video 0 RTP/AVP 96\r\n"), "sdp:\n{sdp}");
        assert!(sdp.contains("a=rtpmap:96 H264/90000\r\n"), "sdp:\n{sdp}");
        assert!(
            sdp.contains(
                "a=fmtp:96 packetization-mode=1;profile-level-id=42001F;\
                 sprop-parameter-sets=Z0IAH6vN,aM48gA==\r\n"
            ),
            "sdp:\n{sdp}"
        );
        assert!(sdp.contains("a=control:trackID=0\r\n"), "sdp:\n{sdp}");
    }

    #[test]
    fn h265_fmtp_matches_rfc_7798_minimal_template_shape() {
        let stream = StreamInfo {
            name: "camera1".to_string(),
            tracks: vec![TrackInfo::H265 {
                vps: vec![0x40, 0x01, 0x0c],
                sps: vec![0x42, 0x01, 0x02],
                pps: vec![0x44, 0x01],
            }],
        };
        let sdp = sdp_as_str(&stream);

        assert!(sdp.contains("m=video 0 RTP/AVP 96\r\n"), "sdp:\n{sdp}");
        assert!(sdp.contains("a=rtpmap:96 H265/90000\r\n"), "sdp:\n{sdp}");
        assert!(
            sdp.contains("a=fmtp:96 sprop-vps=QAEM;sprop-sps=QgEC;sprop-pps=RAE=\r\n"),
            "sdp:\n{sdp}"
        );
        assert!(
            !sdp.contains("profile-level-id"),
            "H.265's minimal fmtp form carries no profile-level-id; sdp:\n{sdp}"
        );
    }

    #[test]
    fn aac_fmtp_matches_rfc_3640_mpeg4_generic_template_shape() {
        let stream = StreamInfo {
            name: "camera1".to_string(),
            tracks: vec![TrackInfo::Aac {
                audio_specific_config: vec![0x12, 0x10],
                sample_rate: 48_000,
                channels: 2,
            }],
        };
        let sdp = sdp_as_str(&stream);

        assert!(sdp.contains("m=audio 0 RTP/AVP 96\r\n"), "sdp:\n{sdp}");
        assert!(
            sdp.contains("a=rtpmap:96 mpeg4-generic/48000/2\r\n"),
            "sdp:\n{sdp}"
        );
        assert!(
            sdp.contains(
                "a=fmtp:96 streamtype=5;profile-level-id=1;mode=AAC-hbr;sizelength=13;\
                 indexlength=3;indexdeltalength=3;config=1210\r\n"
            ),
            "sdp:\n{sdp}"
        );
    }

    #[test]
    fn assigns_sequential_payload_types_and_track_ids_in_declaration_order() {
        let stream = StreamInfo {
            name: "camera1".to_string(),
            tracks: vec![
                TrackInfo::H264 {
                    sps: vec![0x67, 0x42, 0x00, 0x1f],
                    pps: vec![0x68, 0xce],
                },
                TrackInfo::Aac {
                    audio_specific_config: vec![0x11, 0x90],
                    sample_rate: 16_000,
                    channels: 1,
                },
            ],
        };
        let sdp = sdp_as_str(&stream);

        assert!(sdp.contains("m=video 0 RTP/AVP 96\r\n"), "sdp:\n{sdp}");
        assert!(sdp.contains("a=control:trackID=0\r\n"), "sdp:\n{sdp}");
        assert!(sdp.contains("m=audio 0 RTP/AVP 97\r\n"), "sdp:\n{sdp}");
        assert!(sdp.contains("a=control:trackID=1\r\n"), "sdp:\n{sdp}");
    }

    #[test]
    #[should_panic(expected = "H.264 SPS must be at least 4 bytes")]
    fn panics_on_a_too_short_h264_sps() {
        let stream = StreamInfo {
            name: "camera1".to_string(),
            tracks: vec![TrackInfo::H264 {
                sps: vec![0x67, 0x42],
                pps: vec![0x68],
            }],
        };
        let _ = build_sdp(&stream);
    }
}
