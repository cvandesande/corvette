//! The `/events` page: a date-ranged review feed over the retention window.

use corvette_api::ReviewEvent;
use leptos::prelude::*;
use std::collections::BTreeMap;

use crate::activity::{
    ActivityFilter, ActivityFilters, EventListLayout, ReviewEventList, review_and_motion_events,
    review_event_day_severity, severity_class,
};
use crate::calendar::{
    ActivityLegend, CalendarRecordingStatus, calendar_day_label, recording_day_available,
};
use crate::local_time::{
    CalendarDay, CalendarSelection, browser_timezone, local_day_start, next_local_day,
    recent_calendar_days, select_calendar_range,
};
use crate::shell::{PageShell, Status, StatusGlyph};

#[component]
pub(crate) fn EventBrowser() -> impl IntoView {
    let calendar_days = recent_calendar_days(21);
    let calendar_after = calendar_days.first().map_or(0.0, |day| day.start_time);
    let default_day = local_day_start(&js_sys::Date::new_0());
    let calendar_selection = RwSignal::new(None::<CalendarSelection>);
    let activity_filter = RwSignal::new(ActivityFilter::All);
    let timezone = browser_timezone();
    let recording_days = LocalResource::new(move || {
        let timezone = timezone.clone();
        async move { crate::api::fetch_recording_days("all", &timezone).await }
    });
    let calendar_activity = LocalResource::new(move || {
        crate::api::fetch_reviews(calendar_after, js_sys::Date::now() / 1_000.0)
    });
    let events = LocalResource::new(move || {
        let selection = calendar_selection
            .get()
            .unwrap_or_else(|| CalendarSelection::single(default_day));
        crate::api::fetch_reviews(selection.start_day, next_local_day(selection.end_day))
    });
    let motion_events = LocalResource::new(move || {
        let selection = calendar_selection
            .get()
            .unwrap_or_else(|| CalendarSelection::single(default_day));
        crate::api::fetch_motion_activity(selection.start_day, next_local_day(selection.end_day))
    });

    view! {
        <PageShell active_path="/events">
            <section class="page-section" aria-labelledby="all-events-heading">
                <p class="eyebrow">"History"</p>
                <h1 id="all-events-heading">"Events"</h1>
                <div class="event-browser">
                    <div class="calendar-heading"><div>
                        <h2>"Choose dates"</h2>
                        <p>"Tap a start day, then an end day to browse a range."</p>
                    </div></div>
                    <EventCalendarDays
                        calendar_days
                        default_day
                        calendar_selection
                        calendar_activity
                        recording_days
                    />
                    <CalendarRecordingStatus
                        recording_days
                        empty_detail="No retained recordings are available."
                    />
                    <ActivityLegend/>
                </div>
                <ActivityFilters filter=activity_filter/>
                {move || match (events.get(), motion_events.get()) {
                    (None, _) | (_, None) => view! { <Status
                        heading="Loading events"
                        detail="Fetching review activity for the selected day."
                        glyph=StatusGlyph::Event
                    /> }.into_any(),
                    (Some(Err(error)), _) | (_, Some(Err(error))) => view! { <Status
                        heading="Events unavailable"
                        detail=error
                        glyph=StatusGlyph::Event
                    /> }.into_any(),
                    (Some(Ok(events)), Some(Ok(motion))) => view! { <ReviewEventList
                        events=review_and_motion_events(events, &motion)
                        filter=activity_filter
                        layout=EventListLayout::Complete
                        empty_heading=if calendar_selection.get()
                            .is_some_and(|selection| selection.start_day < selection.end_day)
                        {
                            "No events in this date range".to_owned()
                        } else {
                            "No events on this day".to_owned()
                        }
                    /> }.into_any(),
                }}
            </section>
        </PageShell>
    }
}

/// Renders the day strip, dimming days with no retained recordings and marking
/// each remaining day with its highest review severity.
#[component]
fn EventCalendarDays(
    calendar_days: Vec<CalendarDay>,
    default_day: f64,
    calendar_selection: RwSignal<Option<CalendarSelection>>,
    calendar_activity: LocalResource<Result<Vec<ReviewEvent>, String>>,
    recording_days: LocalResource<Result<BTreeMap<String, bool>, String>>,
) -> impl IntoView {
    let selection_of = move || {
        calendar_selection
            .get()
            .unwrap_or_else(|| CalendarSelection::single(default_day))
    };
    view! { <div class="calendar-grid" role="group" aria-label="Event days">
        {calendar_days.into_iter().map(|day| {
            let start_time = day.start_time;
            let end_time = next_local_day(start_time);
            let date_key = day.date_key.clone();
            let label_key = date_key.clone();
            let accessible_label = day.accessible_label.clone();
            view! { <button
                type="button"
                disabled=move || !recording_day_available(recording_days.get(), &date_key)
                class:in-range=move || selection_of().contains(start_time)
                aria-pressed=move || selection_of().contains(start_time).to_string()
                aria-label=move || calendar_day_label(
                    recording_days.get(),
                    &accessible_label,
                    &label_key,
                )
                on:click=move |_| calendar_selection.update(|selection| {
                    *selection = Some(select_calendar_range(*selection, start_time));
                })
            >
                <span>{day.weekday}</span>
                <strong>{day.day_number}</strong>
                <small>{day.month}</small>
                <i class=move || calendar_activity.get().and_then(Result::ok)
                    .and_then(|reviews| review_event_day_severity(&reviews, start_time, end_time))
                    .map_or("activity-none", severity_class)
                    aria-hidden="true"
                ></i>
            </button> }
        }).collect_view()}
    </div> }
}
