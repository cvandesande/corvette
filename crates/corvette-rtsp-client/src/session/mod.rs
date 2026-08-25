//! RTSP session/protocol layer.
//!
//! Connects to one camera, performs the Digest-authenticated
//! `DESCRIBE`/`SETUP`/`PLAY` handshake, schedules keep-alives at a real
//! margin against the camera's own declared timeout, and demuxes the
//! resulting interleaved RTP/RTCP frames from any RTSP control traffic
//! sharing the same socket.
//!
//! This module hands callers raw, per-track RTP payloads
//! ([`RtpPacket`]) -- it does not depacketize them into codec access units.
//! That is a later item's job; see `docs/design/api-contracts.md` for the
//! overall RTSP camera contract this module implements.

mod digest;
mod keepalive;
mod message_stream;
mod sdp;

pub use sdp::{SdpError, TrackDescription};

use digest::DigestAuth;
use keepalive::{KeepAliveMethod, KeepAliveScheduler};
use message_stream::{MessageStream, StreamError};
use rtsp_types::{Message, Method, Request, StatusCode, Url, Version, headers};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::time::{Interval, MissedTickBehavior};

/// A camera's Digest credentials.
///
/// `Debug` deliberately redacts `password` -- callers may log a `Credentials`
/// value (e.g. alongside connection config) without printing a camera's
/// plaintext password.
#[derive(Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .finish()
    }
}

impl Credentials {
    #[must_use]
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }
}

/// One demuxed RTP or RTCP packet read off the interleaved binary channel a
/// camera's `SETUP` response granted.
///
/// `channel` is the interleaved channel byte from the `$`-prefixed frame
/// (RTP and RTCP for one track occupy adjacent channels, e.g. `0`/`1`);
/// `payload` is the frame's raw bytes, undepacketized.
#[derive(Debug, Clone)]
pub struct RtpPacket {
    pub channel: u8,
    pub payload: bytes::Bytes,
}

/// Errors from establishing or running an RTSP session.
#[derive(Debug)]
pub enum SessionError {
    Io(std::io::Error),
    /// The connection closed or a message failed to parse; see the message
    /// for detail (kept as text rather than an internal type this crate
    /// does not want to commit to as public API).
    Stream(String),
    Write(rtsp_types::WriteError),
    InvalidUrl,
    Sdp(SdpError),
    /// The camera's `WWW-Authenticate` challenge could not be used; see the
    /// message for detail.
    Challenge(String),
    HeaderParse(headers::HeaderParseError),
    /// The camera returned `401 Unauthorized` without a `WWW-Authenticate`
    /// header to challenge against.
    MissingAuthChallenge,
    /// The camera rejected the Digest response computed from its own
    /// challenge.
    AuthenticationFailed,
    /// `SETUP` succeeded but the response carried no `Session` header.
    MissingSessionHeader,
    /// A request other than the expected next one arrived (a camera should
    /// never send an RTSP *request* to this client).
    UnexpectedRequestFromServer,
    UnexpectedStatus {
        method: &'static str,
        status: StatusCode,
    },
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "RTSP connection I/O error: {err}"),
            Self::Stream(message) | Self::Challenge(message) => write!(f, "{message}"),
            Self::Write(err) => write!(f, "failed to serialize an RTSP request: {err}"),
            Self::InvalidUrl => write!(f, "could not build a valid RTSP URL for this camera"),
            Self::Sdp(err) => write!(f, "{err}"),
            Self::HeaderParse(_) => write!(f, "malformed RTSP header in camera response"),
            Self::MissingAuthChallenge => {
                write!(f, "camera returned 401 with no WWW-Authenticate header")
            }
            Self::AuthenticationFailed => {
                write!(
                    f,
                    "camera rejected Digest credentials computed from its own challenge"
                )
            }
            Self::MissingSessionHeader => {
                write!(f, "SETUP response carried no Session header")
            }
            Self::UnexpectedRequestFromServer => {
                write!(
                    f,
                    "camera sent an RTSP request; only responses and data frames are expected"
                )
            }
            Self::UnexpectedStatus { method, status } => {
                write!(f, "{method} failed: {status} ({})", u16::from(*status))
            }
        }
    }
}

impl std::error::Error for SessionError {}

impl From<std::io::Error> for SessionError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<StreamError> for SessionError {
    fn from(err: StreamError) -> Self {
        Self::Stream(err.to_string())
    }
}

/// Establishes an authenticated RTSP session against a camera at `addr`,
/// requesting the stream at `path` (e.g. `"/stream/"`).
///
/// Performs the full handshake -- Digest challenge/response, `DESCRIBE` and
/// video-track resolution, TCP-interleaved `SETUP`, and `PLAY` -- and
/// returns the resolved [`TrackDescription`] alongside a [`PlayingSession`]
/// ready to yield demuxed RTP packets.
///
/// # Errors
///
/// Returns an error if the TCP connection fails, any handshake step
/// receives a non-2xx response, Digest authentication is rejected, or the
/// `DESCRIBE` response's SDP body has no resolvable video track.
pub async fn connect(
    addr: SocketAddr,
    path: &str,
    credentials: Credentials,
) -> Result<(TrackDescription, PlayingSession), SessionError> {
    let stream_url =
        Url::parse(&format!("rtsp://{addr}{path}")).map_err(|_| SessionError::InvalidUrl)?;

    let tcp = TcpStream::connect(addr).await?;
    let (read_half, write_half) = tcp.into_split();
    let mut conn = Connection {
        reader: MessageStream::new(read_half),
        writer: write_half,
        cseq: 0,
        digest: None,
        credentials,
        pending_data: VecDeque::new(),
    };

    let describe = conn
        .request_authenticated(Method::Describe, &stream_url, None, &[])
        .await?;
    expect_ok("DESCRIBE", &describe)?;

    let content_base = describe
        .header(&headers::CONTENT_BASE)
        .and_then(|value| Url::parse(value.as_str()).ok())
        .unwrap_or_else(|| stream_url.clone());
    let track =
        sdp::resolve_video_track(describe.body(), &content_base).map_err(SessionError::Sdp)?;

    let transport_header = (
        headers::TRANSPORT,
        "RTP/AVP/TCP;unicast;interleaved=0-1".to_string(),
    );
    let setup = conn
        .request_authenticated(
            Method::Setup,
            &track.control_url,
            None,
            std::slice::from_ref(&transport_header),
        )
        .await?;
    expect_ok("SETUP", &setup)?;

    let session_header = setup
        .typed_header::<headers::Session>()
        .map_err(SessionError::HeaderParse)?
        .ok_or(SessionError::MissingSessionHeader)?;
    // A `timeout=0` is not distinguishable from "no usable timeout was
    // declared" -- RTSP's `Session` header carries whole seconds only, and
    // no real camera declares an instantly-expiring session -- so it is
    // treated the same as an omitted `timeout=` (see `keepalive` module
    // docs for why 60 seconds is the applied default).
    let declared_timeout = session_header
        .1
        .filter(|&timeout| timeout > 0)
        .map_or(keepalive::DEFAULT_DECLARED_TIMEOUT, Duration::from_secs);
    let session_id = session_header.0.clone();

    let play = conn
        .request_authenticated(Method::Play, &stream_url, Some(&session_id), &[])
        .await?;
    expect_ok("PLAY", &play)?;

    let scheduler = KeepAliveScheduler::new(declared_timeout);
    let mut ticker = tokio::time::interval(scheduler.interval());
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let session = PlayingSession {
        conn,
        stream_url,
        session_id,
        declared_timeout,
        scheduler,
        last_keep_alive_method: KeepAliveMethod::GetParameter,
        ticker,
    };
    Ok((track, session))
}

fn expect_ok(
    method: &'static str,
    response: &rtsp_types::Response<Vec<u8>>,
) -> Result<(), SessionError> {
    if response.status() == StatusCode::Ok {
        Ok(())
    } else {
        Err(SessionError::UnexpectedStatus {
            method,
            status: response.status(),
        })
    }
}

/// The raw request/response half of an RTSP connection: request
/// construction, Digest authentication, and reading back the matching
/// response. Shared by the handshake in [`connect`] and by
/// [`PlayingSession`]'s keep-alive sends.
struct Connection {
    reader: MessageStream<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    cseq: u32,
    digest: Option<DigestAuth>,
    credentials: Credentials,
    /// RTP/RTCP data frames read while awaiting a response in
    /// [`Connection::send_and_receive`]. Some cameras start streaming on the
    /// interleaved channel immediately around a handshake request (observed
    /// directly around `PLAY`, and not provably bounded to that one request),
    /// racing that request's own text response on the same socket. Such a
    /// frame is real camera data, not noise, so it is buffered here rather
    /// than discarded; [`PlayingSession::next_packet`] drains it before
    /// reading fresh messages off the socket.
    pending_data: VecDeque<RtpPacket>,
}

impl Connection {
    /// Sends `method` to `uri` and returns the response, transparently
    /// handling a `401` Digest challenge on the first request of the
    /// connection (or a re-challenge, if the camera ever issues a fresh
    /// nonce mid-session).
    async fn request_authenticated(
        &mut self,
        method: Method,
        uri: &Url,
        session_id: Option<&str>,
        extra_headers: &[(rtsp_types::HeaderName, String)],
    ) -> Result<rtsp_types::Response<Vec<u8>>, SessionError> {
        let response = self
            .send_and_receive(method.clone(), uri, session_id, extra_headers)
            .await?;
        if response.status() != StatusCode::Unauthorized {
            return Ok(response);
        }

        let challenge_header = response
            .header(&headers::WWW_AUTHENTICATE)
            .ok_or(SessionError::MissingAuthChallenge)?;
        let challenge = digest::parse_challenge(challenge_header.as_str())
            .map_err(|err| SessionError::Challenge(err.to_string()))?;
        match &mut self.digest {
            Some(existing) => existing.set_challenge(challenge),
            None => {
                self.digest = Some(DigestAuth::new(
                    self.credentials.username.clone(),
                    self.credentials.password.clone(),
                    challenge,
                ));
            }
        }

        let retried = self
            .send_and_receive(method, uri, session_id, extra_headers)
            .await?;
        if retried.status() == StatusCode::Unauthorized {
            return Err(SessionError::AuthenticationFailed);
        }
        Ok(retried)
    }

    /// Sends `method` and reads messages off the connection until the
    /// matching `Response` arrives, buffering any RTP/RTCP data frame that
    /// arrives first rather than treating it as an error -- a camera is free
    /// to start streaming on the interleaved channel before its own response
    /// to this request lands on the same socket.
    async fn send_and_receive(
        &mut self,
        method: Method,
        uri: &Url,
        session_id: Option<&str>,
        extra_headers: &[(rtsp_types::HeaderName, String)],
    ) -> Result<rtsp_types::Response<Vec<u8>>, SessionError> {
        self.send(method, uri, session_id, extra_headers).await?;
        loop {
            match self.reader.read_message().await? {
                Message::Response(response) => return Ok(response),
                Message::Data(data) => self.pending_data.push_back(RtpPacket {
                    channel: data.channel_id(),
                    payload: bytes::Bytes::from(data.into_body()),
                }),
                Message::Request(_) => return Err(SessionError::UnexpectedRequestFromServer),
            }
        }
    }

    async fn send(
        &mut self,
        method: Method,
        uri: &Url,
        session_id: Option<&str>,
        extra_headers: &[(rtsp_types::HeaderName, String)],
    ) -> Result<(), SessionError> {
        self.cseq += 1;
        let method_str: &str = (&method).into();
        let auth_header = self
            .digest
            .as_ref()
            .map(|digest| digest.authorization_header(method_str, uri.as_str()));

        let mut builder = Request::builder(method, Version::V1_0)
            .request_uri(uri.clone())
            .header(headers::CSEQ, self.cseq.to_string());
        if let Some(session_id) = session_id {
            builder = builder.header(headers::SESSION, session_id.to_string());
        }
        if let Some(auth_header) = auth_header {
            builder = builder.header(headers::AUTHORIZATION, auth_header);
        }
        for (name, value) in extra_headers {
            builder = builder.header(name.clone(), value.clone());
        }

        let request = builder.empty();
        let mut bytes = Vec::new();
        request.write(&mut bytes).map_err(SessionError::Write)?;
        tokio::io::AsyncWriteExt::write_all(&mut self.writer, &bytes).await?;
        Ok(())
    }
}

/// An RTSP session past `PLAY`, yielding demuxed RTP packets while
/// transparently sending keep-alives at a real margin against the camera's
/// declared timeout (INV-3).
#[derive(Debug)]
pub struct PlayingSession {
    conn: Connection,
    stream_url: Url,
    session_id: String,
    declared_timeout: Duration,
    scheduler: KeepAliveScheduler,
    /// The method most recently sent as a keep-alive. There is only ever
    /// one keep-alive in flight at a time (the schedule interval is far
    /// larger than one round trip), so the next `Response` read off the
    /// connection is assumed to answer it.
    last_keep_alive_method: KeepAliveMethod,
    ticker: Interval,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("cseq", &self.cseq)
            .field("authenticated", &self.digest.is_some())
            .finish_non_exhaustive()
    }
}

impl PlayingSession {
    /// The camera's own declared `SETUP` timeout (INV-3's basis value).
    #[must_use]
    pub const fn declared_timeout(&self) -> Duration {
        self.declared_timeout
    }

    /// The keep-alive send interval computed from [`Self::declared_timeout`]
    /// (INV-3): strictly less than it, never closer than half.
    #[must_use]
    pub const fn keep_alive_interval(&self) -> Duration {
        self.scheduler.interval()
    }

    /// Reads the next demuxed RTP/RTCP packet, sending keep-alives on
    /// schedule in the background of the same read loop.
    ///
    /// Drains any frame the handshake buffered while racing a request's
    /// response first, in the order it arrived, before reading fresh
    /// messages off the socket -- such a frame must not be lost.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection closes, a message fails to parse,
    /// or the camera sends something other than a response or a data frame.
    pub async fn next_packet(&mut self) -> Result<RtpPacket, SessionError> {
        if let Some(packet) = self.conn.pending_data.pop_front() {
            return Ok(packet);
        }
        loop {
            tokio::select! {
                _ = self.ticker.tick() => {
                    self.send_keep_alive().await?;
                }
                message = self.conn.reader.read_message() => {
                    match message? {
                        Message::Data(data) => {
                            return Ok(RtpPacket {
                                channel: data.channel_id(),
                                payload: bytes::Bytes::from(data.into_body()),
                            });
                        }
                        Message::Response(response) => {
                            self.scheduler
                                .observe_response(self.last_keep_alive_method, response.status());
                        }
                        Message::Request(_) => return Err(SessionError::UnexpectedRequestFromServer),
                    }
                }
            }
        }
    }

    async fn send_keep_alive(&mut self) -> Result<(), SessionError> {
        let method = self.scheduler.method();
        self.last_keep_alive_method = method;
        self.conn
            .send(
                method.as_rtsp_method(),
                &self.stream_url,
                Some(&self.session_id),
                &[],
            )
            .await
    }

    /// Sends `TEARDOWN` and consumes the session.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails to send or the camera responds
    /// with anything other than `200 OK`.
    pub async fn teardown(mut self) -> Result<(), SessionError> {
        let response = self
            .conn
            .request_authenticated(
                Method::Teardown,
                &self.stream_url,
                Some(&self.session_id),
                &[],
            )
            .await?;
        expect_ok("TEARDOWN", &response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_debug_never_prints_the_plaintext_password() {
        let credentials = Credentials::new("admin", "test-password-not-real");
        let debug = format!("{credentials:?}");
        assert!(debug.contains("admin"));
        assert!(
            !debug.contains("test-password-not-real"),
            "Debug output must redact the password: {debug}"
        );
    }
}
