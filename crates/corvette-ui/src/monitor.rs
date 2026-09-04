//! The bookmarkable camera wall: one tile per enabled camera, each running
//! its own MoQ-first/HLS-fallback live session (issue #20 item M3, reusing
//! `expanded_view`'s own [`ExpandedSession`]/[`LivePath`] verbatim).
//!
//! Reached by direct bookmark at `/monitor`, not from primary navigation.
//! Aspect-ratio-aware layout (M4) and per-tile fullscreen (M5) build on top
//! of this scaffold.

use corvette_api::Camera;
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::expanded_view::{ExpandedSession, LivePath};
use crate::shell::{Status, StatusGlyph};

/// Delay applied to each tile's initial dial, scaled by the tile's position
/// in the fetched camera list (`tile_index * STAGGER_MS`), so `/monitor`
/// never opens N simultaneous relay connections at once -- D-3's own
/// mitigation for `RESEARCH-moq-relay-concurrency.md`'s unverified
/// under-load behavior (`.agents/issue-20/DESIGN-monitor-mode.md`). 200ms
/// comfortably exceeds a healthy local handshake's own setup cost, so
/// tiles visibly settle one after another rather than in a burst, while
/// staying small enough that even a double-digit camera wall's last tile
/// still starts its dial well inside `expanded_view`'s own multi-second
/// per-tile timeout budget.
const STAGGER_MS: i32 = 200;

#[component]
pub(crate) fn Monitor() -> impl IntoView {
    let cameras = LocalResource::new(crate::api::fetch_cameras);

    view! {
        <main class="monitor-wall">
            {move || match cameras.get() {
                None => view! { <Status heading="Loading cameras" detail="Connecting to the Frigate API." glyph=StatusGlyph::Camera/> }.into_any(),
                Some(Err(error)) => view! { <Status heading="Cameras unavailable" detail=error glyph=StatusGlyph::Camera/> }.into_any(),
                Some(Ok(cameras)) if cameras.is_empty() => view! { <Status heading="No cameras configured" detail="Add or enable a camera in Frigate, then reload this page." glyph=StatusGlyph::Camera/> }.into_any(),
                Some(Ok(cameras)) => view! {
                    <div class="monitor-grid" aria-label="Configured cameras">
                        {cameras.into_iter().enumerate().map(|(index, camera)| view! {
                            <MonitorTile camera=camera index=index/>
                        }).collect_view()}
                    </div>
                }.into_any(),
            }}
        </main>
    }
}

/// One wall tile: names its camera and runs its own live session, staggered
/// by `index * STAGGER_MS` on initial mount. Layout and fullscreen are
/// added by later plan items on top of this element.
#[component]
fn MonitorTile(camera: Camera, index: usize) -> impl IntoView {
    let Camera {
        name: camera_name,
        display_name,
        ..
    } = camera;
    let tile_camera_name = camera_name.clone();
    let player_container = NodeRef::<html::Div>::new();
    let path = RwSignal::new(LivePath::Connecting);
    // Non-`Send`/non-`Sync` browser objects live behind these; `on_cleanup`
    // still requires a `Send + Sync` closure regardless of target, which is
    // sound here because a WASM build is always single-threaded -- see
    // `expanded_view.rs`'s own `ExpandedView` for the identical rationale.
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

    view! {
        <div class="monitor-tile" data-camera=tile_camera_name>
            <h2>{display_name}</h2>
            <div
                class="monitor-tile-player"
                data-live-path=move || path.get().data_attr()
                node_ref=player_container
            ></div>
        </div>
    }
}
