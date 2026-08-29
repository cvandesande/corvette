//! Hosts the expanded-view fallback's HLS transport (issue #12 item G3,
//! DP-4).
//!
//! A fourth independent per-camera subscription to the same
//! `corvette_rtsp_client::Frame` broadcast G1's restream-feed/MoQ-publish
//! roles and G2's fMP4-repackaging role already read from. Video only: this
//! item inherits the same audio-track gap G1's own Premise names (see
//! `crate`'s top-level doc), and does not fix it.
//!
//! # Module boundary with G2's `crate::fmp4`
//!
//! `crate::fmp4` already builds exactly the CMAF building blocks this item
//! needs -- one initialization segment per camera ([`crate::fmp4::
//! InitSegmentTracker`]) and one `moof`/`mdat` fragment per access unit
//! ([`crate::fmp4::Fragmenter`]) -- and its own module doc already
//! anticipated this reuse ("so a later HLS/CMAF packager can reuse the same
//! fragmentation core through this module's own public API rather than
//! through `ws_repackager`"). This module imports both types directly rather
//! than duplicating or wrapping them: no new module boundary is drawn between
//! "fMP4 box building" and "HLS", because G2 already drew that boundary
//! correctly (pure box-building free of any transport concern). The one
//! change made to `crate::fmp4` for this item's sake is [`crate::fmp4::
//! Fragment`] -- `Fragmenter::next` used to return bare `Bytes`, which was
//! enough for G2's own push-every-fragment-to-every-viewer transport, but
//! this item's own segment-boundary policy (below) needs to know a
//! fragment's keyframe-ness and duration without re-parsing the `moof` box
//! it was just handed. That struct is the "shared internal helper" the
//! plan's own Do step 2 and Scope guard anticipate; G2's own call site
//! (`ws_repackager::run_fmp4_repackage`) was updated mechanically to read the
//! new struct's `.bytes` field, with no other change to G1's or G2's own
//! behavior.
//!
//! What *is* new in this module, because nothing upstream already solved it:
//! grouping the fragment stream into CMAF media segments ([`SegmentBuilder`]),
//! the per-camera pull-based state a GET request reads from ([`CameraHls`]),
//! and the `m3u8` media playlist text itself ([`CameraHls::playlist_text`]).
//! This is a genuinely different consumption model than G2's
//! push-to-every-connected-viewer WebSocket transport: an HTTP client polls a
//! playlist and pulls whichever segments it names, so this item's own shared
//! state is a plain [`std::sync::Mutex`]-guarded snapshot rather than a
//! [`tokio::sync::broadcast`] channel.
//!
//! # Segment duration
//!
//! [`TARGET_SEGMENT_DURATION_TICKS`] (2 seconds) balances live-view latency
//! against segment-request overhead, the same tradeoff X2's own RTP
//! fragmentation-threshold precedent named: shorter segments bound how far
//! behind live a viewer's buffer can be, but every segment boundary costs one
//! more HTTP round trip. 2 seconds is within the 1-6 second range real HLS
//! deployments commonly use (Apple's own HLS authoring guidelines recommend
//! target durations around 6 seconds for broad compatibility, but a
//! LAN-local expanded-view fallback -- this item's actual use, per OPEN-2 --
//! favors the low end of that range for lower glass-to-glass latency, and 2
//! seconds still keeps segment-request overhead to no more than one request
//! every two seconds per viewer).
//!
//! A segment is only cut on a keyframe (never mid-GOP), so every segment
//! after the first is independently decodable from its own first sample --
//! except the very first segment ever produced for a camera, which begins
//! with whichever access unit [`crate::fmp4::Fragmenter`] emits first. A real
//! camera conventionally emits its parameter sets immediately followed by an
//! IDR, so this is expected to hold in practice, but is not asserted as a
//! guarantee (see [`SegmentBuilder`]'s own doc) -- a known, named
//! simplification of this "minimal" packager, matching this crate's existing
//! precedent of stating such limits plainly (`crate::fmp4`'s own `hvcC`/AAC
//! gaps) rather than silently.
//!
//! # Playlist window
//!
//! [`PLAYLIST_WINDOW_SEGMENT_COUNT`] (6 segments, 12 seconds at the chosen
//! target duration) bounds this item's own per-camera memory use to a live
//! sliding window, matching every real HLS live-playlist deployment's own
//! convention (a live playlist is not an ever-growing VOD manifest) and
//! comfortably covers the few-segment buffer a real player keeps before
//! playback.
//!
//! # Path convention (for the future N1 nginx proxy)
//!
//! `GET /<camera name>/playlist.m3u8`, `GET /<camera name>/init.mp4`, and
//! `GET /<camera name>/segment-<sequence>.m4s` -- mirroring `ws_repackager`'s
//! own `/<camera name>` path convention. N1 (a later item, not this one) is
//! expected to proxy nginx's own `/live/hls/` location here with that prefix
//! stripped (`proxy_pass http://<this listener>/;`, nginx's own standard
//! trailing-slash prefix-stripping convention), so this listener never needs
//! to know about the `/live/hls/` prefix itself.
//!
//! # Named simplifications
//!
//! This is a plain HLS packager, not a true low-latency HLS one: no partial
//! segments, no `EXT-X-PART`/preload hints, no blocking playlist reload. A
//! real player (or `ffprobe`) already handles a standard segmented `m3u8`
//! playlist correctly, which is what the "expanded-view fallback" use case
//! (DP-4/OPEN-2) actually needs -- true LL-HLS's own lower-latency machinery
//! is out of this item's scope. Every HTTP response also closes the
//! connection (`Connection: close`) rather than keeping it alive for a
//! following request; a player's own periodic playlist/segment polling
//! simply opens a new connection each time, at the cost of one extra TCP
//! handshake per request this minimal server does not attempt to avoid.

use crate::fmp4::{Fragment, InitSegmentTracker, Fragmenter};
use crate::rtp_clock::VIDEO_CLOCK_RATE_HZ;
use crate::supervise::log_event;
use bytes::{Bytes, BytesMut};
use corvette_rtsp_client::depacketize::Frame as ClientFrame;
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

/// See this module's own top-level doc, "Segment duration".
const TARGET_SEGMENT_DURATION_TICKS: u32 = VIDEO_CLOCK_RATE_HZ * 2;

/// See this module's own top-level doc, "Playlist window".
const PLAYLIST_WINDOW_SEGMENT_COUNT: usize = 6;

/// The largest a request's own header block is ever allowed to grow while
/// this server is still waiting for a complete one -- defensive only, so one
/// misbehaving connection can't grow this server's own read buffer without
/// bound. Every real request this item's own contract expects (a bare `GET`
/// for one of three fixed path shapes) fits in a small fraction of this.
const MAX_REQUEST_HEADER_BYTES: usize = 8192;

/// Bounds the total time [`handle_connection`] will wait on one accepted
/// connection, covering both reading a complete request and writing its
/// response. Without this, a peer that opens a connection and then never
/// finishes sending a request -- nginx's own reverse-proxy connection to this
/// server, under real conditions, occasionally does exactly this -- leaves
/// `read_request_line_and_path`'s read loop parked on `stream.read().await`
/// forever: nothing in that loop, or anywhere else in this function, ever
/// gives up on its own. Each such connection leaks one Tokio task and one
/// file descriptor for the rest of this process's life, accumulating
/// unboundedly under real, sustained traffic (confirmed directly against a
/// live deployment: roughly seventy such connections stuck in `ESTABLISHED`,
/// never freed, after less than an hour of ordinary use -- see
/// `.agents/issue-12/evidence/V1-ui-bundle-staleness-incident.md`).
/// Five seconds is generous for a LAN-local reverse-proxy hop -- nginx is
/// this server's only real client, per this module's own top-level doc,
/// "Path convention" -- since every real request this server serves resolves
/// in microseconds once its bytes have arrived (`route`'s own work is a
/// handful of in-memory `Mutex`-guarded lookups, no I/O).
const CONNECTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Groups a per-camera [`Fragment`] stream into CMAF media segments, cutting
/// only on a keyframe once the in-progress segment has run at least
/// [`TARGET_SEGMENT_DURATION_TICKS`] -- see this module's own top-level doc,
/// "Segment duration", for why a keyframe-aligned cut matters and why the
/// very first segment is the one exception.
///
/// Pure and synchronous, matching `crate::fmp4::Fragmenter`'s own shape: no
/// I/O, no task, no shared state. [`run_hls_segment`] is what drives one of
/// these per camera and publishes its finished segments.
///
/// Deliberately does not assign a sequence number to what it produces --
/// see [`FinishedSegment`]'s own doc for why that has to live in
/// [`CameraHls`] instead.
#[derive(Debug, Default)]
struct SegmentBuilder {
    current: BytesMut,
    duration_ticks: u64,
}

/// One finished CMAF media segment, before it has been assigned the sequence
/// number that makes it externally addressable (`CameraHls::push_segment`'s
/// own job).
///
/// A fresh [`SegmentBuilder`] is constructed on every [`run_hls_segment`]
/// invocation -- including a restart after a supervised panic (INV-5(b)) --
/// so a sequence counter kept on `SegmentBuilder` itself would restart from
/// zero after every restart, while [`CameraHls`]'s own segment window (keyed
/// by sequence number, and surviving a restart of the task that feeds it)
/// would not: a second, unrelated segment could then collide with an
/// already-published "segment-0.m4s" URL still sitting in the live window.
/// Keeping sequence assignment in `CameraHls` -- the one thing that actually
/// outlives a restart -- avoids that collision entirely.
#[derive(Debug, Clone)]
struct FinishedSegment {
    duration_ticks: u64,
    bytes: Bytes,
}

/// One finished CMAF media segment, now assigned the sequence number that
/// makes it externally addressable: everything a playlist entry and a
/// segment `GET` response both need.
#[derive(Debug, Clone)]
struct StoredSegment {
    sequence: u64,
    duration_ticks: u64,
    bytes: Bytes,
}

impl SegmentBuilder {
    /// Feeds one fragment in, returning a finished segment exactly when this
    /// fragment's own arrival closes out the previous one (`None` while a
    /// segment is still accumulating).
    fn push(&mut self, fragment: &Fragment) -> Option<FinishedSegment> {
        let should_cut = !self.current.is_empty()
            && fragment.is_keyframe
            && self.duration_ticks >= u64::from(TARGET_SEGMENT_DURATION_TICKS);
        let finished = should_cut.then(|| self.cut());

        self.current.extend_from_slice(&fragment.bytes);
        self.duration_ticks += u64::from(fragment.duration_ticks);
        finished
    }

    /// Takes the in-progress segment's bytes and duration, resetting both for
    /// the next one.
    fn cut(&mut self) -> FinishedSegment {
        FinishedSegment {
            duration_ticks: std::mem::take(&mut self.duration_ticks),
            bytes: std::mem::take(&mut self.current).freeze(),
        }
    }
}

/// One camera's HLS state as an HTTP request sees it: the current
/// initialization segment (`None` until this camera's own segmenting task
/// has resolved a parameter set), a bounded live window of the most recently
/// finished media segments, and the next sequence number to assign (see
/// [`FinishedSegment`]'s own doc for why this lives here rather than on the
/// per-invocation [`SegmentBuilder`]).
///
/// A plain [`Mutex`], not `crate::ws_repackager`'s `watch`/`broadcast` pair:
/// an HTTP client pulls a snapshot on its own schedule rather than being
/// pushed updates over a held-open connection, so there is no
/// "no receivers yet" race to solve here (contrast `ws_repackager`'s own
/// `watch::Sender::send_replace` doc) -- every write just replaces the
/// current snapshot outright.
#[derive(Debug, Default)]
struct CameraHlsState {
    init: Option<Bytes>,
    segments: VecDeque<StoredSegment>,
    next_sequence: u64,
}

#[derive(Debug, Default)]
struct CameraHls {
    state: Mutex<CameraHlsState>,
}

impl CameraHls {
    fn set_init(&self, segment: Bytes) {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).init = Some(segment);
    }

    /// Assigns `finished` the next sequence number this camera has ever
    /// handed out (monotonic for the process's own lifetime, regardless of
    /// how many times this camera's segmenting task has restarted -- see
    /// [`FinishedSegment`]'s own doc) and stores it, evicting the oldest
    /// segment once the live window is full.
    fn push_segment(&self, finished: FinishedSegment) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sequence = state.next_sequence;
        state.next_sequence += 1;
        state.segments.push_back(StoredSegment {
            sequence,
            duration_ticks: finished.duration_ticks,
            bytes: finished.bytes,
        });
        while state.segments.len() > PLAYLIST_WINDOW_SEGMENT_COUNT {
            state.segments.pop_front();
        }
        drop(state);
    }

    fn init_bytes(&self) -> Option<Bytes> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .init
            .clone()
    }

    fn segment_bytes(&self, sequence: u64) -> Option<Bytes> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .segments
            .iter()
            .find(|segment| segment.sequence == sequence)
            .map(|segment| segment.bytes.clone())
    }

    /// Builds this camera's current media playlist.
    ///
    /// Always a structurally valid live playlist, even before this camera
    /// has resolved any parameter set: a real camera's own init-segment
    /// resolution can lag its first RTSP frames by an observable amount (see
    /// `crate::fmp4::InitSegmentTracker`'s own doc), and a player -- or, as
    /// this item's own browser-based Verify step found directly, `hls.js`
    /// itself -- that requests the playlist during that window needs a
    /// genuine "nothing yet, keep polling" live document, not a 404. A 404
    /// stays reserved for "no such camera at all" (`route`'s own dispatch,
    /// one level up) -- a registered camera's playlist always exists, even
    /// when it is currently empty. The one thing gated on init resolution is
    /// the `#EXT-X-MAP` line itself: emitting it before any initialization
    /// segment exists would point a player at a segment resource this server
    /// cannot yet serve.
    fn playlist_text(&self) -> String {
        // Collects everything needed from the locked state up front (plain
        // integers plus each segment's own sequence/duration, not the
        // segment bytes themselves) so the lock is held only for that
        // snapshot, not while the playlist text below is built.
        let (has_init, target_duration_secs, media_sequence, segments) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let has_init = state.init.is_some();

            // EXT-X-TARGETDURATION must be an integer at least as large as
            // every EXTINF in the playlist (RFC 8216 section 4.3.3.1), so
            // this is the ceiling of the larger of the configured target and
            // the longest segment actually in the current window -- not
            // simply the configured target -- since a sparse-keyframe stream
            // can produce an individual segment longer than
            // [`TARGET_SEGMENT_DURATION_TICKS`].
            let longest_ticks = state
                .segments
                .iter()
                .map(|segment| segment.duration_ticks)
                .max()
                .unwrap_or(0)
                .max(u64::from(TARGET_SEGMENT_DURATION_TICKS));
            let target_duration_secs = longest_ticks.div_ceil(u64::from(VIDEO_CLOCK_RATE_HZ));
            let media_sequence = state.segments.front().map_or(0, |segment| segment.sequence);
            let segments: Vec<(u64, u64)> = state
                .segments
                .iter()
                .map(|segment| (segment.sequence, segment.duration_ticks))
                .collect();
            drop(state);
            (has_init, target_duration_secs, media_sequence, segments)
        };

        let mut playlist = format!(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{target_duration_secs}\n#EXT-X-MEDIA-SEQUENCE:{media_sequence}\n"
        );
        if has_init {
            playlist.push_str("#EXT-X-MAP:URI=\"init.mp4\"\n");
        }
        for (sequence, duration_ticks) in segments {
            // Millisecond-resolution EXTINF via integer arithmetic (no
            // float): real-world tick counts here are always well under
            // `u64::MAX / 1000`, so this can't overflow.
            let millis = duration_ticks * 1000 / u64::from(VIDEO_CLOCK_RATE_HZ);
            let _ = writeln!(
                playlist,
                "#EXTINF:{}.{:03},\nsegment-{sequence}.m4s",
                millis / 1000,
                millis % 1000
            );
        }
        playlist
    }
}

/// Dispatches by camera name, matching `crate::ws_repackager::
/// MultiCameraFmp4Store`'s own shape.
///
/// The camera set is fixed at construction (every configured camera is
/// [`register`](Self::register)ed before any task that reads or serves it is
/// spawned), so `&self` methods need no synchronization for the map itself.
#[derive(Default)]
pub struct MultiCameraHlsStore {
    cameras: HashMap<String, Arc<CameraHls>>,
}

impl std::fmt::Debug for MultiCameraHlsStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiCameraHlsStore")
            .field("cameras", &self.cameras.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl MultiCameraHlsStore {
    /// Registers `name`, so a later [`run_hls_segment`] call and an inbound
    /// HTTP request can both find its shared state.
    ///
    /// # Panics
    ///
    /// Panics if `name` is already registered -- this item's own camera set
    /// is fixed at startup, matching `MultiCameraFmp4Store::register`'s own
    /// contract.
    pub fn register(&mut self, name: &str) {
        let previous = self
            .cameras
            .insert(name.to_string(), Arc::new(CameraHls::default()));
        assert!(previous.is_none(), "camera {name:?} registered twice");
    }

    fn camera(&self, name: &str) -> Option<Arc<CameraHls>> {
        self.cameras.get(name).cloned()
    }
}

/// Runs one camera's HLS-segmenting role for as long as `client_frames` keeps
/// producing.
///
/// Never returns on its own (except when the upstream camera's own `Client`
/// is dropped, matching `ws_repackager::run_fmp4_repackage`'s own shutdown
/// convention) -- intended to be spawned via
/// `crate::supervise::spawn_supervised_with`, matching DP-4/INV-5.
///
/// # Panics
///
/// Panics if `name` was never [`MultiCameraHlsStore::register`]ed -- a caller
/// bug, matching `run_fmp4_repackage`'s own contract.
pub async fn run_hls_segment(
    name: String,
    mut client_frames: broadcast::Receiver<ClientFrame>,
    store: Arc<MultiCameraHlsStore>,
) {
    let camera = store
        .camera(&name)
        .unwrap_or_else(|| panic!("camera {name:?} was never registered"));
    let mut init_tracker = InitSegmentTracker::default();
    let mut fragmenter = Fragmenter::default();
    let mut builder = SegmentBuilder::default();

    loop {
        match client_frames.recv().await {
            Ok(frame) => {
                if let Some(segment) = init_tracker.observe(&frame) {
                    camera.set_init(segment);
                }
                if let Some(fragment) = fragmenter.next(&frame)
                    && let Some(finished) = builder.push(&fragment)
                {
                    camera.push_segment(finished);
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => {
                log_event(&name, "hls-segment-disconnect", "camera client dropped");
                return;
            }
        }
    }
}

/// The whole-process HTTP listener (Do step 3, D-3's "configuration point"
/// convention for its bind address).
#[derive(Debug)]
pub struct HlsServer {
    listener: TcpListener,
}

impl HlsServer {
    /// Binds `addr`. Does not yet accept any connection -- see [`Self::serve`].
    ///
    /// # Errors
    ///
    /// Returns an error if binding fails.
    pub async fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(addr).await?,
        })
    }

    /// The address this server actually bound to -- useful when constructed
    /// with an ephemeral port (`:0`), as this item's own tests do.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying socket has no local address (never
    /// expected for a bound, non-closed listener).
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Accepts connections forever, dispatching each to the camera and
    /// resource its request path names. Never returns on success, matching
    /// `rtsp_restream::RtspServer::serve`'s own shape.
    pub async fn serve(self, store: Arc<MultiCameraHlsStore>) -> ! {
        loop {
            match self.listener.accept().await {
                Ok((stream, peer_addr)) => {
                    let store = Arc::clone(&store);
                    tokio::spawn(async move {
                        handle_connection(stream, &store).await;
                    });
                    let _ = peer_addr; // identifies the connection in a real log line only
                }
                Err(err) => {
                    log_event("hls-listener", "accept-error", err);
                }
            }
        }
    }
}

/// The one resource shape a request path can name, per this module's own
/// top-level doc, "Path convention".
enum Resource {
    Playlist,
    Init,
    Segment(u64),
}

/// Parses a request path's resource component (the part after `/<camera
/// name>/`), or `None` if it names none of the three shapes this server
/// understands.
fn parse_resource(resource: &str) -> Option<Resource> {
    match resource {
        "playlist.m3u8" => Some(Resource::Playlist),
        "init.mp4" => Some(Resource::Init),
        other => other
            .strip_prefix("segment-")
            .and_then(|rest| rest.strip_suffix(".m4s"))
            .and_then(|sequence| sequence.parse().ok())
            .map(Resource::Segment),
    }
}

/// Splits a request path (its query string, if any, already stripped) into
/// `(camera name, resource)`, or `None` if it does not have exactly that
/// shape.
fn parse_path(path: &str) -> Option<(&str, &str)> {
    path.strip_prefix('/')?.split_once('/')
}

/// Handles one inbound HTTP connection end to end: reads a single request,
/// resolves and sends its response, then closes -- see this module's own
/// top-level doc, "Named simplifications", for why this server does not keep
/// a connection alive across more than one request.
async fn handle_connection(stream: TcpStream, store: &MultiCameraHlsStore) {
    handle_connection_with_timeout(stream, store, CONNECTION_TIMEOUT).await;
}

/// Which stage of one connection's lifecycle failed -- kept distinct so a
/// timeout firing during the read half is still logged as a timeout, not
/// folded into a generic "something went wrong" label that would lose the
/// same read-vs-write distinction the two error branches below already give
/// a real log reader.
enum ConnectionOutcome {
    Read(std::io::Error),
    Write(std::io::Error),
}

/// [`handle_connection`]'s real body, taking its own timeout as a parameter
/// so a test can exercise the give-up-on-a-stalled-peer path without waiting
/// out this server's real, production-sized [`CONNECTION_TIMEOUT`].
async fn handle_connection_with_timeout(
    mut stream: TcpStream,
    store: &MultiCameraHlsStore,
    timeout: std::time::Duration,
) {
    let outcome = tokio::time::timeout(timeout, async {
        let request = read_request_line_and_path(&mut stream)
            .await
            .map_err(ConnectionOutcome::Read)?;
        let response = route(&request, store);
        send_response(&mut stream, response)
            .await
            .map_err(ConnectionOutcome::Write)
    })
    .await;

    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(ConnectionOutcome::Read(err))) => {
            log_event("hls-listener", "request-read-error", err);
        }
        Ok(Err(ConnectionOutcome::Write(err))) => {
            log_event("hls-listener", "response-write-error", err);
        }
        Err(_elapsed) => {
            // `stream` drops here (closing the socket) once this function
            // returns -- see this function's own doc for why leaving it open
            // any longer, waiting on a peer that has already missed its
            // budget, is exactly the leak this timeout exists to prevent.
            log_event(
                "hls-listener",
                "connection-timeout",
                format!(
                    "peer did not complete a request within {timeout:?} -- \
                     dropped to avoid leaking this connection forever"
                ),
            );
        }
    }
}

/// Reads bytes off `stream` until `httparse` can parse a complete request
/// header block, then returns the request's own path (query string
/// stripped).
///
/// `httparse` (already this crate's own transitive dependency via
/// `tokio-tungstenite`'s WebSocket handshake -- see this crate's own
/// `Cargo.toml`) parses the one genuinely error-prone part of HTTP/1.x
/// (the request line and header framing) with a well-tested implementation,
/// the same "don't hand-roll a well-tested protocol parser" precedent
/// `ws_repackager`'s own `Cargo.toml` doc gives for depending on
/// `tokio-tungstenite` itself. This server needs nothing else HTTP/1.x
/// offers (no header values, no body, no keep-alive), so nothing past the
/// path is read or interpreted.
async fn read_request_line_and_path(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut request = httparse::Request::new(&mut headers);
        if request.parse(&buffer).is_ok_and(|status| status.is_complete()) {
            let path = request.path.unwrap_or("/");
            let path = path.split('?').next().unwrap_or(path);
            return Ok(path.to_string());
        }
        if buffer.len() >= MAX_REQUEST_HEADER_BYTES {
            return Err(std::io::Error::other(
                "request header block exceeded this server's own size limit",
            ));
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(std::io::Error::other(
                "connection closed before a complete request header block arrived",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

struct Response {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    fn not_found(reason: &str) -> Self {
        Self {
            status: "404 Not Found",
            content_type: "text/plain; charset=utf-8",
            body: reason.as_bytes().to_vec(),
        }
    }
}

/// Resolves one request path against `store`, per this module's own
/// top-level doc, "Path convention".
fn route(path: &str, store: &MultiCameraHlsStore) -> Response {
    let Some((camera_name, resource)) = parse_path(path) else {
        return Response::not_found("expected /<camera name>/<resource>");
    };
    let Some(camera) = store.camera(camera_name) else {
        return Response::not_found("no such camera is configured");
    };
    let Some(resource) = parse_resource(resource) else {
        return Response::not_found(
            "expected playlist.m3u8, init.mp4, or segment-<sequence>.m4s",
        );
    };

    match resource {
        // Always 200: see `CameraHls::playlist_text`'s own doc for why a
        // registered camera's playlist is never a 404, even before it has
        // resolved a parameter set.
        Resource::Playlist => Response {
            status: "200 OK",
            content_type: "application/vnd.apple.mpegurl",
            body: camera.playlist_text().into_bytes(),
        },
        Resource::Init => camera.init_bytes().map_or_else(
            || Response::not_found("this camera has not resolved a parameter set yet"),
            |segment| Response {
                status: "200 OK",
                content_type: "video/mp4",
                body: segment.to_vec(),
            },
        ),
        Resource::Segment(sequence) => camera.segment_bytes(sequence).map_or_else(
            || Response::not_found("no such segment (already rolled out of the live window, or never produced)"),
            |segment| Response {
                status: "200 OK",
                content_type: "video/mp4",
                body: segment.to_vec(),
            },
        ),
    }
}

/// Writes `response` as a complete HTTP/1.1 response and closes the
/// connection (`Connection: close`, per this module's own top-level doc).
///
/// `Access-Control-Allow-Origin: *` is set unconditionally: this listener
/// serves no per-viewer state and no credentialed request, so an
/// unrestricted origin costs nothing and lets a browser-hosted player fetch
/// this server's own playlist/segments from a different origin (this item's
/// own browser-based Verify step needs exactly this; the future N1 nginx
/// proxy makes every real deployment request same-origin anyway, so this
/// header is inert there).
async fn send_response(stream: &mut TcpStream, response: Response) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&response.body).await?;
    stream.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvette_rtsp_client::depacketize::Codec as ClientCodec;

    // The exact fixture bytes `fmp4::tests` and this crate's own G1/G2
    // integration tests use -- reused rather than invented.
    const SPS: &[u8] = &[
        0x67, 0x42, 0xc0, 0x1f, 0xda, 0x01, 0x40, 0x16, 0xe9, 0xb8, 0x08, 0x08, 0x0a, 0x00, 0x00,
        0x07, 0xd0, 0x00, 0x01, 0xd4, 0xc0, 0x80,
    ];
    const PPS: &[u8] = &[0x68, 0xce, 0x3c, 0x80];
    const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x21];
    const NON_IDR: &[u8] = &[0x41, 0x9a, 0x24, 0x6c];

    fn annex_b(nal: &[u8]) -> Bytes {
        let mut payload = Vec::with_capacity(nal.len() + 4);
        payload.extend_from_slice(&[0, 0, 0, 1]);
        payload.extend_from_slice(nal);
        Bytes::from(payload)
    }

    fn frame(timestamp: u32, nal: &[u8]) -> ClientFrame {
        ClientFrame {
            codec: ClientCodec::H264,
            timestamp,
            payload: annex_b(nal),
        }
    }

    #[test]
    fn parse_resource_recognizes_all_three_named_shapes() {
        assert!(matches!(
            parse_resource("playlist.m3u8"),
            Some(Resource::Playlist)
        ));
        assert!(matches!(parse_resource("init.mp4"), Some(Resource::Init)));
        assert!(matches!(
            parse_resource("segment-42.m4s"),
            Some(Resource::Segment(42))
        ));
        assert!(parse_resource("unknown.txt").is_none());
        assert!(parse_resource("segment-notanumber.m4s").is_none());
    }

    #[test]
    fn parse_path_splits_camera_and_resource() {
        assert_eq!(
            parse_path("/front_door/playlist.m3u8"),
            Some(("front_door", "playlist.m3u8"))
        );
        assert_eq!(parse_path("/front_door"), None);
        assert_eq!(parse_path("/"), None);
    }

    /// Sends one throwaway keyframe through `fragmenter` at t=0 so every
    /// later `next()` call in a test measures a real timestamp delta rather
    /// than `Fragmenter`'s own `DEFAULT_FIRST_SAMPLE_DURATION_TICKS`
    /// approximation for its very first sample ever (see `Fragmenter`'s own
    /// doc) -- keeps this test module's own duration arithmetic exact.
    fn prime(fragmenter: &mut Fragmenter) {
        fragmenter.next(&frame(0, IDR));
    }

    #[test]
    fn segment_builder_does_not_cut_before_the_target_duration_is_reached() {
        let mut fragmenter = Fragmenter::default();
        let mut builder = SegmentBuilder::default();
        prime(&mut fragmenter);

        // 90_000 ticks/frame is 1 second at the 90 kHz video clock -- two
        // keyframes one second apart is under TARGET_SEGMENT_DURATION_TICKS
        // (2 seconds), so neither should cut a segment yet.
        let first = fragmenter.next(&frame(90_000, IDR)).unwrap();
        assert!(builder.push(&first).is_none());
        let second = fragmenter.next(&frame(2 * 90_000, IDR)).unwrap();
        assert!(
            builder.push(&second).is_none(),
            "one second of accumulated duration must not yet reach the 2-second target"
        );
    }

    #[test]
    fn segment_builder_cuts_only_on_a_keyframe_once_the_target_duration_is_reached() {
        let mut fragmenter = Fragmenter::default();
        let mut builder = SegmentBuilder::default();
        prime(&mut fragmenter);

        let first = fragmenter.next(&frame(90_000, IDR)).unwrap();
        assert!(builder.push(&first).is_none());

        // 3 seconds later (past the 2-second target), but a non-keyframe --
        // must not cut here.
        let non_keyframe = fragmenter.next(&frame(4 * 90_000, NON_IDR)).unwrap();
        assert!(
            builder.push(&non_keyframe).is_none(),
            "must not cut mid-GOP even once the target duration has passed"
        );

        // The next keyframe, arbitrarily later, closes out the first segment.
        let keyframe = fragmenter.next(&frame(5 * 90_000, IDR)).unwrap();
        let finished = builder
            .push(&keyframe)
            .expect("a keyframe past the target duration must cut the segment");
        assert_eq!(finished.duration_ticks, 4 * 90_000);
    }

    #[test]
    fn camera_hls_playlist_is_valid_but_carries_no_ext_x_map_until_init_resolves() {
        let camera = CameraHls::default();
        let playlist = camera.playlist_text();
        assert!(
            playlist.starts_with("#EXTM3U\n"),
            "a registered camera's playlist is always structurally valid, even before it has \
             resolved a parameter set (a real player -- or hls.js itself, as this item's own \
             browser-based Verify step found directly -- must never see a 404 here): {playlist}"
        );
        assert!(
            !playlist.contains("#EXT-X-MAP"),
            "no EXT-X-MAP before an initialization segment actually exists to point at: {playlist}"
        );
    }

    #[test]
    fn camera_hls_playlist_lists_every_segment_currently_in_the_window() {
        let camera = CameraHls::default();
        camera.set_init(Bytes::from_static(b"fake-init"));
        camera.push_segment(FinishedSegment {
            duration_ticks: 2 * u64::from(VIDEO_CLOCK_RATE_HZ),
            bytes: Bytes::from_static(b"seg0"),
        });
        camera.push_segment(FinishedSegment {
            duration_ticks: 2 * u64::from(VIDEO_CLOCK_RATE_HZ),
            bytes: Bytes::from_static(b"seg1"),
        });

        let playlist = camera.playlist_text();
        assert!(playlist.starts_with("#EXTM3U\n"));
        assert!(playlist.contains("#EXT-X-MAP:URI=\"init.mp4\"\n"));
        assert!(playlist.contains("#EXT-X-MEDIA-SEQUENCE:0\n"));
        assert!(playlist.contains("segment-0.m4s"));
        assert!(playlist.contains("segment-1.m4s"));
    }

    #[test]
    fn camera_hls_playlist_drops_the_oldest_segment_past_the_window() {
        let camera = CameraHls::default();
        camera.set_init(Bytes::from_static(b"fake-init"));
        for _ in 0..(PLAYLIST_WINDOW_SEGMENT_COUNT + 2) {
            camera.push_segment(FinishedSegment {
                duration_ticks: 2 * u64::from(VIDEO_CLOCK_RATE_HZ),
                bytes: Bytes::from_static(b"seg"),
            });
        }

        let playlist = camera.playlist_text();
        assert!(
            !playlist.contains("segment-0.m4s"),
            "the oldest segment must have rolled out of the live window"
        );
        assert!(playlist.contains("#EXT-X-MEDIA-SEQUENCE:2\n"));
        assert!(camera.segment_bytes(0).is_none());
        assert!(camera.segment_bytes(2).is_some());
    }

    #[test]
    fn camera_hls_playlist_target_duration_covers_a_longer_than_configured_segment() {
        let camera = CameraHls::default();
        camera.set_init(Bytes::from_static(b"fake-init"));
        camera.push_segment(FinishedSegment {
            duration_ticks: 5 * u64::from(VIDEO_CLOCK_RATE_HZ), // 5s, past the 2s target
            bytes: Bytes::from_static(b"seg0"),
        });

        let playlist = camera.playlist_text();
        assert!(
            playlist.contains("#EXT-X-TARGETDURATION:5\n"),
            "EXT-X-TARGETDURATION must cover the longest actual segment, not only the configured target: {playlist}"
        );
    }

    /// The real defect the INV-5(b) mutation cycle (`.agents/issue-12/
    /// evidence/G3-mutation.log`) surfaced during this item's own
    /// development: a naive per-invocation sequence counter on
    /// `SegmentBuilder` would restart at 0 after every supervised restart,
    /// colliding with an already-published, still-in-window "segment-0.m4s"
    /// from before the restart. `CameraHls` -- the one thing that actually
    /// survives a restart -- owns sequence assignment instead, so it never
    /// repeats regardless of how many times `push_segment` is called across
    /// however many separate `SegmentBuilder` instances.
    #[test]
    fn camera_hls_sequence_numbers_never_repeat_across_a_simulated_restart() {
        let camera = CameraHls::default();
        camera.set_init(Bytes::from_static(b"fake-init"));

        // First "task instance": a fresh `SegmentBuilder`/`Fragmenter` pair,
        // exactly as `run_hls_segment` constructs on entry, cuts one segment.
        // The first fragment's own duration (200_000 ticks, past the
        // 180_000-tick target) is enough on its own, so a second keyframe
        // immediately cuts it -- no need for a third fragment the way
        // `segment_builder_cuts_only_on_a_keyframe_...` above demonstrates
        // separately (a non-keyframe must not cut mid-GOP).
        let mut fragmenter = Fragmenter::default();
        let mut builder = SegmentBuilder::default();
        prime(&mut fragmenter);
        let first = fragmenter.next(&frame(200_000, IDR)).unwrap();
        assert!(builder.push(&first).is_none());
        let cutting = fragmenter.next(&frame(300_000, IDR)).unwrap();
        let finished = builder
            .push(&cutting)
            .expect("the first fragment's own duration already passed the 2-second target");
        camera.push_segment(finished);

        // A brand-new `SegmentBuilder`/`Fragmenter` pair -- as
        // `run_hls_segment` constructs on a fresh invocation after a
        // supervised restart -- would, without this fix, start cutting from
        // sequence 0 again.
        let mut restarted_fragmenter = Fragmenter::default();
        let mut restarted_builder = SegmentBuilder::default();
        prime(&mut restarted_fragmenter);
        let first_after_restart = restarted_fragmenter.next(&frame(200_000, IDR)).unwrap();
        assert!(restarted_builder.push(&first_after_restart).is_none());
        let cutting_after_restart = restarted_fragmenter.next(&frame(300_000, IDR)).unwrap();
        let finished_after_restart = restarted_builder
            .push(&cutting_after_restart)
            .expect("the first fragment's own duration already passed the 2-second target");
        camera.push_segment(finished_after_restart);

        let playlist = camera.playlist_text();
        assert!(
            playlist.contains("segment-0.m4s") && playlist.contains("segment-1.m4s"),
            "one segment from each of two separate SegmentBuilder instances must get two \
             distinct, never-repeating sequence numbers: {playlist}"
        );
        assert!(
            camera.segment_bytes(0).is_some() && camera.segment_bytes(1).is_some(),
            "the post-restart segment must be reachable at its own, unique sequence number \
             rather than colliding with the pre-restart one"
        );
    }

    /// End-to-end through `run_hls_segment` itself (not just `SegmentBuilder`/
    /// `CameraHls` in isolation): a synthetic Annex-B H.264 sequence -- SPS,
    /// PPS, then a run of IDR/non-IDR access units spanning past the target
    /// segment duration -- produces a resolved init segment, a playlist that
    /// names at least one real segment, and a fetchable segment whose bytes
    /// actually start with a `moof` box. This is this item's own Verify
    /// step's "synthetic Annex-B sequence produces a valid m3u8 playlist and
    /// a sequence of CMAF segments" requirement exercised through the real
    /// per-camera worker function, not only its constituent pure pieces.
    #[tokio::test]
    async fn run_hls_segment_produces_a_resolved_playlist_and_fetchable_segments() {
        let mut store = MultiCameraHlsStore::default();
        store.register("cam");
        let store = Arc::new(store);

        let (sender, receiver) = broadcast::channel(64);
        let worker = tokio::spawn(run_hls_segment("cam".to_string(), receiver, store.clone()));

        sender.send(frame(0, SPS)).unwrap();
        sender.send(frame(0, PPS)).unwrap();
        // The first sample's own duration is a fixed default (see
        // `Fragmenter`'s own doc), then two more access units carry the
        // accumulated duration past the 2-second (180_000-tick) target
        // before the run ends on a keyframe, cutting segment 0.
        sender.send(frame(0, IDR)).unwrap();
        sender.send(frame(180_000, NON_IDR)).unwrap();
        sender.send(frame(270_000, IDR)).unwrap();

        // `run_hls_segment` processes its channel as fast as it's fed (no
        // internal delay), so a bounded poll for the playlist to resolve
        // avoids a fixed sleep racing against scheduling.
        let camera = store.camera("cam").expect("registered above");
        let playlist = wait_for_playlist(&camera).await;

        assert!(playlist.contains("#EXT-X-MAP:URI=\"init.mp4\"\n"));
        assert!(
            playlist.contains("segment-0.m4s"),
            "a full 2+ second run ending on a keyframe must have cut segment 0: {playlist}"
        );
        assert!(camera.init_bytes().is_some());
        let segment = camera
            .segment_bytes(0)
            .expect("segment 0 named in the playlist must be fetchable");
        assert_eq!(&segment[4..8], b"moof");

        worker.abort();
    }

    async fn wait_for_playlist(camera: &CameraHls) -> String {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let playlist = camera.playlist_text();
                if playlist.contains(".m4s") {
                    return playlist;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("a playlist with at least one segment resolves before the test timeout")
    }

    /// The real defect a live deployment surfaced directly (see
    /// [`CONNECTION_TIMEOUT`]'s own doc): a peer that opens a connection and
    /// then never finishes sending a request left `handle_connection` parked
    /// on `stream.read().await` forever, one leaked task and file descriptor
    /// per occurrence, for as long as the process ran. Uses
    /// `handle_connection_with_timeout` directly with a short timeout so
    /// this test does not have to wait out the real, production-sized
    /// [`CONNECTION_TIMEOUT`]; the outer timeout below is only a test-hang
    /// guard in case a future change reintroduces the unbounded wait.
    #[tokio::test]
    async fn handle_connection_gives_up_on_a_peer_that_never_sends_a_complete_request() {
        let store = MultiCameraHlsStore::default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // Kept alive (not dropped) until after the call below: a peer that
        // has already disconnected would make `stream.read()` return a
        // normal "connection closed" error immediately, which is a different
        // code path (`ConnectionOutcome::Read`, already covered by every
        // other test's happy-path request/response round trip) than the one
        // this test means to exercise -- a peer that is still connected but
        // silent.
        let client = TcpStream::connect(addr).await.unwrap();
        let (server_stream, _) = listener.accept().await.unwrap();

        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            handle_connection_with_timeout(
                server_stream,
                &store,
                std::time::Duration::from_millis(50),
            ),
        )
        .await
        .expect(
            "a peer that opens a connection and never sends a complete request must eventually \
             be dropped, not leak this task and its socket forever",
        );

        drop(client);
    }
}
