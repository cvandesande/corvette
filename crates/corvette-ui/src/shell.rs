//! Chrome every page shares: primary navigation and the async status block.

use leptos::prelude::*;

pub(crate) const NAVIGATION: [(&str, &str); 4] = [
    ("Live", "/#live"),
    ("Events", "/events"),
    ("Recordings", "/recordings"),
    ("System", "/#system"),
];

#[component]
pub(crate) fn PageShell(active_path: &'static str, children: Children) -> impl IntoView {
    view! {
        <header class="site-header">
            <a class="wordmark" href="/" aria-label="Corvette home">
                <span class="wordmark-mark" aria-hidden="true">"C"</span>
                <span>"Corvette"</span>
            </a>
            <span class="connection-state">"Frigate API"</span>
        </header>
        <div class="app-frame">
            <nav aria-label="Primary navigation">
                <ul>{NAVIGATION.map(|(label, href)| view! {
                    <li><a
                        href=href
                        class:active=move || href == active_path
                        aria-current=(href == active_path).then_some("page")
                    >{label}</a></li>
                })}</ul>
            </nav>
            <main>{children()}</main>
        </div>
    }
}

#[derive(Clone, Copy)]
pub(crate) enum StatusGlyph {
    Camera,
    Event,
    Recording,
}

/// Renders an asynchronous content state with a consistent accessible heading.
#[component]
pub(crate) fn Status(
    heading: impl IntoView,
    detail: impl IntoView,
    glyph: StatusGlyph,
) -> impl IntoView {
    let glyph_class = match glyph {
        StatusGlyph::Camera => "status-glyph camera-glyph",
        StatusGlyph::Event => "status-glyph event-glyph",
        StatusGlyph::Recording => "status-glyph recording-glyph",
    };

    view! {
        <div class="empty-state" role="status">
            <div class=glyph_class aria-hidden="true"></div>
            <div>
                <h2>{heading}</h2>
                <p>{detail}</p>
            </div>
        </div>
    }
}
