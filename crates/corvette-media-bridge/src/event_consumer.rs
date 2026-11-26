//! Persists the detection service's track lifecycle and motion tick lines into the recording index, dialed from an environment-named socket path and kept alive across disconnects with backoff.

use crate::index::{EventWrite, IndexWrite, ReviewActivity};
use crate::record_config::Review;
use crate::supervise::log_event;
use corvette_detect_proto::{TrackFrame, TrackInfo, decode_track_line};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;

pub const TRACK_EVENTS_SOCKET_ENV_VAR: &str = "CORVETTE_TRACK_EVENTS_SOCKET";
pub const EVENT_ROW_REFRESH_MS: i64 = 5_000;
pub const EVENT_SWEEP_INTERVAL: Duration = Duration::from_secs(15);
pub const EVENT_RECONNECT_BACKOFF: Duration = Duration::from_secs(2);
const EVENT_LINE_MAX_BYTES: usize = 16_384;
const EVENT_READ_CHUNK: usize = 4_096;

fn millis(seconds: f64) -> i64 {
    // The bound keeps the millisecond count inside i64; callers pass finite times.
    let bounded = seconds.clamp(0.0, 4_000_000_000.0);
    i64::try_from(Duration::from_secs_f64(bounded).as_millis()).unwrap_or(i64::MAX)
}

fn opens_event(review: &Review, label: &str) -> bool {
    review
        .detection_labels
        .as_ref()
        .is_none_or(|labels| labels.iter().any(|candidate| *candidate == label))
}

#[derive(Clone, Copy, Debug)]
struct OpenTrack {
    start_ts: i64,
    last_write_ms: i64,
    is_alert: bool,
}

#[derive(Debug)]
pub struct EventWriter {
    review: HashMap<String, Review>,
    last_write: HashMap<(String, String), OpenTrack>,
}

impl EventWriter {
    #[must_use]
    pub fn new(review: HashMap<String, Review>) -> Self {
        Self {
            review,
            last_write: HashMap::new(),
        }
    }

    /// The event writes and the review activity this line causes at `now_ms` from the caller's clock; a line time ahead of that clock is used as that clock's time, and a line for an unseen or throttled track yields neither.
    pub fn apply(
        &mut self,
        frame: &TrackFrame,
        now_ms: i64,
    ) -> (Vec<EventWrite>, Option<ReviewActivity>) {
        match frame {
            TrackFrame::Start(info) | TrackFrame::Update(info) => self.info(info, now_ms),
            TrackFrame::Motion { .. } => (Vec::new(), None),
            TrackFrame::End { camera, track_id } => self
                .last_write
                .remove(&(camera.clone(), track_id.clone()))
                .map_or((Vec::new(), None), |open| {
                    (
                        vec![EventWrite::Close {
                            camera: camera.clone(),
                            track_id: track_id.clone(),
                            start_ts: open.start_ts,
                            end_ts: now_ms,
                        }],
                        Some(ReviewActivity {
                            camera: camera.clone(),
                            activity_ts: now_ms,
                            is_alert: open.is_alert,
                        }),
                    )
                }),
        }
    }

    #[must_use]
    pub fn sweep(&self, now_ms: i64) -> IndexWrite {
        let alert_labels = self
            .review
            .iter()
            .map(|(camera, review)| (camera.clone(), review.alert_labels.clone()))
            .collect();
        IndexWrite::Sweep {
            now_ms,
            alert_labels,
        }
    }

    fn info(&mut self, info: &TrackInfo, now_ms: i64) -> (Vec<EventWrite>, Option<ReviewActivity>) {
        let fallback = Review::default();
        let review = self.review.get(&info.camera).unwrap_or(&fallback);
        let is_alert = review.alert_labels.contains(&info.label);
        if !opens_event(review, &info.label) {
            return (Vec::new(), None);
        }
        let Some(first_motion) = info.first_motion else {
            return (Vec::new(), None);
        };
        if !(first_motion.is_finite() && info.last_seen.is_finite()) {
            log_event(
                "events",
                "track-events-time",
                "non-finite time; line skipped",
            );
            return (Vec::new(), None);
        }
        let Ok(box_json) = serde_json::to_string(&info.bounds) else {
            log_event(
                "events",
                "track-events-box",
                "non-finite bounds; line skipped",
            );
            return (Vec::new(), None);
        };
        let Ok(zones_json) = serde_json::to_string(&info.zones) else {
            log_event(
                "events",
                "track-events-zones",
                "zones did not encode; line skipped",
            );
            return (Vec::new(), None);
        };
        let key = (info.camera.clone(), info.track_id.clone());
        let start_ts = millis(first_motion);
        let last_motion_ts = millis(info.last_seen).min(now_ms);
        let open = OpenTrack {
            start_ts,
            last_write_ms: now_ms,
            is_alert,
        };
        match self.last_write.get(&key).copied() {
            Some(previous) if previous.start_ts == start_ts => {
                if now_ms - previous.last_write_ms < EVENT_ROW_REFRESH_MS {
                    return (Vec::new(), None);
                }
                self.last_write.insert(key, open);
                (
                    vec![EventWrite::Refresh {
                        camera: info.camera.clone(),
                        track_id: info.track_id.clone(),
                        start_ts,
                        last_motion_ts,
                        box_json,
                        zones_json,
                    }],
                    Some(ReviewActivity {
                        camera: info.camera.clone(),
                        activity_ts: last_motion_ts,
                        is_alert,
                    }),
                )
            }
            Some(_) | None => {
                self.last_write.insert(key, open);
                (
                    vec![EventWrite::Open {
                        camera: info.camera.clone(),
                        track_id: info.track_id.clone(),
                        label: info.label.clone(),
                        start_ts,
                        first_motion_ts: start_ts,
                        last_motion_ts,
                        box_json,
                        zones_json,
                    }],
                    Some(ReviewActivity {
                        camera: info.camera.clone(),
                        activity_ts: last_motion_ts,
                        is_alert,
                    }),
                )
            }
        }
    }
}

const MILLIS_PER_SECOND: i64 = 1_000;

/// Forwards the index writes one decoded line causes: a motion tick becomes one write with its second clamped to the bridge clock's current second, a track line becomes its event writes and review activity.
fn forward_frame(
    sender: &SyncSender<IndexWrite>,
    writer: &mut EventWriter,
    frame: &TrackFrame,
    now_ms: i64,
) -> Result<(), ()> {
    if let TrackFrame::Motion { camera, second } = frame {
        let second = i64::try_from(*second)
            .unwrap_or(i64::MAX)
            .min(now_ms / MILLIS_PER_SECOND);
        return forward(
            sender,
            IndexWrite::Motion {
                camera: camera.clone(),
                second,
            },
        );
    }
    let (line_writes, review_activity) = writer.apply(frame, now_ms);
    for event in line_writes {
        forward(sender, IndexWrite::Event(event))?;
    }
    if let Some(activity) = review_activity {
        forward(sender, IndexWrite::Review(activity))?;
    }
    Ok(())
}

fn forward(sender: &SyncSender<IndexWrite>, write: IndexWrite) -> Result<(), ()> {
    match sender.try_send(write) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            log_event(
                "events",
                "track-events-queue",
                "index queue full; write dropped",
            );
            Ok(())
        }
        Err(TrySendError::Disconnected(_)) => Err(()),
    }
}

/// Reads newline-framed track lines from `stream`, forwarding their writes to `sender`, and emits the cutoff sweep every [`EVENT_SWEEP_INTERVAL`].
///
/// Ok ends on stream end, read failure or an over-long line (the caller redials); Err means the index writer is gone.
async fn pump(
    mut stream: UnixStream,
    sender: &SyncSender<IndexWrite>,
    writer: &mut EventWriter,
    clock: &(impl Fn() -> i64 + Send + Sync),
) -> Result<(), ()> {
    let mut buffer: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; EVENT_READ_CHUNK];
    let mut ticker = tokio::time::interval(EVENT_SWEEP_INTERVAL);
    loop {
        tokio::select! {
            read = stream.read(&mut chunk) => {
                match read {
                    Ok(0) => return Ok(()),
                    Ok(size) => {
                        buffer.extend_from_slice(&chunk[..size]);
                        while let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
                            let line: Vec<u8> = buffer.drain(..=position).collect();
                            match decode_track_line(&line[..line.len() - 1]) {
                                Err(source) => log_event("events", "track-events-line", source),
                                Ok(frame) => {
                                    forward_frame(sender, writer, &frame, clock())?;
                                }
                            }
                        }
                        if buffer.len() > EVENT_LINE_MAX_BYTES {
                            log_event("events", "track-events-line", "line exceeds the size limit; redialing");
                            return Ok(());
                        }
                    }
                    Err(source) => {
                        log_event("events", "track-events-read", format_args!("{source}"));
                        return Ok(());
                    }
                }
            }
            _ = ticker.tick() => {
                forward(sender, writer.sweep(clock()))?;
            }
        }
    }
}

/// The consumer task: dial the detection service's subscriber socket and persist track lines until the index writer is gone.
///
/// Logs `open_events` once as the restart recovery line (nothing is closed at startup), then dials `socket` forever with `backoff` between attempts, applying each decoded line's writes to `sender`; it requires that index writer and returns when it is gone.
pub async fn run(
    socket: PathBuf,
    sender: SyncSender<IndexWrite>,
    mut writer: EventWriter,
    open_events: i64,
    clock: impl Fn() -> i64 + Send + Sync,
    backoff: Duration,
) {
    log_event(
        "events",
        "recovery",
        format_args!("open-events={open_events}"),
    );
    loop {
        match UnixStream::connect(&socket).await {
            Err(source) => log_event(
                "events",
                "track-events-dial",
                format_args!("path={} failed={source}", socket.display()),
            ),
            Ok(stream) => {
                log_event(
                    "events",
                    "track-events-dial",
                    format_args!("connected path={}", socket.display()),
                );
                if pump(stream, &sender, &mut writer, &clock).await.is_err() {
                    return;
                }
                log_event(
                    "events",
                    "track-events-disconnected",
                    format_args!("path={}", socket.display()),
                );
            }
        }
        tokio::time::sleep(backoff).await;
    }
}
