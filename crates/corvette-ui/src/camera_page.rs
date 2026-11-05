//! One camera at its own address: the live screen for the camera named in the path.

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::{use_navigate, use_params_map};

use crate::expanded_view::{ExpandedCamera, ExpandedView};
use crate::shell::{Status, StatusGlyph};

#[component]
pub(crate) fn CameraPage() -> impl IntoView {
    let params = use_params_map();
    let camera_name = move || params.with(|map| map.get("name").unwrap_or_default());
    let cameras = LocalResource::new(crate::api::fetch_camera_entries);

    view! {
        <main>
            <section class="page-section">
                {move || match cameras.get() {
                    None => view! { <Status heading="Loading camera" detail="Connecting to the camera list." glyph=StatusGlyph::Camera/> }.into_any(),
                    Some(Err(error)) => view! {
                        <Status heading="Camera unavailable" detail=error glyph=StatusGlyph::Camera/>
                        <a href="/">"All cameras"</a>
                    }.into_any(),
                    Some(Ok(cameras)) => {
                        let name = camera_name();
                        match cameras.into_iter().find(|entry| entry.name == name) {
                            None => view! {
                                <Status heading="Camera not found" detail=format!("No camera is called {name}.") glyph=StatusGlyph::Camera/>
                                <a href="/">"All cameras"</a>
                            }.into_any(),
                            Some(entry) if !entry.enabled => view! {
                                <Status heading=format!("{} is off", entry.display_name) detail="This camera is switched off." glyph=StatusGlyph::Camera/>
                                <a href="/">"All cameras"</a>
                            }.into_any(),
                            Some(entry) => view! {
                                <CameraLive name=entry.name title=format!("{} live video", entry.display_name)/>
                            }.into_any(),
                        }
                    }
                }}
            </section>
        </main>
    }
}

#[component]
fn CameraLive(name: String, title: String) -> impl IntoView {
    let expanded = RwSignal::new(Some(ExpandedCamera { name, title }));
    let navigate = use_navigate();
    Effect::new(move |_| {
        if expanded.get().is_none() {
            navigate("/", NavigateOptions::default());
        }
    });

    view! {
        {move || expanded.get().map(|camera| view! { <ExpandedView camera=camera expanded=expanded/> })}
    }
}
