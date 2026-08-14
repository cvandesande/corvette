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

// Deployed nginx already owns these URL prefixes for its own purposes -- a
// JSON directory autoindex, proxied auth-gated API routes, static donor
// assets -- so no client-side route declared above may equal or be prefixed
// by one of them. Kept verbatim from the deployed
// configuration (`tests/nginx-parity/vendor/nginx.conf`), trailing slashes
// preserved exactly as nginx matches them: every entry carries one except
// `/ws` and `/auth`, which nginx matches without.
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

// The two `App` components above are the only place this crate declares
// client-side routes; both cfg branches declare the same three paths. There
// is no way to introspect a leptos_router `path!()` declaration at test time,
// so this is a hand-maintained mirror -- keep it in sync with the
// `<Route path=path!(...)>` list above whenever a route is added, renamed, or
// removed, and re-confirm the two match at review time.
#[cfg(test)]
const CLIENT_ROUTES: &[&str] = &["", "events", "recordings"];

// The one route KNOWN to collide with a reserved prefix, and only in its
// trailing-slash form (verified against the deployed configuration):
// `/recordings` renders this site's own shell, but `/recordings/` is nginx's
// JSON autoindex of the
// recordings directory -- the same path, two unrelated resources. Declared
// here so the collision is asserted as understood rather than passing
// silently. Do NOT rename or remove the `recordings` route to make this list
// empty -- that is a client-route rename, a decision outside this guard's
// scope.
#[cfg(test)]
const KNOWN_TRAILING_SLASH_EXCEPTIONS: &[&str] = &["recordings"];

#[cfg(test)]
mod reserved_route_tests {
    use super::{CLIENT_ROUTES, KNOWN_TRAILING_SLASH_EXCEPTIONS, RESERVED_NGINX_PREFIXES};
    use crate::shell::NAVIGATION;

    /// The absolute path a route renders as, with and without a trailing
    /// slash -- the two forms nginx's location matching treats as different
    /// resources.
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

    /// No declared client route may equal or be prefixed by a reserved
    /// nginx location, except the one collision on record. A new
    /// route that lands on a reserved prefix fails this test by name; the
    /// declared exception failing to collide (e.g. because the reserved list
    /// or the route changed) fails it too, so the exception can never go
    /// stale silently.
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

    /// `NAVIGATION` (`shell::NAVIGATION`) is the only mechanism this crate
    /// actually uses to render a self-route navigation link (`PageShell` in
    /// `shell.rs`, consumed by `events.rs` and `recordings.rs`, and again
    /// independently in `dashboard.rs`) -- every `href` on screen comes from
    /// this array, never from a literal string in a `view!` macro. So unlike
    /// `check_no_trailing_slash_hrefs.sh`, which greps source text for a
    /// quoted `href="..."` literal and is structurally blind to this, this
    /// test inspects the actual data feeding those links. A trailing slash
    /// on a self-route entry would silently misroute a click into nginx's
    /// own resource for that prefix instead of the SPA (see
    /// `RESERVED_NGINX_PREFIXES` above); `"/"` and hash-fragment links are
    /// exempt because neither is a narrower path a trailing slash could
    /// misroute.
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
