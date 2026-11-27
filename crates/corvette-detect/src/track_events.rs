//! Track-event subscriber socket: per-camera publishers read the frame watch channel and push `start`, `update`, `end` and `motion` lines to every accepted connection; the service never reads from a subscriber.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use corvette_detect_proto::{TrackFrame, TrackInfo, encode_track_line};
use tokio::io::AsyncWriteExt;
use tokio::net::unix::SocketAddr;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, watch};

use crate::log_event;
use crate::object_stage::CameraObjects;

// Bound on one subscriber's queued lines; start and end lines enqueue past this bound instead of dropping.
pub const TRACK_EVENTS_QUEUE_LINES: usize = 256;
// Hard bound on every queued line; a stalled subscriber that reaches it is closed, its consumer reconnects and its sweep recovers.
const TRACK_EVENTS_MAX_QUEUED_LINES: usize = 4_096;
const _: () = assert!(TRACK_EVENTS_MAX_QUEUED_LINES > TRACK_EVENTS_QUEUE_LINES);
pub const TRACK_EVENTS_MAX_SUBSCRIBERS: usize = 16;
// Concurrent GET /events streams counted after the head parses; over the limit answers 503.
const MAX_EVENT_STREAMS: usize = 16;
pub const TRACK_EVENTS_ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
const CLOCK_SECONDS_MAX: f64 = 4_000_000_000.0;

type Subscribers = Arc<Mutex<Vec<Arc<Subscriber>>>>;

struct QueuedLine {
    droppable: bool,
    bytes: Arc<[u8]>,
}

// Centre travel that counts as the first motion: above the IoU-0.6 jitter band of the stationary classifier for boxes up to a quarter of the frame, below one second of walking.
const FIRST_MOTION_MIN_TRAVEL: f64 = 0.04;

#[derive(Default)]
struct QueueState {
    lines: VecDeque<QueuedLine>,
    dropped: u64,
    closed: bool,
}

struct LineState {
    first_motion: Option<f64>,
    first_centre: [f64; 2],
}

struct Subscriber {
    id: u64,
    queue: Mutex<QueueState>,
    wake: Notify,
    closing: Notify,
}

impl Subscriber {
    fn new(id: u64) -> Self {
        Self {
            id,
            queue: Mutex::default(),
            wake: Notify::new(),
            closing: Notify::new(),
        }
    }

    fn lock_queue(&self) -> std::sync::MutexGuard<'_, QueueState> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn push(&self, bytes: Arc<[u8]>, droppable: bool) {
        let mut state = self.lock_queue();
        if state.closed {
            return;
        }
        if state.lines.len() >= TRACK_EVENTS_MAX_QUEUED_LINES {
            state.closed = true;
            drop(state);
            log_event(
                "track-events",
                "queue-full",
                format!("subscriber={} closed", self.id),
            );
            self.wake.notify_one();
            self.closing.notify_one();
            return;
        }
        if droppable && state.lines.len() >= TRACK_EVENTS_QUEUE_LINES {
            state.dropped += 1;
            let oldest = state.lines.iter().position(|line| line.droppable);
            let Some(oldest) = oldest else {
                let dropped = state.dropped;
                drop(state);
                log_event(
                    "track-events",
                    "queue-drop",
                    format!("subscriber={} dropped={dropped}", self.id),
                );
                self.wake.notify_one();
                return;
            };
            state.lines.remove(oldest);
            log_event(
                "track-events",
                "queue-drop",
                format!("subscriber={} dropped={}", self.id, state.dropped),
            );
        }
        state.lines.push_back(QueuedLine { droppable, bytes });
        drop(state);
        self.wake.notify_one();
    }

    /// Returns the next queued line, or `None` once the subscriber is closed.
    async fn next_bytes(&self) -> Option<Arc<[u8]>> {
        loop {
            let notified = self.wake.notified();
            let popped = {
                let mut state = self.lock_queue();
                if state.closed {
                    return None;
                }
                state.lines.pop_front()
            };
            if let Some(line) = popped {
                return Some(line.bytes);
            }
            notified.await;
        }
    }
}

/// One HTTP event stream: the queue registered by [`EventFeed::admit_stream`], drained until the queue closes and freed by [`EventFeed::release_stream`].
pub struct EventStream {
    subscriber: Arc<Subscriber>,
}

impl std::fmt::Debug for EventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventStream").finish_non_exhaustive()
    }
}

impl EventStream {
    /// The next line in queue order; `None` once the queue closed.
    pub async fn next_line(&self) -> Option<Arc<[u8]>> {
        self.subscriber.next_bytes().await
    }

    /// Completes when the queue closed, the hard bound having reached it.
    pub async fn closed(&self) {
        self.subscriber.closing.notified().await;
    }
}

/// One shared track-line feed: per-camera publishers write each encoded line to every subscriber of both transports, the track-events socket and the HTTP event streams.
pub struct EventFeed {
    sockets: Subscribers,
    streams: Subscribers,
    ids: AtomicU64,
}

impl std::fmt::Debug for EventFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventFeed").finish_non_exhaustive()
    }
}

impl Default for EventFeed {
    fn default() -> Self {
        Self::new()
    }
}

impl EventFeed {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sockets: Arc::new(Mutex::new(Vec::new())),
            streams: Arc::new(Mutex::new(Vec::new())),
            ids: AtomicU64::new(1),
        }
    }

    /// Spawns one publisher task per camera; each publishes that camera's frames until its objects sender is dropped, and closes its own open tracks when the camera-stopped value arrives. Call inside the runtime that should own the tasks.
    pub fn spawn_publishers(
        self: &Arc<Self>,
        cameras: Vec<(String, watch::Receiver<CameraObjects>)>,
    ) {
        for (camera, objects) in cameras {
            tokio::spawn(publish(camera, objects, Arc::clone(self)));
        }
    }

    fn push(&self, frame: &TrackFrame) {
        broadcast(&self.sockets, frame);
        broadcast(&self.streams, frame);
    }

    fn admit_socket(&self) -> Result<Arc<Subscriber>, u64> {
        let id = self.ids.fetch_add(1, Ordering::Relaxed);
        let subscriber = {
            let mut sockets = self
                .sockets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if sockets.len() >= TRACK_EVENTS_MAX_SUBSCRIBERS {
                return Err(id);
            }
            let subscriber = Arc::new(Subscriber::new(id));
            sockets.push(Arc::clone(&subscriber));
            subscriber
        };
        Ok(subscriber)
    }

    fn release_socket(&self, subscriber: &Subscriber) {
        self.sockets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|kept| kept.id != subscriber.id);
    }

    pub(crate) fn admit_stream(&self) -> Option<EventStream> {
        let subscriber = {
            let mut streams = self
                .streams
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if streams.len() >= MAX_EVENT_STREAMS {
                return None;
            }
            let subscriber = Arc::new(Subscriber::new(self.ids.fetch_add(1, Ordering::Relaxed)));
            streams.push(Arc::clone(&subscriber));
            subscriber
        };
        Some(EventStream { subscriber })
    }

    pub(crate) fn release_stream(&self, stream: &EventStream) {
        self.streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|kept| kept.id != stream.subscriber.id);
    }
}

fn broadcast(subscribers: &Subscribers, frame: &TrackFrame) {
    let droppable = matches!(frame, TrackFrame::Update(_) | TrackFrame::Motion { .. });
    let bytes: Arc<[u8]> = Arc::from(encode_track_line(frame).into_boxed_slice());
    let targets: Vec<Arc<Subscriber>> = subscribers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    for subscriber in targets {
        subscriber.push(Arc::clone(&bytes), droppable);
    }
}

const fn centre(bounds: [f64; 4]) -> [f64; 2] {
    [
        f64::midpoint(bounds[0], bounds[2]),
        f64::midpoint(bounds[1], bounds[3]),
    ]
}

fn note_travel(state: &mut LineState, bounds: [f64; 4], frame_time: f64) {
    if state.first_motion.is_some() {
        return;
    }
    let now = centre(bounds);
    let dx = now[0] - state.first_centre[0];
    let dy = now[1] - state.first_centre[1];
    if dx.hypot(dy) > FIRST_MOTION_MIN_TRAVEL {
        state.first_motion = Some(frame_time);
    }
}

/// A source of incoming subscriber connections for `serve_track_events`.
pub trait TrackAccept {
    /// Accepts the next subscriber connection.
    fn accept(&self) -> impl Future<Output = std::io::Result<(UnixStream, SocketAddr)>> + Send;
}

impl TrackAccept for UnixListener {
    fn accept(&self) -> impl Future<Output = std::io::Result<(UnixStream, SocketAddr)>> + Send {
        Self::accept(self)
    }
}

/// Serves track frames to subscriber connections until the task is stopped.
///
/// Lines come from the feed's publishers; the feed and its publishers are created and spawned with [`EventFeed::spawn_publishers`].
/// Each accepted connection is a subscriber until it closes and receives only pushed lines.
/// A subscriber whose socket stalls drops queued `update` and `motion` lines, counted in a log line, and keeps every `start` and `end` line; a queue that reaches `TRACK_EVENTS_MAX_QUEUED_LINES` closes that subscriber so its consumer reconnects and recovers.
/// Publishing never blocks on a subscriber.
/// Connections beyond `TRACK_EVENTS_MAX_SUBSCRIBERS` are closed immediately; a failed `accept` is logged and retried after `TRACK_EVENTS_ACCEPT_BACKOFF`.
pub async fn serve_track_events<L: TrackAccept + Send + 'static>(
    listener: L,
    feed: Arc<EventFeed>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                let subscriber = match feed.admit_socket() {
                    Ok(subscriber) => subscriber,
                    Err(id) => {
                        log_event(
                            "track-events",
                            "subscriber-limit",
                            format!("refused subscriber {id} at {TRACK_EVENTS_MAX_SUBSCRIBERS}"),
                        );
                        continue;
                    }
                };
                tokio::spawn(pump(stream, subscriber, Arc::clone(&feed)));
            }
            Err(error) => {
                log_event("track-events", "accept-failed", error);
                tokio::time::sleep(TRACK_EVENTS_ACCEPT_BACKOFF).await;
            }
        }
    }
}

async fn publish(
    camera: String,
    mut objects: watch::Receiver<CameraObjects>,
    feed: Arc<EventFeed>,
) {
    let mut lines: HashMap<String, LineState> = HashMap::new();
    let mut tick_second: Option<u64> = None;
    loop {
        if objects.changed().await.is_err() {
            return;
        }
        let known = match &*objects.borrow_and_update() {
            CameraObjects::Frame(known) => known.clone(),
            CameraObjects::Cleared => continue,
            CameraObjects::Stopped => {
                // The camera ended without a per-frame end list; this publisher owns the open set, so it ends every open line, sorted by track id for a stable order.
                let mut ended: Vec<&String> = lines.keys().collect();
                ended.sort();
                for track_id in ended {
                    feed.push(&TrackFrame::End {
                        camera: camera.clone(),
                        track_id: track_id.clone(),
                    });
                }
                lines.clear();
                continue;
            }
        };
        if known.motion_triggered {
            // The bound matches the consumer's millisecond clamp, so the frame time always converts.
            let bounded = known.frame_time.clamp(0.0, CLOCK_SECONDS_MAX);
            let second = Duration::from_secs_f64(bounded).as_secs();
            if tick_second.is_none_or(|sent| second > sent) {
                tick_second = Some(second);
                feed.push(&TrackFrame::Motion {
                    camera: camera.clone(),
                    second,
                });
            }
        }
        for ended in &known.ended {
            if lines.remove(ended).is_some() {
                feed.push(&TrackFrame::End {
                    camera: camera.clone(),
                    track_id: ended.clone(),
                });
            }
        }
        for object in &known.objects {
            if !object.confirmed {
                continue;
            }
            let (first_motion, started) = match lines.get_mut(&object.id) {
                None => {
                    lines.insert(
                        object.id.clone(),
                        LineState {
                            first_motion: None,
                            first_centre: centre(object.bounds),
                        },
                    );
                    (None, false)
                }
                Some(state) => {
                    note_travel(state, object.bounds, known.frame_time);
                    (state.first_motion, true)
                }
            };
            let info = TrackInfo {
                camera: camera.clone(),
                track_id: object.id.clone(),
                label: object.label.clone(),
                bounds: object.bounds,
                zones: object.zones_entered.clone(),
                first_motion,
                last_seen: object.last_seen,
            };
            let frame = if started {
                TrackFrame::Update(info)
            } else {
                TrackFrame::Start(info)
            };
            feed.push(&frame);
        }
    }
}

async fn pump(mut stream: UnixStream, subscriber: Arc<Subscriber>, feed: Arc<EventFeed>) {
    loop {
        let Some(bytes) = subscriber.next_bytes().await else {
            break;
        };
        tokio::select! {
            () = subscriber.closing.notified() => break,
            written = stream.write_all(&bytes) => {
                if let Err(error) = written {
                    log_event(
                        "track-events",
                        "subscriber-write-failed",
                        format!("subscriber={} {error}", subscriber.id),
                    );
                    break;
                }
            }
        }
    }
    feed.release_socket(&subscriber);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object_stage::{FrameObjects, TrackedObject};
    use std::sync::PoisonError;

    fn queued_lines(subscriber: &Subscriber) -> Vec<String> {
        subscriber
            .lock_queue()
            .lines
            .iter()
            .map(|line| String::from_utf8_lossy(&line.bytes).into_owned())
            .collect()
    }

    async fn wait_for_lines(subscriber: &Subscriber, expected: usize) -> Vec<String> {
        for _ in 0..100 {
            if queued_lines(subscriber).len() >= expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        queued_lines(subscriber)
    }

    fn motion_frame(frame_time: f64, triggered: bool) -> FrameObjects {
        FrameObjects {
            frame_time,
            objects: Vec::new(),
            ended: Vec::new(),
            motion_triggered: triggered,
        }
    }

    fn start_info() -> TrackInfo {
        TrackInfo {
            camera: "front".to_owned(),
            track_id: "7".to_owned(),
            label: "person".to_owned(),
            bounds: [0.1, 0.2, 0.3, 0.4],
            zones: Vec::new(),
            first_motion: None,
            last_seen: 10.5,
        }
    }

    #[tokio::test]
    async fn triggered_frames_emit_at_most_one_motion_tick_per_camera_per_second() {
        let subscriber = Arc::new(Subscriber::new(61));
        let feed = Arc::new(EventFeed::new());
        feed.sockets.lock().unwrap().push(Arc::clone(&subscriber));
        let (sender, receiver) = watch::channel(CameraObjects::Cleared);
        tokio::spawn(publish("front".to_owned(), receiver, feed));
        for time in [100.5, 100.9, 101.2] {
            sender
                .send(CameraObjects::Frame(motion_frame(time, true)))
                .expect("frame published");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            wait_for_lines(&subscriber, 2).await,
            vec!["1\tm\tfront\t100\n", "1\tm\tfront\t101\n"],
            "one tick per wall-clock second, none for a second already ticked"
        );
    }

    #[tokio::test]
    async fn frames_without_a_motion_trigger_emit_no_tick_line() {
        let subscriber = Arc::new(Subscriber::new(62));
        let feed = Arc::new(EventFeed::new());
        feed.sockets.lock().unwrap().push(Arc::clone(&subscriber));
        let (sender, receiver) = watch::channel(CameraObjects::Cleared);
        tokio::spawn(publish("front".to_owned(), receiver, feed));
        for time in [100.5, 101.2] {
            sender
                .send(CameraObjects::Frame(motion_frame(time, false)))
                .expect("frame published");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            wait_for_lines(&subscriber, 1).await.is_empty(),
            "a frame whose motion stage did not trigger gets no tick"
        );
    }

    #[test]
    fn a_queue_at_the_soft_bound_drops_the_incoming_tick_and_keeps_start_and_end_lines() {
        let subscriber = Arc::new(Subscriber::new(45));
        let subscribers: Subscribers = Arc::new(Mutex::new(vec![Arc::clone(&subscriber)]));
        for index in 0..=TRACK_EVENTS_QUEUE_LINES {
            subscriber.push(Arc::from(format!("boundary-{index}").into_bytes()), false);
        }
        broadcast(&subscribers, &TrackFrame::Start(start_info()));
        broadcast(
            &subscribers,
            &TrackFrame::Motion {
                camera: "front".to_owned(),
                second: 100,
            },
        );
        broadcast(
            &subscribers,
            &TrackFrame::End {
                camera: "front".to_owned(),
                track_id: "7".to_owned(),
            },
        );
        let (dropped, queued, texts) = {
            let state = subscriber.lock_queue();
            (
                state.dropped,
                state.lines.len(),
                state
                    .lines
                    .iter()
                    .map(|line| String::from_utf8_lossy(&line.bytes).into_owned())
                    .collect::<Vec<String>>(),
            )
        };
        assert_eq!(dropped, 1);
        assert_eq!(queued, TRACK_EVENTS_QUEUE_LINES + 3);
        assert!(
            texts.iter().any(|text| text.starts_with("1\ts\t")),
            "the start line survived: {texts:?}"
        );
        assert!(
            texts.last().is_some_and(|text| text.starts_with("1\te\t")),
            "the end line survived: {texts:?}"
        );
        assert!(
            !texts.iter().any(|text| text.starts_with("1\tm\t")),
            "the tick is droppable like an update line"
        );
    }

    fn logged_drops(subscriber: u64, dropped: u64) -> bool {
        crate::TEST_LOG
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|line| {
                line.contains("event=queue-drop")
                    && line.contains(&format!("subscriber={subscriber}"))
                    && line.contains(&format!("dropped={dropped}"))
            })
    }

    #[test]
    fn an_overflowing_queue_drops_oldest_updates_and_keeps_start_and_end_lines() {
        let subscriber = Subscriber::new(41);
        subscriber.push(Arc::from(&b"start"[..]), false);
        for index in 0..=TRACK_EVENTS_QUEUE_LINES {
            subscriber.push(Arc::from(format!("update-{index}").into_bytes()), true);
        }
        subscriber.push(Arc::from(&b"end"[..]), false);
        let state = subscriber.lock_queue();
        assert_eq!(state.dropped, 2);
        assert_eq!(state.lines.len(), TRACK_EVENTS_QUEUE_LINES + 1);
        assert!(!state.lines.front().expect("queue has a front").droppable);
        assert!(!state.lines.back().expect("queue has a back").droppable);
        let oldest_update = state
            .lines
            .iter()
            .find(|line| line.droppable)
            .expect("queue has an update");
        assert_eq!(&*oldest_update.bytes, b"update-2");
        drop(state);
        assert!(logged_drops(41, 2));
    }

    #[test]
    fn a_queue_of_boundaries_only_drops_the_incoming_update() {
        let subscriber = Subscriber::new(42);
        for index in 0..=TRACK_EVENTS_QUEUE_LINES {
            subscriber.push(Arc::from(format!("boundary-{index}").into_bytes()), false);
        }
        subscriber.push(Arc::from(&b"incoming"[..]), true);
        let state = subscriber.lock_queue();
        assert_eq!(state.lines.len(), TRACK_EVENTS_QUEUE_LINES + 1);
        assert!(state.lines.iter().all(|line| !line.droppable));
        assert_eq!(state.dropped, 1);
        drop(state);
        assert!(logged_drops(42, 1));
    }

    #[test]
    fn a_stalled_queue_at_the_hard_bound_closes_the_subscriber() {
        let subscriber = Subscriber::new(43);
        for index in 0..TRACK_EVENTS_MAX_QUEUED_LINES {
            subscriber.push(Arc::from(format!("line-{index}").into_bytes()), false);
        }
        subscriber.push(Arc::from(&b"one-past"[..]), false);
        let state = subscriber.lock_queue();
        assert!(state.closed);
        assert_eq!(state.lines.len(), TRACK_EVENTS_MAX_QUEUED_LINES);
        drop(state);
        assert!(
            crate::TEST_LOG
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .any(|line| line.contains("event=queue-full") && line.contains("subscriber=43"))
        );
        subscriber.push(Arc::from(&b"after"[..]), false);
        assert_eq!(
            subscriber.lock_queue().lines.len(),
            TRACK_EVENTS_MAX_QUEUED_LINES
        );
    }

    #[test]
    fn a_still_track_then_a_move_stamps_first_motion_on_the_moving_frame() {
        let still = [0.2, 0.3, 0.3, 0.5];
        let mut state = LineState {
            first_motion: None,
            first_centre: centre(still),
        };
        for time in [0.1, 0.2] {
            note_travel(&mut state, still, time);
            assert_eq!(state.first_motion, None);
        }
        note_travel(&mut state, [0.4, 0.3, 0.5, 0.5], 0.3);
        assert_eq!(state.first_motion, Some(0.3));
    }

    #[test]
    fn a_track_moving_from_its_first_frame_gets_first_motion_on_that_frame() {
        let mut state = LineState {
            first_motion: None,
            first_centre: centre([0.2, 0.3, 0.3, 0.5]),
        };
        note_travel(&mut state, [0.3, 0.3, 0.4, 0.5], 0.1);
        assert_eq!(state.first_motion, Some(0.1));
    }

    #[test]
    fn a_track_that_stays_still_within_jitter_never_stamps_first_motion() {
        let mut state = LineState {
            first_motion: None,
            first_centre: centre([0.2, 0.3, 0.3, 0.5]),
        };
        for step in 0..40 {
            let shift = if step % 2 == 0 { 0.01 } else { -0.01 };
            note_travel(
                &mut state,
                [0.2 + shift, 0.3 - shift, 0.3 + shift, 0.5 - shift],
                f64::from(step) / 10.0,
            );
        }
        assert_eq!(state.first_motion, None);
    }

    #[tokio::test]
    async fn the_stopped_value_ends_every_open_line_and_clears_the_set() {
        let subscriber = Arc::new(Subscriber::new(63));
        let feed = Arc::new(EventFeed::new());
        feed.sockets.lock().unwrap().push(Arc::clone(&subscriber));
        let (sender, receiver) = watch::channel(CameraObjects::Cleared);
        tokio::spawn(publish("front".to_owned(), receiver, feed));
        let line_start = |id: &str| {
            String::from_utf8_lossy(&encode_track_line(&TrackFrame::Start(TrackInfo {
                camera: "front".to_owned(),
                track_id: id.to_owned(),
                label: "person".to_owned(),
                bounds: [0.1, 0.2, 0.3, 0.4],
                zones: Vec::new(),
                first_motion: None,
                last_seen: 1.0,
            })))
            .into_owned()
        };
        let line_end = |id: &str| {
            String::from_utf8_lossy(&encode_track_line(&TrackFrame::End {
                camera: "front".to_owned(),
                track_id: id.to_owned(),
            }))
            .into_owned()
        };
        for track_id in ["7", "8"] {
            sender
                .send(CameraObjects::Frame(FrameObjects {
                    frame_time: 1.0,
                    objects: vec![TrackedObject {
                        id: track_id.to_owned(),
                        label: "person".to_owned(),
                        score: 0.9,
                        confirmed: true,
                        bounds: [0.1, 0.2, 0.3, 0.4],
                        edges: [100, 200, 300, 400],
                        first_seen: 1.0,
                        last_seen: 1.0,
                        zones_now: Vec::new(),
                        zones_entered: Vec::new(),
                        stationary: false,
                    }],
                    ended: Vec::new(),
                    motion_triggered: false,
                }))
                .expect("frame published");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        sender.send(CameraObjects::Stopped).expect("stop published");
        assert_eq!(
            wait_for_lines(&subscriber, 4).await,
            vec![
                line_start("7"),
                line_start("8"),
                line_end("7"),
                line_end("8"),
            ],
            "both open tracks end once, in track-id order"
        );
    }

    #[test]
    fn a_centre_jump_above_the_constant_stamps_once_and_never_clears() {
        let mut state = LineState {
            first_motion: None,
            first_centre: centre([0.2, 0.3, 0.3, 0.5]),
        };
        note_travel(&mut state, [0.25, 0.3, 0.35, 0.5], 1.0);
        assert_eq!(state.first_motion, Some(1.0));
        note_travel(&mut state, [0.2, 0.3, 0.3, 0.5], 2.0);
        assert_eq!(state.first_motion, Some(1.0));
    }
}
