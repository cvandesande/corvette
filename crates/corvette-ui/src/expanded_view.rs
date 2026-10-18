//! The grid tile's click-to-enlarge live view: a MoQ-first attempt that races a bounded timeout, then falls back to HLS.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, Event, HtmlVideoElement};

/// A deployment sets this window global, via an inline `<script>` in the served page shell, to the relay's public WebTransport connect URL; an unset value falls back to HLS immediately.
const MOQ_RELAY_URL_PROPERTY: &str = "__corvetteMoqRelayUrl";

/// Vendored `@moq/watch` bundle: loading this classic script defines the `<moq-watch>` element as a side effect.
const MOQ_WATCH_SCRIPT_SRC: &str = "/vendor/moq-watch.bundle.js";

const HLS_SCRIPT_SRC: &str = "/vendor/hls.min.js";

const HLS_PLAYLIST_PATH_PREFIX: &str = "/live/hls/";

/// Below the vendored library's own 10-second reconnect deadline, so this timeout is what a viewer experiences against an unreachable relay.
const MOQ_ATTEMPT_TIMEOUT_MS: i32 = 4_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LivePath {
    Connecting,
    Moq,
    Hls,
    None,
}

impl LivePath {
    pub(crate) const fn data_attr(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Moq => "moq",
            Self::Hls => "hls",
            Self::None => "none",
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting…",
            Self::Moq => "Live (MoQ)",
            Self::Hls => "Live (HLS fallback)",
            Self::None => "Live view unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExpandedCamera {
    pub(crate) name: String,
    pub(crate) title: String,
}

#[component]
pub(crate) fn ExpandedView(
    camera: ExpandedCamera,
    expanded: RwSignal<Option<ExpandedCamera>>,
) -> impl IntoView {
    let ExpandedCamera {
        name: camera_name,
        title,
    } = camera;
    let path = RwSignal::new(LivePath::Connecting);
    let player_container = NodeRef::<html::Div>::new();
    let close_button = NodeRef::<html::Button>::new();
    // StoredValue::new_local because on_cleanup requires a Send + Sync closure while the session owns non-Send browser objects; sound because a WASM build is single-threaded.
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

pub(crate) struct ExpandedSession {
    state: Rc<RefCell<SessionState>>,
}

impl ExpandedSession {
    pub(crate) fn start(
        camera_name: String,
        container: Element,
        set_path: WriteSignal<LivePath>,
    ) -> Self {
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

    pub(crate) fn stop(self) {
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

/// `<moq-watch>` cannot be redirected to a new camera or relay in place, so every change of attempt tears the whole mount down.
struct MoqMount {
    element: Element,
    container: Element,
    timeout_id: i32,
    /// Disposed before the `Closure` field drops, so the callback can never fire after the closure is freed.
    status_dispose: js_sys::Function,
    _on_status: Closure<dyn FnMut(JsValue)>,
}

impl Drop for MoqMount {
    fn drop(&mut self) {
        if let Some(window) = web_sys::window() {
            window.clear_timeout_with_handle(self.timeout_id);
        }
        let _ = self.status_dispose.call0(&JsValue::UNDEFINED);
        // Closed explicitly: web components have no destructor, so the library's own disconnectedCallback teardown would only run at garbage-collection time.
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

/// On the native-HLS path `hls_instance` is UNDEFINED and `_on_fatal_error` is None: there is no hls.js instance to destroy or subscribe `"hlsError"` on.
struct HlsMount {
    video: HtmlVideoElement,
    container: Element,
    hls_instance: JsValue,
    _on_fatal_error: Option<Closure<dyn FnMut(JsValue, JsValue)>>,
    _on_video_error: Closure<dyn FnMut(Event)>,
}

impl Drop for HlsMount {
    fn drop(&mut self) {
        self.video.set_onerror(None);
        // hls.js's destroy() also unbinds every listener it owns, including this module's own `"hlsError"` subscription, so no separate `off()` is needed first.
        if let Ok(destroy) = js_sys::Reflect::get(&self.hls_instance, &JsValue::from_str("destroy"))
            .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
        {
            let _ = destroy.call0(&self.hls_instance);
        }
        let _ = self.container.remove_child(&self.video);
    }
}

fn mount_moq(state: &Rc<RefCell<SessionState>>) {
    let (camera_name, relay_url) = {
        let borrowed = state.borrow();
        if borrowed.stopped {
            return;
        }
        let Some(relay_url) = moq_relay_url().filter(|_| video_decoder_available()) else {
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
    // `<moq-watch>` renders only into a nested `<canvas>` child, so one is always appended.
    if let Ok(canvas) = document.create_element("canvas") {
        let _ = element.append_child(&canvas);
    }
    if borrowed.container.append_child(&element).is_err() {
        return;
    }

    let Some((status_signal, subscribe_fn, closed_promise)) = moq_connection_handles(&element)
    else {
        let _ = borrowed.container.remove_child(&element);
        drop(borrowed);
        mount_hls_fallback(Rc::clone(state));
        return;
    };

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

    // then2 keeps the callbacks alive for as long as the promise lives and offers no way to reclaim them, so both are deliberately leaked via forget(); settled makes each one harmless once the attempt has settled.
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
        // Only resolves after MoqMount::drop calls connection.close(): never a failure by itself.
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

/// The race is run once, not continuously: a `MoQ` success commits the session to `MoQ` for good rather than re-arming a supervisor that could hot-swap it back out.
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

fn create_video_element(borrowed: &SessionState) -> Option<HtmlVideoElement> {
    let window = web_sys::window()?;
    let document = window.document()?;
    let video: HtmlVideoElement = document.create_element("video").ok()?.dyn_into().ok()?;
    video.set_muted(true);
    video.set_autoplay(true);
    let _ = video.set_attribute("playsinline", "");
    borrowed.container.append_child(&video).ok()?;
    Some(video)
}

/// `preferManagedMediaSource: true` overrides hls.js's shipped false default, which picks the plain `MediaSource` global on iOS Safari 17.1+ -- a non-functional stand-in -- instead of the working `ManagedMediaSource`.
/// No `Hls.isSupported()` gate: its codec detection is unreliable on iOS; a fatal `hlsError` or `<video>` error falls back to native HLS instead.
fn create_hls_video(state: &Rc<RefCell<SessionState>>, camera_name: &str) {
    let mut borrowed = state.borrow_mut();
    if borrowed.stopped {
        return;
    }
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(hls_ctor) = js_sys::Reflect::get(&window, &JsValue::from_str("Hls"))
        .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
    else {
        drop(borrowed);
        try_native_hls_or_give_up(state, camera_name);
        return;
    };
    let config = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &config,
        &JsValue::from_str("preferManagedMediaSource"),
        &JsValue::TRUE,
    );
    let Ok(hls_instance) = js_sys::Reflect::construct(&hls_ctor, &js_sys::Array::of1(&config))
    else {
        drop(borrowed);
        try_native_hls_or_give_up(state, camera_name);
        return;
    };

    let Some(video) = create_video_element(&borrowed) else {
        return;
    };

    let on_fatal_error = {
        let state = Rc::clone(state);
        let camera_name = camera_name.to_string();
        Closure::<dyn FnMut(JsValue, JsValue)>::new(move |_event: JsValue, data: JsValue| {
            let fatal = js_sys::Reflect::get(&data, &JsValue::from_str("fatal"))
                .ok()
                .is_some_and(|value| value.is_truthy());
            if fatal {
                try_native_hls_or_give_up(&state, &camera_name);
            }
        })
    };
    if let Ok(on_fn) = js_sys::Reflect::get(&hls_instance, &JsValue::from_str("on"))
        .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
    {
        // "hlsError" is the literal wire value of `Hls.Events.ERROR`.
        let _ = on_fn.call2(
            &hls_instance,
            &JsValue::from_str("hlsError"),
            on_fatal_error.as_ref().unchecked_ref(),
        );
    }

    let on_video_error = {
        let state = Rc::clone(state);
        let camera_name = camera_name.to_string();
        Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
            try_native_hls_or_give_up(&state, &camera_name);
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
    let _ = video.play();

    borrowed.hls = Some(HlsMount {
        container: borrowed.container.clone(),
        video,
        hls_instance,
        _on_fatal_error: Some(on_fatal_error),
        _on_video_error: on_video_error,
    });
}

/// Last resort before giving up: play the playlist with Safari's native HLS support (`<video src>`, no `MediaSource` or hls.js involved).
fn try_native_hls_or_give_up(state: &Rc<RefCell<SessionState>>, camera_name: &str) {
    let mut borrowed = state.borrow_mut();
    if borrowed.stopped {
        return;
    }
    borrowed.hls = None;
    let Some(video) = create_video_element(&borrowed) else {
        borrowed.set_path.set(LivePath::None);
        return;
    };
    if video
        .can_play_type("application/vnd.apple.mpegurl")
        .is_empty()
    {
        let _ = borrowed.container.remove_child(&video);
        borrowed.set_path.set(LivePath::None);
        return;
    }

    let on_video_error = {
        let state = Rc::clone(state);
        Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
            mark_no_path_available(&state);
        })
    };
    video.set_onerror(Some(on_video_error.as_ref().unchecked_ref()));
    video.set_src(&hls_playlist_url(camera_name));
    let _ = video.play();

    borrowed.hls = Some(HlsMount {
        container: borrowed.container.clone(),
        video,
        hls_instance: JsValue::UNDEFINED,
        _on_fatal_error: None,
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

/// A test-only window global: when the page sets it to an origin, HLS requests dial it directly instead of the page's own origin (no nginx in between); production never sets it.
const HLS_ORIGIN_OVERRIDE_PROPERTY: &str = "__corvetteHlsOrigin";

fn hls_playlist_url(camera_name: &str) -> String {
    let camera = js_sys::encode_uri_component(camera_name);
    if let Some(origin) = hls_origin_override() {
        // The override bypasses nginx, so this uses the packager's bare per-camera path directly; `HLS_PLAYLIST_PATH_PREFIX` only exists for nginx to strip.
        return format!("{origin}/{camera}/playlist.m3u8");
    }
    format!("{HLS_PLAYLIST_PATH_PREFIX}{camera}/playlist.m3u8")
}

fn hls_origin_override() -> Option<String> {
    let window = web_sys::window()?;
    js_sys::Reflect::get(&window, &JsValue::from_str(HLS_ORIGIN_OVERRIDE_PROPERTY))
        .ok()
        .and_then(|value| value.as_string())
        .filter(|value| !value.is_empty())
}

fn moq_relay_url() -> Option<String> {
    let window = web_sys::window()?;
    js_sys::Reflect::get(&window, &JsValue::from_str(MOQ_RELAY_URL_PROPERTY))
        .ok()
        .and_then(|value| value.as_string())
        .filter(|value| !value.is_empty())
}

fn video_decoder_available() -> bool {
    js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("VideoDecoder"))
        .is_ok_and(|value| value.is_function())
}

enum ScriptLoad {
    NotRequested,
    Loading(Vec<Box<dyn FnOnce()>>),
    Ready,
}

thread_local! {
    static MOQ_WATCH_LOAD: RefCell<ScriptLoad> = const { RefCell::new(ScriptLoad::NotRequested) };
    static HLS_JS_LOAD: RefCell<ScriptLoad> = const { RefCell::new(ScriptLoad::NotRequested) };
}

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
