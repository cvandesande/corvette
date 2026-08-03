//! Browser entry point and initial application shell for Corvette.

// Leptos 0.8.20's proc-macro stack currently selects multiple major versions of
// syn, thiserror, and convert_case; this binary does not select any of them directly.
#![allow(clippy::multiple_crate_versions)]

use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use wasm_bindgen::JsValue;

use corvette_api::{Camera, Event, RecordingSegment, ReviewSegment, ReviewSeverity};

mod api;
#[cfg(feature = "split")]
mod recordings;

#[cfg(feature = "split")]
struct RecordingsRoute;

const NAVIGATION: [(&str, &str); 4] = [
    ("Live", "/#live"),
    ("Events", "/#events"),
    ("Recordings", "/recordings"),
    ("System", "/#system"),
];

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
    let events = LocalResource::new(api::fetch_events);
    let active_section = RwSignal::new("/#live");
    let events_expanded = RwSignal::new(false);

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
                    {move || match events.get() {
                        None => view! { <Status heading="Loading events" detail="Fetching recent activity." glyph=StatusGlyph::Event/> }.into_any(),
                        Some(Err(error)) => view! { <Status heading="Events unavailable" detail=error glyph=StatusGlyph::Event/> }.into_any(),
                        Some(Ok(events)) if events.is_empty() => view! { <Status heading="No events yet" detail="Detected activity will appear here." glyph=StatusGlyph::Event/> }.into_any(),
                        Some(Ok(events)) => {
                            let hidden_event_count = events.len().saturating_sub(4);
                            view! {
                                <div class="event-grid" aria-label="Recent events">
                                {events.into_iter().enumerate().map(|(index, event)| {
                                    let snapshot_url = format!(
                                        "/api/events/{}/snapshot.jpg?crop=1&height=360&quality=80",
                                        event.id,
                                    );
                                    let thumbnail_alt = format!("{} event from {}", event.label, event.camera);
                                    let detail = event.sub_label.as_deref().unwrap_or(&event.label).to_owned();
                                    let zones = if event.zones.is_empty() {
                                        "No zone".to_owned()
                                    } else {
                                        event.zones.join(", ")
                                    };
                                    let event_for_selection = event.clone();
                                    view! {
                                        <article
                                            class="event-card"
                                            class:mobile-hidden=move || {
                                                index >= 4 && !events_expanded.get()
                                            }
                                        >
                                            <a
                                                class="event-card-link"
                                                href=format!(
                                                    "/recordings?event={}",
                                                    event_for_selection.id,
                                                )
                                            >
                                                <img src=snapshot_url alt=thumbnail_alt loading="lazy"/>
                                                <div class="event-card-body">
                                                    <div class="event-card-heading">
                                                        <h2>{detail}</h2>
                                                        <span>{format_event_time(event.start_time)}</span>
                                                    </div>
                                                    <p>{event.camera}</p>
                                                    <p class="event-zone">{zones}</p>
                                                </div>
                                            </a>
                                        </article>
                                    }
                                }).collect_view()}
                                </div>
                                {(hidden_event_count > 0).then(|| view! {
                                    <button
                                        class="event-toggle"
                                        type="button"
                                        aria-expanded=move || events_expanded.get().to_string()
                                        on:click=move |_| events_expanded.update(|expanded| *expanded = !*expanded)
                                    >
                                        {move || if events_expanded.get() {
                                            "Show fewer events".to_owned()
                                        } else {
                                            format!("Show {hidden_event_count} more events")
                                        }}
                                    </button>
                                })}
                            }.into_any()
                        },
                    }}
                </section>
            </main>
        </div>
    }
}

/// Renders the recording browser behind the route-level lazy boundary.
#[component]
pub(crate) fn RecordingBrowser() -> impl IntoView {
    let cameras = LocalResource::new(api::fetch_cameras);
    let events = LocalResource::new(api::fetch_events);
    let selected_event = RwSignal::new(None);
    let recording_range = RwSignal::new(None::<RecordingRange>);
    let recording_failed = RwSignal::new(false);
    let recording_error = RwSignal::new(None::<String>);
    let recording_camera = RwSignal::new(String::new());
    let recording_filter = RwSignal::new(RecordingFilter::All);
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
                return Ok(Vec::new());
            };
            api::fetch_recording_segments(&range.camera, range.start_time, range.end_time)
                .await
                .map(|segments| contiguous_recordings(&range, segments))
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
        recording_failed,
        recording_error,
        recording_camera,
        recording_filter,
        calendar_days,
        start_clock,
        end_clock,
        review_activity,
        recording_days,
        recording_clips,
    });

    view! {
        <RecordingPageShell>
            <section id="recordings" class="page-section" aria-label="Recording playback">
                <p class="eyebrow">"Playback"</p>
                <h1>"Recordings"</h1>
                <RecordingControls/>
                <RecordingFilters/>
                <RecordingPlayback/>
            </section>
        </RecordingPageShell>
    }
}

#[derive(Clone)]
struct RecordingContext {
    cameras: LocalResource<Result<Vec<Camera>, String>>,
    selected_event: RwSignal<Option<Event>>,
    recording_range: RwSignal<Option<RecordingRange>>,
    recording_failed: RwSignal<bool>,
    recording_error: RwSignal<Option<String>>,
    recording_camera: RwSignal<String>,
    recording_filter: RwSignal<RecordingFilter>,
    calendar_days: Vec<CalendarDay>,
    start_clock: RwSignal<String>,
    end_clock: RwSignal<String>,
    review_activity: LocalResource<Result<Vec<ReviewSegment>, String>>,
    recording_days: LocalResource<Result<std::collections::BTreeMap<String, bool>, String>>,
    recording_clips: LocalResource<Result<Vec<RecordingClip>, String>>,
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
fn RecordingPageShell(children: Children) -> impl IntoView {
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
                        class:active=move || href == "/recordings"
                        aria-current=(href == "/recordings").then_some("page")
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
    view! {
        <div class="recording-browser">
            <aside class="recording-presets" aria-label="Recording shortcuts">
                <h2>"Quick ranges"</h2>
                {RECORDING_PRESETS.map(|preset| {
                    let context = context.clone();
                    view! { <button type="button" on:click=move |_| {
                        let (start_time, end_time) = preset.range();
                        context.load(start_time, end_time);
                    }>{preset.label()}</button> }
                })}
            </aside>
            <div class="recording-calendar">
                <CameraAndTimeControls/>
                <div class="calendar-heading"><div>
                    <h2>"Choose a day"</h2>
                    <p>"Select a date to load its recordings."</p>
                </div></div>
                <RecordingCalendarDays/>
                <RecordingCalendarStatus/>
                <div class="activity-legend" aria-label="Activity colors">
                    <span><i class="activity-motion"></i>"Motion"</span>
                    <span><i class="activity-detection"></i>"Detection"</span>
                    <span><i class="activity-alert"></i>"Alert"</span>
                </div>
            </div>
        </div>
    }
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
                disabled=move || !context.recording_days.get().and_then(Result::ok)
                    .is_some_and(|days| has_recording(&days, &date_key))
                class:in-range=move || context.recording_range.get().is_some_and(|range| {
                    range.start_time < next_local_day(day_start) && range.end_time > day_start
                })
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
    let start_time = local_day_time(day_start, &context.start_clock.get_untracked());
    let end_time = local_day_time(day_start, &context.end_clock.get_untracked());
    if start_time < end_time {
        context.load(start_time, end_time);
    } else {
        context.recording_error.set(Some(
            "The end time must be later than the start time.".to_owned(),
        ));
    }
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

#[component]
fn RecordingCalendarStatus() -> impl IntoView {
    let context = expect_context::<RecordingContext>();
    move || {
        match context.recording_days.get() {
        None => view! { <p class="calendar-status">"Checking recording availability…"</p> }.into_any(),
        Some(Err(error)) => view! { <p class="recording-error" role="alert">{error}</p> }.into_any(),
        Some(Ok(days)) if days.is_empty() => view! { <p class="calendar-status">"No retained recordings are available for this camera."</p> }.into_any(),
        Some(Ok(_)) => view! { <p class="calendar-status">"Dimmed dates have no retained recordings."</p> }.into_any(),
    }
    }
}

#[component]
fn RecordingFilters() -> impl IntoView {
    let context = expect_context::<RecordingContext>();
    view! {
        <div class="recording-filters" aria-label="Recording activity filter">
            {RecordingFilter::ALL.map(|filter| view! {
                <button type="button"
                    class:active=move || context.recording_filter.get() == filter
                    aria-pressed=move || (context.recording_filter.get() == filter).to_string()
                    on:click=move |_| context.recording_filter.set(filter)
                >{filter.label()}</button>
            })}
        </div>
        {move || context.recording_error.get().map(|error| view! {
            <p class="recording-error" role="alert">{error}</p>
        })}
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
        {move || match (context.recording_clips.get(), context.review_activity.get()) {
            (None, _) | (_, None) => view! { <Status
                heading="Loading recordings"
                detail="Checking retained footage and activity in the selected range."
                glyph=StatusGlyph::Recording
            /> }.into_any(),
            (Some(Err(error)), _) | (_, Some(Err(error))) => {
                view! { <p class="recording-error" role="alert">{error}</p> }.into_any()
            }
            (Some(Ok(clips)), Some(Ok(_))) if clips.is_empty() => view! {
                <p class="recording-error" role="alert">
                    "Frigate has no retained recordings for this camera and time range."
                </p>
            }.into_any(),
            (Some(Ok(clips)), Some(Ok(reviews))) => view! {
                <RecordingList clips reviews/>
            }.into_any(),
        }}
    }
}

#[component]
fn RecordingList(clips: Vec<RecordingClip>, reviews: Vec<ReviewSegment>) -> impl IntoView {
    let context = expect_context::<RecordingContext>();
    let filter_context = context.clone();
    view! {
        {move || {
            let filter = filter_context.recording_filter.get();
            let clips = recordings_for_filter(clips.clone(), &reviews, filter);
            if clips.is_empty() {
                return view! { <p class="recording-empty" role="status">{format!(
                    "No recordings overlap {} activity in this range.",
                    filter.label().to_lowercase(),
                )}</p> }
                .into_any();
            }
            view! {
                <div class="recording-list" aria-label="Selected recordings">
                    {clips.into_iter().map(|clip| view! {
                        <RecordingPreview clip/>
                    }).collect_view()}
                </div>
            }.into_any()
        }}
        {move || context.recording_failed.get().then(|| view! {
            <p class="recording-error" role="alert">"One or more recordings could not be loaded."</p>
        })}
    }.into_any()
}

#[component]
fn RecordingPreview(clip: RecordingClip) -> impl IntoView {
    let context = expect_context::<RecordingContext>();
    let clip_url = clip.range.clip_url();
    let poster_url = clip.range.poster_url();
    let detail = format!(
        "{} – {}",
        format_event_time(clip.range.start_time),
        format_event_time(clip.range.end_time),
    );
    let play_label = format!("Play recording from {detail}");
    let is_playing = RwSignal::new(false);
    view! { <article><h3>{detail}</h3>{move || if is_playing.get() {
        view! { <video class="recording-player" src=clip_url.clone()
            poster=poster_url.clone() controls autoplay playsinline
            on:error=move |_| context.recording_failed.set(true)
        >"This browser cannot play the selected recording."</video> }.into_any()
    } else {
        view! { <button class="recording-preview" type="button" aria-label=play_label.clone()
            on:click=move |_| is_playing.set(true)
        ><img src=poster_url.clone() alt="" loading="lazy"/><span aria-hidden="true"></span></button> }
            .into_any()
    }}</article> }
}

#[derive(Clone, Copy)]
enum StatusGlyph {
    Camera,
    Event,
    Recording,
}

/// Renders an asynchronous content state with a consistent accessible heading.
#[component]
fn Status(heading: &'static str, detail: impl IntoView, glyph: StatusGlyph) -> impl IntoView {
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
    has_motion: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordingFilter {
    All,
    SignificantMotion,
    Detection,
    Alert,
}

impl RecordingFilter {
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

fn contiguous_recordings(
    selection: &RecordingRange,
    mut segments: Vec<RecordingSegment>,
) -> Vec<RecordingClip> {
    const MAX_SEGMENT_GAP_SECONDS: f64 = 1.0;

    segments.sort_by(|left, right| left.start_time.total_cmp(&right.start_time));
    let mut recordings = Vec::<RecordingClip>::new();
    for segment in segments {
        let start_time = segment.start_time.max(selection.start_time);
        let end_time = segment.end_time.min(selection.end_time);
        let has_motion = segment.motion.is_some_and(|motion| motion > 0.0);
        if start_time >= end_time {
            continue;
        }

        if let Some(recording) = recordings.last_mut()
            && start_time <= recording.range.end_time + MAX_SEGMENT_GAP_SECONDS
        {
            recording.range.end_time = recording.range.end_time.max(end_time);
            recording.has_motion |= has_motion;
            continue;
        }

        recordings.push(RecordingClip {
            range: RecordingRange {
                camera: selection.camera.clone(),
                start_time,
                end_time,
            },
            has_motion,
        });
    }
    recordings.reverse();
    recordings
}

fn recordings_for_filter(
    recordings: Vec<RecordingClip>,
    reviews: &[ReviewSegment],
    filter: RecordingFilter,
) -> Vec<RecordingClip> {
    if filter == RecordingFilter::All {
        return recordings;
    }
    if filter == RecordingFilter::SignificantMotion {
        return recordings
            .into_iter()
            .filter(|recording| recording.has_motion)
            .collect();
    }
    let severity = filter
        .severity()
        .expect("non-motion activity filter should have a review severity");
    recordings
        .into_iter()
        .filter(|recording| {
            reviews.iter().any(|review| {
                review.severity == severity
                    && review.start_time < recording.range.end_time
                    && review.end_time.unwrap_or(recording.range.end_time)
                        > recording.range.start_time
            })
        })
        .collect()
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

fn recent_calendar_days(count: u32) -> Vec<CalendarDay> {
    let today = js_sys::Date::new_0();
    (0..count)
        .rev()
        .map(|offset| {
            let date = js_sys::Date::new(&JsValue::from_f64(
                f64::from(offset).mul_add(-86_400_000.0, today.get_time()),
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
    let local = format!(
        "{:04}-{:02}-{:02}T00:00:00",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
    );
    js_sys::Date::parse(&local) / 1_000.0
}

fn local_day_time(day_start: f64, clock: &str) -> f64 {
    let date = js_sys::Date::new(&JsValue::from_f64(day_start * 1_000.0));
    let local = format!(
        "{:04}-{:02}-{:02}T{clock}:00",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
    );
    js_sys::Date::parse(&local) / 1_000.0
}

fn next_local_day(day_start: f64) -> f64 {
    let date = js_sys::Date::new(&JsValue::from_f64(
        day_start.mul_add(1_000.0, 129_600_000.0),
    ));
    local_day_start(&date)
}

fn day_severity(reviews: &[ReviewSegment], day_start: f64, day_end: f64) -> Option<ReviewSeverity> {
    reviews
        .iter()
        .filter(|review| {
            review.start_time < day_end && review.end_time.unwrap_or(day_end) > day_start
        })
        .map(|review| review.severity)
        .reduce(ReviewSeverity::highest)
}

const fn severity_class(severity: ReviewSeverity) -> &'static str {
    match severity {
        ReviewSeverity::SignificantMotion => "activity-motion",
        ReviewSeverity::Detection => "activity-detection",
        ReviewSeverity::Alert => "activity-alert",
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
        _ => "",
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
        _ => "",
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
            contiguous_recordings(&selection, segments),
            [
                RecordingClip {
                    range: RecordingRange {
                        camera: "front".to_owned(),
                        start_time: 150.0,
                        end_time: 170.0,
                    },
                    has_motion: true,
                },
                RecordingClip {
                    range: RecordingRange {
                        camera: "front".to_owned(),
                        start_time: 100.0,
                        end_time: 110.0,
                    },
                    has_motion: true,
                },
            ]
        );
    }

    #[test]
    fn activity_filter_keeps_only_overlapping_severity() {
        let recordings = vec![
            RecordingClip {
                range: RecordingRange {
                    camera: "front".to_owned(),
                    start_time: 100.0,
                    end_time: 120.0,
                },
                has_motion: true,
            },
            RecordingClip {
                range: RecordingRange {
                    camera: "front".to_owned(),
                    start_time: 200.0,
                    end_time: 220.0,
                },
                has_motion: false,
            },
        ];
        let reviews = [
            ReviewSegment {
                start_time: 105.0,
                end_time: Some(110.0),
                severity: ReviewSeverity::Detection,
            },
            ReviewSegment {
                start_time: 205.0,
                end_time: Some(210.0),
                severity: ReviewSeverity::Alert,
            },
        ];

        assert_eq!(
            recordings_for_filter(recordings.clone(), &reviews, RecordingFilter::All),
            recordings
        );
        assert_eq!(
            recordings_for_filter(recordings.clone(), &reviews, RecordingFilter::Detection),
            [recordings[0].clone()]
        );
        assert_eq!(
            recordings_for_filter(recordings.clone(), &reviews, RecordingFilter::Alert),
            [recordings[1].clone()]
        );
        assert_eq!(
            recordings_for_filter(
                recordings.clone(),
                &reviews,
                RecordingFilter::SignificantMotion
            ),
            [recordings[0].clone()]
        );
    }
}
