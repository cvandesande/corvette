//! The "All cameras" grid on `/recordings`: one tile per camera, packed by
//! `monitor_layout::pack_tiles` (issue #20's aspect-ratio-aware algorithm,
//! reused verbatim) and re-packed on resize via a `ResizeObserver` -- D-2(b)
//! in `.agents/issue-22/DESIGN-all-cameras-scrubber.md`. Unlike `monitor.rs`'s
//! own wall, this page is ordinary in-flow content, not a fixed kiosk
//! viewport, so its own container's size can change after mount and the
//! layout has to follow it.
//!
//! Issue #22 item R3 adds real playback: one shared `selected_time`/
//! `uses_preview` pair drives every tile's own, unmodified
//! `crate::timeline::TimelinePlayer` (D-1(c)) -- N independent per-camera
//! state machines, never a single shared player, and never a cross-tile
//! barrier (D-1's rejected option (b)). Each tile owns its own recording
//! segments fetch and its own "not yet settled" signal.

use corvette_api::{Camera, PreviewClip};
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, ResizeObserver, ResizeObserverEntry};

use crate::local_time::format_event_time;
use crate::media::{RecordingRange, recording_media};
use crate::monitor_layout::{TileRect, pack_tiles};
use crate::shell::{Status, StatusGlyph};
use crate::timeline::TimelinePlayer;

/// Renders every configured camera for the loaded range
/// (`start_time`/`end_time`; the range's own `camera` field is always `"all"`
/// and carries nothing a tile needs), reusing `cameras` and `preview_clips`
/// (`RecordingBrowser`'s already-fetched resources -- D-5's own cost analysis
/// for `cameras`, and F-13 for `preview_clips` already answering every
/// camera at once under `camera == "all"`) rather than fetching either again.
///
/// Owns the one `selected_time`/`uses_preview` pair every tile's own
/// `TimelinePlayer` is driven by -- the "one scrub position driving N tiles"
/// mechanism D-1(c) describes.
#[component]
pub(crate) fn AllCamerasPlayback(
    start_time: f64,
    end_time: f64,
    cameras: LocalResource<Result<Vec<Camera>, String>>,
    preview_clips: LocalResource<Result<Vec<PreviewClip>, String>>,
) -> impl IntoView {
    let detail = format!(
        "{} – {}",
        format_event_time(start_time),
        format_event_time(end_time),
    );
    let selected_time = RwSignal::new(start_time);
    let uses_preview = RwSignal::new(true);

    view! {
        <div class="playback-heading"><h2>"All cameras"</h2><p>{detail}</p></div>
        <div class="all-cameras-scrub">
            <input
                type="range"
                min=start_time
                max=end_time
                step="1"
                prop:value=move || selected_time.get()
                aria-label="All-cameras playhead"
                on:input=move |event| {
                    let requested_time = event_target_value(&event)
                        .parse::<f64>()
                        .unwrap_or(start_time);
                    uses_preview.set(true);
                    selected_time.set(requested_time);
                }
            />
        </div>
        {move || match (cameras.get(), preview_clips.get()) {
            (None, _) | (_, None) => view! {
                <Status heading="Loading cameras" detail="Connecting to the Frigate API." glyph=StatusGlyph::Camera/>
            }.into_any(),
            (Some(Err(error)), _) | (_, Some(Err(error))) => view! {
                <Status heading="Cameras unavailable" detail=error glyph=StatusGlyph::Camera/>
            }.into_any(),
            (Some(Ok(cameras)), _) if cameras.is_empty() => view! {
                <Status heading="No cameras configured" detail="Add or enable a camera in Frigate, then reload this page." glyph=StatusGlyph::Camera/>
            }.into_any(),
            (Some(Ok(cameras)), Some(Ok(previews))) => view! {
                <AllCamerasGrid
                    cameras
                    previews
                    start_time
                    end_time
                    selected_time
                    uses_preview
                />
            }.into_any(),
        }}
    }
}

/// One tile per camera, packed into `.all-cameras-grid`'s own real pixel box
/// on mount and again whenever that box resizes (D-2(b)) -- the resize-aware
/// counterpart to `monitor.rs`'s `MonitorGrid`, which deliberately does not
/// recompute on resize (that wall targets a fixed-size display loaded once).
#[component]
fn AllCamerasGrid(
    cameras: Vec<Camera>,
    previews: Vec<PreviewClip>,
    start_time: f64,
    end_time: f64,
    selected_time: RwSignal<f64>,
    uses_preview: RwSignal<bool>,
) -> impl IntoView {
    let grid_container = NodeRef::<html::Div>::new();
    let camera_dims: Vec<(u32, u32)> = cameras
        .iter()
        .map(|camera| (camera.width, camera.height))
        .collect();
    let layout = RwSignal::new(Vec::<TileRect>::new());
    // Non-`Send`/non-`Sync` browser objects live behind this; `on_cleanup`
    // still requires a `Send + Sync` closure regardless of target -- see
    // `monitor.rs`'s own `session` field for the identical rationale.
    let resize_watcher = StoredValue::new_local(None::<GridResizeWatcher>);

    Effect::new(move |_| {
        let Some(container) = grid_container.get() else {
            return;
        };
        let bounds = container.get_bounding_client_rect();
        if bounds.width() > 0.0 && bounds.height() > 0.0 {
            layout.set(pack_tiles(&camera_dims, bounds.width(), bounds.height()));
        }
        resize_watcher.set_value(GridResizeWatcher::start(
            &container,
            camera_dims.clone(),
            layout,
        ));
    });

    on_cleanup(move || {
        if let Some(Some(watcher)) = resize_watcher.try_update_value(Option::take) {
            watcher.stop();
        }
    });

    view! {
        <div class="all-cameras-grid" node_ref=grid_container aria-label="All cameras">
            {cameras.into_iter().enumerate().map(|(index, camera)| {
                let camera_previews = previews
                    .iter()
                    .filter(|preview| preview.camera == camera.name)
                    .cloned()
                    .collect::<Vec<_>>();
                view! {
                    <AllCameraTile
                        camera=camera
                        index=index
                        layout=layout
                        start_time
                        end_time
                        previews=camera_previews
                        selected_time
                        uses_preview
                    />
                }
            }).collect_view()}
        </div>
    }
}

/// Keeps a `ResizeObserver` and its JS-bound callback alive for exactly as
/// long as `.all-cameras-grid` stays mounted, re-running `pack_tiles`
/// against the container's own new content box on every observed resize.
/// `monitor_layout::pack_tiles` itself is untouched -- this only adds a new
/// caller and a new trigger to re-call it.
struct GridResizeWatcher {
    observer: ResizeObserver,
    _on_resize: Closure<dyn FnMut(js_sys::Array)>,
}

impl GridResizeWatcher {
    fn start(
        container: &Element,
        camera_dims: Vec<(u32, u32)>,
        layout: RwSignal<Vec<TileRect>>,
    ) -> Option<Self> {
        let on_resize = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
            let Some(entry) = entries.get(0).dyn_into::<ResizeObserverEntry>().ok() else {
                return;
            };
            let rect = entry.content_rect();
            if rect.width() <= 0.0 || rect.height() <= 0.0 {
                return;
            }
            layout.set(pack_tiles(&camera_dims, rect.width(), rect.height()));
        });
        let observer = ResizeObserver::new(on_resize.as_ref().unchecked_ref()).ok()?;
        observer.observe(container);
        Some(Self {
            observer,
            _on_resize: on_resize,
        })
    }

    fn stop(self) {
        self.observer.disconnect();
    }
}

/// Renders `rect`'s placement as an inline `style` value -- duplicated from
/// `monitor.rs`'s identical `tile_placement_style` rather than hoisted into
/// `monitor_layout.rs` itself (plan's own "Implementation-level choices" #4).
fn tile_placement_style(rect: TileRect) -> String {
    format!(
        "left: {:.4}px; top: {:.4}px; width: {:.4}px; height: {:.4}px",
        rect.x, rect.y, rect.width, rect.height
    )
}

/// One live tile: fetches this camera's own retained segments for the loaded
/// range (`/{camera}/recordings` has no bulk form, F-12, so each tile owns
/// its own `LocalResource`, matching this crate's existing
/// one-resource-per-component idiom) and renders the existing single-camera
/// `TimelinePlayer` (`crate::timeline`, unmodified) against them, following
/// the shared `selected_time`/`uses_preview` signals `AllCamerasPlayback`
/// owns -- the "one scrub position driving N tiles" mechanism D-1(c)
/// describes. `active_activity` is this tile's own, permanently-`None`
/// signal: "All" mode never plays a clicked activity through to its end
/// (Implementation-level choice 2), so nothing here needs it shared across
/// tiles.
///
/// This tile's own `settled` signal is independent of every other tile's --
/// no cross-tile barrier, D-1's rejected option (b) -- and drives a CSS
/// overlay shown while this tile's own `TimelinePlayer` has not yet
/// confirmed landing on the shared scrub position. A camera with no
/// recordings at all in the loaded range, or a real gap at the current scrub
/// position, still renders `TimelinePlayer` as-is here; the dedicated
/// no-footage placeholder is a later item (R4).
#[component]
fn AllCameraTile(
    camera: Camera,
    index: usize,
    layout: RwSignal<Vec<TileRect>>,
    start_time: f64,
    end_time: f64,
    previews: Vec<PreviewClip>,
    selected_time: RwSignal<f64>,
    uses_preview: RwSignal<bool>,
) -> impl IntoView {
    let Camera {
        name: camera_name,
        display_name,
        ..
    } = camera;
    let recording_clips = LocalResource::new({
        let camera_name = camera_name.clone();
        move || {
            let camera_name = camera_name.clone();
            async move {
                crate::api::fetch_recording_segments(&camera_name, start_time, end_time)
                    .await
                    .map(|segments| {
                        let range = RecordingRange {
                            camera: camera_name,
                            start_time,
                            end_time,
                        };
                        recording_media(&range, segments)
                    })
            }
        }
    });
    let settled = RwSignal::new(false);

    view! {
        <div
            class="all-cameras-tile"
            data-camera=camera_name
            style=move || {
                tile_placement_style(layout.get().get(index).copied().unwrap_or_default())
            }
        >
            <h2>{display_name}</h2>
            {move || match recording_clips.get() {
                None => ().into_any(),
                Some(Err(error)) => view! {
                    <p class="all-cameras-tile-error" role="alert">{error}</p>
                }.into_any(),
                Some(Ok(media)) => view! {
                    <TimelinePlayer
                        clips=media.clips
                        previews=previews.clone()
                        activities=Vec::new()
                        active_activity=RwSignal::new(None)
                        selected_time
                        uses_preview
                        settled=settled
                    />
                }.into_any(),
            }}
            {move || (!settled.get()).then(|| view! {
                <div class="all-cameras-tile-unsettled" aria-hidden="true"></div>
            })}
        </div>
    }
}
