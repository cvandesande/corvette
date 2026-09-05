//! The "All cameras" grid on `/recordings` (issue #22 item R2): one tile per
//! camera, packed by `monitor_layout::pack_tiles` (issue #20's
//! aspect-ratio-aware algorithm, reused verbatim) and re-packed on resize via
//! a `ResizeObserver` -- D-2(b) in
//! `.agents/issue-22/DESIGN-all-cameras-scrubber.md`. Unlike `monitor.rs`'s
//! own wall, this page is ordinary in-flow content, not a fixed kiosk
//! viewport, so its own container's size can change after mount and the
//! layout has to follow it.
//!
//! This item renders name-only placeholder tiles; per-tile video and the
//! shared scrub position land in a later item (R3).

use corvette_api::Camera;
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, ResizeObserver, ResizeObserverEntry};

use crate::local_time::format_event_time;
use crate::monitor_layout::{TileRect, pack_tiles};
use crate::shell::{Status, StatusGlyph};

/// Renders every configured camera for the loaded range
/// (`start_time`/`end_time`; the range's own `camera` field is always `"all"`
/// and carries nothing a tile needs), reusing `cameras`
/// (`RecordingBrowser`'s already-fetched resource, D-5's own cost analysis)
/// rather than fetching the camera list again.
#[component]
pub(crate) fn AllCamerasPlayback(
    start_time: f64,
    end_time: f64,
    cameras: LocalResource<Result<Vec<Camera>, String>>,
) -> impl IntoView {
    let detail = format!(
        "{} – {}",
        format_event_time(start_time),
        format_event_time(end_time),
    );
    view! {
        <div class="playback-heading"><h2>"All cameras"</h2><p>{detail}</p></div>
        {move || match cameras.get() {
            None => view! {
                <Status heading="Loading cameras" detail="Connecting to the Frigate API." glyph=StatusGlyph::Camera/>
            }.into_any(),
            Some(Err(error)) => view! {
                <Status heading="Cameras unavailable" detail=error glyph=StatusGlyph::Camera/>
            }.into_any(),
            Some(Ok(cameras)) if cameras.is_empty() => view! {
                <Status heading="No cameras configured" detail="Add or enable a camera in Frigate, then reload this page." glyph=StatusGlyph::Camera/>
            }.into_any(),
            Some(Ok(cameras)) => view! { <AllCamerasGrid cameras=cameras/> }.into_any(),
        }}
    }
}

/// One tile per camera, packed into `.all-cameras-grid`'s own real pixel box
/// on mount and again whenever that box resizes (D-2(b)) -- the resize-aware
/// counterpart to `monitor.rs`'s `MonitorGrid`, which deliberately does not
/// recompute on resize (that wall targets a fixed-size display loaded once).
#[component]
fn AllCamerasGrid(cameras: Vec<Camera>) -> impl IntoView {
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
            {cameras.into_iter().enumerate().map(|(index, camera)| view! {
                <AllCamerasTile camera=camera index=index layout=layout/>
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

/// One placeholder tile: names its camera and sits at `layout`'s entry for
/// `index`. Video/playback lands in a later item (R3); this renders name
/// text only, mirroring issue #20 M2's own placeholder-first tile.
#[component]
fn AllCamerasTile(camera: Camera, index: usize, layout: RwSignal<Vec<TileRect>>) -> impl IntoView {
    let Camera {
        name: camera_name,
        display_name,
        ..
    } = camera;
    view! {
        <div
            class="all-cameras-tile"
            data-camera=camera_name
            style=move || {
                tile_placement_style(layout.get().get(index).copied().unwrap_or_default())
            }
        >
            <h2>{display_name}</h2>
        </div>
    }
}
