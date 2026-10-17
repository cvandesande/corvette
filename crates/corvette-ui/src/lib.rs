//! Browser entry point and the client-side routes it mounts.

#![allow(clippy::multiple_crate_versions)]
// `redundant_pub_crate` and `unreachable_pub` demand opposite visibility here:
// shared items need `pub(crate)`, and `#[component]` emits `pub` shims.
#![allow(clippy::redundant_pub_crate)]
#![allow(unreachable_pub)]

mod activity;
mod all_cameras;
mod api;
mod calendar;
mod dashboard;
mod events;
mod expanded_view;
#[cfg(feature = "split")]
mod lazy_route;
mod live_view;
mod local_time;
mod media;
mod monitor;
mod monitor_layout;
mod recordings;
mod shell;
mod timeline;

use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;

use crate::dashboard::Dashboard;
use crate::events::EventBrowser;
use crate::monitor::Monitor;
use crate::recordings::RecordingBrowser;

#[cfg(feature = "split")]
struct RecordingsRoute;

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn mount() {
    leptos::mount::mount_to_body(App);
}

#[component]
#[cfg(not(feature = "split"))]
fn App() -> impl IntoView {
    view! {
        <Router>
            <Routes fallback=|| "Page not found">
                <Route path=path!("") view=Dashboard/>
                <Route path=path!("events") view=EventBrowser/>
                <Route path=path!("recordings") view=RecordingBrowser/>
                <Route path=path!("monitor") view=Monitor/>
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
                <Route path=path!("monitor") view=Monitor/>
            </Routes>
        </Router>
    }
}

#[cfg(test)]
const RESERVED_NGINX_PREFIXES: &[&str] = &[
    "/api/",
    "/vod/",
    "/stream/",
    "/clips/",
    "/cache/",
    "/recordings/",
    "/exports/",
    "/ws",
    "/live/",
    "/assets/",
    "/fonts/",
    "/locales/",
    "/auth",
];

#[cfg(test)]
const CLIENT_ROUTES: &[&str] = &["", "events", "recordings", "monitor"];

#[cfg(test)]
const KNOWN_TRAILING_SLASH_EXCEPTIONS: &[&str] = &["recordings"];

#[cfg(test)]
mod reserved_route_tests {
    use super::{CLIENT_ROUTES, KNOWN_TRAILING_SLASH_EXCEPTIONS, RESERVED_NGINX_PREFIXES};
    use crate::shell::NAVIGATION;

    fn route_forms(route: &str) -> (String, String) {
        let bare = format!("/{route}");
        let with_slash = format!("{bare}/");
        (bare, with_slash)
    }

    fn collides(bare: &str, with_slash: &str) -> bool {
        RESERVED_NGINX_PREFIXES.iter().any(|reserved| {
            bare == *reserved
                || with_slash == *reserved
                || bare.starts_with(reserved)
                || with_slash.starts_with(reserved)
        })
    }

    #[test]
    fn no_client_route_claims_a_reserved_nginx_prefix_except_the_known_one() {
        let mut unexpected = Vec::new();
        let mut exceptions_seen = Vec::new();

        for route in CLIENT_ROUTES {
            let (bare, with_slash) = route_forms(route);
            if collides(&bare, &with_slash) {
                if KNOWN_TRAILING_SLASH_EXCEPTIONS.contains(route) {
                    exceptions_seen.push(*route);
                } else {
                    unexpected.push(with_slash);
                }
            }
        }

        assert!(
            unexpected.is_empty(),
            "client route(s) collide with a reserved nginx prefix and are not \
             declared as a known exception: {unexpected:?}"
        );
        assert_eq!(
            exceptions_seen.as_slice(),
            KNOWN_TRAILING_SLASH_EXCEPTIONS,
            "the declared known exception no longer collides with a reserved \
             prefix, or an undeclared collision was found -- update \
             KNOWN_TRAILING_SLASH_EXCEPTIONS to match what actually collides"
        );
    }

    #[test]
    fn no_navigation_entry_is_a_self_route_with_a_trailing_slash() {
        let offenders: Vec<&str> = NAVIGATION
            .iter()
            .filter(|(_, href)| *href != "/" && !href.starts_with('#') && href.ends_with('/'))
            .map(|(_, href)| *href)
            .collect();

        assert!(
            offenders.is_empty(),
            "NAVIGATION entry/entries render a self-route href ending in '/' \
             and would misroute a click into nginx's own resource instead of \
             the SPA: {offenders:?}"
        );
    }
}
