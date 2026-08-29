//! The grid tile's live camera view (issue #12 item U1): a native `<video>`
//! element driven by `MediaSource`/`SourceBuffer`, fed by G2's
//! fMP4-over-WebSocket transport
//! (`crates/corvette-media-bridge/src/ws_repackager.rs`) -- replacing the
//! embedded go2rtc WebRTC player page (INV-7).
//!
//! No fMP4 repackaging happens here: every box G2 sends over the WebSocket
//! is appended to the `SourceBuffer` unchanged. This module's own work is
//! (1) building the WebSocket URL, (2) determining the `SourceBuffer`'s
//! required MIME `codecs` parameter by reading the sample-entry box already
//! inside G2's own initialization segment -- the first message on every
//! connection, since G2 sends no codec metadata alongside the raw bytes --
//! and (3) reconnecting with backoff when the connection drops, so one lost
//! WebSocket does not leave the tile permanently blank.
//!
//! `sourceBuffer.mode` is set to `"sequence"` before any append, per the
//! transport contract `docs/design/api-contracts.md`'s "grid-tile...carries
//! no per-viewer timestamp epoch" section records: G2 broadcasts identical
//! bytes -- and identical `tfdt` values -- to every current viewer of a
//! camera regardless of when each one connected, so a viewer must play
//! appended segments back-to-back on its own timeline rather than trusting
//! each fragment's own absolute decode time.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    BinaryType, CloseEvent, Event, HtmlVideoElement, MediaSource, MessageEvent, SourceBuffer,
    SourceBufferAppendMode, Url, WebSocket,
};

/// Relative path prefix nginx is expected to proxy to G2's own
/// fMP4-over-WebSocket listener once issue #12 item N2 lands (N2's own Do
/// text: "add a NEW location replacing `/live/mse/api/ws`'s function,
/// proxying to G2's WebSocket endpoint"). The camera name is appended as the
/// final path segment, matching G2's own per-camera path convention
/// (`ws_repackager::handle_connection`: the WebSocket request path, trimmed
/// of its leading slash, names the camera) -- so nginx only needs to strip
/// this prefix and forward the remainder unchanged. N2 must proxy this exact
/// prefix for this client to reach G2 in production.
const MEDIA_BRIDGE_WS_PATH_PREFIX: &str = "/live/mse/ws/";

/// A test-only escape hatch: when the page's global object carries this
/// property (a plain string, `"ws://host:port"` or `"wss://host:port"`), it
/// is used as the WebSocket origin instead of this page's own origin. This
/// is what lets this item's own real-endpoint Playwright check
/// (`tests/ui/live_view.spec.cjs`) dial a real `corvette-media-bridge`
/// process directly, without needing issue #12 item N2's nginx proxy route
/// (not yet built) inside this crate's own test harness -- the same shape as
/// `corvette-media-bridge`'s own `G2_MUTATION_BARE_SPAWN`-style test-only
/// environment hooks. Production never sets this property, so every real
/// deployment always takes the `same_origin_ws_base` branch below.
const WS_ORIGIN_OVERRIDE_PROPERTY: &str = "__corvetteMediaBridgeWsOrigin";

/// Mirrors `corvette_rtsp_client`'s own reconnect backoff
/// (`crates/corvette-rtsp-client/src/client/task.rs`'s `Backoff`): the same
/// doubling shape and the same starting constants, reimplemented here
/// because that crate runs server-side under Tokio and this one runs
/// entirely inside a browser's own event loop, which cannot reach it.
/// Milliseconds, not `std::time::Duration`, since every browser timer API
/// this module calls (`Window::set_timeout_with_callback_and_timeout_and_arguments_0`)
/// takes one. The values themselves are kept identical to the server-side
/// crate's: this is a same-origin (or, once N2 lands, same-cluster) dial
/// with no camera-firmware-specific timing consideration to argue for a
/// different starting point.
const INITIAL_BACKOFF_MS: i32 = 250;
const MAX_BACKOFF_MS: i32 = 30_000;
/// A connection that stayed open at least this long is treated as having
/// recovered, so a later drop starts backing off from the beginning again
/// rather than continuing from whatever multiple an older, unrelated string
/// of failures had already reached -- matching
/// `corvette_rtsp_client::client::task::STABLE_CONNECTION_THRESHOLD`'s own
/// value and rationale exactly.
const STABLE_CONNECTION_THRESHOLD_MS: f64 = 10_000.0;

/// Renders one grid tile's live camera view: a bare `<video>` element whose
/// `MediaSource`/`WebSocket` wiring is entirely imperative (see
/// [`LiveSession`]) -- there is no reactive state to render here beyond the
/// element itself, since every frame arrives out-of-band over the socket.
#[component]
pub(crate) fn LiveCameraTile(camera_name: String, title: String) -> impl IntoView {
    let video = NodeRef::<html::Video>::new();
    // `LiveSession` owns non-`Send`/non-`Sync` browser objects (`Rc`,
    // `web_sys` handles), but `on_cleanup` below requires a `Send + Sync`
    // closure regardless of target -- `StoredValue::new_local` is leptos's
    // own escape hatch for exactly this (a `Copy`, `Send`/`Sync` handle into
    // a thread-local arena slot that itself holds a non-`Send` value; sound
    // here because a WASM build is always single-threaded).
    let session = StoredValue::new_local(None::<LiveSession>);

    Effect::new(move |_| {
        let Some(video_element) = video.get() else {
            return;
        };
        session.set_value(Some(LiveSession::start(camera_name.clone(), video_element)));
    });

    on_cleanup(move || {
        if let Some(Some(session)) = session.try_update_value(Option::take) {
            session.stop();
        }
    });

    view! {
        <video
            node_ref=video
            class="camera-player"
            title=title
            muted
            autoplay
            playsinline
        ></video>
    }
}

/// One grid tile's live connection: owns the current attempt's `WebSocket`
/// and `MediaSource` for as long as the tile stays mounted, reconnecting
/// with backoff on any close or error (Do step 4) rather than leaving the
/// tile permanently blank after one dropped connection.
struct LiveSession {
    state: Rc<RefCell<SessionState>>,
}

impl LiveSession {
    fn start(camera_name: String, video: HtmlVideoElement) -> Self {
        let state = Rc::new(RefCell::new(SessionState {
            camera_name,
            video,
            backoff: Backoff::new(),
            stopped: false,
            attempt: None,
        }));
        connect_once(Rc::clone(&state));
        Self { state }
    }

    /// Ends this session for good: no further reconnect is scheduled, and
    /// the current attempt (if any) is torn down cleanly via
    /// [`Attempt`]'s own `Drop`.
    fn stop(self) {
        let mut state = self.state.borrow_mut();
        state.stopped = true;
        state.attempt = None;
    }
}

struct SessionState {
    camera_name: String,
    video: HtmlVideoElement,
    backoff: Backoff,
    stopped: bool,
    attempt: Option<Attempt>,
}

/// Everything belonging to one connection attempt -- torn down and replaced
/// wholesale on every reconnect, since neither a `MediaSource` nor a
/// `SourceBuffer` can be reset in place once the stream has ended or errored;
/// starting over from a fresh `MediaSource` is what every consumer of this
/// transport is expected to do (see this module's own top-level doc).
struct Attempt {
    socket: WebSocket,
    media_source: MediaSource,
    object_url: String,
    /// Set once the socket's `open` event fires; read back when the
    /// connection ends to decide whether this attempt counts as "stable"
    /// (see [`STABLE_CONNECTION_THRESHOLD_MS`]).
    connected_at: Option<f64>,
    // Kept alive for exactly as long as this attempt is current. Dropped
    // (via this struct's own `Drop`, below) once a reconnect replaces it --
    // calling a `wasm_bindgen::closure::Closure` after it has been dropped
    // panics, so every handler is explicitly detached first.
    _on_open: Closure<dyn FnMut()>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_close: Closure<dyn FnMut(CloseEvent)>,
    _on_error: Closure<dyn FnMut(Event)>,
    _on_source_open: Closure<dyn FnMut()>,
}

impl Drop for Attempt {
    fn drop(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onclose(None);
        self.socket.set_onerror(None);
        let _ = self.socket.close();
        self.media_source.set_onsourceopen(None);
        let _ = Url::revoke_object_url(&self.object_url);
    }
}

/// The `SourceBuffer` this attempt is appending to, once the first message
/// (G2's own initialization segment) has resolved its codec, plus the
/// FIFO of fragments received before the buffer is done with whatever it is
/// currently appending (`SourceBuffer::updating`).
struct SourceBufferState {
    buffer: SourceBuffer,
    pending: VecDeque<Vec<u8>>,
    _on_update_end: Closure<dyn FnMut()>,
}

impl Drop for SourceBufferState {
    fn drop(&mut self) {
        self.buffer.set_onupdateend(None);
    }
}

/// Constructs a fresh media source, preferring Safari's `ManagedMediaSource`
/// over the plain `MediaSource` global whenever the browser exposes it. On
/// iOS/iPadOS Safari 17.1+, plain `MediaSource` exists as a global but is a
/// non-functional stand-in inaccessible to third-party pages -- confirmed
/// live (issue #12, 2026-08-29): this tile stayed entirely blank on a real
/// iPhone, with no error anywhere, until this fix. `web_sys` has no typed
/// binding for `ManagedMediaSource` (a Safari-only, non-standard-track
/// interface), so it is resolved and constructed via `js_sys::Reflect` and
/// cast back with `unchecked_into` -- safe here because `ManagedMediaSource`
/// implements the same `addSourceBuffer`/`sourceopen` surface this module
/// already calls through the typed `MediaSource` API, and every downstream
/// call is a plain JS method dispatch on the underlying object regardless of
/// what Rust's type checker believes it is. Sets `disableRemotePlayback` on
/// the video element when constructing a `ManagedMediaSource` instance,
/// matching Safari's own documented requirement for it -- confirmed against
/// the vendored `hls.js` (`public/vendor/hls.min.js`), which sets this exact
/// property under this exact condition for its own equivalent fallback.
fn create_media_source(video: &HtmlVideoElement) -> Option<MediaSource> {
    let window = web_sys::window()?;
    if let Ok(managed_ctor) =
        js_sys::Reflect::get(&window, &wasm_bindgen::JsValue::from_str("ManagedMediaSource"))
            .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
        && let Ok(instance) = js_sys::Reflect::construct(&managed_ctor, &js_sys::Array::new())
    {
        // `web_sys` has no typed binding for `disableRemotePlayback` in this
        // crate's enabled feature set; set it directly rather than adding a
        // feature just for one property.
        let _ = js_sys::Reflect::set(
            video,
            &wasm_bindgen::JsValue::from_str("disableRemotePlayback"),
            &wasm_bindgen::JsValue::TRUE,
        );
        return Some(instance.unchecked_into::<MediaSource>());
    }
    MediaSource::new().ok()
}

/// Starts (or restarts, after a reconnect) one connection attempt: opens a
/// fresh `WebSocket` and a fresh `MediaSource`, wires every event this
/// module needs, and stores the result as `state`'s current [`Attempt`].
///
/// Any failure constructing the socket or the media source (e.g. this
/// camera's name failing URI encoding, or the browser refusing a second
/// `MediaSource` for an exhausted resource budget) is treated the same as a
/// dropped connection: schedule a backed-off retry rather than leaving the
/// tile stuck.
fn connect_once(state: Rc<RefCell<SessionState>>) {
    let (camera_name, video) = {
        let borrowed = state.borrow();
        if borrowed.stopped {
            return;
        }
        (borrowed.camera_name.clone(), borrowed.video.clone())
    };

    let Some(url) = websocket_url(&camera_name) else {
        schedule_reconnect(state);
        return;
    };
    let Ok(socket) = WebSocket::new(&url) else {
        schedule_reconnect(state);
        return;
    };
    socket.set_binary_type(BinaryType::Arraybuffer);

    let Some(media_source) = create_media_source(&video) else {
        schedule_reconnect(state);
        return;
    };
    let Ok(object_url) = Url::create_object_url_with_source(&media_source) else {
        schedule_reconnect(state);
        return;
    };
    video.set_src(&object_url);

    let source_buffer_slot: Rc<RefCell<Option<SourceBufferState>>> = Rc::new(RefCell::new(None));

    // `add_source_buffer` requires `MediaSource::ready_state() == "open"`,
    // which only holds once `sourceopen` has fired -- but the first WS
    // message (handled below) is what actually knows the codec, so this
    // listener's only job is documenting that ordering constraint; nothing
    // needs to run here directly, since a real browser fires `sourceopen`
    // synchronously once `video.src` is assigned, well before any WS
    // message can arrive.
    let on_source_open = Closure::<dyn FnMut()>::new(move || {});
    media_source.set_onsourceopen(Some(on_source_open.as_ref().unchecked_ref()));

    let on_message = {
        let media_source = media_source.clone();
        let source_buffer_slot = Rc::clone(&source_buffer_slot);
        Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            handle_message(&media_source, &source_buffer_slot, &event);
        })
    };
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    let on_open = {
        let state = Rc::clone(&state);
        Closure::<dyn FnMut()>::new(move || {
            if let Some(attempt) = state.borrow_mut().attempt.as_mut() {
                attempt.connected_at = Some(js_sys::Date::now());
            }
        })
    };
    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));

    let on_close = {
        let state = Rc::clone(&state);
        Closure::<dyn FnMut(CloseEvent)>::new(move |_event: CloseEvent| {
            schedule_reconnect(Rc::clone(&state));
        })
    };
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

    let on_error = {
        let state = Rc::clone(&state);
        Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
            schedule_reconnect(Rc::clone(&state));
        })
    };
    socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));

    state.borrow_mut().attempt = Some(Attempt {
        socket,
        media_source,
        object_url,
        connected_at: None,
        _on_open: on_open,
        _on_message: on_message,
        _on_close: on_close,
        _on_error: on_error,
        _on_source_open: on_source_open,
    });
}

/// Ends the current attempt (if this session still has one -- see below) and
/// schedules a fresh [`connect_once`] after the session's own backoff delay.
///
/// A real WebSocket `error` event is always followed by a `close` event for
/// the same connection, and both handlers call this function -- so the
/// `borrowed.attempt.is_none()` guard below is not defensive filler: it is
/// what keeps the pair from scheduling two reconnects (the first call tears
/// the attempt down and schedules one retry; the second call finds no
/// attempt left and does nothing).
fn schedule_reconnect(state: Rc<RefCell<SessionState>>) {
    let delay_ms = {
        let mut borrowed = state.borrow_mut();
        if borrowed.stopped || borrowed.attempt.is_none() {
            return;
        }
        let connected_at = borrowed
            .attempt
            .as_ref()
            .and_then(|attempt| attempt.connected_at);
        if let Some(connected_at) = connected_at
            && js_sys::Date::now() - connected_at >= STABLE_CONNECTION_THRESHOLD_MS
        {
            borrowed.backoff.reset();
        }
        // Dropping the attempt here (rather than leaving it in place until
        // `connect_once` overwrites it) detaches and closes it immediately,
        // via `Attempt`'s own `Drop` -- not just whenever the timer below
        // eventually fires.
        borrowed.attempt = None;
        borrowed.backoff.advance()
    };

    let Some(window) = web_sys::window() else {
        return;
    };
    let callback = Closure::once_into_js(move || connect_once(Rc::clone(&state)));
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        callback.unchecked_ref(),
        delay_ms,
    );
}

/// Handles one WebSocket binary message: the first one on a connection is
/// G2's own initialization segment (used to create the `SourceBuffer`),
/// every one after it is a `moof`/`mdat` fragment queued for that same
/// buffer.
fn handle_message(
    media_source: &MediaSource,
    source_buffer_slot: &Rc<RefCell<Option<SourceBufferState>>>,
    event: &MessageEvent,
) {
    let Ok(buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() else {
        return;
    };
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();

    {
        let mut slot = source_buffer_slot.borrow_mut();
        if slot.is_none() {
            let Some(codec) = mse_codec_string(&bytes) else {
                return;
            };
            let Ok(buffer) = media_source.add_source_buffer(&codec) else {
                return;
            };
            // "sequence" mode: see this module's own top-level doc and
            // `docs/design/api-contracts.md`'s "no per-viewer timestamp
            // epoch" section -- G2 broadcasts identical, non-viewer-relative
            // `tfdt` values to every connection, so this client plays
            // appended segments back-to-back on its own timeline rather
            // than trusting each one's absolute decode time.
            buffer.set_mode(SourceBufferAppendMode::Sequence);
            let on_update_end = {
                let source_buffer_slot = Rc::clone(source_buffer_slot);
                Closure::<dyn FnMut()>::new(move || {
                    drain_pending(&source_buffer_slot);
                })
            };
            buffer.set_onupdateend(Some(on_update_end.as_ref().unchecked_ref()));
            *slot = Some(SourceBufferState {
                buffer,
                pending: VecDeque::new(),
                _on_update_end: on_update_end,
            });
        }

        let Some(source_buffer_state) = slot.as_mut() else {
            return;
        };
        source_buffer_state.pending.push_back(bytes);
    }
    drain_pending(source_buffer_slot);
}

/// Appends the next queued fragment if the `SourceBuffer` exists and is not
/// already mid-append -- called both right after a message is queued and
/// from the buffer's own `updateend` event, matching the
/// queue-and-drain shape this item's own G2 browser check
/// (`crates/corvette-media-bridge/tests/browser/g2_fmp4_ws.spec.cjs`)
/// establishes in JavaScript, reimplemented here in Rust for the production
/// client.
fn drain_pending(source_buffer_slot: &Rc<RefCell<Option<SourceBufferState>>>) {
    let mut slot = source_buffer_slot.borrow_mut();
    let Some(source_buffer_state) = slot.as_mut() else {
        return;
    };
    if source_buffer_state.buffer.updating() {
        return;
    }
    let Some(mut bytes) = source_buffer_state.pending.pop_front() else {
        return;
    };
    let _ = source_buffer_state.buffer.append_buffer_with_u8_array(&mut bytes);
}

/// Builds this camera's WebSocket URL: [`WS_ORIGIN_OVERRIDE_PROPERTY`] when
/// a test has set it, otherwise this page's own origin (see this module's
/// own top-level doc for why each exists).
fn websocket_url(camera_name: &str) -> Option<String> {
    let window = web_sys::window()?;
    let camera = js_sys::encode_uri_component(camera_name);
    let override_origin =
        js_sys::Reflect::get(&window, &JsValue::from_str(WS_ORIGIN_OVERRIDE_PROPERTY))
            .ok()
            .and_then(|value| value.as_string());
    if let Some(origin) = override_origin {
        // The override means "dial G2 directly, with no nginx in between"
        // (see this constant's own doc) -- so this uses G2's own bare
        // per-camera path convention directly
        // (`ws_repackager::handle_connection`: the WebSocket request path,
        // trimmed of its leading slash, names the camera), not
        // [`MEDIA_BRIDGE_WS_PATH_PREFIX`], which only exists for nginx to
        // strip on the way to that same bare path.
        return Some(format!("{origin}/{camera}"));
    }
    let origin = same_origin_ws_base(&window);
    Some(format!("{origin}{MEDIA_BRIDGE_WS_PATH_PREFIX}{camera}"))
}

/// This page's own origin, translated to the matching WebSocket scheme
/// (`https:` -> `wss:`, everything else -> `ws:`).
fn same_origin_ws_base(window: &web_sys::Window) -> String {
    let location = window.location();
    let protocol = location.protocol().unwrap_or_default();
    let ws_scheme = if protocol == "https:" { "wss:" } else { "ws:" };
    let host = location.host().unwrap_or_default();
    format!("{ws_scheme}//{host}")
}

/// The 78-byte fixed `VisualSampleEntry` header (ISO/IEC 14496-12 section
/// 12.1.3) that precedes the codec configuration box (`avcC`/`hvcC`) inside
/// an `avc1`/`hvc1` sample entry -- see
/// `crates/corvette-media-bridge/src/fmp4.rs`'s own `sample_entry_header`,
/// which writes exactly these 78 bytes.
const VISUAL_SAMPLE_ENTRY_HEADER_LEN: usize = 78;

/// Reads the `codecs` MIME parameter a `SourceBuffer` needs from G2's own
/// initialization segment (`ftyp`+`moov`, carrying exactly one `avc1`
/// (H.264) or `hvc1` (H.265) sample entry -- `crate::fmp4`'s own doc). G2
/// sends no codec metadata alongside the raw bytes, so this is the only way
/// a consumer of this transport learns which codec a camera uses.
fn mse_codec_string(init_segment: &[u8]) -> Option<String> {
    let moov = find_box(init_segment, *b"moov")?;
    let trak = find_box(moov, *b"trak")?;
    let mdia = find_box(trak, *b"mdia")?;
    let minf = find_box(mdia, *b"minf")?;
    let stbl = find_box(minf, *b"stbl")?;
    let stsd = find_box(stbl, *b"stsd")?;
    // `stsd` is a `FullBox`: version(1) + flags(3) + entry_count(4), then
    // its entries -- each an ordinary box -- start at byte 8.
    let entries = stsd.get(8..)?;

    if let Some(avc1) = find_box(entries, *b"avc1") {
        let avcc = find_box(avc1.get(VISUAL_SAMPLE_ENTRY_HEADER_LEN..)?, *b"avcC")?;
        let &[_config_version, profile, compatibility, level, ..] = avcc else {
            return None;
        };
        return Some(format!(
            "video/mp4; codecs=\"avc1.{profile:02X}{compatibility:02X}{level:02X}\""
        ));
    }
    if let Some(hvc1) = find_box(entries, *b"hvc1") {
        let hvcc = find_box(hvc1.get(VISUAL_SAMPLE_ENTRY_HEADER_LEN..)?, *b"hvcC")?;
        return hevc_codec_string(hvcc).map(|codec| format!("video/mp4; codecs=\"{codec}\""));
    }
    None
}

/// Finds the first top-level box of type `fourcc` inside `data` and returns
/// its content (the bytes after its 8-byte size+type header). Only the
/// standard 32-bit size form is handled -- the 64-bit `largesize` extension
/// ISO/IEC 14496-12 allows is never produced by this transport's own sender
/// (`crate::fmp4`'s own `write_box`, in `corvette-media-bridge`, always
/// writes a plain `u32` size).
fn find_box(data: &[u8], fourcc: [u8; 4]) -> Option<&[u8]> {
    let mut offset = 0;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        if size < 8 || offset + size > data.len() {
            return None;
        }
        if data[offset + 4..offset + 8] == fourcc {
            return Some(&data[offset + 8..offset + size]);
        }
        offset += size;
    }
    None
}

/// Builds an RFC 6381 HEVC codec parameter string
/// (`hvc1.<profile>.<compatibility>.<tier><level>.<constraint bytes>`) from
/// an `hvcC` box's content.
///
/// Best effort only, matching `corvette-media-bridge`'s own
/// `fmp4::boxes::write_hvcc_body` doc: no real H.265 camera or decoder has
/// ever exercised either side of this transport in this workspace, so this
/// is written to the specification but -- unlike the `avc1` path above,
/// which this item's own Verify step (a real browser reaching
/// `HAVE_CURRENT_DATA`) exercises directly -- it is not verified against a
/// real browser.
fn hevc_codec_string(hvcc: &[u8]) -> Option<String> {
    use std::fmt::Write as _;

    let &[_config_version, byte1, c0, c1, c2, c3, k0, k1, k2, k3, k4, k5, level, ..] = hvcc else {
        return None;
    };
    let profile_space = (byte1 >> 6) & 0x3;
    let tier_flag = (byte1 >> 5) & 0x1;
    let profile_idc = byte1 & 0x1F;
    let space_prefix = match profile_space {
        1 => "A",
        2 => "B",
        3 => "C",
        _ => "",
    };
    let compatibility = u32::from_be_bytes([c0, c1, c2, c3]);
    let tier = if tier_flag == 0 { "L" } else { "H" };

    let constraint_bytes = [k0, k1, k2, k3, k4, k5];
    let mut significant = constraint_bytes.len();
    while significant > 0 && constraint_bytes[significant - 1] == 0 {
        significant -= 1;
    }

    let mut codec = format!("hvc1.{space_prefix}{profile_idc}.{compatibility:X}.{tier}{level}");
    for byte in &constraint_bytes[..significant] {
        let _ = write!(codec, ".{byte:X}");
    }
    Some(codec)
}

/// Exponential backoff between reconnect attempts, doubling from
/// [`INITIAL_BACKOFF_MS`] up to a [`MAX_BACKOFF_MS`] ceiling -- the browser
/// counterpart of `corvette_rtsp_client::client::task::Backoff` (see this
/// module's own top-level doc).
#[derive(Debug, Clone, Copy)]
struct Backoff {
    next_ms: i32,
}

impl Backoff {
    const fn new() -> Self {
        Self {
            next_ms: INITIAL_BACKOFF_MS,
        }
    }

    fn advance(&mut self) -> i32 {
        let delay = self.next_ms;
        self.next_ms = self.next_ms.saturating_mul(2).min(MAX_BACKOFF_MS);
        delay
    }

    const fn reset(&mut self) {
        self.next_ms = INITIAL_BACKOFF_MS;
    }
}

#[cfg(test)]
mod backoff_tests {
    use super::{Backoff, INITIAL_BACKOFF_MS, MAX_BACKOFF_MS};

    #[test]
    fn doubles_from_the_initial_delay_up_to_the_ceiling_then_holds() {
        let mut backoff = Backoff::new();
        assert_eq!(backoff.advance(), INITIAL_BACKOFF_MS);
        assert_eq!(backoff.advance(), INITIAL_BACKOFF_MS * 2);
        assert_eq!(backoff.advance(), INITIAL_BACKOFF_MS * 4);

        let mut delay = INITIAL_BACKOFF_MS * 4;
        while delay < MAX_BACKOFF_MS {
            delay = backoff.advance();
        }
        assert_eq!(delay, MAX_BACKOFF_MS);
        assert_eq!(backoff.advance(), MAX_BACKOFF_MS);
    }

    #[test]
    fn reset_returns_to_the_initial_delay() {
        let mut backoff = Backoff::new();
        backoff.advance();
        backoff.advance();
        backoff.reset();
        assert_eq!(backoff.advance(), INITIAL_BACKOFF_MS);
    }
}

#[cfg(test)]
mod codec_sniff_tests {
    use super::mse_codec_string;

    fn write_box(out: &mut Vec<u8>, fourcc: [u8; 4], body: &[u8]) {
        let size = u32::try_from(8 + body.len()).unwrap();
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(&fourcc);
        out.extend_from_slice(body);
    }

    /// Builds a minimal, real-shaped `ftyp`-less `moov` tree carrying one
    /// `avc1` sample entry, mirroring the box nesting
    /// `corvette-media-bridge`'s own `fmp4::boxes::init_segment_h264`
    /// produces (`moov > trak > mdia > minf > stbl > stsd > avc1 > avcC`) --
    /// built independently here (this crate cannot depend on
    /// `corvette-media-bridge`, a Tokio server crate, from WASM) rather than
    /// reusing its byte output directly.
    fn h264_init_segment(profile: u8, compatibility: u8, level: u8) -> Vec<u8> {
        let avc_config = vec![1, profile, compatibility, level, 0xFF, 0xE1, 0, 0, 1, 0];
        let mut config_box = Vec::new();
        write_box(&mut config_box, *b"avcC", &avc_config);

        let mut sample_entry = vec![0u8; super::VISUAL_SAMPLE_ENTRY_HEADER_LEN];
        sample_entry.extend_from_slice(&config_box);
        let mut entry_box = Vec::new();
        write_box(&mut entry_box, *b"avc1", &sample_entry);

        let mut stsd_body = vec![0, 0, 0, 0, 0, 0, 0, 1]; // version/flags + entry_count=1
        stsd_body.extend_from_slice(&entry_box);
        let mut stsd = Vec::new();
        write_box(&mut stsd, *b"stsd", &stsd_body);

        let mut stbl = Vec::new();
        write_box(&mut stbl, *b"stbl", &stsd);
        let mut minf = Vec::new();
        write_box(&mut minf, *b"minf", &stbl);
        let mut mdia = Vec::new();
        write_box(&mut mdia, *b"mdia", &minf);
        let mut trak = Vec::new();
        write_box(&mut trak, *b"trak", &mdia);
        let mut moov = Vec::new();
        write_box(&mut moov, *b"moov", &trak);
        moov
    }

    #[test]
    fn reads_the_avc1_codec_string_from_a_real_shaped_init_segment() {
        let segment = h264_init_segment(0x42, 0xC0, 0x1F);
        assert_eq!(
            mse_codec_string(&segment),
            Some("video/mp4; codecs=\"avc1.42C01F\"".to_string())
        );
    }

    #[test]
    fn a_truncated_segment_reports_no_codec_rather_than_panicking() {
        assert_eq!(mse_codec_string(&[0, 0, 0, 1]), None);
        assert_eq!(mse_codec_string(&[]), None);
    }
}
