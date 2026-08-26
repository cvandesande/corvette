//! Role (b) of the plan's Do step 3.
//!
//! Drives the MoQ-publish loop directly via
//! `moq_mux::codec::{h264, h265}::{Split, Import}` / `aac::Import` (D-9),
//! dialing the relay via `moq_net`/`web_transport_quinn` directly (D-5).
//!
//! One camera's publish task owns one dial: on a relay disconnect it
//! reconnects with backoff, matching `corvette-rtsp-client`'s own reconnect
//! convention (`client/task.rs`'s `run_camera`/`Backoff`) shape-for-shape,
//! while reusing the SAME upstream `corvette_rtsp_client::Frame` subscription
//! across relay reconnects (a relay hiccup does not need a new camera
//! subscription -- only a new `MoQ` session).
//!
//! No container format appears anywhere in this module (INV-8): frames flow
//! from `corvette_rtsp_client::depacketize::Frame` (Annex-B / raw AAC)
//! straight into `moq_mux`'s codec-level `Split`/`Import` layer.

use crate::config::MoqConfig;
use crate::rtp_clock::RtpClock;
use crate::supervise::log_event;
use corvette_rtsp_client::depacketize::{Codec as ClientCodec, Frame as ClientFrame};
use moq_mux::codec::{h264, h265};
use std::time::Duration;
use tokio::sync::broadcast;

const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const STABLE_CONNECTION_THRESHOLD: Duration = Duration::from_secs(10);

/// Runs one camera's MoQ-publish role for as long as `client_frames` keeps
/// producing.
///
/// That is forever, in production -- the owning `Client` outlives this task:
/// dial the relay, publish, and on any relay-session failure reconnect with
/// backoff. Intended to be spawned via
/// `crate::supervise::spawn_supervised_with`, matching DP-3/INV-5.
pub async fn run_publish_loop(
    name: String,
    config: MoqConfig,
    mut client_frames: broadcast::Receiver<ClientFrame>,
) {
    let mut backoff = Backoff::new();
    loop {
        let started = std::time::Instant::now();
        if let Err(err) = run_publish_once(&name, &config, &mut client_frames).await {
            log_event(&name, "moq-publish-disconnect", &err);
        } else {
            // The upstream camera subscription itself ended (the owning
            // `Client` was dropped): nothing left to reconnect for.
            return;
        }
        if started.elapsed() >= STABLE_CONNECTION_THRESHOLD {
            backoff.reset();
        }
        let delay = backoff.advance();
        log_event(
            &name,
            "moq-publish-reconnect",
            format_args!("retrying in {delay:?}"),
        );
        tokio::time::sleep(delay).await;
    }
}

/// One relay-connection attempt: dial, publish, and run until the upstream
/// camera subscription closes (`Ok(())`, no reconnect needed) or the relay
/// session itself fails (`Err`, reconnect with backoff).
async fn run_publish_once(
    name: &str,
    config: &MoqConfig,
    client_frames: &mut broadcast::Receiver<ClientFrame>,
) -> Result<(), PublishError> {
    let session = dial_relay(config).await.map_err(PublishError::Dial)?;
    let origin = moq_net::Origin::random().produce();
    let moq_client = moq_net::Client::new()
        .with_versions(moq_net::Versions::all())
        .with_publisher(origin.consume());
    let (_moq_session, driver) = moq_client
        .connect(session)
        .await
        .map_err(PublishError::Handshake)?;
    let mut driver_task = tokio::spawn(driver);

    let mut moq_broadcast = origin
        .create_broadcast(name, moq_net::broadcast::Route::new().with_announce(true))
        .map_err(PublishError::Broadcast)?;
    let catalog =
        moq_mux::catalog::Producer::new(&mut moq_broadcast).map_err(PublishError::Broadcast)?;

    log_event(
        name,
        "moq-publish-connect",
        format_args!("relay={}", config.relay_url),
    );

    let mut track: Option<PublishTrack> = None;
    let mut clock = RtpClock::video();

    let result = loop {
        tokio::select! {
            biased;
            frame = client_frames.recv() => {
                match frame {
                    Ok(frame) => {
                        if let Err(err) = feed_frame(&mut moq_broadcast, &catalog, &mut track, &mut clock, &frame) {
                            log_event(name, "moq-publish-frame-error", &err);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break Ok(()),
                }
            }
            joined = &mut driver_task => {
                let cause = match joined {
                    Ok(Ok(())) => "relay session driver ended".to_string(),
                    Ok(Err(err)) => format!("relay session driver failed: {err}"),
                    Err(join_error) => format!("relay session driver panicked: {join_error}"),
                };
                break Err(PublishError::RelayClosed(cause));
            }
        }
    };

    if let Some(track) = track {
        track.finish();
    }
    if !driver_task.is_finished() {
        driver_task.abort();
    }
    result
}

/// Dials `config.relay_url` over WebTransport/QUIC, offering every `MoQ`
/// version this workspace's pinned `moq-net` supports (D-5: directly against
/// `moq_net`/`web_transport_quinn`, not through `moq-native` or a `moq-cli`
/// subprocess).
async fn dial_relay(config: &MoqConfig) -> Result<web_transport_quinn::Session, DialError> {
    let builder = web_transport_quinn::ClientBuilder::new();
    let client = if config.tls_disable_verify {
        builder
            .dangerous()
            .with_no_certificate_verification()
            .map_err(DialError::Tls)?
    } else {
        builder.with_system_roots().map_err(DialError::Tls)?
    };

    let mut request = web_transport_quinn::proto::ConnectRequest::new(config.relay_url.clone());
    for alpn in moq_net::Versions::all().alpns() {
        request = request.with_protocol(alpn.to_string());
    }

    client.connect(request).await.map_err(DialError::Connect)
}

/// Either video importer this module drives, selected from the first
/// frame's own [`ClientCodec`] -- the mirror image of
/// `corvette-rtsp-client`'s own client-task `Depacketizer` enum and
/// `rtsp-restream`'s own server-side `Packetizer` enum.
enum PublishTrack {
    H264 {
        split: h264::Split,
        import: h264::Import,
    },
    H265 {
        split: h265::Split,
        import: h265::Import,
    },
}

impl PublishTrack {
    fn finish(self) {
        match self {
            Self::H264 { mut import, .. } => {
                let _ = import.finish();
            }
            Self::H265 { mut import, .. } => {
                let _ = import.finish();
            }
        }
    }
}

/// Feeds one upstream frame into this camera's publish state, lazily
/// initializing `track` from the frame's own codec on first use.
fn feed_frame(
    moq_broadcast: &mut moq_net::broadcast::Producer,
    catalog: &moq_mux::catalog::Producer,
    track: &mut Option<PublishTrack>,
    clock: &mut RtpClock,
    frame: &ClientFrame,
) -> Result<(), FrameError> {
    if track.is_none() {
        *track = Some(new_track(moq_broadcast, catalog, frame.codec)?);
    }

    let pts = moq_net::Timestamp::try_from(clock.elapsed(frame.timestamp))
        .map_err(FrameError::Timestamp)?;

    match (track.as_mut().expect("just initialized above"), frame.codec) {
        (PublishTrack::H264 { split, import }, ClientCodec::H264) => {
            let units = split
                .decode(&frame.payload, pts)
                .map_err(FrameError::Split)?;
            import.decode(units).map_err(FrameError::Import)
        }
        (PublishTrack::H265 { split, import }, ClientCodec::H265) => {
            let units = split
                .decode(&frame.payload, pts)
                .map_err(FrameError::Split)?;
            import.decode(units).map_err(FrameError::Import)
        }
        (_, other) => Err(FrameError::CodecChanged(other)),
    }
}

fn new_track(
    moq_broadcast: &mut moq_net::broadcast::Producer,
    catalog: &moq_mux::catalog::Producer,
    codec: ClientCodec,
) -> Result<PublishTrack, FrameError> {
    match codec {
        ClientCodec::H264 => {
            let producer = moq_broadcast
                .create_track("video", hang::container::track_info())
                .map_err(FrameError::CreateTrack)?;
            let import = h264::Import::new(
                producer,
                catalog.reserve(),
                moq_mux::catalog::VideoHint::default(),
            )
            .map_err(FrameError::Import)?;
            Ok(PublishTrack::H264 {
                split: h264::Split::new(),
                import,
            })
        }
        ClientCodec::H265 => {
            let producer = moq_broadcast
                .create_track("video", hang::container::track_info())
                .map_err(FrameError::CreateTrack)?;
            let import = h265::Import::new(
                producer,
                catalog.reserve(),
                moq_mux::catalog::VideoHint::default(),
            )
            .map_err(FrameError::Import)?;
            Ok(PublishTrack::H265 {
                split: h265::Split::new(),
                import,
            })
        }
        // A real, currently-shipped gap this item inherits rather than fixes
        // (see this crate's top-level doc and G1's own Premise):
        // `corvette-rtsp-client`'s SDP resolution never resolves an audio
        // track, so no `Codec::Aac` frame is ever actually produced today.
        // `aac::Import::new` needs a resolved `AudioConfig` (sample rate,
        // channel count, AudioSpecificConfig bytes) this module has no
        // source for without that upstream fix, so this arm can only ever
        // report the gap rather than construct a nonsensical track.
        ClientCodec::Aac => Err(FrameError::AacUnavailable),
    }
}

#[derive(Debug)]
enum DialError {
    Tls(web_transport_quinn::ClientError),
    Connect(web_transport_quinn::ClientError),
}

impl std::fmt::Display for DialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tls(err) => write!(f, "building the WebTransport TLS client: {err}"),
            Self::Connect(err) => write!(f, "dialing the relay: {err}"),
        }
    }
}

#[derive(Debug)]
enum PublishError {
    Dial(DialError),
    Handshake(moq_net::Error),
    Broadcast(moq_net::Error),
    RelayClosed(String),
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dial(err) => write!(f, "{err}"),
            Self::Handshake(err) => write!(f, "MoQ handshake with the relay failed: {err}"),
            Self::Broadcast(err) => write!(f, "publishing this camera's broadcast failed: {err}"),
            Self::RelayClosed(cause) => write!(f, "{cause}"),
        }
    }
}

#[derive(Debug)]
enum FrameError {
    CreateTrack(moq_net::Error),
    Split(moq_mux::Error),
    Import(moq_mux::Error),
    Timestamp(moq_net::TimeOverflow),
    /// The upstream frame's codec differs from the one this camera's track
    /// was already initialized with -- never expected in practice (one
    /// camera's SDP resolves exactly one video codec for its lifetime), but
    /// handled defensively rather than panicking on a malformed upstream.
    CodecChanged(ClientCodec),
    AacUnavailable,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateTrack(err) => write!(f, "creating this camera's MoQ track: {err}"),
            Self::Split(err) => write!(f, "splitting an access unit: {err}"),
            Self::Import(err) => write!(f, "importing an access unit: {err}"),
            Self::Timestamp(err) => write!(f, "converting a frame timestamp: {err}"),
            Self::CodecChanged(codec) => write!(
                f,
                "camera's codec changed mid-stream to {codec:?}, dropping frame"
            ),
            Self::AacUnavailable => write!(
                f,
                "an AAC frame arrived, but corvette-rtsp-client never resolves audio-track SDP today, so no \
                 codec parameters are available to initialize an AAC track; dropping"
            ),
        }
    }
}

/// Exponential backoff between relay reconnect attempts, matching
/// `corvette-rtsp-client`'s own `client/task.rs::Backoff` shape-for-shape
/// (a private detail of that crate, not shared code -- see this crate's
/// `supervise` module doc for the same reasoning).
struct Backoff {
    next: Duration,
}

impl Backoff {
    const fn new() -> Self {
        Self {
            next: INITIAL_BACKOFF,
        }
    }

    fn advance(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(MAX_BACKOFF);
        delay
    }

    const fn reset(&mut self) {
        self.next = INITIAL_BACKOFF;
    }
}
