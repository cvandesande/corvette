//! The grid tile's expanded/click-to-enlarge view (issue #12 item U2):
//! MoQ-first via `@moq/watch`'s `<moq-watch>` custom element, falling back to
//! an HLS `<video>` (driven by `hls.js`, matching the pattern G3's own
//! browser check already established:
//! `crates/corvette-media-bridge/tests/browser/g3_hls.spec.cjs`) when the
//! relay does not answer within a bounded window.
//!
//! # `<moq-watch>`'s actual contract (verified by source read, not assumed)
//!
//! `js/hang` (`@moq/hang`) defines no custom element at all -- the plan's own
//! R1 evidence (`.agents/issue-12/evidence/R1-dependency-pin.md`) already
//! found this. The element this module vendors and mounts is the sibling
//! package `@moq/watch`'s `<moq-watch>`
//! (`js/watch/src/element.ts:518`, read directly at the pinned commit
//! `7b73c43381a7f9c309e3045a8f0f858aa32ef48c`):
//!
//! - Two attributes matter here: `url` (the relay's own connect URL, e.g.
//!   `https://host:port/anon` -- the same shape
//!   `corvette-media-bridge`'s own `moq_publish.rs` dials, confirmed by its
//!   own evidence's `relay=https://127.0.0.1:34097/anon` log line) and `name`
//!   (the broadcast path *relative to that URL's root* -- G1's own
//!   `moq_publish::run_publish_once` calls
//!   `origin.create_broadcast(name, ...)` with the bare camera name, no
//!   prefix of its own, so this module sets `name` to the plain camera name).
//! - It renders only into a nested `<canvas>` **child** element -- not a
//!   shadow-DOM canvas, not a `<video>` child (`element.ts`'s own
//!   `setCanvas`: "a `<video>` child does nothing"). This module always
//!   appends one.
//! - It has no `dispatchEvent`/`CustomEvent` failure surface at all (verified
//!   by an exhaustive `grep -rn dispatchEvent` across `js/watch/src` and
//!   `js/net/src`, which finds only an unrelated audio-unlock test). What it
//!   *does* expose, as ordinary public fields (not attributes) on the
//!   element instance, is `element.connection`, a `Moq.Connection.Reload`
//!   (`js/net/src/connection/reload.ts`):
//!     - `.status`: a `Signal<"connecting"|"connected"|"disconnected">`
//!       (`Signal.subscribe(fn)` is a public, documented method --
//!       `js/signals/src/index.ts`).
//!     - `.closed`: a `Promise<void>` that **rejects** once the internal
//!       reconnect loop gives up (`#retry`'s own deadline, default 10
//!       seconds from the first failure -- `reload.ts`'s own
//!       `DEFAULT_TIMEOUT`), or resolves if *we* call `.close()` ourselves.
//!
//! This module races its own bounded timeout against both: `.status`
//! reaching `"connected"` is success; `.closed` rejecting, or the timeout
//! firing first, is failure. This is the "confirmed failure event" the
//! plan's own Premise asks this item to find and use -- not a polling
//! workaround.
//!
//! `js/watch/src/element.ts` and `js/net/src/connection/reload.ts` are
//! byte-identical between the pinned commit and every npm-published
//! `@moq/watch`/`@moq/hang` version from `0.5.0`/`0.4.0` through the current
//! `0.5.2`/`0.4.2` (confirmed via `gh api .../compare/...` diffs covering
//! that whole range) -- see `crates/corvette-ui/public/vendor/PROVENANCE.md`
//! for the exact versions vendored and why.
//!
//! # The bounded timeout
//!
//! [`MOQ_ATTEMPT_TIMEOUT_MS`] is 4 seconds: comfortably under
//! `reload.ts`'s own 10-second internal give-up window (so this module's own
//! timeout -- not the library's -- is what a viewer actually experiences on
//! a genuinely unreachable relay), while still generous next to a real
//! WebTransport/QUIC handshake's actual cost (typically well under a second
//! on a healthy network; a few seconds still comfortably covers a slow or
//! lossy one before the tile gives up and shows something else). This sits
//! well below this crate's own precedent for a *media-delivery* timeout --
//! G3's browser check waits up to 20 seconds for a first HLS segment, G2's
//! waits up to 15 for a first WebSocket frame -- because a relay handshake
//! carries none of the encoding/segmenting latency either of those does; it
//! is pure connection setup.
//!
//! # The relay's public URL: a configuration point (D-3), not hard-coded
//!
//! D-3 requires the relay's exposure mechanism to be configurable, and this
//! value has nowhere else to live yet: it is not Frigate's own `/api/config`
//! (`crate::api`'s `FrigateConfig` is Frigate's own backend contract, out of
//! this item's reach per the plan's own gates), and unlike U1's WebSocket
//! origin it has no same-origin default to fall back to -- the relay is
//! reachable over its own `NodePort` (D-3/K1), a different host/port than the
//! page's own origin, not something nginx proxies. `docs/design/
//! architecture.md`'s "Risk boundary" section is explicit that Corvette does
//! not yet own configuration generally, so this module reads
//! [`MOQ_RELAY_URL_PROPERTY`], a `window` global a deployment sets via a
//! small inline `<script>` in the served page shell -- the same mechanism
//! `live_view.rs`'s own `WS_ORIGIN_OVERRIDE_PROPERTY` uses, generalized from
//! a test-only override (which has a same-origin production default) to the
//! primary mechanism here (which has none). An unset value degrades to the
//! HLS fallback immediately rather than hanging in "Connecting" forever.
//!
//! # The audio gap
//!
//! No camera publishes an audio track over `MoQ` today (`corvette-rtsp-client`
//! only ever resolves `m=video`, per G1's own Premise). This module makes no
//! special allowance for that: `<moq-watch>`'s own `Audio.Source` probes
//! whatever the catalog actually announces and simply plays nothing when no
//! audio rendition exists, so a video-only broadcast already degrades
//! cleanly with no code here aware of the distinction.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, Event, HtmlVideoElement};

/// A `window` global a deployment sets (via a small inline `<script>` in the
/// served page shell) to the relay's public WebTransport connect URL, e.g.
/// `"https://relay.example.org:30443/anon"` -- see this module's own
/// top-level doc, "The relay's public URL".
const MOQ_RELAY_URL_PROPERTY: &str = "__corvetteMoqRelayUrl";

/// Vendored, pre-bundled copy of `@moq/watch`'s `element` entry point (see
/// `public/vendor/PROVENANCE.md`). A plain classic script: loading it runs
/// `customElements.define("moq-watch", ...)` as a side effect.
const MOQ_WATCH_SCRIPT_SRC: &str = "/vendor/moq-watch.bundle.js";

/// Vendored copy of `hls.js`'s own official UMD build (`hls.min.js`),
/// already an npm devDependency for G3's own browser check
/// (`crates/corvette-media-bridge/tests/browser/g3_hls.spec.cjs`). Loading it
/// attaches `window.Hls`.
const HLS_SCRIPT_SRC: &str = "/vendor/hls.min.js";

/// G3's own served route prefix (`.agents/issue-12/evidence/N1-mutation.log`:
/// `/live/hls/<camera>/playlist.m3u8` -> `application/vnd.apple.mpegurl`).
const HLS_PLAYLIST_PATH_PREFIX: &str = "/live/hls/";

/// How long this module waits for `<moq-watch>`'s own connection to reach
/// `"connected"` before tearing it down and mounting the HLS fallback -- see
/// this module's own top-level doc, "The bounded timeout".
const MOQ_ATTEMPT_TIMEOUT_MS: i32 = 4_000;

/// Which transport is actually serving the expanded view right now --
/// surfaced in the UI via a `data-live-path` attribute and a short label
/// (Do step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LivePath {
    /// The `MoQ` attempt is still in flight and the HLS fallback has not
    /// started.
    Connecting,
    /// `<moq-watch>`'s own connection reached `"connected"`.
    Moq,
    /// The `MoQ` attempt failed or timed out; the HLS fallback is mounted.
    Hls,
    /// Both paths failed.
    None,
}

impl LivePath {
    const fn data_attr(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Moq => "moq",
            Self::Hls => "hls",
            Self::None => "none",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting…",
            Self::Moq => "Live (MoQ)",
            Self::Hls => "Live (HLS fallback)",
            Self::None => "Live view unavailable",
        }
    }
}

/// One camera the reader has asked to see expanded -- identifies the
/// broadcast/route (`name`) and what to show as a heading (`title`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExpandedCamera {
    pub(crate) name: String,
    pub(crate) title: String,
}

/// Renders the click-to-enlarge modal for one camera: a `MoQ`-first player
/// that falls back to HLS, with [`LivePath`] surfaced for the reader.
///
/// Mirrors `activity.rs`'s own `ReviewPlayback` modal shape (click-outside
/// and Escape both close it, the close button receives focus on open) --
/// the established precedent for exactly this kind of overlay in this crate.
#[component]
pub(crate) fn ExpandedView(
    camera: ExpandedCamera,
    expanded: RwSignal<Option<ExpandedCamera>>,
) -> impl IntoView {
    // Consumes `camera` in one move rather than field-borrowing it
    // repeatedly below -- both fields are needed as independently owned,
    // `'static` values regardless (one goes into a reactive effect closure,
    // the other into the view multiple times).
    let ExpandedCamera {
        name: camera_name,
        title,
    } = camera;
    let path = RwSignal::new(LivePath::Connecting);
    let player_container = NodeRef::<html::Div>::new();
    let close_button = NodeRef::<html::Button>::new();
    // See `live_view.rs`'s own `LiveCameraTile` for why `new_local` is
    // needed: this session owns non-`Send`/non-`Sync` browser objects, but
    // `on_cleanup` requires a `Send + Sync` closure regardless of target --
    // sound here because a WASM build is always single-threaded.
    let session = StoredValue::new_local(None::<ExpandedSession>);

    Effect::new(move |_| {
        if let Some(button) = close_button.get() {
            _ = button.focus();
        }
    });

    Effect::new(move |_| {
        let Some(container) = player_container.get() else {
            return;
        };
        session.set_value(Some(ExpandedSession::start(
            camera_name.clone(),
            container.unchecked_into(),
            path.write_only(),
        )));
    });

    on_cleanup(move || {
        if let Some(Some(session)) = session.try_update_value(Option::take) {
            session.stop();
        }
    });

    view! {
        <div
            class="playback-modal"
            on:click=move |_| expanded.set(None)
            on:keydown=move |event| {
                if event.key() == "Escape" {
                    expanded.set(None);
                }
            }
        >
            <section
                class="expanded-view"
                role="dialog"
                aria-modal="true"
                aria-label=format!("{title} expanded live view")
                on:click=|event| event.stop_propagation()
            >
                <div class="recording-heading playback-heading">
                    <div>
                        <h2>{title.clone()}</h2>
                        <p
                            class="live-path-indicator"
                            data-live-path=move || path.get().data_attr()
                            aria-live="polite"
                        >
                            {move || path.get().label()}
                        </p>
                    </div>
                    <button
                        type="button"
                        node_ref=close_button
                        on:click=move |_| expanded.set(None)
                    >
                        "Close"
                    </button>
                </div>
                <div class="expanded-player" node_ref=player_container></div>
            </section>
        </div>
    }
}

/// Owns one expanded view's live connection attempt for as long as the
/// modal stays mounted: `MoQ` first, HLS fallback on failure or timeout.
struct ExpandedSession {
    state: Rc<RefCell<SessionState>>,
}

impl ExpandedSession {
    fn start(camera_name: String, container: Element, set_path: WriteSignal<LivePath>) -> Self {
        set_path.set(LivePath::Connecting);
        let state = Rc::new(RefCell::new(SessionState {
            camera_name,
            container,
            set_path,
            stopped: false,
            moq: None,
            hls: None,
        }));
        mount_moq(&state);
        Self { state }
    }

    /// Ends this session for good: both mounts (if any) are torn down via
    /// their own `Drop`.
    fn stop(self) {
        let mut state = self.state.borrow_mut();
        state.stopped = true;
        state.moq = None;
        state.hls = None;
    }
}

struct SessionState {
    camera_name: String,
    container: Element,
    set_path: WriteSignal<LivePath>,
    stopped: bool,
    moq: Option<MoqMount>,
    hls: Option<HlsMount>,
}

/// Everything belonging to one `<moq-watch>` attempt -- torn down wholesale
/// on success (the timeout/subscription are no longer needed) or failure
/// (the element itself is discarded; `<moq-watch>` cannot be redirected to a
/// new camera or relay in place).
struct MoqMount {
    element: Element,
    container: Element,
    timeout_id: i32,
    /// The `Dispose` function `Signal.subscribe` returned -- called before
    /// this struct's own `Closure` field drops, so the closure can never be
    /// invoked again first (see `live_view.rs`'s own `Attempt::drop` for the
    /// identical detach-before-drop discipline).
    status_dispose: js_sys::Function,
    _on_status: Closure<dyn FnMut(JsValue)>,
}

impl Drop for MoqMount {
    fn drop(&mut self) {
        if let Some(window) = web_sys::window() {
            window.clear_timeout_with_handle(self.timeout_id);
        }
        let _ = self.status_dispose.call0(&JsValue::UNDEFINED);
        // Explicit rather than relying on `disconnectedCallback`'s implicit
        // teardown: closes the underlying connection deterministically
        // instead of only on this element's eventual garbage collection
        // (`element.ts`'s own doc: "There's no destructor for web
        // components... this is the best we can do").
        if let Ok(connection) =
            js_sys::Reflect::get(&self.element, &JsValue::from_str("connection"))
            && let Ok(close) = js_sys::Reflect::get(&connection, &JsValue::from_str("close"))
                .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
        {
            let _ = close.call0(&connection);
        }
        let _ = self.container.remove_child(&self.element);
    }
}

/// Everything belonging to one HLS fallback attempt.
struct HlsMount {
    video: HtmlVideoElement,
    container: Element,
    hls_instance: JsValue,
    _on_fatal_error: Closure<dyn FnMut(JsValue, JsValue)>,
    _on_video_error: Closure<dyn FnMut(Event)>,
}

impl Drop for HlsMount {
    fn drop(&mut self) {
        self.video.set_onerror(None);
        // hls.js's own documented `destroy()` contract: stops loading,
        // unbinds every listener it owns (including this module's own
        // `"hlsError"` subscription below), and frees its resources -- so no
        // separate `.off()` call is needed first.
        if let Ok(destroy) = js_sys::Reflect::get(&self.hls_instance, &JsValue::from_str("destroy"))
            .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
        {
            let _ = destroy.call0(&self.hls_instance);
        }
        let _ = self.container.remove_child(&self.video);
    }
}

/// Starts (or restarts, after a failure) the `MoQ` attempt: loads the vendored
/// `<moq-watch>` bundle if needed, then creates and mounts the element.
fn mount_moq(state: &Rc<RefCell<SessionState>>) {
    let (camera_name, relay_url) = {
        let borrowed = state.borrow();
        if borrowed.stopped {
            return;
        }
        let Some(relay_url) = moq_relay_url() else {
            drop(borrowed);
            mount_hls_fallback(Rc::clone(state));
            return;
        };
        (borrowed.camera_name.clone(), relay_url)
    };

    let state = Rc::clone(state);
    ensure_script_loaded(&MOQ_WATCH_LOAD, MOQ_WATCH_SCRIPT_SRC, move || {
        create_moq_watch_element(&state, &camera_name, &relay_url);
    });
}

/// Resolves `element.connection.status`, its `subscribe` method, and
/// `element.connection.closed` -- the three real handles
/// [`create_moq_watch_element`]'s own race needs (this module's own
/// top-level doc has the underlying contract). `None` if the loaded script
/// does not actually expose them as documented.
fn moq_connection_handles(
    element: &Element,
) -> Option<(JsValue, js_sys::Function, js_sys::Promise)> {
    let connection = js_sys::Reflect::get(element, &JsValue::from_str("connection")).ok()?;
    let status_signal = js_sys::Reflect::get(&connection, &JsValue::from_str("status")).ok()?;
    let subscribe_fn = js_sys::Reflect::get(&status_signal, &JsValue::from_str("subscribe"))
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()?;
    let closed_promise = js_sys::Reflect::get(&connection, &JsValue::from_str("closed"))
        .ok()?
        .dyn_into::<js_sys::Promise>()
        .ok()?;
    Some((status_signal, subscribe_fn, closed_promise))
}

/// Creates, configures, and mounts one `<moq-watch>` element, wiring its
/// connection-status subscription, its `closed` promise, and this module's
/// own bounded timeout -- whichever of the three settles first decides
/// success (`moq_succeeded`) or failure (`moq_failed`).
fn create_moq_watch_element(state: &Rc<RefCell<SessionState>>, camera_name: &str, relay_url: &str) {
    let mut borrowed = state.borrow_mut();
    if borrowed.stopped {
        return;
    }
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Ok(element) = document.create_element("moq-watch") else {
        return;
    };
    let _ = element.set_attribute("url", relay_url);
    let _ = element.set_attribute("name", camera_name);
    // `<moq-watch>` renders only into a nested `<canvas>` child -- see this
    // module's own top-level doc.
    if let Ok(canvas) = document.create_element("canvas") {
        let _ = element.append_child(&canvas);
    }
    if borrowed.container.append_child(&element).is_err() {
        return;
    }

    let Some((status_signal, subscribe_fn, closed_promise)) = moq_connection_handles(&element)
    else {
        // The library's own contract (this module's top-level doc) did not
        // hold against the real loaded script; drop the element and fail
        // over rather than leave a dead, unmonitorable one mounted.
        let _ = borrowed.container.remove_child(&element);
        drop(borrowed);
        mount_hls_fallback(Rc::clone(state));
        return;
    };

    // Guards the race below so exactly one of "succeeded" / "failed" ever
    // runs, regardless of which of the three signals fires first.
    let settled = Rc::new(Cell::new(false));

    let on_status = {
        let state = Rc::clone(state);
        let settled = Rc::clone(&settled);
        Closure::<dyn FnMut(JsValue)>::new(move |value: JsValue| {
            if value.as_string().as_deref() != Some("connected") {
                return;
            }
            if settled.replace(true) {
                return;
            }
            moq_succeeded(&state);
        })
    };
    let Ok(status_dispose) = subscribe_fn
        .call1(&status_signal, on_status.as_ref().unchecked_ref())
        .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
    else {
        let _ = borrowed.container.remove_child(&element);
        drop(borrowed);
        mount_hls_fallback(Rc::clone(state));
        return;
    };

    // `Promise::then2` (this workspace's pinned js-sys) borrows the two
    // callbacks as `&Closure`, not `&Function` -- there is no safe way to
    // reclaim them afterward (the JS engine keeps its own reference for as
    // long as the promise itself lives), so both are deliberately leaked via
    // `forget()` once registered. `settled` guards each against ever firing
    // more than once and against firing after this module has already torn
    // everything down for its own reasons.
    let on_closed_rejected = {
        let state = Rc::clone(state);
        let settled = Rc::clone(&settled);
        Closure::<dyn FnMut(JsValue)>::new(move |_reason: JsValue| {
            if settled.replace(true) {
                return;
            }
            moq_failed(&state);
        })
    };
    let on_closed_fulfilled = Closure::<dyn FnMut(JsValue)>::new(move |_resolved: JsValue| {
        // Only resolves after this module's own `MoqMount::drop` calls
        // `connection.close()` -- i.e. after this attempt already
        // succeeded or failed on its own terms, never a failure by itself.
    });
    let _ = closed_promise.then2(&on_closed_fulfilled, &on_closed_rejected);
    on_closed_fulfilled.forget();
    on_closed_rejected.forget();

    let on_timeout = {
        let state = Rc::clone(state);
        let settled = Rc::clone(&settled);
        Closure::once_into_js(move || {
            if settled.replace(true) {
                return;
            }
            moq_failed(&state);
        })
    };
    let Ok(timeout_id) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        on_timeout.unchecked_ref(),
        MOQ_ATTEMPT_TIMEOUT_MS,
    ) else {
        let _ = borrowed.container.remove_child(&element);
        drop(borrowed);
        mount_hls_fallback(Rc::clone(state));
        return;
    };

    borrowed.moq = Some(MoqMount {
        container: borrowed.container.clone(),
        element,
        timeout_id,
        status_dispose,
        _on_status: on_status,
    });
}

/// The `MoQ` attempt reached `"connected"`: this item races the two paths
/// once, not continuously, so this session commits to `MoQ` for good rather
/// than re-arming a supervisor that could later hot-swap it back out.
fn moq_succeeded(state: &Rc<RefCell<SessionState>>) {
    let borrowed = state.borrow();
    if borrowed.stopped {
        return;
    }
    if let (Some(moq), Some(window)) = (borrowed.moq.as_ref(), web_sys::window()) {
        window.clear_timeout_with_handle(moq.timeout_id);
    }
    borrowed.set_path.set(LivePath::Moq);
}

/// The `MoQ` attempt failed (its own `closed` promise rejected) or timed out:
/// tear it down and mount the HLS fallback.
fn moq_failed(state: &Rc<RefCell<SessionState>>) {
    {
        let mut borrowed = state.borrow_mut();
        if borrowed.stopped {
            return;
        }
        borrowed.moq = None;
    }
    mount_hls_fallback(Rc::clone(state));
}

/// Starts the HLS fallback: loads the vendored `hls.js` bundle if needed,
/// then creates and mounts a `<video>` against N1's own contract.
fn mount_hls_fallback(state: Rc<RefCell<SessionState>>) {
    let camera_name = {
        let borrowed = state.borrow();
        if borrowed.stopped {
            return;
        }
        borrowed.set_path.set(LivePath::Hls);
        borrowed.camera_name.clone()
    };

    ensure_script_loaded(&HLS_JS_LOAD, HLS_SCRIPT_SRC, move || {
        create_hls_video(&state, &camera_name);
    });
}

/// Creates a `<video>` and a real `hls.js` player against N1's real HLS
/// route, matching `g3_hls.spec.cjs`'s own established wiring exactly (same
/// `"hlsError"` literal, same `loadSource`/`attachMedia`/`play()` sequence).
fn create_hls_video(state: &Rc<RefCell<SessionState>>, camera_name: &str) {
    let mut borrowed = state.borrow_mut();
    if borrowed.stopped {
        return;
    }
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Ok(video_element) = document.create_element("video") else {
        return;
    };
    let Ok(video) = video_element.dyn_into::<HtmlVideoElement>() else {
        return;
    };
    video.set_muted(true);
    video.set_autoplay(true);
    let _ = video.set_attribute("playsinline", "");
    if borrowed.container.append_child(&video).is_err() {
        return;
    }

    let Ok(hls_ctor) = js_sys::Reflect::get(&window, &JsValue::from_str("Hls"))
        .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
    else {
        let _ = borrowed.container.remove_child(&video);
        borrowed.set_path.set(LivePath::None);
        return;
    };
    let Ok(hls_instance) = js_sys::Reflect::construct(&hls_ctor, &js_sys::Array::new()) else {
        let _ = borrowed.container.remove_child(&video);
        borrowed.set_path.set(LivePath::None);
        return;
    };

    let on_fatal_error = {
        let state = Rc::clone(state);
        Closure::<dyn FnMut(JsValue, JsValue)>::new(move |_event: JsValue, data: JsValue| {
            let fatal = js_sys::Reflect::get(&data, &JsValue::from_str("fatal"))
                .ok()
                .is_some_and(|value| value.is_truthy());
            if fatal {
                mark_no_path_available(&state);
            }
        })
    };
    if let Ok(on_fn) = js_sys::Reflect::get(&hls_instance, &JsValue::from_str("on"))
        .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
    {
        // "hlsError": `Hls.Events.ERROR`'s own literal wire value
        // (`node_modules/hls.js/dist/hls.js.d.ts`'s `Events` enum, read
        // directly), matching `g3_hls.spec.cjs`'s identical subscription.
        let _ = on_fn.call2(
            &hls_instance,
            &JsValue::from_str("hlsError"),
            on_fatal_error.as_ref().unchecked_ref(),
        );
    }

    let on_video_error = {
        let state = Rc::clone(state);
        Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
            mark_no_path_available(&state);
        })
    };
    video.set_onerror(Some(on_video_error.as_ref().unchecked_ref()));

    if let Ok(load_source) = js_sys::Reflect::get(&hls_instance, &JsValue::from_str("loadSource"))
        .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
    {
        let _ = load_source.call1(
            &hls_instance,
            &JsValue::from_str(&hls_playlist_url(camera_name)),
        );
    }
    if let Ok(attach_media) = js_sys::Reflect::get(&hls_instance, &JsValue::from_str("attachMedia"))
        .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
    {
        let _ = attach_media.call1(&hls_instance, &video);
    }
    // A muted video may autoplay; playback is what makes a real browser
    // actually invoke its decoder (matches `g3_hls.spec.cjs`'s own comment).
    let _ = video.play();

    borrowed.hls = Some(HlsMount {
        container: borrowed.container.clone(),
        video,
        hls_instance,
        _on_fatal_error: on_fatal_error,
        _on_video_error: on_video_error,
    });
}

fn mark_no_path_available(state: &Rc<RefCell<SessionState>>) {
    let borrowed = state.borrow();
    if borrowed.stopped {
        return;
    }
    borrowed.set_path.set(LivePath::None);
}

/// A test-only escape hatch, mirroring `live_view.rs`'s own
/// `WS_ORIGIN_OVERRIDE_PROPERTY` exactly: when the page's global object
/// carries this property (a plain string origin), HLS requests dial it
/// directly instead of this page's own origin. This is what lets this
/// item's own real-endpoint Playwright check reach a real
/// `corvette-media-bridge` HLS listener directly, without needing N1's
/// nginx route inside this crate's own test harness. Production never sets
/// this property, so every real deployment always uses the relative,
/// same-origin path N1's own route proxies.
const HLS_ORIGIN_OVERRIDE_PROPERTY: &str = "__corvetteHlsOrigin";

/// N1's own route contract: `/live/hls/<camera>/playlist.m3u8`.
fn hls_playlist_url(camera_name: &str) -> String {
    let camera = js_sys::encode_uri_component(camera_name);
    if let Some(origin) = hls_origin_override() {
        // The override means "dial G3 directly, with no nginx in between"
        // (see [`HLS_ORIGIN_OVERRIDE_PROPERTY`]'s own doc) -- so this uses
        // G3's own bare per-camera path convention directly
        // (`hls::HlsServer`'s own routing: `/<camera>/playlist.m3u8`), not
        // [`HLS_PLAYLIST_PATH_PREFIX`], which only exists for nginx to
        // strip on the way to that same bare path (N1's own route).
        return format!("{origin}/{camera}/playlist.m3u8");
    }
    format!("{HLS_PLAYLIST_PATH_PREFIX}{camera}/playlist.m3u8")
}

/// Reads [`HLS_ORIGIN_OVERRIDE_PROPERTY`] -- see this constant's own doc.
fn hls_origin_override() -> Option<String> {
    let window = web_sys::window()?;
    js_sys::Reflect::get(&window, &JsValue::from_str(HLS_ORIGIN_OVERRIDE_PROPERTY))
        .ok()
        .and_then(|value| value.as_string())
        .filter(|value| !value.is_empty())
}

/// Reads [`MOQ_RELAY_URL_PROPERTY`], or `None` if a deployment has not set
/// it -- see this module's own top-level doc, "The relay's public URL".
fn moq_relay_url() -> Option<String> {
    let window = web_sys::window()?;
    js_sys::Reflect::get(&window, &JsValue::from_str(MOQ_RELAY_URL_PROPERTY))
        .ok()
        .and_then(|value| value.as_string())
        .filter(|value| !value.is_empty())
}

/// Whether a lazily-loaded classic `<script>` has been requested, is still
/// loading (with callbacks queued to run once it finishes), or has already
/// finished -- shared across every expanded view a reader opens, so
/// reopening the modal never injects the same script twice.
enum ScriptLoad {
    NotRequested,
    Loading(Vec<Box<dyn FnOnce()>>),
    Ready,
}

thread_local! {
    static MOQ_WATCH_LOAD: RefCell<ScriptLoad> = const { RefCell::new(ScriptLoad::NotRequested) };
    static HLS_JS_LOAD: RefCell<ScriptLoad> = const { RefCell::new(ScriptLoad::NotRequested) };
}

/// Runs `on_ready` once `src` has loaded, loading it at most once per page
/// regardless of how many times this is called (a reader can open and close
/// the expanded view repeatedly; each open must not inject a second
/// `<script src>` for the same file).
fn ensure_script_loaded(
    cell: &'static std::thread::LocalKey<RefCell<ScriptLoad>>,
    src: &'static str,
    on_ready: impl FnOnce() + 'static,
) {
    let mut on_ready: Option<Box<dyn FnOnce()>> = Some(Box::new(on_ready));
    let should_inject = cell.with(|state| {
        let mut state = state.borrow_mut();
        match &mut *state {
            ScriptLoad::Ready => false,
            ScriptLoad::Loading(waiters) => {
                if let Some(on_ready) = on_ready.take() {
                    waiters.push(on_ready);
                }
                false
            }
            ScriptLoad::NotRequested => {
                let mut waiters = Vec::with_capacity(1);
                if let Some(on_ready) = on_ready.take() {
                    waiters.push(on_ready);
                }
                *state = ScriptLoad::Loading(waiters);
                true
            }
        }
    });
    if let Some(on_ready) = on_ready {
        on_ready();
    }
    if !should_inject {
        return;
    }

    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Ok(script_element) = document.create_element("script") else {
        return;
    };
    let Ok(script) = script_element.dyn_into::<web_sys::HtmlScriptElement>() else {
        return;
    };
    script.set_src(src);

    let on_load = Closure::once_into_js(move || {
        let waiters =
            cell.with(|state| std::mem::replace(&mut *state.borrow_mut(), ScriptLoad::Ready));
        if let ScriptLoad::Loading(waiters) = waiters {
            for waiter in waiters {
                waiter();
            }
        }
    });
    script.set_onload(Some(on_load.unchecked_ref()));

    if let Some(root) = document.document_element() {
        let _ = root.append_child(&script);
    }
}

/// Only pure enough to unit-test without a browser: the `data-live-path`
/// values this item's Playwright tests assert on.
#[cfg(test)]
mod live_path_tests {
    use super::LivePath;

    #[test]
    fn every_path_state_has_a_distinct_data_attribute() {
        let all = [
            LivePath::Connecting,
            LivePath::Moq,
            LivePath::Hls,
            LivePath::None,
        ];
        let attrs: std::collections::HashSet<_> = all.iter().map(|path| path.data_attr()).collect();
        assert_eq!(
            attrs.len(),
            all.len(),
            "data-live-path values must be distinct"
        );
    }
}
