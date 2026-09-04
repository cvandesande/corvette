//! The bookmarkable camera wall: one tile per enabled camera, each running
//! its own MoQ-first/HLS-fallback live session (issue #20 item M3, reusing
//! `expanded_view`'s own [`ExpandedSession`]/[`LivePath`] verbatim) and
//! placed by `monitor_layout`'s aspect-ratio-aware packing (item M4) to
//! exactly fill the viewport with no scrolling.
//!
//! Reached by direct bookmark at `/monitor`, not from primary navigation.
//! Per-tile fullscreen (M5) builds on top of this scaffold.

use corvette_api::Camera;
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::expanded_view::{ExpandedSession, LivePath};
use crate::monitor_layout::{TileRect, pack_tiles};
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
                Some(Ok(cameras)) => view! { <MonitorGrid cameras=cameras/> }.into_any(),
            }}
        </main>
    }
}

/// Lays out and mounts one tile per camera inside `.monitor-grid`, sized by
/// `monitor_layout::pack_tiles` to exactly tile the grid's own real pixel
/// box the moment that box and every camera's resolution are both known.
///
/// This does not wait on any tile's own MoQ/HLS connection state (D-6,
/// `.agents/issue-20/DESIGN-monitor-mode.md`): `Camera.width`/`height` are
/// already known from the same fetch that produced `cameras`, so layout
/// runs once, right after this element mounts, and is never recomputed on
/// a later connection event. It is also not recomputed on a browser
/// window resize -- the wall's real deployment target is a fixed-size TV
/// display loaded once per bookmark open, not a resizable desktop window.
#[component]
fn MonitorGrid(cameras: Vec<Camera>) -> impl IntoView {
    let grid_container = NodeRef::<html::Div>::new();
    let camera_dims: Vec<(u32, u32)> = cameras
        .iter()
        .map(|camera| (camera.width, camera.height))
        .collect();
    let layout = RwSignal::new(Vec::<TileRect>::new());

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
                <MonitorTile camera=camera index=index layout=layout/>
            }).collect_view()}
        </div>
    }
}

/// Renders `rect`'s placement as an inline `style` value, in the same
/// convention `timeline.rs`'s own `timeline_segment_style` uses for its
/// reactive per-element positioning.
fn tile_placement_style(rect: TileRect) -> String {
    format!(
        "left: {:.4}px; top: {:.4}px; width: {:.4}px; height: {:.4}px",
        rect.x, rect.y, rect.width, rect.height
    )
}

/// One wall tile: names its camera, runs its own live session (staggered by
/// `index * STAGGER_MS` on initial mount), and positions itself at
/// `layout`'s entry for `index`, which `MonitorGrid` fills in once the
/// wall's own packed layout is computed.
#[component]
fn MonitorTile(camera: Camera, index: usize, layout: RwSignal<Vec<TileRect>>) -> impl IntoView {
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
        <div
            class="monitor-tile"
            data-camera=tile_camera_name
            style=move || {
                tile_placement_style(layout.get().get(index).copied().unwrap_or_default())
            }
        >
            <h2>{display_name}</h2>
            <div
                class="monitor-tile-player"
                data-live-path=move || path.get().data_attr()
                node_ref=player_container
            ></div>
        </div>
    }
}
