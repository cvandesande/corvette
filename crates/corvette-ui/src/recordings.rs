//! Lazy route boundary for the recording browser.

use leptos::prelude::*;
use leptos_router::{LazyRoute, lazy_route};

#[lazy_route]
impl LazyRoute for crate::RecordingsRoute {
    fn data() -> Self {
        Self
    }

    fn view(this: Self) -> AnyView {
        let _ = this;
        view! { <crate::RecordingBrowser/> }.into_any()
    }
}
