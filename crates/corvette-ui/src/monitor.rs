//! The bookmarkable camera wall: one tile per enabled camera, each running
//! its own live session, placed to fill the viewport exactly with no scrolling.

use corvette_api::Camera;
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use crate::expanded_view::{ExpandedSession, LivePath};
use crate::monitor_layout::{TileRect, pack_tiles};
use crate::shell::{Status, StatusGlyph};

// Each tile's dial is delayed by its index so the wall opens its relay connections one at a time.
const STAGGER_MS: i32 = 200;

const TV_BRO_HINT: &str = "tv-bro's long-press menu also has a Direct Navigation Mode toggle, for D-pad/keyboard navigation instead of its default touch-emulating cursor.";

fn is_activation_key(key: &str) -> bool {
    key == "Enter" || key == " "
}

const FULLSCREEN_DISPLAY_MODE: &str = "(display-mode: fullscreen)";

/// True when the browser reports the page already fullscreen: by display mode, fullscreen element, or viewport coverage.
fn page_is_fullscreen() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    if js_sys::Reflect::get(window.as_ref(), &"matchMedia".into())
        .ok()
        .filter(JsValue::is_function)
        .is_some_and(|match_media| {
            match_media
                .dyn_into::<js_sys::Function>()
                .ok()
                .and_then(|match_media| {
                    match_media
                        .call1(window.as_ref(), &FULLSCREEN_DISPLAY_MODE.into())
                        .ok()
                })
                .and_then(|query| js_sys::Reflect::get(&query, &"matches".into()).ok())
                .is_some_and(|matches| matches.as_bool() == Some(true))
        })
    {
        return true;
    }
    if window
        .document()
        .is_some_and(|document| document.fullscreen_element().is_some())
    {
        return true;
    }
    let Ok(screen) = window.screen() else {
        return false;
    };
    let (Ok(width), Ok(height)) = (screen.width(), screen.height()) else {
        return false;
    };
    if width <= 0 || height <= 0 {
        return false;
    }
    let (Some(inner_width), Some(inner_height)) = (
        js_sys::Reflect::get(window.as_ref(), &"innerWidth".into())
            .ok()
            .and_then(|value| value.as_f64()),
        js_sys::Reflect::get(window.as_ref(), &"innerHeight".into())
            .ok()
            .and_then(|value| value.as_f64()),
    ) else {
        return false;
    };
    inner_width + 1.0 >= f64::from(width) && inner_height + 1.0 >= f64::from(height)
}

#[component]
pub(crate) fn Monitor() -> impl IntoView {
    let cameras = LocalResource::new(crate::api::fetch_cameras);

    view! {
        <main class="monitor-wall">
            {move || match cameras.get() {
                None => view! { <Status heading="Loading cameras" detail="Connecting to the Frigate API." glyph=StatusGlyph::Camera/> }.into_any(),
                Some(Err(error)) => view! { <Status heading="Cameras unavailable" detail=error glyph=StatusGlyph::Camera/> }.into_any(),
                Some(Ok(cameras)) if cameras.is_empty() => view! { <Status heading="No cameras configured" detail="Add or enable a camera in Frigate, then reload this page." glyph=StatusGlyph::Camera/> }.into_any(),
                Some(Ok(cameras)) => view! { <MonitorGrid cameras=cameras/> }.into_any(),
            }}
        </main>
    }
}

#[component]
fn MonitorGrid(cameras: Vec<Camera>) -> impl IntoView {
    let grid_container = NodeRef::<html::Div>::new();
    let camera_dims: Vec<(u32, u32)> = cameras
        .iter()
        .map(|camera| (camera.width, camera.height))
        .collect();
    let layout = RwSignal::new(Vec::<TileRect>::new());
    let armed = RwSignal::new(page_is_fullscreen());
    // Some browsers enter fullscreen a moment after load, so arming is re-checked on resize.
    let fullscreen_watch = window_event_listener(leptos::ev::resize, move |_| {
        if page_is_fullscreen() {
            armed.set(true);
        }
    });
    on_cleanup(move || fullscreen_watch.remove());

    Effect::new(move |_| {
        let Some(container) = grid_container.get() else {
            return;
        };
        let bounds = container.get_bounding_client_rect();
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return;
        }
        layout.set(pack_tiles(&camera_dims, bounds.width(), bounds.height()));
    });

    view! {
        <div class="monitor-grid" node_ref=grid_container aria-label="Configured cameras">
            {cameras.into_iter().enumerate().map(|(index, camera)| view! {
                <MonitorTile camera=camera index=index layout=layout armed=armed/>
            }).collect_view()}
            {move || (!armed.get()).then(|| view! { <EnterFullscreenButton armed=armed/> })}
        </div>
    }
}

/// The small top-left button supplying the one real user gesture a browser
/// requires before `requestFullscreen()` can succeed, shown only until it has
/// been pressed. Auto-focuses so a remote's first "OK" press reaches it.
#[component]
fn EnterFullscreenButton(armed: RwSignal<bool>) -> impl IntoView {
    let button_ref = NodeRef::<html::Button>::new();

    Effect::new(move |_| {
        if let Some(element) = button_ref.get() {
            _ = element.focus();
        }
    });

    view! {
        <div class="monitor-enter-fullscreen-wrap">
            <button
                type="button"
                class="monitor-enter-fullscreen"
                node_ref=button_ref
                aria-label="Enter fullscreen"
                title=TV_BRO_HINT
                on:click=move |_| armed.set(true)
            ></button>
            <span class="monitor-enter-fullscreen-hint" aria-hidden="true">{TV_BRO_HINT}</span>
        </div>
    }
}

fn tile_placement_style(rect: TileRect) -> String {
    format!(
        "left: {:.4}px; top: {:.4}px; width: {:.4}px; height: {:.4}px",
        rect.x, rect.y, rect.width, rect.height
    )
}

#[component]
fn MonitorTile(
    camera: Camera,
    index: usize,
    layout: RwSignal<Vec<TileRect>>,
    armed: RwSignal<bool>,
) -> impl IntoView {
    let Camera {
        name: camera_name,
        display_name,
        ..
    } = camera;
    let tile_camera_name = camera_name.clone();
    let tile_aria_label = format!("Fullscreen {display_name}");
    let tile_container = NodeRef::<html::Div>::new();
    let player_container = NodeRef::<html::Div>::new();
    let path = RwSignal::new(LivePath::Connecting);
    let session = StoredValue::new_local(None::<ExpandedSession>);
    let pending_start = StoredValue::new(None::<i32>);

    Effect::new(move |_| {
        let Some(container) = player_container.get() else {
            return;
        };
        let camera_name = camera_name.clone();
        let start_session = move || {
            pending_start.set_value(None);
            session.set_value(Some(ExpandedSession::start(
                camera_name,
                container.unchecked_into(),
                path.write_only(),
            )));
        };

        let delay_ms = i32::try_from(index)
            .unwrap_or(i32::MAX)
            .saturating_mul(STAGGER_MS);
        if delay_ms <= 0 {
            start_session();
            return;
        }
        let Some(window) = web_sys::window() else {
            start_session();
            return;
        };
        let on_ready = Closure::once_into_js(start_session);
        if let Ok(timeout_id) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            on_ready.unchecked_ref(),
            delay_ms,
        ) {
            pending_start.set_value(Some(timeout_id));
        }
    });

    on_cleanup(move || {
        if let Some(Some(timeout_id)) = pending_start.try_update_value(Option::take)
            && let Some(window) = web_sys::window()
        {
            window.clear_timeout_with_handle(timeout_id);
        }
        if let Some(Some(session)) = session.try_update_value(Option::take) {
            session.stop();
        }
    });

    let enter_fullscreen = move || {
        if !armed.get_untracked() {
            return;
        }
        if let Some(element) = tile_container.get_untracked() {
            _ = element.request_fullscreen();
        }
    };

    let exit_fullscreen = move || {
        if let Some(document) = web_sys::window().and_then(|window| window.document()) {
            document.exit_fullscreen();
        }
    };

    view! {
        <div
            class="monitor-tile"
            node_ref=tile_container
            data-camera=tile_camera_name
            tabindex="0"
            role="button"
            aria-label=tile_aria_label
            style=move || {
                tile_placement_style(layout.get().get(index).copied().unwrap_or_default())
            }
            on:click=move |_| enter_fullscreen()
            on:keydown=move |event| {
                if is_activation_key(&event.key()) {
                    event.prevent_default();
                    enter_fullscreen();
                }
            }
        >
            <h2>{display_name}</h2>
            <button
                type="button"
                class="monitor-tile-exit-fullscreen"
                aria-label="Exit fullscreen"
                on:click=move |event| {
                    event.stop_propagation();
                    exit_fullscreen();
                }
                on:keydown=|event| event.stop_propagation()
            ></button>
            <div
                class="monitor-tile-player"
                data-live-path=move || path.get().data_attr()
                node_ref=player_container
            ></div>
        </div>
    }
}
