//! Browser entry point and initial application shell for Corvette.

// Leptos 0.8.20's proc-macro stack currently selects multiple major versions of
// syn, thiserror, and convert_case; this binary does not select any of them directly.
#![allow(clippy::multiple_crate_versions)]

use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use wasm_bindgen::JsValue;

use corvette_api::{
    Camera, Event, MotionActivity, PreviewClip, RecordingSegment, ReviewEvent, ReviewSegment,
    ReviewSeverity,
};

mod api;
#[cfg(feature = "split")]
mod recordings;

#[cfg(feature = "split")]
struct RecordingsRoute;

const NAVIGATION: [(&str, &str); 4] = [
    ("Live", "/#live"),
    ("Events", "/events"),
    ("Recordings", "/recordings"),
    ("System", "/#system"),
];
const RECENT_ACTIVITY_HOURS: u32 = 6;
const MOTION_BUCKET_SECONDS: f64 = 30.0;

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

#[component]
fn Dashboard() -> impl IntoView {
    let cameras = LocalResource::new(api::fetch_cameras);
    let recent_activity = LocalResource::new(|| {
        let before = js_sys::Date::now() / 1_000.0;
        let after = f64::from(RECENT_ACTIVITY_HOURS).mul_add(-3_600.0, before);
        api::fetch_reviews(after, before)
    });
    let recent_motion = LocalResource::new(|| {
        let before = js_sys::Date::now() / 1_000.0;
        let after = f64::from(RECENT_ACTIVITY_HOURS).mul_add(-3_600.0, before);
        api::fetch_motion_activity(after, before)
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
                                    let player_url = format!(
                                        "/go2rtc/stream.html?src={}&mode=mse",
                                        camera.name,
                                    );
                                    let player_title = format!("{} live video", camera.display_name);
                                    view! {
                                        <article class="camera-card">
                                            <iframe
                                                class="camera-player"
                                                src=player_url
                                                title=player_title
                                                allow="autoplay; fullscreen"
                                            ></iframe>
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

#[component]
fn ActivityFilters(filter: RwSignal<ActivityFilter>) -> impl IntoView {
    view! {
        <div class="activity-filters" aria-label="Recent event type">
            {ActivityFilter::ALL.map(|choice| view! {
                <button
                    type="button"
                    class:active=move || filter.get() == choice
                    aria-pressed=move || (filter.get() == choice).to_string()
                    on:click=move |_| filter.set(choice)
                >{choice.label()}</button>
            })}
        </div>
    }
}

#[derive(Clone, Copy)]
enum EventListLayout {
    Compact,
    Complete,
}

#[component]
fn ReviewEventList(
    events: Vec<ReviewEvent>,
    filter: RwSignal<ActivityFilter>,
    layout: EventListLayout,
    empty_heading: String,
) -> impl IntoView {
    let expanded = RwSignal::new(false);
    let selected_review = RwSignal::new(None::<ReviewEvent>);
    move || {
        let events = review_events_for_filter(events.clone(), filter.get());
        if events.is_empty() {
            return view! { <Status
                heading=empty_heading.clone()
                detail="Frigate reported no matching review activity during this period."
                glyph=StatusGlyph::Event
            /> }
            .into_any();
        }
        let hidden_event_count = match layout {
            EventListLayout::Compact => events.len().saturating_sub(4),
            EventListLayout::Complete => 0,
        };
        view! {
            <div class="event-grid" aria-label="Events">
                {events.into_iter().enumerate().map(|(index, event)| {
                    let snapshot_url = review_thumbnail_url(&event.thumb_path);
                    let detail = review_event_title(&event);
                    let thumbnail_alt = format!("{detail} activity from {}", event.camera);
                    let event_for_selection = event.clone();
                    let zones = if event.data.zones.is_empty() {
                        "No zone".to_owned()
                    } else {
                        event.data.zones.join(", ")
                    };
                    view! {
                        <article
                            class="event-card"
                            class:mobile-hidden=move || {
                                hidden_event_count > 0 && index >= 4 && !expanded.get()
                            }
                        >
                            <button
                                class="event-card-link"
                                type="button"
                                on:click=move |_| selected_review.set(Some(event_for_selection.clone()))
                            >
                                <img src=snapshot_url alt=thumbnail_alt loading="lazy"/>
                                <div class="event-card-body">
                                    <div class="event-card-heading">
                                        <h2>{detail}</h2>
                                        <span>{format_event_time(event.start_time)}</span>
                                    </div>
                                    <p>{event.camera}</p>
                                    <p class=format!(
                                        "event-severity {}",
                                        severity_class(event.severity),
                                    )>{severity_label(event.severity)}</p>
                                    <p class="event-zone">{zones}</p>
                                </div>
                            </button>
                        </article>
                    }
                }).collect_view()}
            </div>
            {(hidden_event_count > 0).then(|| view! {
                <button
                    class="event-toggle"
                    type="button"
                    aria-expanded=move || expanded.get().to_string()
                    on:click=move |_| expanded.update(|is_expanded| *is_expanded = !*is_expanded)
                >{move || if expanded.get() {
                    "Show fewer events".to_owned()
                } else {
                    format!("Show {hidden_event_count} more events")
                }}</button>
            })}
            {selected_review.get().map(|review| view! {
                <ReviewPlayback review selected_review/>
            })}
        }
        .into_any()
    }
}

#[component]
fn ReviewPlayback(
    review: ReviewEvent,
    selected_review: RwSignal<Option<ReviewEvent>>,
) -> impl IntoView {
    let playback_failed = RwSignal::new(false);
    let close_button = NodeRef::<leptos::html::Button>::new();
    Effect::new(move |_| {
        if let Some(button) = close_button.get() {
            _ = button.focus();
        }
    });
    let clip_url = RecordingRange {
        camera: review.camera.clone(),
        start_time: review.start_time,
        end_time: review
            .end_time
            .unwrap_or_else(|| js_sys::Date::now() / 1_000.0),
    }
    .clip_url();
    let heading = review_event_title(&review);
    view! {
        <div
            class="playback-modal"
            on:click=move |_| selected_review.set(None)
            on:keydown=move |event| {
                if event.key() == "Escape" {
                    selected_review.set(None);
                }
            }
        >
            <section
                class="review-playback"
                role="dialog"
                aria-modal="true"
                aria-label="Selected event playback"
                on:click=|event| event.stop_propagation()
            >
                <div class="recording-heading playback-heading">
                    <div><h2>{heading}</h2><p>{format_event_time(review.start_time)}</p></div>
                    <button
                        type="button"
                        node_ref=close_button
                        on:click=move |_| selected_review.set(None)
                    >"Close"</button>
                </div>
                <video class="recording-player" src=clip_url controls autoplay playsinline
                    on:error=move |_| playback_failed.set(true)
                >"This browser cannot play the event recording."</video>
                {move || playback_failed.get().then(|| view! {
                    <p class="recording-error" role="alert">
                        "The high-resolution clip is not available from Frigate."
                    </p>
                })}
            </section>
        </div>
    }
}

#[component]
fn EventBrowser() -> impl IntoView {
    let calendar_days = recent_calendar_days(21);
    let calendar_after = calendar_days.first().map_or(0.0, |day| day.start_time);
    let default_day = local_day_start(&js_sys::Date::new_0());
    let calendar_selection = RwSignal::new(None::<CalendarSelection>);
    let activity_filter = RwSignal::new(ActivityFilter::All);
    let timezone = browser_timezone();
    let recording_days = LocalResource::new(move || {
        let timezone = timezone.clone();
        async move { api::fetch_recording_days("all", &timezone).await }
    });
    let calendar_activity = LocalResource::new(move || {
        api::fetch_reviews(calendar_after, js_sys::Date::now() / 1_000.0)
    });
    let events = LocalResource::new(move || {
        let selection = calendar_selection
            .get()
            .unwrap_or_else(|| CalendarSelection::single(default_day));
        api::fetch_reviews(selection.start_day, next_local_day(selection.end_day))
    });
    let motion_events = LocalResource::new(move || {
        let selection = calendar_selection
            .get()
            .unwrap_or_else(|| CalendarSelection::single(default_day));
        api::fetch_motion_activity(selection.start_day, next_local_day(selection.end_day))
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
                    <div class="calendar-grid" role="group" aria-label="Event days">
                        {calendar_days.into_iter().map(|day| {
                            let start_time = day.start_time;
                            let end_time = next_local_day(start_time);
                            let date_key = day.date_key.clone();
                            let label_key = date_key.clone();
                            let accessible_label = day.accessible_label.clone();
                            view! { <button
                                type="button"
                                disabled=move || !recording_day_available(
                                    recording_days.get(), &date_key,
                                )
                                class:in-range=move || {
                                    calendar_selection.get()
                                        .unwrap_or_else(|| CalendarSelection::single(default_day))
                                        .contains(start_time)
                                }
                                aria-pressed=move || calendar_selection.get()
                                    .unwrap_or_else(|| CalendarSelection::single(default_day))
                                    .contains(start_time)
                                    .to_string()
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
                                    .and_then(|reviews| review_event_day_severity(
                                        &reviews, start_time, end_time,
                                    ))
                                    .map_or("activity-none", severity_class)
                                    aria-hidden="true"
                                ></i>
                            </button> }
                        }).collect_view()}
                    </div>
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

fn review_events_for_filter(events: Vec<ReviewEvent>, filter: ActivityFilter) -> Vec<ReviewEvent> {
    let Some(severity) = filter.severity() else {
        return events;
    };
    events
        .into_iter()
        .filter(|event| event.severity == severity)
        .collect()
}

fn review_and_motion_events(
    mut reviews: Vec<ReviewEvent>,
    motion: &[MotionActivity],
) -> Vec<ReviewEvent> {
    reviews.extend(
        consolidated_motion_ranges(motion)
            .into_iter()
            .map(motion_review_event),
    );
    reviews.sort_by(|left, right| right.start_time.total_cmp(&left.start_time));
    reviews
}

fn consolidated_motion_ranges(motion: &[MotionActivity]) -> Vec<RecordingRange> {
    let mut ranges = motion
        .iter()
        .filter(|activity| activity.motion > 0.0)
        .flat_map(|activity| {
            activity
                .camera
                .split(',')
                .filter(|camera| !camera.is_empty())
                .map(|camera| RecordingRange {
                    camera: camera.to_owned(),
                    start_time: activity.start_time,
                    end_time: activity.start_time + MOTION_BUCKET_SECONDS,
                })
        })
        .collect::<Vec<_>>();
    ranges.sort_by(|left, right| {
        left.camera
            .cmp(&right.camera)
            .then_with(|| left.start_time.total_cmp(&right.start_time))
    });

    let mut consolidated = Vec::<RecordingRange>::new();
    for range in ranges {
        match consolidated.last_mut() {
            Some(previous) if motion_ranges_touch(previous, &range) => {
                previous.end_time = previous.end_time.max(range.end_time);
            }
            _ => consolidated.push(range),
        }
    }
    consolidated
}

fn motion_ranges_touch(previous: &RecordingRange, next: &RecordingRange) -> bool {
    let same_camera = previous.camera == next.camera;
    let intervals_touch = next.start_time <= previous.end_time;
    same_camera && intervals_touch
}

fn motion_review_event(range: RecordingRange) -> ReviewEvent {
    let encoded_camera = js_sys::encode_uri_component(&range.camera);
    ReviewEvent {
        id: format!("motion-{}-{}", range.camera, range.start_time),
        camera: range.camera,
        start_time: range.start_time,
        end_time: Some(range.end_time),
        severity: ReviewSeverity::SignificantMotion,
        thumb_path: format!(
            "/api/{encoded_camera}/start/{}/end/{}/preview.gif",
            range.start_time, range.end_time
        ),
        data: corvette_api::ReviewEventData::default(),
    }
}

fn review_event_day_severity(
    events: &[ReviewEvent],
    day_start: f64,
    day_end: f64,
) -> Option<ReviewSeverity> {
    day_severity_from_ranges(
        events
            .iter()
            .map(|event| (event.start_time, event.end_time, event.severity)),
        day_start,
        day_end,
    )
}

fn recent_activity_empty_heading() -> String {
    format!("No events in the last {RECENT_ACTIVITY_HOURS} hours")
}

fn review_event_title(event: &ReviewEvent) -> String {
    event
        .data
        .sub_labels
        .first()
        .or_else(|| event.data.objects.first())
        .or_else(|| event.data.audio.first())
        .cloned()
        .unwrap_or_else(|| severity_label(event.severity).to_owned())
}

fn review_thumbnail_url(thumb_path: &str) -> String {
    let relative_path = thumb_path
        .strip_prefix("/media/frigate/")
        .unwrap_or_else(|| thumb_path.trim_start_matches('/'));
    format!("/{relative_path}")
}

/// Renders the recording browser behind the route-level lazy boundary.
#[component]
pub(crate) fn RecordingBrowser() -> impl IntoView {
    let cameras = LocalResource::new(api::fetch_cameras);
    let events = LocalResource::new(api::fetch_events);
    let selected_event = RwSignal::new(None);
    let recording_range = RwSignal::new(None::<RecordingRange>);
    let calendar_selection = RwSignal::new(None::<CalendarSelection>);
    let recording_failed = RwSignal::new(false);
    let recording_error = RwSignal::new(None::<String>);
    let recording_camera = RwSignal::new(String::new());
    let calendar_days = recent_calendar_days(21);
    let calendar_after = calendar_days.first().map_or(0.0, |day| day.start_time);
    let timezone = browser_timezone();
    let start_clock = RwSignal::new("00:00".to_owned());
    let end_clock = RwSignal::new("23:59".to_owned());
    let review_activity = LocalResource::new(move || {
        let camera = recording_camera.get();
        async move {
            api::fetch_review_activity(&camera, calendar_after, js_sys::Date::now() / 1_000.0).await
        }
    });
    let recording_days = LocalResource::new(move || {
        let camera = recording_camera.get();
        let timezone = timezone.clone();
        async move { api::fetch_recording_days(&camera, &timezone).await }
    });
    let recording_clips = LocalResource::new(move || {
        let range = recording_range.get();
        async move {
            let Some(range) = range else {
                return Ok(RecordingMedia {
                    clips: Vec::new(),
                    motion_ranges: Vec::new(),
                });
            };
            api::fetch_recording_segments(&range.camera, range.start_time, range.end_time)
                .await
                .map(|segments| recording_media(&range, segments))
        }
    });
    let preview_clips = LocalResource::new(move || {
        let range = recording_range.get();
        async move {
            let Some(range) = range else {
                return Ok(Vec::new());
            };
            api::fetch_preview_clips(&range.camera, range.start_time, range.end_time).await
        }
    });

    Effect::new(move |_| {
        if recording_camera.get().is_empty()
            && let Some(Ok(cameras)) = cameras.get()
            && let Some(camera) = cameras.first()
        {
            recording_camera.set(camera.name.clone());
        }
    });

    Effect::new(move |_| {
        let event_id = web_sys::window()
            .and_then(|window| window.location().search().ok())
            .and_then(|query| query.strip_prefix("?event=").map(str::to_owned));
        let Some(event_id) = event_id else {
            return;
        };
        let Some(Ok(events)) = events.get() else {
            return;
        };
        selected_event.set(events.into_iter().find(|event| event.id == event_id));
    });

    provide_context(RecordingContext {
        cameras,
        selected_event,
        recording_range,
        calendar_selection,
        recording_failed,
        recording_error,
        recording_camera,
        calendar_days,
        start_clock,
        end_clock,
        review_activity,
        recording_days,
        recording_clips,
        preview_clips,
    });

    view! {
        <PageShell active_path="/recordings">
            <section id="recordings" class="page-section" aria-label="Recording playback">
                <p class="eyebrow">"Playback"</p>
                <h1>"Recordings"</h1>
                <RecordingControls/>
                <RecordingPlayback/>
            </section>
        </PageShell>
    }
}

#[derive(Clone)]
struct RecordingContext {
    cameras: LocalResource<Result<Vec<Camera>, String>>,
    selected_event: RwSignal<Option<Event>>,
    recording_range: RwSignal<Option<RecordingRange>>,
    calendar_selection: RwSignal<Option<CalendarSelection>>,
    recording_failed: RwSignal<bool>,
    recording_error: RwSignal<Option<String>>,
    recording_camera: RwSignal<String>,
    calendar_days: Vec<CalendarDay>,
    start_clock: RwSignal<String>,
    end_clock: RwSignal<String>,
    review_activity: LocalResource<Result<Vec<ReviewSegment>, String>>,
    recording_days: LocalResource<Result<std::collections::BTreeMap<String, bool>, String>>,
    recording_clips: LocalResource<Result<RecordingMedia, String>>,
    preview_clips: LocalResource<Result<Vec<PreviewClip>, String>>,
}

impl RecordingContext {
    fn load(&self, start_time: f64, end_time: f64) {
        self.recording_failed.set(false);
        self.recording_error.set(None);
        let camera = self.recording_camera.get_untracked();
        if camera.is_empty() {
            self.recording_error.set(Some(
                "Choose a camera before loading a recording.".to_owned(),
            ));
            return;
        }
        self.selected_event.set(None);
        self.recording_range.set(Some(RecordingRange {
            camera,
            start_time,
            end_time,
        }));
    }
}

#[component]
fn PageShell(active_path: &'static str, children: Children) -> impl IntoView {
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

#[component]
fn RecordingControls() -> impl IntoView {
    let context = expect_context::<RecordingContext>();
    let selection_error = context.recording_error;
    view! {
        {move || selection_error.get().map(|error| view! {
            <p class="recording-error" role="alert">{error}</p>
        })}
        <div class="recording-browser">
            <aside class="recording-presets" aria-label="Recording shortcuts">
                <h2>"Quick ranges"</h2>
                {RECORDING_PRESETS.map(|preset| {
                    let context = context.clone();
                    view! { <button type="button" on:click=move |_| {
                        let (start_time, end_time) = preset.range();
                        context.calendar_selection.set(None);
                        context.load(start_time, end_time);
                    }>{preset.label()}</button> }
                })}
            </aside>
            <div class="recording-calendar">
                <CameraAndTimeControls/>
                <div class="calendar-heading"><div>
                    <h2>"Choose dates"</h2>
                    <p>"Tap a start day, then an end day to load a range."</p>
                </div></div>
                <RecordingCalendarDays/>
                <RecordingCalendarStatus/>
                <ActivityLegend/>
            </div>
        </div>
    }
}

#[component]
fn ActivityLegend() -> impl IntoView {
    view! { <div class="activity-legend" aria-label="Activity colors">
        <span><i class="activity-motion"></i>"Motion"</span>
        <span><i class="activity-detection"></i>"Detection"</span>
        <span><i class="activity-alert"></i>"Alert"</span>
    </div> }
}

#[component]
fn CameraAndTimeControls() -> impl IntoView {
    let context = expect_context::<RecordingContext>();
    let camera_context = context.clone();
    view! {
        <label><span>"Camera"</span><select
            prop:value=move || context.recording_camera.get()
            on:change=move |event| {
                context.recording_camera.set(event_target_value(&event));
                context.recording_range.set(None);
                context.calendar_selection.set(None);
            }
        >{move || camera_context.cameras.get().and_then(Result::ok).unwrap_or_default()
            .into_iter().map(|camera| view! {
                <option value=camera.name>{camera.display_name}</option>
            }).collect_view()}</select></label>
        <div class="recording-times">
            <label><span>"From"</span><input type="time"
                prop:value=move || context.start_clock.get()
                on:input=move |event| context.start_clock.set(event_target_value(&event))
            /></label>
            <label><span>"To"</span><input type="time"
                prop:value=move || context.end_clock.get()
                on:input=move |event| context.end_clock.set(event_target_value(&event))
            /></label>
        </div>
    }
}

#[component]
fn RecordingCalendarDays() -> impl IntoView {
    let context = expect_context::<RecordingContext>();
    view! { <div class="calendar-grid" role="group" aria-label="Recent days">
        {context.calendar_days.clone().into_iter().map(|day| {
            let day_start = day.start_time;
            let date_key = day.date_key.clone();
            let label_key = date_key.clone();
            let accessible_label = day.accessible_label.clone();
            let click_context = context.clone();
            view! { <button type="button"
                disabled=move || !recording_day_available(
                    context.recording_days.get(), &date_key,
                )
                class:in-range=move || context.calendar_selection.get()
                    .is_some_and(|selection| selection.contains(day_start))
                aria-pressed=move || context.calendar_selection.get()
                    .is_some_and(|selection| selection.contains(day_start))
                    .to_string()
                on:click=move |_| select_calendar_day(&click_context, day_start)
                aria-label=move || calendar_day_label(
                    context.recording_days.get(), &accessible_label, &label_key,
                )
            >
                <span>{day.weekday}</span><strong>{day.day_number}</strong><small>{day.month}</small>
                <i class=move || context.review_activity.get().and_then(Result::ok)
                    .and_then(|reviews| day_severity(&reviews, day_start, next_local_day(day_start)))
                    .map_or("activity-none", severity_class) aria-hidden="true"></i>
            </button> }
        }).collect_view()}
    </div> }
}

fn select_calendar_day(context: &RecordingContext, day_start: f64) {
    let selection = select_calendar_range(context.calendar_selection.get_untracked(), day_start);
    let (Some(start_time), Some(end_time)) = (
        local_day_time(selection.start_day, &context.start_clock.get_untracked()),
        local_day_time(selection.end_day, &context.end_clock.get_untracked()),
    ) else {
        context.recording_error.set(Some(
            "Enter both times as 24-hour HH:MM before choosing a date.".to_owned(),
        ));
        return;
    };
    if start_time >= end_time {
        context.recording_error.set(Some(
            "The end time must be later than the start time.".to_owned(),
        ));
        return;
    }

    context.calendar_selection.set(Some(selection));
    context.load(start_time, end_time);
}

fn calendar_day_label(
    days: Option<Result<std::collections::BTreeMap<String, bool>, String>>,
    accessible_label: &str,
    date_key: &str,
) -> String {
    match days {
        None => format!("{accessible_label}, checking recording availability"),
        Some(Err(_)) => format!("{accessible_label}, recording availability unavailable"),
        Some(Ok(days)) if has_recording(&days, date_key) => {
            format!("{accessible_label}, recordings available")
        }
        Some(Ok(_)) => format!("{accessible_label}, no recordings"),
    }
}

fn recording_day_available(
    days: Option<Result<std::collections::BTreeMap<String, bool>, String>>,
    date_key: &str,
) -> bool {
    days.and_then(Result::ok)
        .is_some_and(|days| has_recording(&days, date_key))
}

#[component]
fn RecordingCalendarStatus() -> impl IntoView {
    let context = expect_context::<RecordingContext>();
    view! { <CalendarRecordingStatus
        recording_days=context.recording_days
        empty_detail="No retained recordings are available for this camera."
    /> }
}

#[component]
fn CalendarRecordingStatus(
    recording_days: LocalResource<Result<std::collections::BTreeMap<String, bool>, String>>,
    empty_detail: &'static str,
) -> impl IntoView {
    move || match recording_days.get() {
        None => {
            view! { <p class="calendar-status">"Checking recording availability…"</p> }.into_any()
        }
        Some(Err(error)) => {
            view! { <p class="recording-error" role="alert">{error}</p> }.into_any()
        }
        Some(Ok(days)) if days.is_empty() => {
            view! { <p class="calendar-status">{empty_detail}</p> }.into_any()
        }
        Some(Ok(_)) => {
            view! { <p class="calendar-status">"Dimmed dates have no retained recordings."</p> }
                .into_any()
        }
    }
}

#[component]
fn RecordingPlayback() -> impl IntoView {
    let context = expect_context::<RecordingContext>();
    move || {
        if let Some(event) = context.selected_event.get() {
            view! { <EventPlayback event/> }.into_any()
        } else if let Some(range) = context.recording_range.get() {
            view! { <RangePlayback range/> }.into_any()
        } else {
            view! { <Status
                heading="Choose a time range"
                detail="Load continuous footage above, or choose a recent event."
                glyph=StatusGlyph::Recording
            /> }
            .into_any()
        }
    }
}

#[component]
fn EventPlayback(event: Event) -> impl IntoView {
    let context = expect_context::<RecordingContext>();
    let clip_url = format!("/api/events/{}/clip.mp4", event.id);
    let heading = format!(
        "{} on {}",
        event.sub_label.as_deref().unwrap_or(&event.label),
        event.camera,
    );
    view! {
        <div class="recording-heading playback-heading">
            <div><h2>{heading}</h2><p>{format_event_time(event.start_time)}</p></div>
            <button type="button" on:click=move |_| {
                context.recording_failed.set(false);
                context.selected_event.set(None);
            }>"Close"</button>
        </div>
        <video class="recording-player" src=clip_url controls autoplay playsinline
            on:error=move |_| context.recording_failed.set(true)
        >"This browser cannot play the event recording."</video>
        {move || context.recording_failed.get().then(|| view! {
            <p class="recording-error" role="alert">
                "The recording could not be loaded. It may have expired or still be processing."
            </p>
        })}
    }
}

#[component]
fn RangePlayback(range: RecordingRange) -> impl IntoView {
    let context = expect_context::<RecordingContext>();
    let heading = format!("{} recording", range.camera);
    let detail = format!(
        "{} – {}",
        format_event_time(range.start_time),
        format_event_time(range.end_time),
    );
    view! {
        <div class="playback-heading"><h2>{heading}</h2><p>{detail}</p></div>
        {move || match (
            context.recording_clips.get(),
            context.preview_clips.get(),
            context.review_activity.get(),
        ) {
            (None, _, _) | (_, None, _) | (_, _, None) => view! { <Status
                heading="Loading recordings"
                detail="Checking retained footage and activity in the selected range."
                glyph=StatusGlyph::Recording
            /> }.into_any(),
            (Some(Err(error)), _, _) | (_, Some(Err(error)), _) | (_, _, Some(Err(error))) => {
                view! { <p class="recording-error" role="alert">{error}</p> }.into_any()
            }
            (Some(Ok(media)), _, Some(Ok(_))) if media.clips.is_empty() => view! {
                <p class="recording-error" role="alert">
                    "Frigate has no retained recordings for this camera and time range."
                </p>
            }.into_any(),
            (Some(Ok(media)), Some(Ok(previews)), Some(Ok(reviews))) => view! {
                <RecordingList
                    selection=range.clone()
                    clips=media.clips
                    motion_ranges=media.motion_ranges
                    previews
                    reviews
                />
            }.into_any(),
        }}
    }
}

#[component]
fn RecordingList(
    selection: RecordingRange,
    clips: Vec<RecordingClip>,
    motion_ranges: Vec<RecordingRange>,
    previews: Vec<PreviewClip>,
    reviews: Vec<ReviewSegment>,
) -> impl IntoView {
    let context = expect_context::<RecordingContext>();
    view! {
        <RecordingTimeline selection clips motion_ranges previews reviews/>
        {move || context.recording_failed.get().then(|| view! {
            <p class="recording-error" role="alert">"One or more recordings could not be loaded."</p>
        })}
    }.into_any()
}

#[component]
fn RecordingTimeline(
    selection: RecordingRange,
    clips: Vec<RecordingClip>,
    motion_ranges: Vec<RecordingRange>,
    previews: Vec<PreviewClip>,
    reviews: Vec<ReviewSegment>,
) -> impl IntoView {
    let range_start = selection.start_time;
    let range_end = selection.end_time;
    let timeline_clips = clips.clone();
    let selectable_clips = clips.clone();
    let activity_clips = clips.clone();
    let playback_activities = timeline_activities(&motion_ranges, &reviews, range_start, range_end);
    let active_activity = RwSignal::new(None::<TimelineActivity>);
    let selected_time = RwSignal::new(playable_time(&clips, range_start));
    let uses_preview = RwSignal::new(true);

    view! {
        <section class="recording-timeline" aria-label="Recording timeline">
            <div class="timeline-heading">
                <h3>"Timeline"</h3>
                <output>{move || format_event_time(selected_time.get())}</output>
            </div>
            <div class="timeline-track">
                {timeline_clips.into_iter().map(|clip| {
                    let style = timeline_segment_style(&clip.range, range_start, range_end);
                    let start_time = clip.range.start_time;
                    let label = format!("Play from {}", format_event_time(start_time));
                    view! { <button
                        type="button"
                        class="timeline-availability"
                        style=style
                        aria-label=label
                        on:click=move |_| {
                            active_activity.set(None);
                            uses_preview.set(true);
                            selected_time.set(start_time);
                        }
                    ></button> }
                }).collect_view()}
                {motion_ranges.into_iter().map(|range| {
                    let style = timeline_segment_style(&range, range_start, range_end);
                    let start_time = range.start_time;
                    let click_clips = activity_clips.clone();
                    let label = format!(
                        "Motion recording at {}",
                        format_event_time(start_time),
                    );
                    view! { <button
                        type="button"
                        class="timeline-activity timeline-motion-recording activity-motion"
                        style=style
                        aria-label=label
                        on:click=move |_| {
                            active_activity.set(Some(TimelineActivity {
                                start_time,
                                end_time: range.end_time,
                            }));
                            uses_preview.set(true);
                            selected_time.set(playable_time(&click_clips, start_time));
                        }
                    ></button> }
                }).collect_view()}
                {reviews.into_iter().filter_map(|review| {
                    let is_point = review.end_time.is_some_and(|end_time| {
                        end_time <= review.start_time
                    });
                    let (start_time, end_time) = review_timeline_bounds(
                        review.start_time,
                        review.end_time,
                        range_start,
                        range_end,
                    )?;
                    let range = RecordingRange {
                        camera: String::new(),
                        start_time,
                        end_time,
                    };
                    let style = timeline_segment_style(&range, range_start, range_end);
                    let severity = review.severity;
                    let click_clips = activity_clips.clone();
                    let label = format!(
                        "{} activity at {}",
                        severity_label(severity),
                        format_event_time(start_time),
                    );
                    Some(view! { <button
                        type="button"
                        class=format!(
                            "timeline-activity {}{}",
                            severity_class(severity),
                            if is_point { " timeline-point" } else { "" },
                        )
                        style=style
                        aria-label=label
                        on:click=move |_| {
                            active_activity.set(Some(TimelineActivity {
                                start_time,
                                end_time,
                            }));
                            uses_preview.set(true);
                            selected_time.set(playable_time(&click_clips, start_time));
                        }
                    ></button> })
                }).collect_view()}
                <input
                    type="range"
                    min=range_start
                    max=range_end
                    step="1"
                    prop:value=move || selected_time.get()
                    aria-label="Recording playhead"
                    on:input=move |event| {
                        active_activity.set(None);
                        uses_preview.set(true);
                        let requested_time = event_target_value(&event)
                            .parse::<f64>()
                            .unwrap_or(range_start);
                        selected_time.set(playable_time(&selectable_clips, requested_time));
                    }
                />
            </div>
            <TimelinePlayer
                clips
                previews
                activities=playback_activities
                active_activity
                selected_time
                uses_preview
            />
        </section>
    }
}

#[component]
fn TimelinePlayer(
    clips: Vec<RecordingClip>,
    previews: Vec<PreviewClip>,
    activities: Vec<TimelineActivity>,
    active_activity: RwSignal<Option<TimelineActivity>>,
    selected_time: RwSignal<f64>,
    uses_preview: RwSignal<bool>,
) -> impl IntoView {
    let video = NodeRef::<leptos::html::Video>::new();
    let initial_source = playback_source(&clips, &previews, selected_time.get_untracked(), true);
    let sources = StoredValue::new((clips, previews));
    let activities = StoredValue::new(activities);
    let active_source = RwSignal::new(initial_source);
    let pending_seek = RwSignal::new(None::<f64>);
    let is_playing = RwSignal::new(false);
    let playback_error = RwSignal::new(None::<String>);
    let starts_after_load = RwSignal::new(false);
    let tracks_playback = RwSignal::new(false);
    let activity_playback = ActivityPlaybackState {
        sources,
        activities,
        active_source,
        active_activity,
        selected_time,
        starts_after_load,
        tracks_playback,
        playback_error,
    };

    Effect::new(move |_| {
        let selected_time = selected_time.get();
        let uses_preview = uses_preview.get();
        let next_source = sources.with_value(|(clips, previews)| {
            playback_source(clips, previews, selected_time, uses_preview)
        });
        let source_changed = active_source.with_untracked(|active| {
            active.as_ref().map(|source| &source.url)
                != next_source.as_ref().map(|source| &source.url)
        });
        if source_changed {
            tracks_playback.set(false);
            active_source.set(next_source);
            return;
        }

        let Some(video) = video.get() else {
            return;
        };
        if video.ready_state() == 0 {
            return;
        }
        active_source.with_untracked(|source| {
            let Some(source) = source else {
                return;
            };
            let requested_offset = selected_time - source.start_time;
            if (video.current_time() - requested_offset).abs() < 0.25 {
                return;
            }
            if video.seeking() {
                pending_seek.set(Some(requested_offset));
            } else {
                tracks_playback.set(false);
                video.set_current_time(requested_offset);
            }
        });
    });

    view! {
        <div class="timeline-player-controls">
            <button type="button" on:click=move |_| {
                let Some(video) = video.get() else {
                    return;
                };
                playback_error.set(None);
                if video.paused() {
                    if uses_preview.get_untracked() {
                        starts_after_load.set(true);
                        uses_preview.set(false);
                        return;
                    }
                    if let Err(error) = video.play() {
                        playback_error.set(Some(format!(
                            "The recording could not start: {error:?}",
                        )));
                    }
                } else if let Err(error) = video.pause() {
                    playback_error.set(Some(format!(
                        "The recording could not pause: {error:?}",
                    )));
                }
            }>{move || if is_playing.get() { "Pause" } else { "Play" }}</button>
        </div>
        <video
            node_ref=video
            class="recording-player timeline-player"
            src=move || active_source.get().map(|source| source.url)
            poster=move || active_source.get().and_then(|source| source.poster)
            preload="auto"
            muted
            controls
            playsinline
            on:play=move |_| {
                if uses_preview.get_untracked() {
                    if let Some(video) = video.get()
                        && let Err(error) = video.pause()
                    {
                        playback_error.set(Some(format!(
                            "The preview could not pause: {error:?}",
                        )));
                        return;
                    }
                    starts_after_load.set(true);
                    uses_preview.set(false);
                    return;
                }
                is_playing.set(true);
            }
            on:pause=move |_| is_playing.set(false)
            on:ended=move |_| {
                if let Some(video) = video.get() {
                    activity_playback.advance(&video, true);
                }
            }
            on:loadedmetadata=move |_| {
            let Some(video) = video.get() else {
                return;
            };
            active_source.with_untracked(|source| {
                if let Some(source) = source {
                    let requested_offset =
                        (selected_time.get_untracked() - source.start_time).max(0.0);
                    if (video.current_time() - requested_offset).abs() >= 0.25 {
                        tracks_playback.set(false);
                        video.set_current_time(requested_offset);
                    } else {
                        tracks_playback.set(true);
                    }
                }
            });
            if starts_after_load.get_untracked() {
                starts_after_load.set(false);
                if let Err(error) = video.play() {
                    playback_error.set(Some(format!(
                        "The recording could not start: {error:?}",
                    )));
                }
            }
            }
            on:seeked=move |_| {
            let Some(video) = video.get() else {
                return;
            };
            let Some(requested_offset) = pending_seek.get_untracked() else {
                return;
            };
            if (video.current_time() - requested_offset).abs() >= 0.5 {
                video.set_current_time(requested_offset);
            } else {
                pending_seek.set(None);
                tracks_playback.set(true);
            }
            }
            on:timeupdate=move |_| {
            if !tracks_playback.get_untracked() {
                return;
            }
            let Some(video) = video.get() else {
                return;
            };
            if activity_playback.advance(&video, false) {
                return;
            }
            active_source.with_untracked(|source| {
                if let Some(source) = source {
                    let playback_time = source.start_time + video.current_time();
                    selected_time.set(playback_time.min(source.end_time - 1.0));
                }
            });
            }
        >"This browser cannot play the selected recording."</video>
        {move || playback_error.get().map(|error| view! {
            <p class="recording-error" role="alert">{error}</p>
        })}
    }
}

#[derive(Clone, Copy)]
struct ActivityPlaybackState {
    sources: StoredValue<(Vec<RecordingClip>, Vec<PreviewClip>)>,
    activities: StoredValue<Vec<TimelineActivity>>,
    active_source: RwSignal<Option<PlaybackSource>>,
    active_activity: RwSignal<Option<TimelineActivity>>,
    selected_time: RwSignal<f64>,
    starts_after_load: RwSignal<bool>,
    tracks_playback: RwSignal<bool>,
    playback_error: RwSignal<Option<String>>,
}

impl ActivityPlaybackState {
    fn advance(self, video: &web_sys::HtmlVideoElement, media_ended: bool) -> bool {
        let Some(activity) = self.active_activity.get_untracked() else {
            return false;
        };
        let Some(source) = self.active_source.get_untracked() else {
            return false;
        };
        let playback_time = source.start_time + video.current_time();
        if !media_ended && playback_time < activity.end_time {
            return false;
        }

        let next_activity = self
            .activities
            .with_value(|activities| next_timeline_activity(activities, activity));
        if let Some(next_activity) = next_activity {
            let next_time = self
                .sources
                .with_value(|(clips, _)| playable_time(clips, next_activity.start_time));
            let source_changes = self
                .sources
                .with_value(|(clips, previews)| playback_source(clips, previews, next_time, false))
                .is_some_and(|next_source| next_source.url != source.url);
            if source_changes {
                self.starts_after_load.set(true);
            }
            self.active_activity.set(Some(next_activity));
            self.selected_time.set(next_time);
        } else {
            self.tracks_playback.set(false);
            let final_time = activity.end_time.min(source.end_time - 0.001);
            self.selected_time.set(final_time);
            if let Err(error) = video.pause() {
                self.playback_error
                    .set(Some(format!("The recording could not stop: {error:?}")));
            }
        }
        true
    }
}

#[derive(Clone)]
struct PlaybackSource {
    url: String,
    poster: Option<String>,
    start_time: f64,
    end_time: f64,
}

fn playback_source(
    clips: &[RecordingClip],
    previews: &[PreviewClip],
    time: f64,
    uses_preview: bool,
) -> Option<PlaybackSource> {
    if uses_preview
        && let Some(preview) = previews
            .iter()
            .find(|preview| preview.start <= time && time < preview.end)
    {
        return Some(PlaybackSource {
            url: preview.src.clone(),
            poster: None,
            start_time: preview.start,
            end_time: preview.end,
        });
    }

    clip_at_time(clips, time).map(|clip| PlaybackSource {
        url: clip.range.clip_url(),
        poster: Some(clip.range.poster_url()),
        start_time: clip.range.start_time,
        end_time: clip.range.end_time,
    })
}

fn clip_at_time(clips: &[RecordingClip], time: f64) -> Option<RecordingClip> {
    clips
        .iter()
        .find(|clip| clip.range.start_time <= time && time < clip.range.end_time)
        .cloned()
}

fn playable_time(clips: &[RecordingClip], requested_time: f64) -> f64 {
    if clip_at_time(clips, requested_time).is_some() {
        return requested_time;
    }

    clips
        .iter()
        .map(|clip| {
            let last_playable_time = (clip.range.end_time - 1.0).max(clip.range.start_time);
            requested_time.clamp(clip.range.start_time, last_playable_time)
        })
        .min_by(|left, right| {
            (left - requested_time)
                .abs()
                .total_cmp(&(right - requested_time).abs())
        })
        .expect("recording timeline requires at least one clip")
}

fn timeline_segment_style(range: &RecordingRange, start_time: f64, end_time: f64) -> String {
    let duration = end_time - start_time;
    let left = (range.start_time - start_time) / duration * 100.0;
    let width = (range.end_time - range.start_time) / duration * 100.0;
    format!("left: {left:.4}%; width: {width:.4}%")
}

fn review_timeline_bounds(
    review_start: f64,
    review_end: Option<f64>,
    range_start: f64,
    range_end: f64,
) -> Option<(f64, f64)> {
    let review_end = review_end.unwrap_or(range_end);
    if review_start >= range_end || review_end < range_start {
        return None;
    }
    let start_time = review_start.max(range_start);
    let end_time = review_end
        .min(range_end)
        .max((start_time + 1.0).min(range_end));
    (start_time < end_time).then_some((start_time, end_time))
}

fn timeline_activities(
    motion_ranges: &[RecordingRange],
    reviews: &[ReviewSegment],
    range_start: f64,
    range_end: f64,
) -> Vec<TimelineActivity> {
    let mut activities = motion_ranges
        .iter()
        .map(|range| TimelineActivity {
            start_time: range.start_time,
            end_time: range.end_time,
        })
        .chain(reviews.iter().filter_map(|review| {
            review_timeline_bounds(review.start_time, review.end_time, range_start, range_end).map(
                |(start_time, end_time)| TimelineActivity {
                    start_time,
                    end_time,
                },
            )
        }))
        .collect::<Vec<_>>();
    activities.sort_by(|left, right| left.start_time.total_cmp(&right.start_time));
    activities.dedup();
    activities
}

fn next_timeline_activity(
    activities: &[TimelineActivity],
    current: TimelineActivity,
) -> Option<TimelineActivity> {
    activities
        .iter()
        .copied()
        .find(|activity| activity.start_time >= current.end_time)
}

#[derive(Clone, Copy)]
enum StatusGlyph {
    Camera,
    Event,
    Recording,
}

/// Renders an asynchronous content state with a consistent accessible heading.
#[component]
fn Status(heading: impl IntoView, detail: impl IntoView, glyph: StatusGlyph) -> impl IntoView {
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

fn format_event_time(unix_seconds: f64) -> String {
    let date = js_sys::Date::new(&JsValue::from_f64(unix_seconds * 1_000.0));
    format_date_time_parts(
        date.get_date(),
        date.get_month(),
        date.get_full_year(),
        date.get_hours(),
        date.get_minutes(),
    )
}

fn format_date_time_parts(day: u32, month: u32, year: u32, hour: u32, minute: u32) -> String {
    format!("{day} {} {year}, {hour:02}:{minute:02}", month_name(month))
}

#[derive(Clone, Debug, PartialEq)]
struct RecordingRange {
    camera: String,
    start_time: f64,
    end_time: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct RecordingClip {
    range: RecordingRange,
}

#[derive(Clone, Debug, PartialEq)]
struct RecordingMedia {
    clips: Vec<RecordingClip>,
    motion_ranges: Vec<RecordingRange>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TimelineActivity {
    start_time: f64,
    end_time: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActivityFilter {
    All,
    SignificantMotion,
    Detection,
    Alert,
}

impl ActivityFilter {
    const ALL: [Self; 4] = [
        Self::All,
        Self::Alert,
        Self::Detection,
        Self::SignificantMotion,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::SignificantMotion => "Motion",
            Self::Detection => "Detections",
            Self::Alert => "Alerts",
        }
    }

    const fn severity(self) -> Option<ReviewSeverity> {
        match self {
            Self::All => None,
            Self::SignificantMotion => Some(ReviewSeverity::SignificantMotion),
            Self::Detection => Some(ReviewSeverity::Detection),
            Self::Alert => Some(ReviewSeverity::Alert),
        }
    }
}

impl RecordingRange {
    fn clip_url(&self) -> String {
        let camera = js_sys::encode_uri_component(&self.camera);
        format!(
            "/api/{camera}/start/{}/end/{}/clip.mp4",
            self.start_time, self.end_time
        )
    }

    fn poster_url(&self) -> String {
        let camera = js_sys::encode_uri_component(&self.camera);
        let frame_time = self.start_time.midpoint(self.end_time);
        format!("/api/{camera}/recordings/{frame_time}/snapshot.jpg?height=720")
    }
}

fn recording_media(
    selection: &RecordingRange,
    mut segments: Vec<RecordingSegment>,
) -> RecordingMedia {
    const MAX_SEGMENT_GAP_SECONDS: f64 = 1.0;

    segments.sort_by(|left, right| left.start_time.total_cmp(&right.start_time));
    let mut clip_ranges = Vec::<RecordingRange>::new();
    let mut motion_ranges = Vec::<RecordingRange>::new();
    for segment in segments {
        let start_time = segment.start_time.max(selection.start_time);
        let end_time = segment.end_time.min(selection.end_time);
        let has_motion = segment.motion.is_some_and(|motion| motion > 0.0);
        if start_time >= end_time {
            continue;
        }

        if has_motion {
            push_contiguous_range(
                &mut motion_ranges,
                selection,
                start_time,
                end_time,
                MAX_SEGMENT_GAP_SECONDS,
            );
        }
        push_contiguous_range(
            &mut clip_ranges,
            selection,
            start_time,
            end_time,
            MAX_SEGMENT_GAP_SECONDS,
        );
    }
    let mut clips = clip_ranges
        .into_iter()
        .map(|range| RecordingClip { range })
        .collect::<Vec<_>>();
    clips.reverse();
    RecordingMedia {
        clips,
        motion_ranges,
    }
}

fn push_contiguous_range(
    ranges: &mut Vec<RecordingRange>,
    selection: &RecordingRange,
    start_time: f64,
    end_time: f64,
    maximum_gap: f64,
) {
    if let Some(range) = ranges.last_mut()
        && start_time <= range.end_time + maximum_gap
    {
        range.end_time = range.end_time.max(end_time);
        return;
    }
    ranges.push(RecordingRange {
        camera: selection.camera.clone(),
        start_time,
        end_time,
    });
}

const RECORDING_PRESETS: [RecordingPreset; 5] = [
    RecordingPreset::Today,
    RecordingPreset::Yesterday,
    RecordingPreset::LastTenMinutes,
    RecordingPreset::LastHour,
    RecordingPreset::LastSevenDays,
];

#[derive(Clone, Copy)]
enum RecordingPreset {
    Today,
    Yesterday,
    LastTenMinutes,
    LastHour,
    LastSevenDays,
}

impl RecordingPreset {
    const fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Yesterday => "Yesterday",
            Self::LastTenMinutes => "Last 10 minutes",
            Self::LastHour => "Last hour",
            Self::LastSevenDays => "Last 7 days",
        }
    }

    fn range(self) -> (f64, f64) {
        let now = js_sys::Date::new_0();
        let end_time = now.get_time() / 1_000.0;
        let today = local_day_start(&now);
        match self {
            Self::Today => (today, end_time),
            Self::Yesterday => {
                let yesterday = recent_calendar_days(2)[0].start_time;
                (yesterday, today)
            }
            Self::LastTenMinutes => (end_time - 600.0, end_time),
            Self::LastHour => (end_time - 3_600.0, end_time),
            Self::LastSevenDays => (recent_calendar_days(7)[0].start_time, end_time),
        }
    }
}

#[derive(Clone)]
struct CalendarDay {
    start_time: f64,
    date_key: String,
    weekday: String,
    day_number: u32,
    month: String,
    accessible_label: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CalendarSelection {
    start_day: f64,
    end_day: f64,
    awaiting_end: bool,
}

impl CalendarSelection {
    const fn single(day: f64) -> Self {
        Self {
            start_day: day,
            end_day: day,
            awaiting_end: true,
        }
    }

    fn contains(self, day: f64) -> bool {
        day >= self.start_day && day <= self.end_day
    }
}

fn select_calendar_range(
    current: Option<CalendarSelection>,
    selected_day: f64,
) -> CalendarSelection {
    let Some(current) = current.filter(|selection| selection.awaiting_end) else {
        return CalendarSelection::single(selected_day);
    };
    CalendarSelection {
        start_day: current.start_day.min(selected_day),
        end_day: current.start_day.max(selected_day),
        awaiting_end: false,
    }
}

fn recent_calendar_days(count: u32) -> Vec<CalendarDay> {
    // Days are stepped back from local noon, not from the current time: a fixed
    // 24-hour step crosses two date boundaries on a 23-hour daylight-saving day
    // and none on a 25-hour one, so the strip loses the short day and repeats the
    // long one. No transition moves noon far enough to change its calendar date.
    const NOON_OFFSET_MILLIS: f64 = 43_200_000.0;

    let noon = local_day_start(&js_sys::Date::new_0()).mul_add(1_000.0, NOON_OFFSET_MILLIS);
    (0..count)
        .rev()
        .map(|offset| {
            let date = js_sys::Date::new(&JsValue::from_f64(
                f64::from(offset).mul_add(-86_400_000.0, noon),
            ));
            let start_time = local_day_start(&date);
            let start = js_sys::Date::new(&JsValue::from_f64(start_time * 1_000.0));
            CalendarDay {
                start_time,
                date_key: format!(
                    "{:04}-{:02}-{:02}",
                    start.get_full_year(),
                    start.get_month() + 1,
                    start.get_date(),
                ),
                weekday: weekday_name(start.get_day()).to_owned(),
                day_number: start.get_date(),
                month: month_name(start.get_month()).to_owned(),
                accessible_label: start
                    .to_locale_date_string("en-IE", &JsValue::UNDEFINED)
                    .into(),
            }
        })
        .collect()
}

fn browser_timezone() -> String {
    let formatter =
        js_sys::Intl::DateTimeFormat::new(&js_sys::Array::new(), &js_sys::Object::new());
    js_sys::Reflect::get(
        &formatter.resolved_options(),
        &JsValue::from_str("timeZone"),
    )
    .ok()
    .and_then(|timezone| timezone.as_string())
    .unwrap_or_else(|| "UTC".to_owned())
}

fn has_recording(days: &std::collections::BTreeMap<String, bool>, date: &str) -> bool {
    days.get(date).copied() == Some(true)
}

fn local_day_start(date: &js_sys::Date) -> f64 {
    local_time_of_day(date, 0, 0)
}

/// Returns the Unix seconds of `clock` on `day_start`'s calendar day, or
/// `None` when `clock` is not an `HH:MM` time.
fn local_day_time(day_start: f64, clock: &str) -> Option<f64> {
    let (hours, minutes) = parse_clock(clock)?;
    let date = js_sys::Date::new(&JsValue::from_f64(day_start * 1_000.0));
    Some(local_time_of_day(&date, hours, minutes))
}

/// Parses an `<input type="time">` value into local hours and minutes.
///
/// Returns `None` for anything that is not `HH:MM`, which is what the element
/// reports once the user clears it.
fn parse_clock(clock: &str) -> Option<(u32, u32)> {
    let (hours, minutes) = clock.split_once(':')?;
    let hours = hours.parse::<u32>().ok().filter(|hours| *hours < 24)?;
    let minutes = minutes
        .parse::<u32>()
        .ok()
        .filter(|minutes| *minutes < 60)?;
    Some((hours, minutes))
}

fn local_time_of_day(date: &js_sys::Date, hours: u32, minutes: u32) -> f64 {
    let local = format!(
        "{:04}-{:02}-{:02}T{hours:02}:{minutes:02}:00",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
    );
    // A local date built from a Date's own fields always parses; a
    // spring-forward hour that does not exist is normalized rather than
    // rejected.
    js_sys::Date::parse(&local) / 1_000.0
}

fn next_local_day(day_start: f64) -> f64 {
    let date = js_sys::Date::new(&JsValue::from_f64(
        day_start.mul_add(1_000.0, 129_600_000.0),
    ));
    local_day_start(&date)
}

fn day_severity(reviews: &[ReviewSegment], day_start: f64, day_end: f64) -> Option<ReviewSeverity> {
    day_severity_from_ranges(
        reviews
            .iter()
            .map(|review| (review.start_time, review.end_time, review.severity)),
        day_start,
        day_end,
    )
}

fn day_severity_from_ranges(
    ranges: impl Iterator<Item = (f64, Option<f64>, ReviewSeverity)>,
    day_start: f64,
    day_end: f64,
) -> Option<ReviewSeverity> {
    ranges
        .filter(|(start_time, end_time, _)| {
            *start_time < day_end && end_time.unwrap_or(day_end) > day_start
        })
        .map(|(_, _, severity)| severity)
        .reduce(ReviewSeverity::highest)
}

const fn severity_class(severity: ReviewSeverity) -> &'static str {
    match severity {
        ReviewSeverity::SignificantMotion => "activity-motion",
        ReviewSeverity::Detection => "activity-detection",
        ReviewSeverity::Alert => "activity-alert",
    }
}

const fn severity_label(severity: ReviewSeverity) -> &'static str {
    match severity {
        ReviewSeverity::SignificantMotion => "Motion",
        ReviewSeverity::Detection => "Detection",
        ReviewSeverity::Alert => "Alert",
    }
}

const fn weekday_name(day: u32) -> &'static str {
    match day {
        0 => "Sun",
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        6 => "Sat",
        // ECMA-262 defines getDay as 0-6, so any other value means the
        // argument did not come from a Date.
        _ => panic!("weekday number out of range 0-6"),
    }
}

const fn month_name(month: u32) -> &'static str {
    match month {
        0 => "Jan",
        1 => "Feb",
        2 => "Mar",
        3 => "Apr",
        4 => "May",
        5 => "Jun",
        6 => "Jul",
        7 => "Aug",
        8 => "Sep",
        9 => "Oct",
        10 => "Nov",
        11 => "Dec",
        // ECMA-262 defines getMonth as 0-11, so any other value means the
        // argument did not come from a Date.
        _ => panic!("month number out of range 0-11"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_uses_the_highest_overlapping_review_severity() {
        let reviews = [
            ReviewSegment {
                start_time: 90.0,
                end_time: Some(110.0),
                severity: ReviewSeverity::Detection,
            },
            ReviewSegment {
                start_time: 120.0,
                end_time: Some(130.0),
                severity: ReviewSeverity::Alert,
            },
            ReviewSegment {
                start_time: 140.0,
                end_time: Some(210.0),
                severity: ReviewSeverity::SignificantMotion,
            },
        ];

        assert_eq!(
            day_severity(&reviews, 100.0, 200.0),
            Some(ReviewSeverity::Alert)
        );
        assert_eq!(day_severity(&reviews, 220.0, 230.0), None);
    }

    #[test]
    fn a_cleared_or_malformed_time_input_is_not_a_clock() {
        assert_eq!(parse_clock("00:00"), Some((0, 0)));
        assert_eq!(parse_clock("07:05"), Some((7, 5)));
        assert_eq!(parse_clock("23:59"), Some((23, 59)));
        assert_eq!(parse_clock(""), None);
        assert_eq!(parse_clock("24:00"), None);
        assert_eq!(parse_clock("12:60"), None);
        assert_eq!(parse_clock("12"), None);
        assert_eq!(parse_clock("noon"), None);
    }

    #[test]
    #[should_panic(expected = "weekday number out of range 0-6")]
    fn a_weekday_outside_the_date_contract_is_a_bug() {
        let day = std::hint::black_box(7);
        let _ = weekday_name(day);
    }

    #[test]
    #[should_panic(expected = "month number out of range 0-11")]
    fn a_month_outside_the_date_contract_is_a_bug() {
        let month = std::hint::black_box(12);
        let _ = month_name(month);
    }

    #[test]
    fn event_time_uses_an_unambiguous_month_name_and_padded_clock() {
        assert_eq!(
            format_date_time_parts(3, 7, 2026, 6, 4),
            "3 Aug 2026, 06:04"
        );
    }

    #[test]
    fn only_dates_marked_true_are_playable() {
        let days = std::collections::BTreeMap::from([
            ("2026-07-31".to_owned(), true),
            ("2026-08-01".to_owned(), false),
        ]);

        assert!(has_recording(&days, "2026-07-31"));
        assert!(!has_recording(&days, "2026-08-01"));
        assert!(!has_recording(&days, "2026-08-02"));
    }

    #[test]
    fn contiguous_segments_become_newest_first_recordings() {
        let selection = RecordingRange {
            camera: "front".to_owned(),
            start_time: 100.0,
            end_time: 200.0,
        };
        let segments = vec![
            RecordingSegment {
                start_time: 150.0,
                end_time: 160.0,
                motion: Some(0.0),
            },
            RecordingSegment {
                start_time: 90.0,
                end_time: 110.0,
                motion: Some(2.0),
            },
            RecordingSegment {
                start_time: 160.5,
                end_time: 170.0,
                motion: Some(3.0),
            },
            RecordingSegment {
                start_time: 205.0,
                end_time: 215.0,
                motion: None,
            },
        ];

        assert_eq!(
            recording_media(&selection, segments),
            RecordingMedia {
                clips: vec![
                    RecordingClip {
                        range: RecordingRange {
                            camera: "front".to_owned(),
                            start_time: 150.0,
                            end_time: 170.0,
                        },
                    },
                    RecordingClip {
                        range: RecordingRange {
                            camera: "front".to_owned(),
                            start_time: 100.0,
                            end_time: 110.0,
                        },
                    },
                ],
                motion_ranges: vec![
                    RecordingRange {
                        camera: "front".to_owned(),
                        start_time: 100.0,
                        end_time: 110.0,
                    },
                    RecordingRange {
                        camera: "front".to_owned(),
                        start_time: 160.5,
                        end_time: 170.0,
                    },
                ],
            }
        );
    }

    #[test]
    fn recent_activity_filter_keeps_only_the_selected_severity() {
        let events = [ReviewSeverity::Alert, ReviewSeverity::Detection]
            .into_iter()
            .enumerate()
            .map(|(index, severity)| ReviewEvent {
                id: index.to_string(),
                camera: "front".to_owned(),
                start_time: 100.0,
                end_time: Some(110.0),
                severity,
                thumb_path: "/media/frigate/clips/review/thumb.webp".to_owned(),
                data: corvette_api::ReviewEventData::default(),
            })
            .collect::<Vec<_>>();

        assert_eq!(
            review_events_for_filter(events.clone(), ActivityFilter::All),
            events
        );
        assert_eq!(
            review_events_for_filter(events.clone(), ActivityFilter::Alert),
            [events[0].clone()]
        );
        assert_eq!(
            review_event_day_severity(&events, 90.0, 120.0),
            Some(ReviewSeverity::Alert)
        );
        assert_eq!(
            review_events_for_filter(events, ActivityFilter::Detection),
            [ReviewEvent {
                id: "1".to_owned(),
                camera: "front".to_owned(),
                start_time: 100.0,
                end_time: Some(110.0),
                severity: ReviewSeverity::Detection,
                thumb_path: "/media/frigate/clips/review/thumb.webp".to_owned(),
                data: corvette_api::ReviewEventData::default(),
            }]
        );
        assert_eq!(
            review_thumbnail_url("/media/frigate/clips/review/thumb.webp"),
            "/clips/review/thumb.webp"
        );
        assert_eq!(
            recent_activity_empty_heading(),
            "No events in the last 6 hours"
        );
    }

    #[test]
    fn adjacent_motion_buckets_merge_per_camera_but_gaps_remain() {
        let motion = [
            MotionActivity {
                start_time: 100.0,
                motion: 10.0,
                camera: "front,back".to_owned(),
            },
            MotionActivity {
                start_time: 130.0,
                motion: 20.0,
                camera: "front".to_owned(),
            },
            MotionActivity {
                start_time: 160.0,
                motion: 0.0,
                camera: String::new(),
            },
            MotionActivity {
                start_time: 190.0,
                motion: 30.0,
                camera: "front".to_owned(),
            },
        ];

        assert_eq!(
            consolidated_motion_ranges(&motion),
            [
                RecordingRange {
                    camera: "back".to_owned(),
                    start_time: 100.0,
                    end_time: 130.0,
                },
                RecordingRange {
                    camera: "front".to_owned(),
                    start_time: 100.0,
                    end_time: 160.0,
                },
                RecordingRange {
                    camera: "front".to_owned(),
                    start_time: 190.0,
                    end_time: 220.0,
                },
            ]
        );
    }

    #[test]
    fn calendar_taps_choose_a_range_then_start_a_new_one() {
        let start = select_calendar_range(None, 200.0);
        assert_eq!(start, CalendarSelection::single(200.0));

        let completed = select_calendar_range(Some(start), 100.0);
        assert_eq!(
            completed,
            CalendarSelection {
                start_day: 100.0,
                end_day: 200.0,
                awaiting_end: false,
            }
        );
        assert!(completed.contains(150.0));

        assert_eq!(
            select_calendar_range(Some(completed), 300.0),
            CalendarSelection::single(300.0)
        );
    }

    #[test]
    fn playhead_keeps_available_times_and_snaps_gaps_to_the_nearest_clip() {
        let clips = vec![
            RecordingClip {
                range: RecordingRange {
                    camera: "front".to_owned(),
                    start_time: 100.0,
                    end_time: 120.0,
                },
            },
            RecordingClip {
                range: RecordingRange {
                    camera: "front".to_owned(),
                    start_time: 150.0,
                    end_time: 180.0,
                },
            },
        ];

        assert!((playable_time(&clips, 110.0) - 110.0).abs() < f64::EPSILON);
        assert!((playable_time(&clips, 140.0) - 150.0).abs() < f64::EPSILON);
        assert!((playable_time(&clips, 125.0) - 119.0).abs() < f64::EPSILON);
        assert!((playable_time(&clips, 180.0) - 179.0).abs() < f64::EPSILON);
    }

    #[test]
    fn timeline_segments_are_positioned_within_the_available_span() {
        let range = RecordingRange {
            camera: "front".to_owned(),
            start_time: 125.0,
            end_time: 150.0,
        };

        assert_eq!(
            timeline_segment_style(&range, 100.0, 200.0),
            "left: 25.0000%; width: 25.0000%"
        );
    }

    #[test]
    fn timeline_keeps_zero_duration_review_markers() {
        assert_eq!(
            review_timeline_bounds(125.0, Some(125.0), 100.0, 200.0),
            Some((125.0, 126.0))
        );
        assert_eq!(
            review_timeline_bounds(250.0, Some(250.0), 100.0, 200.0),
            None
        );
    }

    #[test]
    fn activity_playback_advances_after_the_current_event() {
        let activities = [
            TimelineActivity {
                start_time: 100.0,
                end_time: 110.0,
            },
            TimelineActivity {
                start_time: 105.0,
                end_time: 115.0,
            },
            TimelineActivity {
                start_time: 120.0,
                end_time: 130.0,
            },
        ];

        assert_eq!(
            next_timeline_activity(&activities, activities[0]),
            Some(activities[2])
        );
        assert_eq!(next_timeline_activity(&activities, activities[2]), None);
    }

    #[test]
    fn timeline_prefers_frigate_preview_media_for_scrubbing() {
        let clips = vec![RecordingClip {
            range: RecordingRange {
                camera: "front".to_owned(),
                start_time: 100.0,
                end_time: 200.0,
            },
        }];
        let previews = vec![PreviewClip {
            camera: "front".to_owned(),
            src: "/clips/previews/front/hour.mp4".to_owned(),
            media_type: "video/mp4".to_owned(),
            start: 90.0,
            end: 210.0,
        }];

        let source = playback_source(&clips, &previews, 150.0, true)
            .expect("recorded timestamp should have a playback source");

        assert_eq!(source.url, "/clips/previews/front/hour.mp4");
        assert!(source.poster.is_none());
        assert!((source.start_time - 90.0).abs() < f64::EPSILON);
    }
}
