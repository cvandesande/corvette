//! The bookmarkable camera wall: one placeholder tile per enabled camera.
//!
//! Reached by direct bookmark at `/monitor`, not from primary navigation.
//! Live video (issue #20 item M3), aspect-ratio-aware layout (M4), and
//! per-tile fullscreen (M5) all build on top of this scaffold.

use corvette_api::Camera;
use leptos::prelude::*;

use crate::shell::{Status, StatusGlyph};

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
                        {cameras.into_iter().map(|camera| view! {
                            <MonitorTile camera=camera/>
                        }).collect_view()}
                    </div>
                }.into_any(),
            }}
        </main>
    }
}

/// One placeholder wall tile, naming its camera only. Live video, layout,
/// and fullscreen are added by later plan items on top of this element.
#[component]
fn MonitorTile(camera: Camera) -> impl IntoView {
    view! {
        <div class="monitor-tile" data-camera=camera.name>
            <h2>{camera.display_name}</h2>
        </div>
    }
}
