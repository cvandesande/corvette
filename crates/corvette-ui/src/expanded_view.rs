//! The grid tile's click-to-enlarge live view: HLS live playback, mounted directly on open.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, Event, HtmlVideoElement};

const HLS_SCRIPT_SRC: &str = "/vendor/hls.min.js";

const HLS_PLAYLIST_PATH_PREFIX: &str = "/live/hls/";

/// A stalled script request never fires `load` or `error`, so only a timer can end the wait for it.
const SCRIPT_LOAD_TIMEOUT_MS: i32 = 10_000;

/// Provisional until the device measurement sets it: no picture by now means the untapped start failed and the play prompt should appear.
const FIRST_FRAME_BOUND_MS: i32 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LivePath {
    Connecting,
    Hls,
    None,
}

impl LivePath {
    pub(crate) const fn data_attr(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Hls => "hls",
            Self::None => "none",
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting…",
            Self::Hls => "Live",
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
    let needs_tap = RwSignal::new(false);
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
        session.set_value(Some(ExpandedSession::start_with_play_prompt(
            camera_name.clone(),
            container.unchecked_into(),
            path.write_only(),
            needs_tap.write_only(),
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
                <div class="live-player-frame">
                    <div class="expanded-player" data-rotate-fullscreen="" node_ref=player_container></div>
                    <button
                        type="button"
                        class="live-play-button"
                        aria-label="Play live video"
                        hidden=move || !needs_tap.get()
                        on:click=move |_| {
                            session.with_value(|session| {
                                if let Some(session) = session {
                                    session.replay();
                                }
                            });
                        }
                    >
                        "Play"
                    </button>
                </div>
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
        Self::start_inner(camera_name, container, set_path, None)
    }

    pub(crate) fn start_with_play_prompt(
        camera_name: String,
        container: Element,
        set_path: WriteSignal<LivePath>,
        set_needs_tap: WriteSignal<bool>,
    ) -> Self {
        Self::start_inner(camera_name, container, set_path, Some(set_needs_tap))
    }

    fn start_inner(
        camera_name: String,
        container: Element,
        set_path: WriteSignal<LivePath>,
        set_needs_tap: Option<WriteSignal<bool>>,
    ) -> Self {
        set_path.set(LivePath::Connecting);
        let state = Rc::new(RefCell::new(SessionState {
            camera_name,
            container,
            set_path,
            set_needs_tap,
            stopped: false,
            hls: None,
        }));
        mount_hls(Rc::clone(&state));
        Self { state }
    }

    pub(crate) fn stop(self) {
        let mut state = self.state.borrow_mut();
        state.stopped = true;
        state.hls = None;
    }

    pub(crate) fn replay(&self) {
        {
            let mut state = self.state.borrow_mut();
            if state.stopped {
                return;
            }
            if let Some(set_needs_tap) = state.set_needs_tap {
                set_needs_tap.set(false);
            }
            state.hls = None;
        }
        mount_hls(Rc::clone(&self.state));
    }
}

struct SessionState {
    camera_name: String,
    container: Element,
    set_path: WriteSignal<LivePath>,
    set_needs_tap: Option<WriteSignal<bool>>,
    stopped: bool,
    hls: Option<HlsMount>,
}

/// On the native-HLS path `hls_instance` is UNDEFINED and `_on_fatal_error` is None: there is no hls.js instance to destroy or subscribe `"hlsError"` on.
struct HlsMount {
    video: HtmlVideoElement,
    container: Element,
    hls_instance: JsValue,
    _on_fatal_error: Option<Closure<dyn FnMut(JsValue, JsValue)>>,
    _on_video_error: Closure<dyn FnMut(Event)>,
    _first_frame_watch: Option<FirstFrameWatch>,
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

struct FirstFrameWatch {
    video: HtmlVideoElement,
    timeout_id: i32,
    _on_playing: Closure<dyn FnMut(Event)>,
}

impl Drop for FirstFrameWatch {
    fn drop(&mut self) {
        if let Some(window) = web_sys::window() {
            window.clear_timeout_with_handle(self.timeout_id);
        }
        self.video.set_onplaying(None);
    }
}

/// Armed before `play()`: a `playing` event can fire synchronously inside it, so its handler must already be installed.
fn arm_first_frame_watch(
    set_needs_tap: WriteSignal<bool>,
    video: &HtmlVideoElement,
) -> Option<FirstFrameWatch> {
    set_needs_tap.set(false);
    let window = web_sys::window()?;
    let on_bound = Closure::once_into_js(move || set_needs_tap.set(true));
    let Ok(timeout_id) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        on_bound.unchecked_ref(),
        FIRST_FRAME_BOUND_MS,
    ) else {
        return None;
    };
    let on_playing = Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
        window.clear_timeout_with_handle(timeout_id);
        set_needs_tap.set(false);
    });
    video.set_onplaying(Some(on_playing.as_ref().unchecked_ref()));
    Some(FirstFrameWatch {
        video: video.clone(),
        timeout_id,
        _on_playing: on_playing,
    })
}

fn mount_hls(state: Rc<RefCell<SessionState>>) {
    let camera_name = {
        let borrowed = state.borrow();
        if borrowed.stopped {
            return;
        }
        borrowed.set_path.set(LivePath::Hls);
        borrowed.camera_name.clone()
    };

    let fallback_state = Rc::clone(&state);
    let fallback_camera_name = camera_name.clone();
    ensure_script_loaded(
        &HLS_JS_LOAD,
        HLS_SCRIPT_SRC,
        move || {
            create_hls_video(&state, &camera_name);
        },
        move || {
            try_native_hls_or_give_up(&fallback_state, &fallback_camera_name);
        },
    );
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
    let first_frame_watch = borrowed
        .set_needs_tap
        .and_then(|set_needs_tap| arm_first_frame_watch(set_needs_tap, &video));

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
        _first_frame_watch: first_frame_watch,
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
    let first_frame_watch = borrowed
        .set_needs_tap
        .and_then(|set_needs_tap| arm_first_frame_watch(set_needs_tap, &video));

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
        _first_frame_watch: first_frame_watch,
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

type ScriptWaiters = Vec<(Box<dyn FnOnce()>, Box<dyn FnOnce()>)>;

enum ScriptLoad {
    NotRequested,
    Loading(ScriptWaiters),
    Ready,
    /// A script that failed once is never re-injected for the life of the page, so every later caller settles to its failure at once.
    Failed,
}

thread_local! {
    static HLS_JS_LOAD: RefCell<ScriptLoad> = const { RefCell::new(ScriptLoad::NotRequested) };
}

pub(crate) fn ensure_hls_script_loaded(on_ready: impl FnOnce() + 'static) {
    ensure_script_loaded(&HLS_JS_LOAD, HLS_SCRIPT_SRC, on_ready, || {});
}

fn ensure_script_loaded(
    cell: &'static std::thread::LocalKey<RefCell<ScriptLoad>>,
    src: &'static str,
    on_ready: impl FnOnce() + 'static,
    on_failed: impl FnOnce() + 'static,
) {
    let ready: Box<dyn FnOnce()> = Box::new(on_ready);
    let failed: Box<dyn FnOnce()> = Box::new(on_failed);
    let mut settle_now: Option<Box<dyn FnOnce()>> = None;
    let should_inject = cell.with(|state| {
        let mut state = state.borrow_mut();
        match &mut *state {
            ScriptLoad::Ready => {
                settle_now = Some(ready);
                false
            }
            ScriptLoad::Failed => {
                settle_now = Some(failed);
                false
            }
            ScriptLoad::Loading(waiters) => {
                waiters.push((ready, failed));
                false
            }
            ScriptLoad::NotRequested => {
                *state = ScriptLoad::Loading(vec![(ready, failed)]);
                true
            }
        }
    });
    if let Some(settle_now) = settle_now {
        settle_now();
    }
    if !should_inject {
        return;
    }

    let Some(window) = web_sys::window() else {
        settle_script_failed(cell);
        return;
    };
    let Some(document) = window.document() else {
        settle_script_failed(cell);
        return;
    };
    let Ok(script_element) = document.create_element("script") else {
        settle_script_failed(cell);
        return;
    };
    let Ok(script) = script_element.dyn_into::<web_sys::HtmlScriptElement>() else {
        settle_script_failed(cell);
        return;
    };
    script.set_src(src);

    let settled = Rc::new(Cell::new(false));

    let on_load = {
        let settled = Rc::clone(&settled);
        Closure::once_into_js(move || {
            if settled.replace(true) {
                return;
            }
            let waiters =
                cell.with(|state| std::mem::replace(&mut *state.borrow_mut(), ScriptLoad::Ready));
            if let ScriptLoad::Loading(waiters) = waiters {
                for (on_ready, _) in waiters {
                    on_ready();
                }
            }
        })
    };
    script.set_onload(Some(on_load.unchecked_ref()));

    let on_error = {
        let settled = Rc::clone(&settled);
        Closure::once_into_js(move || {
            if settled.replace(true) {
                return;
            }
            settle_script_failed(cell);
        })
    };
    script.set_onerror(Some(on_error.unchecked_ref()));

    let on_timeout = Closure::once_into_js(move || {
        if settled.replace(true) {
            return;
        }
        settle_script_failed(cell);
    });
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        on_timeout.unchecked_ref(),
        SCRIPT_LOAD_TIMEOUT_MS,
    );

    if let Some(root) = document.document_element() {
        let _ = root.append_child(&script);
    }
}

fn settle_script_failed(cell: &'static std::thread::LocalKey<RefCell<ScriptLoad>>) {
    let waiters =
        cell.with(|state| std::mem::replace(&mut *state.borrow_mut(), ScriptLoad::Failed));
    if let ScriptLoad::Loading(waiters) = waiters {
        for (_, on_failed) in waiters {
            on_failed();
        }
    }
}

#[cfg(test)]
mod live_path_tests {
    use super::LivePath;

    #[test]
    fn every_path_state_has_a_distinct_data_attribute() {
        let all = [LivePath::Connecting, LivePath::Hls, LivePath::None];
        let attrs: std::collections::HashSet<_> = all.iter().map(|path| path.data_attr()).collect();
        assert_eq!(
            attrs.len(),
            all.len(),
            "data-live-path values must be distinct"
        );
    }
}
