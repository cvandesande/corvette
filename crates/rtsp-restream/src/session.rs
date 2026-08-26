//! The server-side RTSP session/protocol state machine (D-6/D-7):
//! `OPTIONS`, `DESCRIBE`, `SETUP`, `PLAY`, `TEARDOWN` against a
//! caller-supplied [`StreamProvider`].
//!
//! This module owns no socket or async-runtime code. It parses and builds
//! [`rtsp_types`] messages only; this project's own async I/O layer (issue
//! #12 item X3) owns the `TcpListener`/`accept()` loop, reads/writes bytes
//! over the wire, and drives one [`RtspSession`] per accepted connection.

use crate::provider::{StreamInfo, StreamProvider};
use crate::sdp::build_sdp;
use rtsp_types::headers::{
    CONTENT_BASE, CONTENT_TYPE, CSEQ, RtpLowerTransport, RtpProfile, SESSION, Transport,
};
use rtsp_types::{Method, Request, Response, ResponseBuilder, StatusCode, headers};

use std::sync::atomic::{AtomicU64, Ordering};

/// The exact method set this crate implements, advertised by `OPTIONS` --
/// no more (go2rtc's own `OPTIONS` response advertises `PAUSE`, `ANNOUNCE`,
/// and `RECORD` too but implements none of them; D-7 deliberately narrows
/// this crate to only what it actually implements).
const IMPLEMENTED_METHODS: [Method; 5] = [
    Method::Options,
    Method::Describe,
    Method::Setup,
    Method::Play,
    Method::Teardown,
];

/// One RTSP session's state, scoped to a single client connection.
///
/// A session starts in [`State::Init`]. `DESCRIBE` moves it to
/// [`State::Described`]. A successful `SETUP` -- which this crate only
/// accepts once a `DESCRIBE` on the same connection has named a stream --
/// allocates a session id and moves it to [`State::Ready`]. `PLAY` moves
/// `Ready` to [`State::Playing`]. `TEARDOWN` resets to `Init`.
#[derive(Debug, Default)]
pub struct RtspSession {
    state: State,
}

#[derive(Debug, Default, Clone)]
enum State {
    #[default]
    Init,
    Described {
        stream_name: String,
        stream: StreamInfo,
    },
    Ready {
        session_id: String,
        stream_name: String,
        stream: StreamInfo,
        track_index: usize,
        rtp_channel: u8,
    },
    Playing {
        session_id: String,
        stream_name: String,
        stream: StreamInfo,
        track_index: usize,
        rtp_channel: u8,
    },
}

/// The stream, track, and RTP interleaved channel a session is set up to
/// play, once a client's `SETUP` has succeeded -- returned by
/// [`RtspSession::setup_track`].
///
/// This crate performs no socket I/O of its own (see the module doc), so it
/// never uses this itself; it exists for an embedder's own async I/O layer
/// (issue #12 item X3) to learn, after driving a `PLAY` request through
/// [`RtspSession::handle_request`], which named stream to subscribe to via
/// [`StreamProvider::subscribe`], which of that stream's tracks to
/// packetize, and which interleaved channel to write that track's RTP
/// payloads on -- all state this session already computed and tracked
/// internally during `SETUP`, just not previously exposed.
#[derive(Debug, Clone, Copy)]
pub struct SetupTrack<'a> {
    /// The stream name the client named in `DESCRIBE`/`SETUP`.
    pub stream_name: &'a str,
    /// The stream's full track list, as returned by
    /// [`StreamProvider::describe`] -- `tracks[track_index]` is the one this
    /// session set up.
    pub stream: &'a StreamInfo,
    /// Index into `stream.tracks` naming the track this session set up.
    pub track_index: usize,
    /// The interleaved channel this track's RTP payloads belong on (RTCP,
    /// which this crate does not implement, would belong on
    /// `rtp_channel + 1`, matching the `Transport` header's own
    /// `interleaved=` range this session already returned from `SETUP`).
    pub rtp_channel: u8,
}

/// Allocates session ids unique within this process. A plain counter, not
/// a random token: nothing in this crate's narrowed scope (D-7: no auth,
/// no multi-tenant isolation) needs a session id to be unguessable, only
/// unique per connection.
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

fn allocate_session_id() -> String {
    format!("{:08x}", NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed))
}

impl RtspSession {
    /// Starts a new session in its initial state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles one parsed RTSP request against `provider`, returning the
    /// response to send back.
    ///
    /// Never panics on any request content: an unrecognized method, a
    /// missing/malformed header, an unknown stream name, or a `SETUP`
    /// outside this crate's narrowed transport/track support all produce a
    /// documented RTSP error response rather than propagating an error
    /// value or aborting the connection. Whether to close the underlying
    /// connection after a given response (e.g. after `TEARDOWN`) is this
    /// project's own async I/O layer's decision, not this state machine's.
    pub fn handle_request(
        &mut self,
        provider: &dyn StreamProvider,
        request: &Request<Vec<u8>>,
    ) -> Response<Vec<u8>> {
        match request.method() {
            Method::Options => handle_options(request),
            Method::Describe => self.handle_describe(provider, request),
            Method::Setup => self.handle_setup(request),
            Method::Play => self.handle_play(request),
            Method::Teardown => self.handle_teardown(request),
            _ => respond_empty(request, StatusCode::NotImplemented),
        }
    }

    fn handle_describe(
        &mut self,
        provider: &dyn StreamProvider,
        request: &Request<Vec<u8>>,
    ) -> Response<Vec<u8>> {
        let Some(stream_name) = stream_name_from_uri(request) else {
            return respond_empty(request, StatusCode::BadRequest);
        };

        let Some(stream) = provider.describe(&stream_name) else {
            return respond_empty(request, StatusCode::NotFound);
        };

        let body = build_sdp(&stream);
        self.state = State::Described {
            stream_name,
            stream,
        };

        let content_base = request
            .request_uri()
            .map(std::string::ToString::to_string)
            .unwrap_or_default();

        response_builder(request, StatusCode::Ok)
            .header(CONTENT_TYPE, "application/sdp")
            .header(CONTENT_BASE, content_base)
            .build(body)
    }

    fn handle_setup(&mut self, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
        let State::Described {
            stream_name,
            stream,
        } = &self.state
        else {
            return respond_empty(request, StatusCode::MethodNotValidInThisState);
        };

        let Some(track_index) = parse_track_index(request) else {
            return respond_empty(request, StatusCode::BadRequest);
        };
        if track_index >= stream.tracks.len() {
            return respond_empty(request, StatusCode::BadRequest);
        }

        if !requests_tcp_interleaved_transport(request) {
            return respond_empty(request, StatusCode::UnsupportedTransport);
        }

        let session_id = allocate_session_id();
        let interleaved_low = u8::try_from(track_index * 2).unwrap_or(u8::MAX);
        let interleaved_high = interleaved_low.saturating_add(1);
        let transport =
            format!("RTP/AVP/TCP;unicast;interleaved={interleaved_low}-{interleaved_high}");

        self.state = State::Ready {
            session_id: session_id.clone(),
            stream_name: stream_name.clone(),
            stream: stream.clone(),
            track_index,
            rtp_channel: interleaved_low,
        };

        response_builder(request, StatusCode::Ok)
            .header(SESSION, session_id)
            .header(headers::TRANSPORT, transport)
            .build(Vec::new())
    }

    fn handle_play(&mut self, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
        let (session_id, stream_name, stream, track_index, rtp_channel) = match &self.state {
            State::Ready {
                session_id,
                stream_name,
                stream,
                track_index,
                rtp_channel,
            }
            | State::Playing {
                session_id,
                stream_name,
                stream,
                track_index,
                rtp_channel,
            } => (
                session_id.clone(),
                stream_name.clone(),
                stream.clone(),
                *track_index,
                *rtp_channel,
            ),
            State::Init | State::Described { .. } => {
                return respond_empty(request, StatusCode::MethodNotValidInThisState);
            }
        };

        self.state = State::Playing {
            session_id: session_id.clone(),
            stream_name,
            stream,
            track_index,
            rtp_channel,
        };

        response_builder(request, StatusCode::Ok)
            .header(SESSION, session_id)
            .build(Vec::new())
    }

    fn handle_teardown(&mut self, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
        self.state = State::Init;
        respond_empty(request, StatusCode::Ok)
    }

    /// Returns this session's set-up stream/track/channel, once `SETUP` has
    /// succeeded (`Ready` or `Playing` state); `None` in `Init` or
    /// `Described` state, before a client has completed `SETUP`. See
    /// [`SetupTrack`].
    #[must_use]
    pub fn setup_track(&self) -> Option<SetupTrack<'_>> {
        match &self.state {
            State::Ready {
                stream_name,
                stream,
                track_index,
                rtp_channel,
                ..
            }
            | State::Playing {
                stream_name,
                stream,
                track_index,
                rtp_channel,
                ..
            } => Some(SetupTrack {
                stream_name,
                stream,
                track_index: *track_index,
                rtp_channel: *rtp_channel,
            }),
            State::Init | State::Described { .. } => None,
        }
    }
}

fn handle_options(request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let mut public = headers::Public::builder();
    for method in IMPLEMENTED_METHODS {
        public = public.method(method);
    }
    let public = public.build();

    response_builder(request, StatusCode::Ok)
        .typed_header(&public)
        .build(Vec::new())
}

fn respond_empty(request: &Request<Vec<u8>>, status: StatusCode) -> Response<Vec<u8>> {
    response_builder(request, status).build(Vec::new())
}

/// Starts a response builder at `status`, echoing `request`'s own RTSP
/// version and `CSeq` header (RFC 7826 §18.20: every response must echo
/// the request's `CSeq`).
fn response_builder(request: &Request<Vec<u8>>, status: StatusCode) -> ResponseBuilder {
    let mut builder = Response::builder(request.version(), status);
    if let Some(cseq) = request.header(&CSEQ) {
        builder = builder.header(CSEQ, cseq.as_str());
    }
    builder
}

/// Resolves the stream name a `DESCRIBE` request names: the last non-empty
/// path segment of its request URI.
fn stream_name_from_uri(request: &Request<Vec<u8>>) -> Option<String> {
    let uri = request.request_uri()?;
    uri.path_segments()?
        .rfind(|segment| !segment.is_empty())
        .map(std::string::ToString::to_string)
}

/// Parses the `trackID=<index>` this crate's own `DESCRIBE`/`sdp::build_sdp`
/// assigned back out of a `SETUP` request's URI, checking the query string
/// first and falling back to the path -- matching go2rtc's own
/// `reqTrackID` (`pkg/rtsp/server.go`), which the same clients this crate
/// targets are already tested against.
fn parse_track_index(request: &Request<Vec<u8>>) -> Option<usize> {
    let uri = request.request_uri()?;
    let source = uri.query().map_or_else(|| uri.path(), |query| query);
    let (_, digits) = source.rsplit_once('=')?;
    digits.parse().ok()
}

/// True if `request` carries a `Transport` header naming `RTP/AVP/TCP` --
/// the only transport this crate accepts (D-7), matching go2rtc's own
/// `pkg/rtsp/server.go` `MethodSetup` gate byte-for-byte (independently
/// fact-reviewed, `.agents/issue-12/DESIGN-review-report-D6-D7.md`): a bare
/// prefix check against the profile/lower-transport pair, not against any
/// transport parameter.
fn requests_tcp_interleaved_transport(request: &Request<Vec<u8>>) -> bool {
    let Ok(Some(transports)) = request.typed_header::<headers::Transports>() else {
        return false;
    };
    transports.iter().any(|transport| {
        matches!(
            transport,
            Transport::Rtp(rtp)
                if rtp.profile == RtpProfile::Avp
                    && rtp.lower_transport == Some(RtpLowerTransport::Tcp)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Frame, FrameReceiver, TrackInfo};
    use rtsp_types::{Url, Version};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// An in-memory `StreamProvider` test double: a fixed table of named
    /// streams, no real frame delivery (`subscribe` returns an
    /// already-exhausted receiver -- X1 tests the protocol/session layer
    /// only, never frame delivery, which is X2/X3's scope).
    struct FakeProvider {
        streams: HashMap<String, StreamInfo>,
        subscribe_calls: Mutex<Vec<String>>,
    }

    struct ExhaustedReceiver;

    impl FrameReceiver for ExhaustedReceiver {
        fn recv(&mut self) -> Option<Frame> {
            None
        }
    }

    impl FakeProvider {
        fn with_one_h264_stream(name: &str) -> Self {
            let mut streams = HashMap::new();
            streams.insert(
                name.to_string(),
                StreamInfo {
                    name: name.to_string(),
                    tracks: vec![TrackInfo::H264 {
                        sps: vec![0x67, 0x42, 0x00, 0x1f],
                        pps: vec![0x68, 0xce],
                    }],
                },
            );
            Self {
                streams,
                subscribe_calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl StreamProvider for FakeProvider {
        fn describe(&self, name: &str) -> Option<StreamInfo> {
            self.streams.get(name).cloned()
        }

        fn subscribe(&self, name: &str) -> Option<Box<dyn FrameReceiver>> {
            self.subscribe_calls
                .lock()
                .expect("test-only mutex is never poisoned")
                .push(name.to_string());
            self.streams
                .contains_key(name)
                .then(|| Box::new(ExhaustedReceiver) as Box<dyn FrameReceiver>)
        }
    }

    fn request(method: Method, uri: &str) -> Request<Vec<u8>> {
        Request::builder(method, Version::V1_0)
            .request_uri(Url::parse(uri).expect("valid test URI"))
            .header(CSEQ, "1")
            .build(Vec::new())
    }

    fn request_with_transport(method: Method, uri: &str, transport: &str) -> Request<Vec<u8>> {
        Request::builder(method, Version::V1_0)
            .request_uri(Url::parse(uri).expect("valid test URI"))
            .header(CSEQ, "1")
            .header(headers::TRANSPORT, transport)
            .build(Vec::new())
    }

    #[test]
    fn options_advertises_exactly_the_five_implemented_methods() {
        let provider = FakeProvider::with_one_h264_stream("camera1");
        let mut session = RtspSession::new();
        let response = session.handle_request(
            &provider,
            &request(Method::Options, "rtsp://127.0.0.1:8554/camera1"),
        );

        assert_eq!(response.status(), StatusCode::Ok);
        let public = response
            .typed_header::<headers::Public>()
            .expect("Public header parses")
            .expect("OPTIONS response carries a Public header");
        assert_eq!(
            public.as_ref(),
            &vec![
                Method::Options,
                Method::Describe,
                Method::Setup,
                Method::Play,
                Method::Teardown,
            ]
        );
        assert!(
            !public.contains(&Method::Pause),
            "must not advertise PAUSE, which this crate does not implement"
        );
    }

    #[test]
    fn describe_returns_sdp_for_a_known_stream() {
        let provider = FakeProvider::with_one_h264_stream("camera1");
        let mut session = RtspSession::new();
        let response = session.handle_request(
            &provider,
            &request(Method::Describe, "rtsp://127.0.0.1:8554/camera1"),
        );

        assert_eq!(response.status(), StatusCode::Ok);
        assert_eq!(
            response
                .header(&CONTENT_TYPE)
                .map(rtsp_types::HeaderValue::as_str),
            Some("application/sdp")
        );
        let body = String::from_utf8(response.into_body()).expect("SDP body is ASCII");
        assert!(body.contains("m=video 0 RTP/AVP 96\r\n"), "body:\n{body}");
    }

    #[test]
    fn describe_returns_404_for_an_unknown_stream() {
        let provider = FakeProvider::with_one_h264_stream("camera1");
        let mut session = RtspSession::new();
        let response = session.handle_request(
            &provider,
            &request(Method::Describe, "rtsp://127.0.0.1:8554/no-such-camera"),
        );

        assert_eq!(response.status(), StatusCode::NotFound);
    }

    #[test]
    fn setup_after_describe_accepts_tcp_interleaved_transport() {
        let provider = FakeProvider::with_one_h264_stream("camera1");
        let mut session = RtspSession::new();
        session.handle_request(
            &provider,
            &request(Method::Describe, "rtsp://127.0.0.1:8554/camera1"),
        );

        let response = session.handle_request(
            &provider,
            &request_with_transport(
                Method::Setup,
                "rtsp://127.0.0.1:8554/camera1/trackID=0",
                "RTP/AVP/TCP;unicast;interleaved=0-1",
            ),
        );

        assert_eq!(response.status(), StatusCode::Ok);
        assert!(
            response.header(&SESSION).is_some(),
            "SETUP must allocate a session id"
        );
        assert_eq!(
            response
                .header(&headers::TRANSPORT)
                .map(rtsp_types::HeaderValue::as_str),
            Some("RTP/AVP/TCP;unicast;interleaved=0-1")
        );
    }

    #[test]
    fn setup_rejects_a_udp_transport_request_with_461() {
        let provider = FakeProvider::with_one_h264_stream("camera1");
        let mut session = RtspSession::new();
        session.handle_request(
            &provider,
            &request(Method::Describe, "rtsp://127.0.0.1:8554/camera1"),
        );

        let response = session.handle_request(
            &provider,
            &request_with_transport(
                Method::Setup,
                "rtsp://127.0.0.1:8554/camera1/trackID=0",
                "RTP/AVP/UDP;unicast;client_port=4000-4001",
            ),
        );

        assert_eq!(response.status(), StatusCode::UnsupportedTransport);
        assert_eq!(u16::from(response.status()), 461);
        assert!(
            response.header(&SESSION).is_none(),
            "a rejected SETUP must not allocate a session"
        );
    }

    #[test]
    fn unknown_method_gets_a_clean_error_response_not_a_panic() {
        let provider = FakeProvider::with_one_h264_stream("camera1");
        let mut session = RtspSession::new();

        let response = session.handle_request(
            &provider,
            &request(Method::Pause, "rtsp://127.0.0.1:8554/camera1"),
        );

        assert_eq!(response.status(), StatusCode::NotImplemented);
    }

    #[test]
    fn setup_without_a_prior_describe_is_rejected() {
        let provider = FakeProvider::with_one_h264_stream("camera1");
        let mut session = RtspSession::new();

        let response = session.handle_request(
            &provider,
            &request_with_transport(
                Method::Setup,
                "rtsp://127.0.0.1:8554/camera1/trackID=0",
                "RTP/AVP/TCP;unicast;interleaved=0-1",
            ),
        );

        assert_eq!(response.status(), StatusCode::MethodNotValidInThisState);
    }

    #[test]
    fn setup_rejects_an_out_of_range_track_index() {
        let provider = FakeProvider::with_one_h264_stream("camera1");
        let mut session = RtspSession::new();
        session.handle_request(
            &provider,
            &request(Method::Describe, "rtsp://127.0.0.1:8554/camera1"),
        );

        let response = session.handle_request(
            &provider,
            &request_with_transport(
                Method::Setup,
                "rtsp://127.0.0.1:8554/camera1/trackID=7",
                "RTP/AVP/TCP;unicast;interleaved=0-1",
            ),
        );

        assert_eq!(response.status(), StatusCode::BadRequest);
    }

    #[test]
    fn full_play_teardown_sequence_succeeds_in_order() {
        let provider = FakeProvider::with_one_h264_stream("camera1");
        let mut session = RtspSession::new();
        session.handle_request(
            &provider,
            &request(Method::Describe, "rtsp://127.0.0.1:8554/camera1"),
        );
        session.handle_request(
            &provider,
            &request_with_transport(
                Method::Setup,
                "rtsp://127.0.0.1:8554/camera1/trackID=0",
                "RTP/AVP/TCP;unicast;interleaved=0-1",
            ),
        );

        let play_response = session.handle_request(
            &provider,
            &request(Method::Play, "rtsp://127.0.0.1:8554/camera1"),
        );
        assert_eq!(play_response.status(), StatusCode::Ok);

        let teardown_response = session.handle_request(
            &provider,
            &request(Method::Teardown, "rtsp://127.0.0.1:8554/camera1"),
        );
        assert_eq!(teardown_response.status(), StatusCode::Ok);
    }

    #[test]
    fn play_before_setup_is_rejected() {
        let provider = FakeProvider::with_one_h264_stream("camera1");
        let mut session = RtspSession::new();

        let response = session.handle_request(
            &provider,
            &request(Method::Play, "rtsp://127.0.0.1:8554/camera1"),
        );

        assert_eq!(response.status(), StatusCode::MethodNotValidInThisState);
    }
}
