//! Browser entry point and the client-side routes it mounts.

#![allow(clippy::multiple_crate_versions)]
// Two lint pairs collide with how this crate is laid out, and neither has a
// code fix:
//
// - Items shared between the private modules below have to be `pub(crate)`,
//   which clippy calls redundant because the modules are private. Writing
//   `pub` instead trips the workspace's denied `unreachable_pub`. No
//   visibility satisfies both lints.
// - Leptos's `#[component]` macro emits a `pub` shim beside every component,
//   and `unreachable_pub` reports it against the component's own line.
//
// Both are allowed here, once, rather than at each of the modules involved.
#![allow(clippy::redundant_pub_crate)]
#![allow(unreachable_pub)]

mod activity;
mod api;
mod calendar;
mod dashboard;
mod events;
#[cfg(feature = "split")]
mod lazy_route;
mod local_time;
mod media;
mod recordings;
mod shell;
mod timeline;

use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;

use crate::dashboard::Dashboard;
use crate::events::EventBrowser;
use crate::recordings::RecordingBrowser;

#[cfg(feature = "split")]
struct RecordingsRoute;

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn mount() {
    leptos::mount::mount_to_body(App);
}

/// Renders the top-level Corvette application shell.
#[component]
#[cfg(not(feature = "split"))]
fn App() -> impl IntoView {
    view! {
        <Router>
            <Routes fallback=|| "Page not found">
                <Route path=path!("") view=Dashboard/>
                <Route path=path!("events") view=EventBrowser/>
                <Route path=path!("recordings") view=RecordingBrowser/>
            </Routes>
        </Router>
    }
}

#[component]
#[cfg(feature = "split")]
fn App() -> impl IntoView {
    view! {
        <Router>
            <Routes fallback=|| "Page not found">
                <Route path=path!("") view=Dashboard/>
                <Route path=path!("events") view=EventBrowser/>
                <Route
                    path=path!("recordings")
                    view={leptos_router::Lazy::<RecordingsRoute>::new()}
                />
            </Routes>
        </Router>
    }
}
