//! One camera's supervised background task: run the session handshake,
//! depacketize demuxed RTP payloads, publish the resulting frames, and
//! reconnect with backoff on any session-ending condition. A supervising
//! task restarts this work if it ever panics, so one camera's failure never
//! stops another camera's task or the process.

use crate::client::CameraConfig;
use crate::depacketize::{
    DepacketizeError, Frame, H264Depacketizer, H265Depacketizer, SpropParameterSets,
};
use crate::session::{self, SessionError, TrackDescription};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::task::{AbortHandle, JoinHandle};

/// The interleaved channel a `SETUP` request always negotiates for a
/// track's RTP data (see `crate::session`); channel 1 carries that same
/// track's RTCP, which this crate does not depacketize.
const VIDEO_RTP_CHANNEL: u8 = 0;

const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// A session that stayed up at least this long is treated as having
/// recovered, so a later failure starts backing off from the beginning
/// again rather than staying at whatever multiple a much older, unrelated
/// failure streak had reached.
const STABLE_CONNECTION_THRESHOLD: Duration = Duration::from_secs(10);

/// Spawns the supervising task for one camera and returns its handle
/// alongside the currently-running worker's abort handle (kept up to date
/// by the supervisor itself, so a caller can cancel the worker in progress
/// without waiting for the supervisor to notice).
pub(super) fn spawn_supervised(
    config: CameraConfig,
    sender: broadcast::Sender<Frame>,
) -> (JoinHandle<()>, Arc<Mutex<Option<AbortHandle>>>) {
    let name = config.name.clone();
    spawn_supervised_with(name, move || run_camera(config.clone(), sender.clone()))
}

/// The restart-on-panic loop [`spawn_supervised`] runs for a real camera
/// connection, generalized over what "one camera's work" actually is so a
/// test can substitute a deliberately panicking stand-in without depending
/// on a live RTSP session -- the isolation and restart behavior this
/// function provides has nothing to do with what the supervised work does.
///
/// Spawns `make_worker()`'s output as its own task -- so a panic inside it
/// is caught by the runtime as a [`tokio::task::JoinError`] rather than by
/// this function's own caller -- and respawns it whenever it ends, whether
/// by panic or (unexpectedly, since the real worker otherwise runs forever)
/// by returning. Exits only when the worker is cancelled, which happens
/// when the owning `Client` is dropped.
fn spawn_supervised_with<F, Fut>(
    name: String,
    make_worker: F,
) -> (JoinHandle<()>, Arc<Mutex<Option<AbortHandle>>>)
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let active_worker = Arc::new(Mutex::new(None));
    let supervisor = tokio::spawn({
        let active_worker = Arc::clone(&active_worker);
        async move {
            loop {
                let worker = tokio::spawn(make_worker());
                *active_worker.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some(worker.abort_handle());

                match worker.await {
                    Err(join_error) if join_error.is_cancelled() => return,
                    Err(join_error) => log_event(&name, "panic", &join_error),
                    Ok(()) => log_event(
                        &name,
                        "panic",
                        "camera task returned without panicking; restarting anyway",
                    ),
                }
            }
        }
    });
    (supervisor, active_worker)
}

/// Runs one camera's connection for as long as the process wants it,
/// reconnecting with backoff after any session-ending condition. Never
/// returns on its own -- the only ways this task ends are a panic (caught
/// by [`spawn_supervised_with`]) or cancellation (the owning `Client` was
/// dropped).
async fn run_camera(config: CameraConfig, sender: broadcast::Sender<Frame>) {
    let mut backoff = Backoff::new();
    loop {
        let started = Instant::now();
        if let Err(err) = run_camera_once(&config, &sender).await {
            log_event(&config.name, "disconnect", &err);
        }
        if started.elapsed() >= STABLE_CONNECTION_THRESHOLD {
            backoff.reset();
        }
        let delay = backoff.advance();
        log_event(
            &config.name,
            "reconnect",
            format_args!("retrying in {delay:?}"),
        );
        tokio::time::sleep(delay).await;
    }
}

/// One connection attempt: handshake, then read/depacketize/publish until
/// the session ends.
async fn run_camera_once(
    config: &CameraConfig,
    sender: &broadcast::Sender<Frame>,
) -> Result<(), ClientError> {
    let addr = SocketAddr::new(config.host, config.port);
    let (track, mut playing) =
        session::connect(addr, &config.path, config.credentials.clone()).await?;
    let mut depacketizer = build_depacketizer(&track)?;
    log_event(
        &config.name,
        "connect",
        format_args!("codec={} clock_rate={}", track.codec_name, track.clock_rate),
    );

    loop {
        let packet = playing.next_packet().await?;
        if packet.channel != VIDEO_RTP_CHANNEL {
            continue;
        }
        for frame in depacketize_packet(&mut depacketizer, &config.name, &packet.payload) {
            // No current subscriber is not this task's failure to handle --
            // the frame is simply not delivered to anyone, same as a topic
            // with no listeners.
            let _ = sender.send(frame);
        }
    }
}

/// Depacketizes one RTP payload, logging and dropping it rather than ending
/// the session on error. A single malformed or out-of-order fragment (e.g.
/// [`DepacketizeError::FragmentWithoutStart`] from a dropped or reordered RTP
/// packet) is not evidence the connection itself has failed -- `playing`
/// already proved it healthy by delivering this packet at all -- so only a
/// [`SessionError`] from `next_packet` still ends [`run_camera_once`]'s loop.
fn depacketize_packet(
    depacketizer: &mut Depacketizer,
    camera_name: &str,
    rtp_payload: &[u8],
) -> Vec<Frame> {
    match depacketizer.depacketize(rtp_payload) {
        Ok(frames) => frames,
        Err(err) => {
            log_event(camera_name, "depacketize_error", &err);
            Vec::new()
        }
    }
}

/// Exponential backoff between reconnect attempts, doubling from
/// [`INITIAL_BACKOFF`] up to a [`MAX_BACKOFF`] ceiling.
#[derive(Debug)]
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

/// Errors from one camera connection attempt: the session layer failed, a
/// received payload failed to depacketize, or the camera declared a video
/// codec this crate does not implement.
#[derive(Debug)]
enum ClientError {
    Session(SessionError),
    Depacketize(DepacketizeError),
    UnsupportedCodec(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Session(err) => write!(f, "{err}"),
            Self::Depacketize(err) => write!(f, "{err}"),
            Self::UnsupportedCodec(name) => {
                write!(f, "camera declared an unsupported video codec: {name}")
            }
        }
    }
}

impl std::error::Error for ClientError {}

impl From<SessionError> for ClientError {
    fn from(err: SessionError) -> Self {
        Self::Session(err)
    }
}

impl From<DepacketizeError> for ClientError {
    fn from(err: DepacketizeError) -> Self {
        Self::Depacketize(err)
    }
}

/// Either video depacketizer this crate implements, selected from the
/// resolved track's own declared codec.
enum Depacketizer {
    H264(H264Depacketizer),
    H265(H265Depacketizer),
}

impl Depacketizer {
    fn depacketize(&mut self, rtp_payload: &[u8]) -> Result<Vec<Frame>, DepacketizeError> {
        match self {
            Self::H264(depacketizer) => depacketizer.depacketize(rtp_payload),
            Self::H265(depacketizer) => depacketizer.depacketize(rtp_payload),
        }
    }
}

/// Builds the depacketizer the resolved video track needs, from its own
/// `a=rtpmap` encoding name and `a=fmtp` parameter sets.
///
/// # Errors
///
/// Returns [`ClientError::UnsupportedCodec`] if the track's codec is
/// neither H.264 nor H.265, or a depacketizer construction error if a
/// declared parameter set does not decode as base64.
fn build_depacketizer(track: &TrackDescription) -> Result<Depacketizer, ClientError> {
    match track.codec_name.as_str() {
        "H264" => {
            let sprop = fmtp_value(&track.format_params, "sprop-parameter-sets");
            Ok(Depacketizer::H264(H264Depacketizer::new(sprop)?))
        }
        "H265" => {
            let sprop = SpropParameterSets {
                vps: fmtp_value(&track.format_params, "sprop-vps"),
                sps: fmtp_value(&track.format_params, "sprop-sps"),
                pps: fmtp_value(&track.format_params, "sprop-pps"),
            };
            Ok(Depacketizer::H265(H265Depacketizer::new(sprop)?))
        }
        other => Err(ClientError::UnsupportedCodec(other.to_string())),
    }
}

/// Looks up one `a=fmtp` parameter by name, defaulting to an empty string
/// (an absent parameter is handed to the depacketizer the same way a
/// declared-but-empty one would be) when the track's SDP never declared it.
fn fmtp_value<'a>(params: &'a [(String, Option<String>)], name: &str) -> &'a str {
    params
        .iter()
        .find(|(key, _)| key == name)
        .and_then(|(_, value)| value.as_deref())
        .unwrap_or("")
}

/// Writes one structured log line for `camera`. This crate has no metrics
/// system; a plain, greppable line to standard error is this item's whole
/// observability surface, per its own scope.
fn log_event(camera: &str, event: &str, detail: impl std::fmt::Display) {
    eprintln!("corvette-rtsp-client camera=\"{camera}\" event={event} detail=\"{detail}\"");
}

#[cfg(test)]
mod tests {
    use super::{Depacketizer, depacketize_packet, spawn_supervised_with};
    use crate::depacketize::H264Depacketizer;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    use std::time::Duration;

    /// Builds one RTP/H.264 packet: a minimal 12-byte RTP header (version 2,
    /// no extensions, arbitrary sequence number/SSRC -- the depacketizer
    /// under test reads only the timestamp) followed by `payload` as the
    /// H.264-packetized RTP payload. Mirrors the identical helper in
    /// `depacketize::h264`'s own tests.
    fn rtp_packet(timestamp: u32, payload: &[u8]) -> Vec<u8> {
        let mut packet = vec![0x80, 0xE0, 0, 1];
        packet.extend_from_slice(&timestamp.to_be_bytes());
        packet.extend_from_slice(&[0, 0, 0, 0]); // SSRC
        packet.extend_from_slice(payload);
        packet
    }

    /// Exercises the exact recovery path `run_camera_once` uses when a
    /// packet fails to depacketize: [`DepacketizeError::FragmentWithoutStart`]
    /// (`crate::depacketize::DepacketizeError`) from an FU-A continuation
    /// with no start fragment in progress -- the real, occasional
    /// dropped/reordered RTP condition a real camera's own stream hit
    /// repeatedly -- must be dropped and logged, not fatal, and processing
    /// must continue producing frames from the packets that follow it.
    #[test]
    fn a_depacketize_error_is_dropped_and_processing_continues() {
        let mut depacketizer = Depacketizer::H264(H264Depacketizer::new("").expect("decodes"));
        // Consume the initial (empty, since no sprop-parameter-sets was
        // declared) parameter-set emission so it does not affect the frame
        // counts asserted below.
        depacketize_packet(
            &mut depacketizer,
            "test-camera",
            &rtp_packet(1000, &[0x65, 0xAA]),
        );

        let first = depacketize_packet(
            &mut depacketizer,
            "test-camera",
            &rtp_packet(2000, &[0x65, 0xBB]),
        );
        assert_eq!(
            first.len(),
            1,
            "a valid single-NAL packet before the corruption still produces a frame"
        );

        // An FU-A continuation (type 28) with neither the start nor end bit
        // set, arriving with no fragment in progress: the depacketizer
        // returns DepacketizeError::FragmentWithoutStart.
        let fu_indicator = 0x60 | 0x1C;
        let no_start_or_end_fu_header = 0x05;
        let corrupted = depacketize_packet(
            &mut depacketizer,
            "test-camera",
            &rtp_packet(3000, &[fu_indicator, no_start_or_end_fu_header, 0xCC]),
        );
        assert!(
            corrupted.is_empty(),
            "the corrupted packet is dropped, producing no frames -- and must not panic or propagate"
        );

        let second = depacketize_packet(
            &mut depacketizer,
            "test-camera",
            &rtp_packet(4000, &[0x65, 0xDD]),
        );
        assert_eq!(
            second.len(),
            1,
            "processing continues past the corrupted packet: a later valid frame is still produced"
        );
    }

    /// A stand-in for one camera's connection: increments `ticks` on a
    /// steady interval, panicking on its `panic_after`th tick if that count
    /// is nonzero (used once, then never again).
    async fn instrumented_worker(ticks: Arc<AtomicU32>, panic_after: Arc<AtomicUsize>) {
        loop {
            tokio::time::sleep(Duration::from_millis(5)).await;
            let remaining = panic_after.load(Ordering::SeqCst);
            if remaining == 1 {
                panic_after.store(0, Ordering::SeqCst);
                panic!("deliberately injected panic for the isolation test");
            } else if remaining > 1 {
                panic_after.store(remaining - 1, Ordering::SeqCst);
            }
            ticks.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Exercises the exact restart-on-panic loop [`spawn_supervised`] uses
    /// for a real camera, without depending on a live RTSP session: one
    /// supervised worker panics partway through, a second, independent
    /// supervised worker never does. INV-2 requires the second worker's own
    /// progress to be completely unaffected, and the panicking worker's own
    /// progress to resume once its supervisor restarts it.
    #[tokio::test]
    async fn a_panicking_supervised_worker_recovers_without_affecting_a_sibling() {
        let panicking_ticks = Arc::new(AtomicU32::new(0));
        let panic_after = Arc::new(AtomicUsize::new(20));
        let (panicking_supervisor, _panicking_worker) =
            spawn_supervised_with("panicking".to_string(), {
                let panicking_ticks = Arc::clone(&panicking_ticks);
                let panic_after = Arc::clone(&panic_after);
                move || instrumented_worker(Arc::clone(&panicking_ticks), Arc::clone(&panic_after))
            });

        let healthy_ticks = Arc::new(AtomicU32::new(0));
        let (healthy_supervisor, _healthy_worker) = spawn_supervised_with("healthy".to_string(), {
            let healthy_ticks = Arc::clone(&healthy_ticks);
            move || instrumented_worker(Arc::clone(&healthy_ticks), Arc::new(AtomicUsize::new(0)))
        });

        // Let the panic happen (after ~100ms) and give the supervisor time
        // to restart its worker and resume ticking.
        tokio::time::sleep(Duration::from_millis(400)).await;

        let panicking_after_recovery = panicking_ticks.load(Ordering::SeqCst);
        let healthy_after = healthy_ticks.load(Ordering::SeqCst);

        assert!(
            panicking_after_recovery > 20,
            "the panicking worker's own supervisor must restart it and let it resume ticking, got {panicking_after_recovery} ticks"
        );
        assert!(
            healthy_after > 20,
            "a sibling worker that never panicked must keep ticking throughout, got {healthy_after} ticks"
        );

        panicking_supervisor.abort();
        healthy_supervisor.abort();
    }
}
