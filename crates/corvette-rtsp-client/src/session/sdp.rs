//! Resolves a `DESCRIBE` response's SDP body to the video track's own
//! control URL and codec parameters.
//!
//! A camera's SDP body carries a session-level `a=control:*` attribute
//! before the `m=video` section, and a distinct, track-specific
//! `a=control:trackN` attribute inside it. This project's own real-camera
//! testing hit exactly this shape: a parser that resolves the first
//! `a=control` match in document order picks the session-level line and
//! gets a `404` on `SETUP`. The fix is structural, not a parsing trick: this
//! module never reads the session's own `attributes`, only the video
//! `Media`'s -- there is no code path here that could pick the wrong one.

use rtsp_types::Url;
use sdp_types::{Fmtp, RtpMap, Session as SdpSession};

/// The video track's control URL and codec parameters extracted from one
/// `DESCRIBE` response, for `D1`'s depacketizer to consume.
///
/// `format_params` is exposed as raw `(name, value)` pairs exactly as they
/// appear in the video section's `a=fmtp` attribute (e.g.
/// `sprop-parameter-sets` for H.264, still base64-encoded). This crate does
/// not decode them: which parameters matter, and how, is specific to the
/// codec D1 depacketizes, not something this protocol layer should guess at
/// or hardcode a fixed enum of.
#[derive(Debug, Clone)]
pub struct TrackDescription {
    /// The absolute URL to `SETUP` and address subsequent `PLAY`/interleaved
    /// framing against for this track.
    pub control_url: Url,
    /// The video section's `a=rtpmap` encoding name, e.g. `"H264"` or
    /// `"H265"`.
    pub codec_name: String,
    /// The video section's `a=rtpmap` clock rate, in Hz.
    pub clock_rate: u32,
    /// The video section's `a=fmtp` format-specific parameters, in the
    /// order they appear. Empty if the section has no `fmtp` attribute.
    pub format_params: Vec<(String, Option<String>)>,
}

/// Errors resolving a video track from an SDP body.
#[derive(Debug)]
pub enum SdpError {
    /// The SDP body did not parse per RFC 8866.
    Parse(sdp_types::ParserError),
    /// The SDP body has no `m=video` section at all.
    NoVideoTrack,
    /// The video section has no `a=control` attribute to resolve.
    NoControlAttribute,
    /// The video section has no `a=rtpmap` attribute to read the codec from.
    NoRtpMap,
    /// The video section's own control URL did not parse against the
    /// `Content-Base`.
    InvalidControlUrl,
}

impl std::fmt::Display for SdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(err) => write!(f, "malformed SDP body: {err}"),
            Self::NoVideoTrack => write!(f, "SDP body has no m=video section"),
            Self::NoControlAttribute => write!(f, "video section has no a=control attribute"),
            Self::NoRtpMap => write!(f, "video section has no a=rtpmap attribute"),
            Self::InvalidControlUrl => {
                write!(
                    f,
                    "video section's a=control value did not resolve to a valid URL"
                )
            }
        }
    }
}

impl std::error::Error for SdpError {}

/// Parses `body` as SDP and resolves the video track's own control URL and
/// codec parameters, joining a relative `a=control` value against
/// `content_base`.
pub(super) fn resolve_video_track(
    body: &[u8],
    content_base: &Url,
) -> Result<TrackDescription, SdpError> {
    let sdp = SdpSession::parse(body).map_err(SdpError::Parse)?;

    let video = sdp
        .medias
        .iter()
        .find(|media| media.media == "video")
        .ok_or(SdpError::NoVideoTrack)?;

    // Deliberately `video.get_first_attribute_value`, never
    // `sdp.get_first_attribute_value` -- the session-level `a=control:*`
    // is a different attribute on a different struct and is never read
    // here.
    let control = video
        .get_first_attribute_value("control")
        .flatten()
        .ok_or(SdpError::NoControlAttribute)?;
    let control_url = resolve_control_url(content_base, control)?;

    let rtpmap = video
        .get_first_attribute_typed::<RtpMap>()
        .and_then(Result::ok)
        .ok_or(SdpError::NoRtpMap)?;

    let format_params = video
        .get_first_attribute_typed::<Fmtp>()
        .and_then(Result::ok)
        .map(|fmtp| {
            fmtp.format_specific_params
                .into_iter()
                .map(|param| (param.param, param.val))
                .collect()
        })
        .unwrap_or_default();

    Ok(TrackDescription {
        control_url,
        codec_name: rtpmap.encoding_name,
        clock_rate: rtpmap.clock_rate,
        format_params,
    })
}

/// Resolves an `a=control` value against `content_base`, per RFC 2326
/// §C.1.1: `*` means the base URL itself (aggregate control); an absolute
/// URL is used as-is; anything else is a relative reference joined against
/// the base.
fn resolve_control_url(content_base: &Url, control: &str) -> Result<Url, SdpError> {
    if control == "*" {
        return Ok(content_base.clone());
    }
    if let Ok(absolute) = Url::parse(control) {
        return Ok(absolute);
    }
    content_base
        .join(control)
        .map_err(|_| SdpError::InvalidControlUrl)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SDP_WITH_SESSION_AND_TRACK_CONTROL: &str = "v=0\r\n\
        o=- 0 0 IN IP4 127.0.0.1\r\n\
        s=corvette-rtsp-client mock camera\r\n\
        c=IN IP4 0.0.0.0\r\n\
        t=0 0\r\n\
        a=control:*\r\n\
        m=video 0 RTP/AVP 96\r\n\
        a=rtpmap:96 H264/90000\r\n\
        a=control:track1\r\n";

    #[test]
    fn resolves_the_video_section_control_not_the_session_level_one() {
        let content_base = Url::parse("rtsp://127.0.0.1/stream/").unwrap();
        let track =
            resolve_video_track(SDP_WITH_SESSION_AND_TRACK_CONTROL.as_bytes(), &content_base)
                .expect("resolves the video track");

        assert_eq!(track.control_url.as_str(), "rtsp://127.0.0.1/stream/track1");
        assert_eq!(track.codec_name, "H264");
        assert_eq!(track.clock_rate, 90000);
    }

    #[test]
    fn resolves_an_absolute_control_url_without_joining_it_to_content_base() {
        let sdp = "v=0\r\n\
            o=- 0 0 IN IP4 127.0.0.1\r\n\
            s=s\r\n\
            t=0 0\r\n\
            m=video 0 RTP/AVP 96\r\n\
            a=rtpmap:96 H264/90000\r\n\
            a=control:rtsp://camera.example/absolute/track\r\n";
        let content_base = Url::parse("rtsp://127.0.0.1/stream/").unwrap();
        let track = resolve_video_track(sdp.as_bytes(), &content_base).expect("resolves");

        assert_eq!(
            track.control_url.as_str(),
            "rtsp://camera.example/absolute/track"
        );
    }

    #[test]
    fn exposes_fmtp_format_params_verbatim_for_a_later_item_to_decode() {
        let sdp = "v=0\r\n\
            o=- 0 0 IN IP4 127.0.0.1\r\n\
            s=s\r\n\
            t=0 0\r\n\
            m=video 0 RTP/AVP 96\r\n\
            a=rtpmap:96 H264/90000\r\n\
            a=fmtp:96 packetization-mode=1;sprop-parameter-sets=Z0IAHukBQHu/LAAB,aM4wpIA=\r\n\
            a=control:track1\r\n";
        let content_base = Url::parse("rtsp://127.0.0.1/stream/").unwrap();
        let track = resolve_video_track(sdp.as_bytes(), &content_base).expect("resolves");

        assert_eq!(
            track.format_params,
            vec![
                ("packetization-mode".to_string(), Some("1".to_string())),
                (
                    "sprop-parameter-sets".to_string(),
                    Some("Z0IAHukBQHu/LAAB,aM4wpIA=".to_string())
                ),
            ]
        );
    }

    #[test]
    fn errors_when_the_sdp_has_no_video_section() {
        let sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=s\r\nt=0 0\r\n";
        let content_base = Url::parse("rtsp://127.0.0.1/stream/").unwrap();
        assert!(matches!(
            resolve_video_track(sdp.as_bytes(), &content_base),
            Err(SdpError::NoVideoTrack)
        ));
    }
}
