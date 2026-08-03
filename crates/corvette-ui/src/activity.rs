//! Frigate's review records and sampled motion as one activity model, with the
//! components that present it.

use corvette_api::{MotionActivity, ReviewEvent, ReviewSegment, ReviewSeverity};
use leptos::prelude::*;

use crate::local_time::format_event_time;
use crate::media::RecordingRange;
use crate::shell::{Status, StatusGlyph};

pub(crate) const RECENT_ACTIVITY_HOURS: u32 = 6;

/// Width of the buckets Frigate samples motion into, and so the span a single
/// motion sample stands for. Requested from the API as `scale`, which is why
/// both sides read this one definition.
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

/// Events a compact list shows on mobile before the reader asks for the rest.
/// Both the hidden count and the per-card rule read this, so a change cannot
/// make "Show N more" disagree with what is hidden.
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
                detail="Frigate reported no matching review activity during this period."
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
}
