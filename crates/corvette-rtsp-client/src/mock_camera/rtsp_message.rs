//! Minimal, hand-rolled RTSP/1.0 request parsing and response construction.
//!
//! This mock hand-rolls message framing rather than depending on
//! `rtsp-types`/`sdp-types`: it needs to emit deliberately specific,
//! sometimes non-canonical byte structures (the SDP control-URL ordering
//! quirk below is exactly such a case) that a general-purpose message
//! builder is not designed to produce on request.

use super::digest::REALM;
use std::fmt::Write as _;
use std::io;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt};

/// A parsed RTSP request.
#[derive(Debug)]
pub(super) struct Request {
    pub(super) method: String,
    pub(super) uri: String,
    headers: Vec<(String, String)>,
}

impl Request {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn cseq(&self) -> &str {
        self.header("CSeq").unwrap_or("0")
    }
}

/// Reads one RTSP request from `reader`, or `Ok(None)` on a clean EOF before
/// any request line arrives.
pub(super) async fn read_request<R>(reader: &mut R) -> io::Result<Option<Request>>
where
    R: AsyncBufRead + Unpin,
{
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        return Ok(None);
    }
    let mut parts = request_line.trim_end().splitn(3, ' ');
    let method = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty RTSP request line"))?
        .to_string();
    let uri = parts.next().unwrap_or_default().to_string();

    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_string(), value.trim().to_string()));
        }
    }

    let content_length: usize = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("Content-Length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    if content_length > 0 {
        let mut body = vec![0_u8; content_length];
        reader.read_exact(&mut body).await?;
    }

    Ok(Some(Request {
        method,
        uri,
        headers,
    }))
}

/// An RTSP response, built and serialized in one shot per reply.
pub(super) struct Response {
    status: u16,
    reason: &'static str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn new(status: u16, reason: &'static str, request: &Request) -> Self {
        Self {
            status,
            reason,
            headers: vec![("CSeq".to_string(), request.cseq().to_string())],
            body: Vec::new(),
        }
    }

    fn with_header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    fn with_body(mut self, content_type: &str, body: Vec<u8>) -> Self {
        self.headers
            .push(("Content-Type".to_string(), content_type.to_string()));
        self.body = body;
        self
    }

    pub(super) fn to_bytes(&self) -> Vec<u8> {
        let mut text = format!("RTSP/1.0 {} {}\r\n", self.status, self.reason);
        for (key, value) in &self.headers {
            let _: std::fmt::Result = write!(text, "{key}: {value}\r\n");
        }
        let _: std::fmt::Result = write!(text, "Content-Length: {}\r\n\r\n", self.body.len());
        let mut bytes = text.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

pub(super) fn unauthorized(request: &Request, nonce: &str) -> Response {
    Response::new(401, "Unauthorized", request).with_header(
        "WWW-Authenticate",
        format!(r#"Digest realm="{REALM}", nonce="{nonce}""#),
    )
}

pub(super) fn ok(request: &Request, session_id: Option<&str>) -> Response {
    let response = Response::new(200, "OK", request);
    match session_id {
        Some(id) => response.with_header("Session", id.to_string()),
        None => response,
    }
}

pub(super) fn bad_request(request: &Request) -> Response {
    Response::new(400, "Bad Request", request)
}

pub(super) fn not_implemented(request: &Request) -> Response {
    Response::new(501, "Not Implemented", request)
}

/// Builds the `DESCRIBE` SDP body reproducing the session-vs-track
/// `a=control` ordering quirk: a session-level `a=control:*` line appears
/// BEFORE the `m=video` section, and a distinct, track-specific
/// `a=control:track1` line appears INSIDE it. A parser that resolves the
/// first `a=control` match in document order picks the wrong URL and gets a
/// `404` on `SETUP` against a real camera exhibiting this exact structure.
pub(super) fn describe(request: &Request, content_base: &str) -> Response {
    let sdp = "v=0\r\n\
               o=- 0 0 IN IP4 127.0.0.1\r\n\
               s=corvette-rtsp-client mock camera\r\n\
               c=IN IP4 0.0.0.0\r\n\
               t=0 0\r\n\
               a=control:*\r\n\
               m=video 0 RTP/AVP 96\r\n\
               a=rtpmap:96 H264/90000\r\n\
               a=control:track1\r\n"
        .to_string();
    Response::new(200, "OK", request)
        .with_header("Content-Base", content_base.to_string())
        .with_body("application/sdp", sdp.into_bytes())
}

pub(super) fn setup(request: &Request, session_id: &str, timeout_secs: u64) -> Response {
    Response::new(200, "OK", request)
        .with_header("Session", format!("{session_id};timeout={timeout_secs}"))
        .with_header("Transport", "RTP/AVP/TCP;unicast;interleaved=0-1")
}

pub(super) fn play_ok(request: &Request, session_id: &str) -> Response {
    Response::new(200, "OK", request)
        .with_header("Session", session_id.to_string())
        .with_header("Range", "npt=0.000-")
}
