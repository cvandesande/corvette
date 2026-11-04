//! The activity model built from Frigate's review records and sampled motion.

use std::cell::RefCell;
use std::rc::Rc;

use corvette_api::{ActivityReview, MotionActivity, ReviewEvent, ReviewSegment, ReviewSeverity};
use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Event, HtmlVideoElement};

use crate::expanded_view::ensure_hls_script_loaded;
use crate::local_time::format_event_time;
use crate::media::RecordingRange;
use crate::shell::{Status, StatusGlyph};

pub(crate) const RECENT_ACTIVITY_HOURS: u32 = 6;

/// Width of the buckets Frigate samples motion into, so the span a sample
/// stands for. Sent as `scale`, which is why both sides read one definition.
pub(crate) const MOTION_BUCKET_SECONDS: u32 = 30;

#[component]
pub(crate) fn ActivityFilters(filter: RwSignal<ActivityFilter>) -> impl IntoView {
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
pub(crate) enum EventListLayout {
    Compact,
    Complete,
}

/// Events a compact list shows before the reader asks for the rest; the hidden
/// count and the per-card rule both read this, so they cannot disagree.
const COMPACT_EVENT_LIMIT: usize = 4;

#[component]
pub(crate) fn ReviewEventList(
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
                detail="No matching review activity during this period."
                glyph=StatusGlyph::Event
            /> }
            .into_any();
        }
        let hidden_event_count = match layout {
            EventListLayout::Compact => events.len().saturating_sub(COMPACT_EVENT_LIMIT),
            EventListLayout::Complete => 0,
        };
        view! {
            <div class="event-grid" aria-label="Events">
                {events.into_iter().enumerate().map(|(index, event)| {
                    let thumbnail_url = review_thumbnail_url(&event.thumb_path);
                    let snapshot_url = event.data.detections.first().map(|id| {
                        event_snapshot_url(id)
                    });
                    // Frigate expires a snapshot on its own retention, so
                    // the review thumbnail stands in once it has gone.
                    let shows_snapshot = RwSignal::new(snapshot_url.is_some());
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
                                hidden_event_count > 0
                                    && index >= COMPACT_EVENT_LIMIT
                                    && !expanded.get()
                            }
                        >
                            <button
                                class="event-card-link"
                                type="button"
                                on:click=move |_| selected_review.set(Some(event_for_selection.clone()))
                            >
                                <img
                                    src=move || match snapshot_url.clone() {
                                        Some(snapshot) if shows_snapshot.get() => snapshot,
                                        _ => thumbnail_url.clone(),
                                    }
                                    alt=thumbnail_alt
                                    loading="lazy"
                                    on:error=move |_| shows_snapshot.set(false)
                                />
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
    let close_button = NodeRef::<leptos::html::Button>::new();
    Effect::new(move |_| {
        if let Some(button) = close_button.get() {
            _ = button.focus();
        }
    });
    let camera = review.camera.clone();
    let start_time = review.start_time;
    let end_time = review
        .end_time
        .unwrap_or_else(|| js_sys::Date::now() / 1_000.0);
    let review_id = review.id.clone();
    let clip = LocalResource::new(move || {
        let camera = camera.clone();
        async move { crate::api::fetch_activity_reviews(&camera, start_time, end_time).await }
    });
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
                {move || match clip.get().map(|reviews| clip_outcome(&reviews, &review_id)) {
                    None => view! { <Status
                        heading="Checking availability"
                        detail="Asking the server for this alert's clip."
                        glyph=StatusGlyph::Recording
                    /> }.into_any(),
                    Some(ClipOutcome::Playable(url)) => view! { <ClipPlayer url=url/> }.into_any(),
                    Some(outcome) => view! {
                        <>
                            <p class="recording-error" role="alert">{outcome.message()}</p>
                            <button
                                type="button"
                                on:click=move |_| clip.refetch()
                            >"Retry"</button>
                        </>
                    }.into_any(),
                }}
            </section>
        </div>
    }
}

#[component]
fn ClipPlayer(url: String) -> impl IntoView {
    let attempt = RwSignal::new(0u32);
    let failed = RwSignal::new(false);
    view! {
        // Keyed so a retry builds a new element: a plain reactive block reuses the
        // same <video>, keeping what the failed attempt left on it.
        <For
            each=move || [attempt.get()]
            key=|attempt| *attempt
            children=move |_| view! { <ClipVideo url=url.clone() failed/> }
        />
        {move || failed.get().then(|| view! {
            <>
                <p class="recording-error" role="alert">
                    "This clip did not play in this browser."
                </p>
                <button
                    type="button"
                    on:click=move |_| {
                        failed.set(false);
                        attempt.update(|attempt| *attempt += 1);
                    }
                >"Retry"</button>
            </>
        })}
    }
}

#[component]
fn ClipVideo(url: String, failed: RwSignal<bool>) -> impl IntoView {
    let video_ref = NodeRef::<leptos::html::Video>::new();
    let ladder = StoredValue::new_local(None::<ClipLadder>);
    let playlist_url = url.clone();
    Effect::new(move |_| {
        let Some(video) = video_ref.get() else {
            return;
        };
        ladder.set_value(Some(ClipLadder::start(
            &video,
            playlist_url.clone(),
            failed.write_only(),
        )));
    });
    on_cleanup(move || {
        if let Some(Some(ladder)) = ladder.try_update_value(Option::take) {
            drop(ladder);
        }
    });
    view! {
        <video
            class="recording-player"
            data-clip-url=url
            controls
            autoplay
            playsinline
            node_ref=video_ref
        >"This browser cannot play the event recording."</video>
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClipRung {
    WaitingForScript,
    HlsJs,
    Native,
    Failed,
}

struct ClipLadderState {
    stopped: bool,
    rung: ClipRung,
    video: HtmlVideoElement,
    playlist_url: String,
    hls_instance: JsValue,
    on_hls_error: Option<Closure<dyn FnMut(JsValue, JsValue)>>,
    on_video_error: Option<Closure<dyn FnMut(Event)>>,
    playback_failed: WriteSignal<bool>,
}

/// Every `Closure` handed to hls.js or the element stays in the ladder until the ladder itself
/// drops: freeing a `Closure` while JavaScript is inside its call would crash the page.
struct ClipLadder {
    state: Rc<RefCell<ClipLadderState>>,
}

impl ClipLadder {
    fn start(
        video: &HtmlVideoElement,
        playlist_url: String,
        playback_failed: WriteSignal<bool>,
    ) -> Self {
        let state = Rc::new(RefCell::new(ClipLadderState {
            stopped: false,
            rung: ClipRung::WaitingForScript,
            video: video.clone(),
            playlist_url,
            hls_instance: JsValue::UNDEFINED,
            on_hls_error: None,
            on_video_error: None,
            playback_failed,
        }));
        let on_video_error = {
            let shared = Rc::clone(&state);
            Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
                let Ok(mut state) = shared.try_borrow_mut() else {
                    return;
                };
                if state.stopped {
                    return;
                }
                match state.rung {
                    ClipRung::WaitingForScript | ClipRung::Failed => {}
                    ClipRung::HlsJs => start_native(&mut state),
                    ClipRung::Native => {
                        state.rung = ClipRung::Failed;
                        state.playback_failed.set(true);
                    }
                }
            })
        };
        video.set_onerror(Some(on_video_error.as_ref().unchecked_ref()));
        state.borrow_mut().on_video_error = Some(on_video_error);
        let state_for_script = Rc::clone(&state);
        ensure_hls_script_loaded(move || start_hls_rung(&state_for_script));
        Self { state }
    }
}

impl Drop for ClipLadder {
    fn drop(&mut self) {
        let mut state = self.state.borrow_mut();
        state.stopped = true;
        state.video.set_onerror(None);
        if let Ok(destroy) =
            js_sys::Reflect::get(&state.hls_instance, &JsValue::from_str("destroy"))
                .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
        {
            let _ = destroy.call0(&state.hls_instance);
        }
        state.hls_instance = JsValue::UNDEFINED;
        state.on_hls_error = None;
        state.on_video_error = None;
    }
}

fn start_hls_rung(state: &Rc<RefCell<ClipLadderState>>) {
    let mut borrowed = state.borrow_mut();
    // The script can finish loading after the dialog closed or Retry re-mounted: a stopped
    // ladder ignores it.
    if borrowed.stopped {
        return;
    }
    borrowed.rung = ClipRung::HlsJs;
    let Some(window) = web_sys::window() else {
        return;
    };
    // No `Hls.isSupported()` gate: its codec detection is unreliable on iOS; a fatal error
    // falls to the native rung instead.
    let Ok(hls_ctor) = js_sys::Reflect::get(&window, &JsValue::from_str("Hls"))
        .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
    else {
        start_native(&mut borrowed);
        return;
    };
    // `preferManagedMediaSource: true` overrides hls.js's shipped default, which picks the
    // plain `MediaSource` global on iOS 17.1+ instead of the working `ManagedMediaSource`.
    let config = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &config,
        &JsValue::from_str("preferManagedMediaSource"),
        &JsValue::TRUE,
    );
    let Ok(hls_instance) = js_sys::Reflect::construct(&hls_ctor, &js_sys::Array::of1(&config))
    else {
        start_native(&mut borrowed);
        return;
    };
    let on_hls_error = {
        let shared = Rc::clone(state);
        Closure::<dyn FnMut(JsValue, JsValue)>::new(move |_event: JsValue, data: JsValue| {
            let Ok(mut inner) = shared.try_borrow_mut() else {
                return;
            };
            if inner.stopped || inner.rung != ClipRung::HlsJs {
                return;
            }
            let fatal = js_sys::Reflect::get(&data, &JsValue::from_str("fatal"))
                .ok()
                .is_some_and(|value| value.is_truthy());
            if fatal {
                start_native(&mut inner);
            }
        })
    };
    if let Ok(on_fn) = js_sys::Reflect::get(&hls_instance, &JsValue::from_str("on"))
        .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
    {
        // "hlsError" is the literal wire value of `Hls.Events.ERROR`.
        let _ = on_fn.call2(
            &hls_instance,
            &JsValue::from_str("hlsError"),
            on_hls_error.as_ref().unchecked_ref(),
        );
    }
    borrowed.on_hls_error = Some(on_hls_error);
    borrowed.hls_instance = hls_instance.clone();
    if let Ok(attach_media) = js_sys::Reflect::get(&hls_instance, &JsValue::from_str("attachMedia"))
        .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
    {
        let _ = attach_media.call1(&hls_instance, &borrowed.video);
    }
    if let Ok(load_source) = js_sys::Reflect::get(&hls_instance, &JsValue::from_str("loadSource"))
        .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
    {
        let _ = load_source.call1(&hls_instance, &JsValue::from_str(&borrowed.playlist_url));
    }
    let _ = borrowed.video.play();
}

/// Advanced to from the fatal `hlsError` handler or the element's `error` handler. The
/// element's `error` handler is what detects a native rung that fails.
fn start_native(state: &mut ClipLadderState) {
    state.rung = ClipRung::Native;
    // hls.js's destroy() also unbinds every listener it owns, including this ladder's own
    // `"hlsError"` subscription, so no separate `off()` is needed.
    if let Ok(destroy) = js_sys::Reflect::get(&state.hls_instance, &JsValue::from_str("destroy"))
        .and_then(wasm_bindgen::JsCast::dyn_into::<js_sys::Function>)
    {
        let _ = destroy.call0(&state.hls_instance);
    }
    state.hls_instance = JsValue::UNDEFINED;
    if state
        .video
        .can_play_type("application/vnd.apple.mpegurl")
        .is_empty()
    {
        state.rung = ClipRung::Failed;
        state.playback_failed.set(true);
        return;
    }
    state.video.set_src(&state.playlist_url);
    let _ = state.video.play();
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ClipOutcome {
    Playable(String),
    NoRecording,
    NotReturned,
    InProgress,
    LoadFailed,
}

impl ClipOutcome {
    const fn message(&self) -> &'static str {
        match self {
            Self::Playable(_) => "",
            Self::NoRecording => "There is no recording available for this alert.",
            Self::NotReturned => "The server did not return this alert.",
            Self::InProgress => "This alert is still in progress and has no recording to play yet.",
            Self::LoadFailed => "Could not load this alert's clip.",
        }
    }
}

fn clip_outcome(reviews: &Result<Vec<ActivityReview>, String>, review_id: &str) -> ClipOutcome {
    let Ok(reviews) = reviews else {
        return ClipOutcome::LoadFailed;
    };
    let Some(review) = reviews.iter().find(|review| review.id == review_id) else {
        return ClipOutcome::NotReturned;
    };
    if let Some(clip) = &review.media.clip {
        return ClipOutcome::Playable(clip.url.clone());
    }
    if review.end.is_none() {
        return ClipOutcome::InProgress;
    }
    ClipOutcome::NoRecording
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

pub(crate) fn review_and_motion_events(
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
                    end_time: activity.start_time + f64::from(MOTION_BUCKET_SECONDS),
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

pub(crate) fn review_event_day_severity(
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

pub(crate) fn recent_activity_empty_heading() -> String {
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

fn event_snapshot_url(event_id: &str) -> String {
    let event_id = js_sys::encode_uri_component(event_id);
    format!("/api/events/{event_id}/snapshot.jpg?crop=1&height=360&quality=80")
}

fn review_thumbnail_url(thumb_path: &str) -> String {
    let relative_path = thumb_path
        .strip_prefix("/media/frigate/")
        .unwrap_or_else(|| thumb_path.trim_start_matches('/'));
    format!("/{relative_path}")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActivityFilter {
    All,
    SignificantMotion,
    Detection,
    Alert,
}

impl ActivityFilter {
    pub(crate) const ALL: [Self; 4] = [
        Self::All,
        Self::Alert,
        Self::Detection,
        Self::SignificantMotion,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::SignificantMotion => "Motion",
            Self::Detection => "Detections",
            Self::Alert => "Alerts",
        }
    }

    pub(crate) const fn severity(self) -> Option<ReviewSeverity> {
        match self {
            Self::All => None,
            Self::SignificantMotion => Some(ReviewSeverity::SignificantMotion),
            Self::Detection => Some(ReviewSeverity::Detection),
            Self::Alert => Some(ReviewSeverity::Alert),
        }
    }
}

pub(crate) fn day_severity(
    reviews: &[ReviewSegment],
    day_start: f64,
    day_end: f64,
) -> Option<ReviewSeverity> {
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

pub(crate) const fn severity_class(severity: ReviewSeverity) -> &'static str {
    match severity {
        ReviewSeverity::SignificantMotion => "activity-motion",
        ReviewSeverity::Detection => "activity-detection",
        ReviewSeverity::Alert => "activity-alert",
    }
}

pub(crate) const fn severity_label(severity: ReviewSeverity) -> &'static str {
    match severity {
        ReviewSeverity::SignificantMotion => "Motion",
        ReviewSeverity::Detection => "Detection",
        ReviewSeverity::Alert => "Alert",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvette_api::{MediaLink, ReviewMedia};

    #[test]
    fn day_uses_the_highest_overlapping_review_severity() {
        let reviews = [
            ReviewSegment {
                camera: "front".to_owned(),
                start_time: 90.0,
                end_time: Some(110.0),
                severity: ReviewSeverity::Detection,
            },
            ReviewSegment {
                camera: "front".to_owned(),
                start_time: 120.0,
                end_time: Some(130.0),
                severity: ReviewSeverity::Alert,
            },
            ReviewSegment {
                camera: "front".to_owned(),
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

    fn activity_review(id: &str, end: Option<f64>, clip: Option<&str>) -> ActivityReview {
        ActivityReview {
            id: id.to_owned(),
            camera: "front".to_owned(),
            severity: ReviewSeverity::Alert,
            start: 100.0,
            end,
            objects: Vec::new(),
            zones: Vec::new(),
            detections: Vec::new(),
            sub_labels: Vec::new(),
            audio: Vec::new(),
            media: ReviewMedia {
                thumbnail: None,
                clip: clip.map(|url| MediaLink {
                    url: url.to_owned(),
                }),
            },
        }
    }

    type Reviews = Result<Vec<ActivityReview>, String>;

    #[test]
    fn clip_outcome_gives_the_matching_review_its_link() {
        let reviews: Reviews = Ok(vec![activity_review("alert", Some(110.0), Some("/a.m3u8"))]);
        assert_eq!(
            clip_outcome(&reviews, "alert"),
            ClipOutcome::Playable("/a.m3u8".to_owned())
        );
    }

    #[test]
    fn clip_outcome_of_an_ended_review_without_a_link_is_no_recording() {
        let reviews: Reviews = Ok(vec![activity_review("alert", Some(110.0), None)]);
        assert_eq!(clip_outcome(&reviews, "alert"), ClipOutcome::NoRecording);
    }

    #[test]
    fn clip_outcome_of_a_review_absent_from_the_answer_is_not_returned() {
        let other_review: Reviews = Ok(vec![activity_review(
            "other",
            Some(110.0),
            Some("/other.m3u8"),
        )]);
        assert_eq!(
            clip_outcome(&other_review, "alert"),
            ClipOutcome::NotReturned
        );
        let empty: Reviews = Ok(Vec::new());
        assert_eq!(clip_outcome(&empty, "alert"), ClipOutcome::NotReturned);
    }

    #[test]
    fn clip_outcome_of_an_in_progress_review_is_in_progress() {
        let reviews: Reviews = Ok(vec![activity_review("alert", None, None)]);
        assert_eq!(clip_outcome(&reviews, "alert"), ClipOutcome::InProgress);
    }

    #[test]
    fn clip_outcome_of_an_in_progress_review_with_a_link_is_playable() {
        let reviews: Reviews = Ok(vec![activity_review("alert", None, Some("/alert.m3u8"))]);
        assert_eq!(
            clip_outcome(&reviews, "alert"),
            ClipOutcome::Playable("/alert.m3u8".to_owned())
        );
    }

    #[test]
    fn clip_outcome_of_a_failed_request_is_load_failed() {
        let failed: Reviews = Err("request /corvette/api/v1/activity: no route".to_owned());
        assert_eq!(clip_outcome(&failed, "alert"), ClipOutcome::LoadFailed);
    }

    #[test]
    fn clip_outcome_uses_the_link_of_the_review_whose_id_matches() {
        let reviews: Reviews = Ok(vec![
            activity_review("other", Some(110.0), Some("/other.m3u8")),
            activity_review("alert", Some(110.0), Some("/alert.m3u8")),
        ]);
        assert_eq!(
            clip_outcome(&reviews, "alert"),
            ClipOutcome::Playable("/alert.m3u8".to_owned())
        );
    }
}
