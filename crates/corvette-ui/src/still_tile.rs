//! A dashboard tile showing its camera's latest still picture, refreshed.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Document, HtmlImageElement};

const STILL_HEIGHT_PX: u32 = 270;
const REFRESH_MS: i32 = 30_000;
const STALE_AFTER_MS: f64 = 60_000.0;
const AGE_TICK_MS: i32 = 5_000;
const FIRST_LOAD_FAILURES: u32 = 2;
const JUST_NOW_MS: f64 = 5_000.0;
const SECOND_MS: f64 = 1_000.0;
const MINUTE_MS: f64 = 60_000.0;

#[derive(Clone, Copy, PartialEq)]
enum TileStage {
    Waiting,
    Showing,
    NoPicture,
}

type LoaderWeak = Weak<RefCell<LoaderState>>;

#[component]
pub(crate) fn StillTile(camera_name: String, title: String, paused: Signal<bool>) -> impl IntoView {
    let image = NodeRef::<html::Img>::new();
    let stage = RwSignal::new(TileStage::Waiting);
    let expired = RwSignal::new(false);
    let age_label = RwSignal::new(String::new());
    let loader = StoredValue::new_local(None::<Rc<RefCell<LoaderState>>>);

    Effect::new(move |_| {
        let Some(image_element) = image.get() else {
            return;
        };
        let started = LoaderState::start(
            &camera_name,
            image_element,
            stage,
            expired,
            age_label,
            paused,
        );
        loader.set_value(started);
    });

    let was_paused = Cell::new(paused.get_untracked());
    Effect::new(move |_| {
        let is_paused = paused.get();
        if !was_paused.replace(is_paused) || is_paused {
            return;
        }
        if let Some(loaded) = loader.try_update_value(|state| state.clone()).flatten() {
            resume_refresh(&loaded);
        }
    });

    on_cleanup(move || {
        if let Some(Some(finished)) = loader.try_update_value(Option::take) {
            finished.borrow().stop();
        }
    });

    view! {
        <div
            class="still-tile"
            class:still-tile-waiting=move || stage.get() == TileStage::Waiting
            class:still-tile-showing=move || stage.get() == TileStage::Showing
            class:still-tile-no-picture=move || stage.get() == TileStage::NoPicture
            class:still-tile-stale=move || expired.get()
        >
            <img node_ref=image class="still-tile-image" alt=title />
            <span
                class="still-tile-placeholder"
                aria-busy=move || (stage.get() == TileStage::Waiting).then_some("true")
            >
                "Loading"
            </span>
            <span class="still-tile-age">{move || age_label.get()}</span>
        </div>
    }
}

struct LoaderState {
    document: Document,
    image: HtmlImageElement,
    camera_path: String,
    stage: RwSignal<TileStage>,
    expired: RwSignal<bool>,
    age_label: RwSignal<String>,
    paused: Signal<bool>,
    counter: u32,
    failed_loads: u32,
    last_loaded_at: f64,
    refresh_timer: Option<i32>,
    age_timer: Option<i32>,
    visibility_handler: RefCell<Option<Closure<dyn FnMut()>>>,
}

impl LoaderState {
    fn start(
        camera_name: &str,
        image: HtmlImageElement,
        stage: RwSignal<TileStage>,
        expired: RwSignal<bool>,
        age_label: RwSignal<String>,
        paused: Signal<bool>,
    ) -> Option<Rc<RefCell<Self>>> {
        let document = web_sys::window()?.document()?;
        let loader = Rc::new(RefCell::new(Self {
            camera_path: js_sys::encode_uri_component(camera_name).into(),
            document: document.clone(),
            image,
            stage,
            expired,
            age_label,
            paused,
            counter: 0,
            failed_loads: 0,
            last_loaded_at: 0.0,
            refresh_timer: None,
            age_timer: None,
            visibility_handler: RefCell::new(None),
        }));

        let weak = Rc::downgrade(&loader);
        let handler = Closure::wrap(Box::new(move || {
            if let Some(state) = weak.upgrade() {
                resume_refresh(&state);
            }
        }) as Box<dyn FnMut()>);
        document
            .add_event_listener_with_callback("visibilitychange", handler.as_ref().unchecked_ref())
            .ok();
        loader
            .borrow_mut()
            .visibility_handler
            .replace(Some(handler));

        begin_load(&loader);
        schedule_refresh(&loader);
        Some(loader)
    }

    fn stop(&self) {
        if let Some(window) = web_sys::window() {
            for timer in [self.refresh_timer, self.age_timer].into_iter().flatten() {
                window.clear_timeout_with_handle(timer);
            }
        }
        if let Some(handler) = self.visibility_handler.replace(None) {
            self.document
                .remove_event_listener_with_callback(
                    "visibilitychange",
                    handler.as_ref().unchecked_ref(),
                )
                .ok();
        }
    }
}

fn still_url(camera_path: &str, counter: u32) -> String {
    // The image route is the current backing's, not a Corvette contract, and it
    // reports no frame age, so the label times this tile's own load.
    format!("/api/{camera_path}/latest.jpg?height={STILL_HEIGHT_PX}&r={counter}")
}

fn begin_load(state: &Rc<RefCell<LoaderState>>) {
    let (document, url) = {
        let mut borrowed = state.borrow_mut();
        borrowed.counter += 1;
        let url = still_url(&borrowed.camera_path, borrowed.counter);
        (borrowed.document.clone(), url)
    };
    let Ok(element) = document.create_element("img") else {
        record_failure(state);
        return;
    };
    let Ok(offscreen) = element.dyn_into::<HtmlImageElement>() else {
        record_failure(state);
        return;
    };

    let success_weak = Rc::downgrade(state);
    let loaded_url = url.clone();
    let on_load = Closure::once_into_js(move || {
        if let Some(state) = success_weak.upgrade() {
            record_success(&state, &loaded_url);
        }
    });
    let failure_weak = Rc::downgrade(state);
    let on_error = Closure::once_into_js(move || {
        if let Some(state) = failure_weak.upgrade() {
            record_failure(&state);
        }
    });
    offscreen.set_onload(Some(on_load.unchecked_ref()));
    offscreen.set_onerror(Some(on_error.unchecked_ref()));
    offscreen.set_src(&url);
}

fn record_success(state: &Rc<RefCell<LoaderState>>, url: &str) {
    let start_age_timer = {
        let mut borrowed = state.borrow_mut();
        borrowed.image.set_src(url);
        borrowed.last_loaded_at = js_sys::Date::now();
        borrowed.failed_loads = 0;
        borrowed.stage.set(TileStage::Showing);
        borrowed.expired.set(false);
        borrowed.age_label.set("just now".to_string());
        borrowed.age_timer.is_none()
    };
    if start_age_timer {
        schedule_age_tick(state);
    }
}

fn record_failure(state: &Rc<RefCell<LoaderState>>) {
    let mut borrowed = state.borrow_mut();
    borrowed.failed_loads += 1;
    if borrowed.stage.get_untracked() == TileStage::Waiting
        && borrowed.failed_loads >= FIRST_LOAD_FAILURES
    {
        borrowed.stage.set(TileStage::NoPicture);
        borrowed.age_label.set("No picture".to_string());
    }
}

fn schedule_refresh(state: &Rc<RefCell<LoaderState>>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let weak = Rc::downgrade(state);
    let callback = Closure::once_into_js(move || refresh_tick(&weak));
    if let Ok(timeout_id) = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(callback.unchecked_ref(), REFRESH_MS)
    {
        state.borrow_mut().refresh_timer = Some(timeout_id);
    }
}

fn refresh_tick(weak: &LoaderWeak) {
    let Some(state) = weak.upgrade() else {
        return;
    };
    let skip = {
        let borrowed = state.borrow();
        borrowed.document.hidden() || borrowed.paused.get_untracked()
    };
    if !skip {
        begin_load(&state);
    }
    schedule_refresh(&state);
}

fn resume_refresh(state: &Rc<RefCell<LoaderState>>) {
    {
        let borrowed = state.borrow();
        if borrowed.document.hidden() || borrowed.paused.get_untracked() {
            return;
        }
    }
    if let Some(window) = web_sys::window()
        && let Some(timer) = state.borrow_mut().refresh_timer.take()
    {
        window.clear_timeout_with_handle(timer);
    }
    begin_load(state);
    schedule_refresh(state);
}

fn schedule_age_tick(state: &Rc<RefCell<LoaderState>>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let weak = Rc::downgrade(state);
    let callback = Closure::once_into_js(move || age_tick(&weak));
    if let Ok(timeout_id) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        callback.unchecked_ref(),
        AGE_TICK_MS,
    ) {
        state.borrow_mut().age_timer = Some(timeout_id);
    }
}

fn age_tick(weak: &LoaderWeak) {
    let Some(state) = weak.upgrade() else {
        return;
    };
    let showing = state.borrow().stage.get_untracked() == TileStage::Showing;
    if showing {
        update_age_label(&state);
    }
    schedule_age_tick(&state);
}

fn update_age_label(state: &Rc<RefCell<LoaderState>>) {
    let borrowed = state.borrow();
    let age_ms = js_sys::Date::now() - borrowed.last_loaded_at;
    if age_ms >= STALE_AFTER_MS {
        borrowed.expired.set(true);
        let minutes = (age_ms / MINUTE_MS).floor();
        borrowed
            .age_label
            .set(format!("no update for {minutes:.0} min"));
    } else {
        borrowed.expired.set(false);
        if age_ms < JUST_NOW_MS {
            borrowed.age_label.set("just now".to_string());
        } else if age_ms < STALE_AFTER_MS {
            let seconds = (age_ms / SECOND_MS).floor();
            borrowed.age_label.set(format!("{seconds:.0} s ago"));
        } else {
            let minutes = (age_ms / MINUTE_MS).floor();
            borrowed.age_label.set(format!("{minutes:.0} min ago"));
        }
    }
}
