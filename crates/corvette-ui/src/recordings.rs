//! The `/recordings` page: choosing continuous footage and playing it back.

use corvette_api::{Camera, Event, PreviewClip, ReviewSegment};
use leptos::prelude::*;

use crate::activity::{day_severity, severity_class};
use crate::calendar::{
    ActivityLegend, CalendarRecordingStatus, calendar_day_label, recording_day_available,
};
use crate::local_time::{
    CalendarDay, CalendarSelection, browser_timezone, format_event_time, local_day_time,
    next_local_day, recent_calendar_days, select_calendar_range,
};
use crate::media::{RECORDING_PRESETS, RecordingMedia, RecordingRange, recording_media};
use crate::shell::{PageShell, Status, StatusGlyph};
use crate::timeline::RecordingTimeline;

/// Renders the recording browser behind the route-level lazy boundary.
#[component]
pub(crate) fn RecordingBrowser() -> impl IntoView {
    let cameras = LocalResource::new(crate::api::fetch_cameras);
    let events = LocalResource::new(crate::api::fetch_events);
    let selected_event = RwSignal::new(None);
    let recording_range = RwSignal::new(None::<RecordingRange>);
    let calendar_selection = RwSignal::new(None::<CalendarSelection>);
    let playback_failed = RwSignal::new(false);
    let selection_error = RwSignal::new(None::<String>);
    let recording_camera = RwSignal::new(String::new());
    let calendar_days = recent_calendar_days(21);
    let calendar_after = calendar_days.first().map_or(0.0, |day| day.start_time);
    let timezone = browser_timezone();
    let start_clock = RwSignal::new("00:00".to_owned());
    let end_clock = RwSignal::new("23:59".to_owned());
    let review_activity = LocalResource::new(move || {
        let camera = recording_camera.get();
        async move {
            crate::api::fetch_review_activity(
                &camera,
                calendar_after,
                js_sys::Date::now() / 1_000.0,
            )
            .await
        }
    });
    let recording_days = LocalResource::new(move || {
        let camera = recording_camera.get();
        let timezone = timezone.clone();
        async move { crate::api::fetch_recording_days(&camera, &timezone).await }
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
            crate::api::fetch_recording_segments(&range.camera, range.start_time, range.end_time)
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
            crate::api::fetch_preview_clips(&range.camera, range.start_time, range.end_time).await
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
        playback_failed,
        selection_error,
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
    playback_failed: RwSignal<bool>,
    selection_error: RwSignal<Option<String>>,
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
        self.playback_failed.set(false);
        self.selection_error.set(None);
        let camera = self.recording_camera.get_untracked();
        if camera.is_empty() {
            self.selection_error.set(Some(
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
fn RecordingControls() -> impl IntoView {
    let context = expect_context::<RecordingContext>();
    let selection_error = context.selection_error;
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
        context.selection_error.set(Some(
            "Enter both times as 24-hour HH:MM before choosing a date.".to_owned(),
        ));
        return;
    };
    if start_time >= end_time {
        context.selection_error.set(Some(
            "The end time must be later than the start time.".to_owned(),
        ));
        return;
    }

    context.calendar_selection.set(Some(selection));
    context.load(start_time, end_time);
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
                context.playback_failed.set(false);
                context.selected_event.set(None);
            }>"Close"</button>
        </div>
        <video class="recording-player" src=clip_url controls autoplay playsinline
            on:error=move |_| context.playback_failed.set(true)
        >"This browser cannot play the event recording."</video>
        {move || context.playback_failed.get().then(|| view! {
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
                <RecordingTimeline
                    range_start=range.start_time
                    range_end=range.end_time
                    clips=media.clips
                    motion_ranges=media.motion_ranges
                    previews
                    reviews
                />
                {move || context.playback_failed.get().then(|| view! {
                    <p class="recording-error" role="alert">
                        "One or more recordings could not be loaded."
                    </p>
                })}
            }.into_any(),
        }}
    }
}
