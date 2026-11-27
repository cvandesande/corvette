//! Persists the detection service's track lifecycle and motion tick lines into the recording index, read either as raw lines from the track-events socket or as chunk-framed lines from the events route over HTTP, dialed from an environment-named socket path and kept alive across disconnects with backoff.

use crate::index::{EventWrite, IndexWrite, ReviewActivity};
use crate::record_config::Review;
use crate::supervise::log_event;
use corvette_detect_proto::{TrackFrame, TrackInfo, decode_track_line};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;

pub const TRACK_EVENTS_SOCKET_ENV_VAR: &str = "CORVETTE_TRACK_EVENTS_SOCKET";
pub const EVENTS_SOCKET_ENV_VAR: &str = "CORVETTE_EVENTS_SOCKET";
pub const EVENT_ROW_REFRESH_MS: i64 = 5_000;
pub const EVENT_SWEEP_INTERVAL: Duration = Duration::from_secs(15);
pub const EVENT_RECONNECT_BACKOFF: Duration = Duration::from_secs(2);
const EVENT_LINE_MAX_BYTES: usize = 16_384;
const EVENT_READ_CHUNK: usize = 4_096;
const EVENTS_REQUEST: &[u8] = b"GET /corvette/api/v1/events HTTP/1.1\r\n\r\n";
const EVENT_HEAD_MAX_BYTES: usize = 8_192;

/// Which wire a consumer instance reads: the legacy track-events socket or the events route over HTTP.
#[derive(Clone, Copy, Debug)]
pub enum Transport {
    TrackEvents,
    EventsHttp,
}

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

/// Buffers stream bytes and yields whole newline-terminated lines; a partial line stays buffered until its newline or the connection ends.
struct LineSplitter {
    buffer: Vec<u8>,
}

impl LineSplitter {
    fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(1024),
        }
    }

    fn take_lines(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buffer.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            lines.push(self.buffer.drain(..=position).collect());
        }
        lines
    }

    const fn exceeds_line_limit(&self) -> bool {
        self.buffer.len() > EVENT_LINE_MAX_BYTES
    }
}

/// Forwards each complete line through the splitter; the trailing newline is dropped for the codec.
fn forward_lines(
    sender: &SyncSender<IndexWrite>,
    writer: &mut EventWriter,
    splitter: &mut LineSplitter,
    bytes: &[u8],
    clock: i64,
) -> Result<(), ()> {
    for line in splitter.take_lines(bytes) {
        match decode_track_line(&line[..line.len() - 1]) {
            Err(source) => log_event("events", "track-events-line", source),
            Ok(frame) => forward_frame(sender, writer, &frame, clock)?,
        }
    }
    Ok(())
}

enum ChunkEvent {
    NeedMore,
    Payload(Vec<u8>),
    Terminating,
    Fault(&'static str),
}

/// Takes one complete chunk frame from `pending`: hex size line, payload untouched, CRLF terminator; extensions and over-cap sizes are faults.
fn take_chunk(pending: &mut Vec<u8>) -> ChunkEvent {
    let Some(terminator) = pending.windows(2).position(|window| window == b"\r\n") else {
        if pending.len() > EVENT_LINE_MAX_BYTES {
            return ChunkEvent::Fault("chunk size line");
        }
        return ChunkEvent::NeedMore;
    };
    let size = match parse_chunk_size(&pending[..terminator]) {
        Ok(size) => size,
        Err(fault) => return ChunkEvent::Fault(fault),
    };
    if size == 0 {
        pending.clear();
        return ChunkEvent::Terminating;
    }
    if size > EVENT_LINE_MAX_BYTES {
        return ChunkEvent::Fault("chunk over the line limit");
    }
    let frame_end = terminator + 2 + size + 2;
    if pending.len() < frame_end {
        return ChunkEvent::NeedMore;
    }
    if pending[frame_end - 2..frame_end] != *b"\r\n" {
        return ChunkEvent::Fault("chunk terminator");
    }
    let mut payload: Vec<u8> = pending.drain(terminator + 2..frame_end).collect();
    payload.truncate(size);
    pending.drain(..terminator + 2);
    ChunkEvent::Payload(payload)
}

fn parse_chunk_size(size_line: &[u8]) -> Result<usize, &'static str> {
    if size_line.len() > EVENT_LINE_MAX_BYTES {
        return Err("chunk size line");
    }
    if size_line.contains(&b';') {
        return Err("chunk extension");
    }
    let text = std::str::from_utf8(size_line).map_err(|_| "chunk size")?;
    usize::from_str_radix(text, 16).map_err(|_| "chunk size")
}

/// Reads the response head, keeps any bytes the same read carried after it, and only passes a 200 chunked stream.
async fn read_events_head(stream: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>, &'static str> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; EVENT_READ_CHUNK];
    loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let prefix = buffer.split_off(end + 4);
            buffer.truncate(end);
            return if event_head_is_ok(&buffer) {
                Ok(prefix)
            } else {
                Err("head is not a 200 chunked stream")
            };
        }
        if buffer.len() > EVENT_HEAD_MAX_BYTES {
            return Err("head over the size limit");
        }
        match stream.read(&mut chunk).await {
            Ok(0) => return Err("closed before the head"),
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(_) => return Err("read failed during the head"),
        }
    }
}

fn event_head_is_ok(head: &[u8]) -> bool {
    let mut lines = head
        .split(|byte| *byte == b'\r')
        .map(|line| line.strip_prefix(b"\n").unwrap_or(line));
    let status_ok = lines.next().is_some_and(|status| {
        let mut fields = status.split(|byte| *byte == b' ');
        fields
            .next()
            .is_some_and(|version| version.strip_prefix(b"HTTP/1.").is_some())
            && fields.next().is_some_and(|code| code == b"200")
    });
    let mut transfer_chunked = false;
    for line in lines {
        let Some(colon) = line.iter().position(|byte| *byte == b':') else {
            continue;
        };
        let (name, value) = line.split_at(colon);
        if name.eq_ignore_ascii_case(b"transfer-encoding")
            && value[1..].trim_ascii().eq_ignore_ascii_case(b"chunked")
        {
            transfer_chunked = true;
        }
    }
    status_ok && transfer_chunked
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
    let mut splitter = LineSplitter::new();
    let mut chunk = [0u8; EVENT_READ_CHUNK];
    let mut ticker = tokio::time::interval(EVENT_SWEEP_INTERVAL);
    loop {
        tokio::select! {
            read = stream.read(&mut chunk) => {
                match read {
                    Ok(0) => return Ok(()),
                    Ok(size) => {
                        forward_lines(sender, writer, &mut splitter, &chunk[..size], clock())?;
                        if splitter.exceeds_line_limit() {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamOutcome {
    Ended,
    WriterGone,
}

/// Consumes every complete chunk frame in `pending`; `Some` ends the stream or reports the writer gone.
fn drain_chunks(
    pending: &mut Vec<u8>,
    splitter: &mut LineSplitter,
    sender: &SyncSender<IndexWrite>,
    writer: &mut EventWriter,
    clock: &(impl Fn() -> i64 + Send + Sync),
) -> Option<StreamOutcome> {
    loop {
        match take_chunk(pending) {
            ChunkEvent::NeedMore => return None,
            ChunkEvent::Payload(payload) => {
                if forward_lines(sender, writer, splitter, &payload, clock()).is_err() {
                    return Some(StreamOutcome::WriterGone);
                }
                if splitter.exceeds_line_limit() {
                    log_event(
                        "events",
                        "track-events-line",
                        "line exceeds the size limit; redialing",
                    );
                    return Some(StreamOutcome::Ended);
                }
            }
            ChunkEvent::Terminating => return Some(StreamOutcome::Ended),
            ChunkEvent::Fault(reason) => {
                log_event("events", "events-chunk", reason);
                return Some(StreamOutcome::Ended);
            }
        }
    }
}

/// Reads one `GET /corvette/api/v1/events` stream: sends the request, validates the head, then feeds chunk payloads through the splitter with the sweep keeping time while the stream stalls.
///
/// The write half stays open for as long as the stream is wanted: no bytes are sent after the request and nothing is half-closed.
pub async fn pump_events_http(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    sender: &SyncSender<IndexWrite>,
    writer: &mut EventWriter,
    clock: &(impl Fn() -> i64 + Send + Sync),
) -> StreamOutcome {
    if let Err(source) = stream.write_all(EVENTS_REQUEST).await {
        log_event(
            "events",
            "events-request",
            format_args!("write failed: {source}"),
        );
        return StreamOutcome::Ended;
    }
    let mut pending = match read_events_head(&mut stream).await {
        Err(fault) => {
            log_event("events", "events-head", fault);
            return StreamOutcome::Ended;
        }
        Ok(prefix) => prefix,
    };
    let mut splitter = LineSplitter::new();
    let mut chunk = [0u8; EVENT_READ_CHUNK];
    let mut ticker = tokio::time::interval(EVENT_SWEEP_INTERVAL);
    if let Some(outcome) = drain_chunks(&mut pending, &mut splitter, sender, writer, clock) {
        return outcome;
    }
    loop {
        tokio::select! {
            read = stream.read(&mut chunk) => {
                match read {
                    Ok(0) => {
                        log_event("events", "events-eof", "closed before the terminating zero chunk; redialing");
                        return StreamOutcome::Ended;
                    }
                    Ok(size) => {
                        pending.extend_from_slice(&chunk[..size]);
                        if let Some(outcome) = drain_chunks(&mut pending, &mut splitter, sender, writer, clock) {
                            return outcome;
                        }
                    }
                    Err(source) => {
                        log_event("events", "events-read", format_args!("{source}"));
                        return StreamOutcome::Ended;
                    }
                }
            }
            _ = ticker.tick() => {
                if forward(sender, writer.sweep(clock())).is_err() {
                    return StreamOutcome::WriterGone;
                }
            }
        }
    }
}

/// The HTTP consumer task: dials the ipc socket forever, requests the events stream and persists its lines; the open-track map in `writer` survives every redial and the backoff between streams is this loop's `backoff`, mirroring [`run`].
pub async fn run_http(
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
                "events-dial",
                format_args!("path={} failed={source}", socket.display()),
            ),
            Ok(stream) => {
                log_event(
                    "events",
                    "events-dial",
                    format_args!("connected path={}", socket.display()),
                );
                match pump_events_http(stream, &sender, &mut writer, &clock).await {
                    StreamOutcome::Ended => log_event(
                        "events",
                        "events-disconnected",
                        format_args!("path={}", socket.display()),
                    ),
                    StreamOutcome::WriterGone => return,
                }
            }
        }
        tokio::time::sleep(backoff).await;
    }
}
