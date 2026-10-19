//! The expanded-view fallback's HLS transport: a video-only HTTP packager.
//!
//! Design rationale: `docs/design/api-contracts.md`, "The HLS packager serves a
//! sliding-window CMAF playlist"; audio scope: that document's audio section.

use crate::fmp4::{Fragment, Fragmenter, InitSegmentTracker};
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

const TARGET_SEGMENT_DURATION_TICKS: u32 = VIDEO_CLOCK_RATE_HZ * 2;

const PLAYLIST_WINDOW_SEGMENT_COUNT: usize = 6;

const MAX_REQUEST_HEADER_BYTES: usize = 8192;

const CONNECTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Default)]
struct SegmentBuilder {
    current: BytesMut,
    duration_ticks: u64,
}

/// Sequence assignment lives in [`CameraHls`], never in [`SegmentBuilder`]: a
/// fresh builder per supervised restart would otherwise count from 0 again.
#[derive(Debug, Clone)]
struct FinishedSegment {
    duration_ticks: u64,
    bytes: Bytes,
}

#[derive(Debug, Clone)]
struct StoredSegment {
    sequence: u64,
    duration_ticks: u64,
    bytes: Bytes,
}

impl SegmentBuilder {
    fn push(&mut self, fragment: &Fragment) -> Option<FinishedSegment> {
        let should_cut = !self.current.is_empty()
            && fragment.is_keyframe
            && self.duration_ticks >= u64::from(TARGET_SEGMENT_DURATION_TICKS);
        let finished = should_cut.then(|| self.cut());

        self.current.extend_from_slice(&fragment.bytes);
        self.duration_ticks += u64::from(fragment.duration_ticks);
        finished
    }

    fn cut(&mut self) -> FinishedSegment {
        FinishedSegment {
            duration_ticks: std::mem::take(&mut self.duration_ticks),
            bytes: std::mem::take(&mut self.current).freeze(),
        }
    }
}

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
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .init = Some(segment);
    }

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

    /// A registered camera's playlist is always valid live HLS, empty before
    /// init resolves: a polling player needs "keep polling", never a 404.
    fn playlist_text(&self) -> String {
        let (has_init, target_duration_secs, media_sequence, segments) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let has_init = state.init.is_some();

            // RFC 8216: TARGETDURATION >= every EXTINF, hence the maximum of
            // the target and the longest actual segment duration.
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

/// Cameras are registered once at startup, before serving tasks spawn, so
/// the map needs no lock.
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
    /// # Panics
    ///
    /// If `name` is already registered: the camera set is fixed at startup.
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

/// # Panics
///
/// If `name` was never [`MultiCameraHlsStore::register`]ed -- a caller bug.
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

#[derive(Debug)]
pub struct HlsServer {
    listener: TcpListener,
}

impl HlsServer {
    /// # Errors
    ///
    /// If binding fails.
    pub async fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(addr).await?,
        })
    }

    /// # Errors
    ///
    /// If the socket has no local address (impossible on a bound listener).
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

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

enum Resource {
    Playlist,
    Init,
    Segment(u64),
}

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

fn parse_path(path: &str) -> Option<(&str, &str)> {
    path.strip_prefix('/')?.split_once('/')
}

async fn handle_connection(stream: TcpStream, store: &MultiCameraHlsStore) {
    handle_connection_with_timeout(stream, store, CONNECTION_TIMEOUT).await;
}

enum ConnectionOutcome {
    Read(std::io::Error),
    Write(std::io::Error),
}

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

const MAX_REQUEST_HEADERS: usize = 64;

/// Parse errors are not "incomplete": more bytes never fix a malformed request.
async fn read_request_line_and_path(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        let mut headers = [httparse::EMPTY_HEADER; MAX_REQUEST_HEADERS];
        let mut request = httparse::Request::new(&mut headers);
        match request.parse(&buffer) {
            Ok(status) if status.is_complete() => {
                let path = request.path.unwrap_or("/");
                let path = path.split('?').next().unwrap_or(path);
                return Ok(path.to_string());
            }
            Ok(_) => {}
            Err(err) => {
                return Err(std::io::Error::other(format!(
                    "malformed request header block: {err}"
                )));
            }
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

fn route(path: &str, store: &MultiCameraHlsStore) -> Response {
    let Some((camera_name, resource)) = parse_path(path) else {
        return Response::not_found("expected /<camera name>/<resource>");
    };
    let Some(camera) = store.camera(camera_name) else {
        return Response::not_found("no such camera is configured");
    };
    let Some(resource) = parse_resource(resource) else {
        return Response::not_found("expected playlist.m3u8, init.mp4, or segment-<sequence>.m4s");
    };

    match resource {
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
            || {
                Response::not_found(
                    "no such segment (already rolled out of the live window, or never produced)",
                )
            },
            |segment| Response {
                status: "200 OK",
                content_type: "video/mp4",
                body: segment.to_vec(),
            },
        ),
    }
}

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

    fn prime(fragmenter: &mut Fragmenter) {
        fragmenter.next(&frame(0, IDR));
    }

    #[test]
    fn segment_builder_does_not_cut_before_the_target_duration_is_reached() {
        let mut fragmenter = Fragmenter::default();
        let mut builder = SegmentBuilder::default();
        prime(&mut fragmenter);

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

        let non_keyframe = fragmenter.next(&frame(4 * 90_000, NON_IDR)).unwrap();
        assert!(
            builder.push(&non_keyframe).is_none(),
            "must not cut mid-GOP even once the target duration has passed"
        );

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

    #[test]
    fn camera_hls_sequence_numbers_never_repeat_across_a_simulated_restart() {
        let camera = CameraHls::default();
        camera.set_init(Bytes::from_static(b"fake-init"));

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

        // A brand-new builder pair models a supervised task restart.
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

    #[tokio::test]
    async fn run_hls_segment_produces_a_resolved_playlist_and_fetchable_segments() {
        let mut store = MultiCameraHlsStore::default();
        store.register("cam");
        let store = Arc::new(store);

        let (sender, receiver) = broadcast::channel(64);
        let worker = tokio::spawn(run_hls_segment("cam".to_string(), receiver, store.clone()));

        sender.send(frame(0, SPS)).unwrap();
        sender.send(frame(0, PPS)).unwrap();
        sender.send(frame(0, IDR)).unwrap();
        sender.send(frame(180_000, NON_IDR)).unwrap();
        sender.send(frame(270_000, IDR)).unwrap();

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

    #[tokio::test]
    async fn handle_connection_gives_up_on_a_peer_that_never_sends_a_complete_request() {
        let store = MultiCameraHlsStore::default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // Connected but silent: a dropped client takes the connection-closed
        // path instead.
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

    #[tokio::test]
    async fn read_request_line_and_path_parses_a_request_carrying_more_than_sixteen_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(addr).await.unwrap();
        let (mut server_stream, _) = listener.accept().await.unwrap();

        let mut request = String::from("GET /back/playlist.m3u8 HTTP/1.1\r\nHost: example\r\n");
        for i in 0..20 {
            let _ = write!(request, "X-Extra-{i}: value\r\n");
        }
        request.push_str("\r\n");
        client.write_all(request.as_bytes()).await.unwrap();

        let path = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            read_request_line_and_path(&mut server_stream),
        )
        .await
        .expect("must not hang on a request that legitimately has more than sixteen headers")
        .expect("a well-formed request with many headers must still parse");

        assert_eq!(path, "/back/playlist.m3u8");
    }

    #[tokio::test]
    async fn read_request_line_and_path_fails_fast_on_more_headers_than_it_can_hold() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(addr).await.unwrap();
        let (mut server_stream, _) = listener.accept().await.unwrap();

        let mut request = String::from("GET /back/playlist.m3u8 HTTP/1.1\r\nHost: example\r\n");
        for i in 0..=MAX_REQUEST_HEADERS {
            let _ = write!(request, "X-Extra-{i}: value\r\n");
        }
        request.push_str("\r\n");
        client.write_all(request.as_bytes()).await.unwrap();

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            read_request_line_and_path(&mut server_stream),
        )
        .await
        .expect(
            "a request with more headers than this server can hold must fail immediately, not \
             be mistaken for an incomplete one and wait for bytes that will never come",
        );

        assert!(result.is_err());
    }
}
