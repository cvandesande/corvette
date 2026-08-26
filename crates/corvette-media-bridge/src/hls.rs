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

/// Groups a per-camera [`Fragment`] stream into CMAF media segments, cutting
/// only on a keyframe once the in-progress segment has run at least
/// [`TARGET_SEGMENT_DURATION_TICKS`] -- see this module's own top-level doc,
/// "Segment duration", for why a keyframe-aligned cut matters and why the
/// very first segment is the one exception.
///
/// Pure and synchronous, matching `crate::fmp4::Fragmenter`'s own shape: no
/// I/O, no task, no shared state. [`run_hls_segment`] is what drives one of
/// these per camera and publishes its finished segments.
#[derive(Debug, Default)]
struct SegmentBuilder {
    current: BytesMut,
    duration_ticks: u64,
    next_sequence: u64,
}

/// One finished CMAF media segment: everything a playlist entry and a
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
    fn push(&mut self, fragment: Fragment) -> Option<StoredSegment> {
        let should_cut = !self.current.is_empty()
            && fragment.is_keyframe
            && self.duration_ticks >= u64::from(TARGET_SEGMENT_DURATION_TICKS);
        let finished = should_cut.then(|| self.cut());

        self.current.extend_from_slice(&fragment.bytes);
        self.duration_ticks += u64::from(fragment.duration_ticks);
        finished
    }

    /// Takes the in-progress segment's bytes and duration, resetting both for
    /// the next one, and assigns it the next sequence number.
    fn cut(&mut self) -> StoredSegment {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        StoredSegment {
            sequence,
            duration_ticks: std::mem::take(&mut self.duration_ticks),
            bytes: std::mem::take(&mut self.current).freeze(),
        }
    }
}

/// One camera's HLS state as an HTTP request sees it: the current
/// initialization segment (`None` until this camera's own segmenting task
/// has resolved a parameter set) and a bounded live window of the most
/// recently finished media segments.
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
}

#[derive(Debug, Default)]
struct CameraHls {
    state: Mutex<CameraHlsState>,
}

impl CameraHls {
    fn set_init(&self, segment: Bytes) {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).init = Some(segment);
    }

    fn push_segment(&self, segment: StoredSegment) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.segments.push_back(segment);
        while state.segments.len() > PLAYLIST_WINDOW_SEGMENT_COUNT {
            state.segments.pop_front();
        }
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

    /// Builds this camera's current media playlist, or `None` if no
    /// initialization segment has resolved yet (nothing a player could do
    /// anything useful with -- matching `ws_repackager`'s own "no such
    /// camera"/"not yet resolved" precedent of naming an absence rather than
    /// serving an empty or placeholder body).
    fn playlist_text(&self) -> Option<String> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.init.as_ref()?;

        // EXT-X-TARGETDURATION must be an integer at least as large as every
        // EXTINF in the playlist (RFC 8216 section 4.3.3.1), so this is the
        // ceiling of the larger of the configured target and the longest
        // segment actually in the current window -- not simply the
        // configured target -- since a sparse-keyframe stream can produce an
        // individual segment longer than [`TARGET_SEGMENT_DURATION_TICKS`].
        let longest_ticks = state
            .segments
            .iter()
            .map(|segment| segment.duration_ticks)
            .max()
            .unwrap_or(0)
            .max(u64::from(TARGET_SEGMENT_DURATION_TICKS));
        let target_duration_secs =
            (longest_ticks as f64 / f64::from(VIDEO_CLOCK_RATE_HZ)).ceil() as u64;

        let media_sequence = state.segments.front().map_or(0, |segment| segment.sequence);

        let mut playlist = format!(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{target_duration_secs}\n#EXT-X-MEDIA-SEQUENCE:{media_sequence}\n#EXT-X-MAP:URI=\"init.mp4\"\n"
        );
        for segment in &state.segments {
            let duration_secs = segment.duration_ticks as f64 / f64::from(VIDEO_CLOCK_RATE_HZ);
            playlist.push_str(&format!(
                "#EXTINF:{duration_secs:.3},\nsegment-{}.m4s\n",
                segment.sequence
            ));
        }
        Some(playlist)
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
                    && let Some(finished) = builder.push(fragment)
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
async fn handle_connection(mut stream: TcpStream, store: &MultiCameraHlsStore) {
    let request = match read_request_line_and_path(&mut stream).await {
        Ok(path) => path,
        Err(err) => {
            log_event("hls-listener", "request-read-error", err);
            return;
        }
    };

    let response = route(&request, store);
    if let Err(err) = send_response(&mut stream, response).await {
        log_event("hls-listener", "response-write-error", err);
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
        Resource::Playlist => camera.playlist_text().map_or_else(
            || Response::not_found("this camera has not resolved a parameter set yet"),
            |playlist| Response {
                status: "200 OK",
                content_type: "application/vnd.apple.mpegurl",
                body: playlist.into_bytes(),
            },
        ),
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
        assert!(builder.push(first).is_none());
        let second = fragmenter.next(&frame(2 * 90_000, IDR)).unwrap();
        assert!(
            builder.push(second).is_none(),
            "one second of accumulated duration must not yet reach the 2-second target"
        );
    }

    #[test]
    fn segment_builder_cuts_only_on_a_keyframe_once_the_target_duration_is_reached() {
        let mut fragmenter = Fragmenter::default();
        let mut builder = SegmentBuilder::default();
        prime(&mut fragmenter);

        let first = fragmenter.next(&frame(90_000, IDR)).unwrap();
        assert!(builder.push(first).is_none());

        // 3 seconds later (past the 2-second target), but a non-keyframe --
        // must not cut here.
        let non_keyframe = fragmenter.next(&frame(4 * 90_000, NON_IDR)).unwrap();
        assert!(
            builder.push(non_keyframe).is_none(),
            "must not cut mid-GOP even once the target duration has passed"
        );

        // The next keyframe, arbitrarily later, closes out the first segment.
        let keyframe = fragmenter.next(&frame(5 * 90_000, IDR)).unwrap();
        let finished = builder
            .push(keyframe)
            .expect("a keyframe past the target duration must cut the segment");
        assert_eq!(finished.sequence, 0);
        assert_eq!(finished.duration_ticks, 4 * 90_000);
    }

    #[test]
    fn camera_hls_playlist_is_none_until_an_init_segment_resolves() {
        let camera = CameraHls::default();
        assert!(camera.playlist_text().is_none());
    }

    #[test]
    fn camera_hls_playlist_lists_every_segment_currently_in_the_window() {
        let camera = CameraHls::default();
        camera.set_init(Bytes::from_static(b"fake-init"));
        camera.push_segment(StoredSegment {
            sequence: 0,
            duration_ticks: 2 * VIDEO_CLOCK_RATE_HZ as u64,
            bytes: Bytes::from_static(b"seg0"),
        });
        camera.push_segment(StoredSegment {
            sequence: 1,
            duration_ticks: 2 * VIDEO_CLOCK_RATE_HZ as u64,
            bytes: Bytes::from_static(b"seg1"),
        });

        let playlist = camera.playlist_text().expect("an init segment resolved");
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
        for sequence in 0..(PLAYLIST_WINDOW_SEGMENT_COUNT as u64 + 2) {
            camera.push_segment(StoredSegment {
                sequence,
                duration_ticks: 2 * VIDEO_CLOCK_RATE_HZ as u64,
                bytes: Bytes::from_static(b"seg"),
            });
        }

        let playlist = camera.playlist_text().unwrap();
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
        camera.push_segment(StoredSegment {
            sequence: 0,
            duration_ticks: 5 * VIDEO_CLOCK_RATE_HZ as u64, // 5s, past the 2s target
            bytes: Bytes::from_static(b"seg0"),
        });

        let playlist = camera.playlist_text().unwrap();
        assert!(
            playlist.contains("#EXT-X-TARGETDURATION:5\n"),
            "EXT-X-TARGETDURATION must cover the longest actual segment, not only the configured target: {playlist}"
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
                if let Some(playlist) = camera.playlist_text()
                    && playlist.contains(".m4s")
                {
                    return playlist;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("a playlist with at least one segment resolves before the test timeout")
    }
}
