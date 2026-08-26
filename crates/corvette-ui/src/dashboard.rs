//! The landing page: the live camera grid and the last few hours of activity.

use leptos::prelude::*;

use crate::activity::{
    ActivityFilter, ActivityFilters, EventListLayout, RECENT_ACTIVITY_HOURS, ReviewEventList,
    recent_activity_empty_heading, review_and_motion_events,
};
use crate::live_view::LiveCameraTile;
use crate::shell::{NAVIGATION, Status, StatusGlyph};

#[component]
pub(crate) fn Dashboard() -> impl IntoView {
    let cameras = LocalResource::new(crate::api::fetch_cameras);
    let recent_activity = LocalResource::new(|| {
        let before = js_sys::Date::now() / 1_000.0;
        let after = f64::from(RECENT_ACTIVITY_HOURS).mul_add(-3_600.0, before);
        crate::api::fetch_reviews(after, before)
    });
    let recent_motion = LocalResource::new(|| {
        let before = js_sys::Date::now() / 1_000.0;
        let after = f64::from(RECENT_ACTIVITY_HOURS).mul_add(-3_600.0, before);
        crate::api::fetch_motion_activity(after, before)
    });
    let active_section = RwSignal::new("/#live");
    let activity_filter = RwSignal::new(ActivityFilter::All);

    view! {
        <header class="site-header">
            <a class="wordmark" href="#live" aria-label="Corvette home">
                <span class="wordmark-mark" aria-hidden="true">"C"</span>
                <span>"Corvette"</span>
            </a>
            <span class="connection-state">"Frigate API"</span>
        </header>

        <div class="app-frame">
            <nav aria-label="Primary navigation">
                <ul>
                    {NAVIGATION.map(|(label, href)| view! {
                        <li>
                            <a
                                href=href
                                class:active=move || active_section.get() == href
                                aria-current=move || {
                                    (active_section.get() == href).then_some("location")
                                }
                                on:click=move |_| active_section.set(href)
                            >
                                {label}
                            </a>
                        </li>
                    })}
                </ul>
            </nav>

            <main>
                <section id="live" class="page-section">
                    <p class="eyebrow">"Live view"</p>
                    {move || match cameras.get() {
                        None => view! { <Status heading="Loading cameras" detail="Connecting to the Frigate API." glyph=StatusGlyph::Camera/> }.into_any(),
                        Some(Err(error)) => view! { <Status heading="Cameras unavailable" detail=error glyph=StatusGlyph::Camera/> }.into_any(),
                        Some(Ok(cameras)) if cameras.is_empty() => view! { <Status heading="No cameras configured" detail="Add or enable a camera in Frigate, then reload this page." glyph=StatusGlyph::Camera/> }.into_any(),
                        Some(Ok(cameras)) => view! {
                            <div class="camera-grid" aria-label="Configured cameras">
                                {cameras.into_iter().map(|camera| {
                                    let player_title = format!("{} live video", camera.display_name);
                                    view! {
                                        <article class="camera-card">
                                            <LiveCameraTile camera_name=camera.name.clone() title=player_title/>
                                            <h2>{camera.display_name}</h2>
                                            <p>{camera.name}</p>
                                        </article>
                                    }
                                }).collect_view()}
                            </div>
                        }.into_any(),
                    }}
                </section>

                <section id="events" class="page-section" aria-labelledby="events-heading">
                    <p class="eyebrow">"History"</p>
                    <h1 id="events-heading">"Recent events"</h1>
                    <ActivityFilters filter=activity_filter/>
                    {move || match (recent_activity.get(), recent_motion.get()) {
                        (None, _) | (_, None) => view! { <Status heading="Loading events" detail="Fetching recent activity." glyph=StatusGlyph::Event/> }.into_any(),
                        (Some(Err(error)), _) | (_, Some(Err(error))) => view! { <Status heading="Events unavailable" detail=error glyph=StatusGlyph::Event/> }.into_any(),
                        (Some(Ok(events)), Some(Ok(motion))) if events.is_empty() && motion.is_empty() => view! { <Status
                            heading=recent_activity_empty_heading()
                            detail="Frigate reported no activity during this period."
                            glyph=StatusGlyph::Event
                        /> }.into_any(),
                        (Some(Ok(events)), Some(Ok(motion))) => view! { <ReviewEventList
                            events=review_and_motion_events(events, &motion)
                            filter=activity_filter
                            layout=EventListLayout::Compact
                            empty_heading=recent_activity_empty_heading()
                        /> }.into_any(),
                    }}
                </section>
            </main>
        </div>
    }
}
