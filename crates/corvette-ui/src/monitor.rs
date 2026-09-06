//! The bookmarkable camera wall: one tile per enabled camera, each running
//! its own MoQ-first/HLS-fallback live session (issue #20 item M3, reusing
//! `expanded_view`'s own [`ExpandedSession`]/[`LivePath`] verbatim) and
//! placed by `monitor_layout`'s aspect-ratio-aware packing (item M4) to
//! exactly fill the viewport with no scrolling.
//!
//! Reached by direct bookmark at `/monitor`, not from primary navigation.
//! Selecting a tile fullscreens it in place via the standard Fullscreen API
//! (item M5, D-1 in `.agents/issue-20/DESIGN-monitor-mode.md`) -- the same
//! DOM element that already hosts the tile's live session, so entering or
//! leaving fullscreen never re-mounts or reconnects it. A small, persistent
//! top-left button on first load (issue #21's own follow-up, shrunk from an
//! earlier full-viewport one-time prompt) supplies the single real user
//! gesture Chromium's transient-activation model requires before any
//! `requestFullscreen()` call can succeed (D-4); `RESEARCH-tvbro-
//! fullscreen.md` found no gesture-free path on tv-bro/Android `WebView`.
//! Exiting fullscreen relies primarily on the
//! browser's own default Fullscreen-API behavior (Escape, or the platform's
//! own remote-control equivalent) -- whether tv-bro's own remote Back button
//! honors that default the same way is real-device-only and stays
//! UNVERIFIED until item V1 checks -- plus a real on-page button on the
//! fullscreened tile itself that calls `exitFullscreen()` directly, as a
//! defensive fallback independent of that unverified pathway. The button's
//! visibility is gated purely by CSS's own `:fullscreen` pseudo-class
//! (`styles.css`'s `.monitor-tile:fullscreen`), not any Rust-side
//! `fullscreenchange` tracking.

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

/// The tv-bro Direct Navigation Mode hint (D-1 in
/// `.agents/issue-21/DESIGN-tv-remote-navigation.md`), shared by
/// `EnterFullscreenButton`'s own `title` attribute and its adjacent visible
/// hint span so the two stay identical.
const TV_BRO_HINT: &str = "tv-bro's long-press menu also has a Direct Navigation Mode toggle, for D-pad/keyboard navigation instead of its default touch-emulating cursor.";

/// True for the two keys a `role="button"` element must treat as activation
/// per the WAI-ARIA button pattern this wall's custom (non-`<button>`)
/// clickable tiles follow -- `Enter` and `Space` (`" "` is the modern
/// `KeyboardEvent.key` value; no browser this crate targets still reports
/// the legacy `"Spacebar"`).
fn is_activation_key(key: &str) -> bool {
    key == "Enter" || key == " "
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
    // Starts unarmed on every fresh page load; the prompt below is the only
    // way to set it, and once set it stays set for the rest of this page's
    // own lifetime (D-4) -- there is no code path that clears it back.
    let armed = RwSignal::new(false);

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

/// The small top-left button that supplies the one real user gesture (click,
/// or `Enter`/`Space`) D-4 accepted as the unavoidable cost of Chromium's
/// transient-activation model on a browser (`RESEARCH-tvbro-fullscreen.md`)
/// that offers no gesture-free path to the Fullscreen API. Issue #21's own
/// follow-up: previously a full-viewport modal reading "Press OK to enter
/// fullscreen", now sized and positioned like `MonitorTile`'s own
/// `.monitor-tile-exit-fullscreen` button so the required gesture reads as
/// an on-page control rather than a blocking dialog. A real `<button>`
/// (mirroring that exit button's own markup), so `Enter`/`Space` activation
/// is native and needs no manual keydown handling. Auto-focuses itself so a
/// TV remote's very first "OK" press -- with nothing else yet focused on the
/// page -- actually reaches it. Clicking it sets `armed` and this component
/// stops rendering; nothing sets `armed` back to `false`, so it does not
/// reappear for the rest of this page load.
///
/// The tv-bro Direct Navigation Mode hint (D-1 in
/// `.agents/issue-21/DESIGN-tv-remote-navigation.md`) is carried two ways: as
/// this button's `title`, for a mouse user hovering it, and as the adjacent
/// `.monitor-enter-fullscreen-hint` span, shown only while the button itself
/// has focus (`styles.css`'s `.monitor-enter-fullscreen:focus-visible +
/// .monitor-enter-fullscreen-hint`). `title` alone is not enough: Chromium
/// only ever shows it on mouse hover, never on keyboard/programmatic focus,
/// so a sighted TV-remote/D-pad viewer -- who has no mouse and is exactly
/// who this hint is for -- could never see it that way. The focus-visible
/// span fixes that without permanently occupying screen space, and because
/// this button auto-focuses on mount, it is also the first thing a fresh
/// page load shows. The span is `aria-hidden` since a screen reader already
/// gets the same text as this button's accessible description (`title`,
/// per the HTML-AAM description computation, given `aria-label` already
/// supplies the name) -- without it, the hint would be announced twice.
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

    // Fullscreens this tile's own outer element in place (D-1) -- the same
    // element that already hosts `player_container`/`session` above, so
    // this touches neither: no re-mount, no reconnect, just a standard
    // `Element.requestFullscreen()` call. The outer element (rather than
    // `player_container` alone) is deliberate: it keeps the camera-name
    // heading below visible over the video while fullscreened, via the
    // existing `.monitor-tile h2` overlay style, matching how the tile
    // already looks on the wall. Gated on `armed` (D-4): before the
    // one-time prompt is dismissed, a tile click/keydown is a no-op rather
    // than an attempted `requestFullscreen()` call that would silently
    // fail for lack of a qualifying gesture.
    let enter_fullscreen = move || {
        if !armed.get_untracked() {
            return;
        }
        if let Some(element) = tile_container.get_untracked() {
            _ = element.request_fullscreen();
        }
    };

    // The defensive on-page exit affordance this module's top doc describes
    // (D-4). Unlike `request_fullscreen()`, `web_sys`'s `exit_fullscreen()`
    // returns `()`, not a `Result` -- nothing to ignore, so this just calls
    // it directly.
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
            // A real `<button>`, always mounted (never conditionally
            // rendered), so there is no Rust-side `fullscreenchange` state to
            // race -- `.monitor-tile-exit-fullscreen` in `styles.css` is what
            // hides it outside `:fullscreen` and draws its glyph (kept out of
            // this element's own text content, via `::before`, so it does
            // not show up in `.monitor-tile`'s own aggregate text -- the
            // camera name `<h2>` above is the tile's only real text).
            // `stop_propagation` on both handlers keeps a click/Enter/Space
            // aimed at this button from also bubbling into the tile's own
            // `on:click`/`on:keydown` above and re-triggering
            // `enter_fullscreen` (verified in `monitor.spec.cjs`).
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
