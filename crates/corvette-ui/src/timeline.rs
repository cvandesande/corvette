//! The recording timeline and the single player that follows its playhead.

use corvette_api::{PreviewClip, ReviewSegment};
use leptos::prelude::*;

use crate::activity::{severity_class, severity_label};
use crate::local_time::format_event_time;
use crate::media::{RecordingClip, RecordingRange};

/// The selected window and the playhead every span on the track moves.
#[derive(Clone, Copy)]
struct TimelineControls {
    range_start: f64,
    range_end: f64,
    active_activity: RwSignal<Option<TimelineActivity>>,
    selected_time: RwSignal<f64>,
    uses_preview: RwSignal<bool>,
}

impl TimelineControls {
    /// Moves the playhead to `time`, playing `activity` through to its end when
    /// one was clicked and stopping at `time` when it was plain availability.
    fn select(self, activity: Option<TimelineActivity>, time: f64) {
        self.active_activity.set(activity);
        self.uses_preview.set(true);
        self.selected_time.set(time);
    }

    fn span_style(self, start_time: f64, end_time: f64) -> String {
        timeline_segment_style(start_time, end_time, self.range_start, self.range_end)
    }
}

#[component]
pub(crate) fn RecordingTimeline(
    range_start: f64,
    range_end: f64,
    clips: Vec<RecordingClip>,
    motion_ranges: Vec<RecordingRange>,
    previews: Vec<PreviewClip>,
    reviews: Vec<ReviewSegment>,
) -> impl IntoView {
    let selectable_clips = clips.clone();
    let playback_activities = timeline_activities(&motion_ranges, &reviews, range_start, range_end);
    let controls = TimelineControls {
        range_start,
        range_end,
        active_activity: RwSignal::new(None::<TimelineActivity>),
        selected_time: RwSignal::new(playable_time(&clips, range_start)),
        uses_preview: RwSignal::new(true),
    };
    let selected_time = controls.selected_time;

    view! {
        <section class="recording-timeline" aria-label="Recording timeline">
            <div class="timeline-heading">
                <h3>"Timeline"</h3>
                <output>{move || format_event_time(selected_time.get())}</output>
            </div>
            <div class="timeline-track">
                <AvailabilitySpans clips=clips.clone() controls/>
                <MotionSpans motion_ranges clips=clips.clone() controls/>
                <ReviewMarkers reviews clips=clips.clone() controls/>
                <input
                    type="range"
                    min=range_start
                    max=range_end
                    step="1"
                    prop:value=move || selected_time.get()
                    aria-label="Recording playhead"
                    on:input=move |event| {
                        let requested_time = event_target_value(&event)
                            .parse::<f64>()
                            .unwrap_or(range_start);
                        controls.select(
                            None,
                            playable_time(&selectable_clips, requested_time),
                        );
                    }
                />
            </div>
            <TimelinePlayer
                clips
                previews
                activities=playback_activities
                active_activity=controls.active_activity
                selected_time
                uses_preview=controls.uses_preview
            />
        </section>
    }
}

/// Renders one clickable span per stretch of retained footage.
#[component]
fn AvailabilitySpans(clips: Vec<RecordingClip>, controls: TimelineControls) -> impl IntoView {
    clips
        .into_iter()
        .map(|clip| {
            let start_time = clip.range.start_time;
            let style = controls.span_style(start_time, clip.range.end_time);
            let label = format!("Play from {}", format_event_time(start_time));
            view! { <button
                type="button"
                class="timeline-availability"
                style=style
                aria-label=label
                on:click=move |_| controls.select(None, start_time)
            ></button> }
        })
        .collect_view()
}

/// Renders one playable span per stretch of recorded motion.
#[component]
fn MotionSpans(
    motion_ranges: Vec<RecordingRange>,
    clips: Vec<RecordingClip>,
    controls: TimelineControls,
) -> impl IntoView {
    motion_ranges
        .into_iter()
        .map(|range| {
            let activity = TimelineActivity {
                start_time: range.start_time,
                end_time: range.end_time,
            };
            let style = controls.span_style(activity.start_time, activity.end_time);
            let label = format!(
                "Motion recording at {}",
                format_event_time(activity.start_time),
            );
            let click_clips = clips.clone();
            view! { <button
                type="button"
                class="timeline-activity timeline-motion-recording activity-motion"
                style=style
                aria-label=label
                on:click=move |_| controls.select(
                    Some(activity),
                    playable_time(&click_clips, activity.start_time),
                )
            ></button> }
        })
        .collect_view()
}

/// Renders one marker per review, skipping any that fall outside the window.
#[component]
fn ReviewMarkers(
    reviews: Vec<ReviewSegment>,
    clips: Vec<RecordingClip>,
    controls: TimelineControls,
) -> impl IntoView {
    reviews
        .into_iter()
        .filter_map(|review| {
            let is_point = review
                .end_time
                .is_some_and(|end_time| end_time <= review.start_time);
            let (start_time, end_time) = review_timeline_bounds(
                review.start_time,
                review.end_time,
                controls.range_start,
                controls.range_end,
            )?;
            let activity = TimelineActivity {
                start_time,
                end_time,
            };
            let style = controls.span_style(start_time, end_time);
            let severity = review.severity;
            let label = format!(
                "{} activity at {}",
                severity_label(severity),
                format_event_time(start_time),
            );
            let click_clips = clips.clone();
            Some(view! { <button
                type="button"
                class=format!(
                    "timeline-activity {}{}",
                    severity_class(severity),
                    if is_point { " timeline-point" } else { "" },
                )
                style=style
                aria-label=label
                on:click=move |_| controls.select(
                    Some(activity),
                    playable_time(&click_clips, start_time),
                )
            ></button> })
        })
        .collect_view()
}

/// Playhead offsets closer together than this count as already positioned:
/// seeking to one restarts decoding for no visible change.
const SEEK_TOLERANCE_SECONDS: f64 = 0.25;

/// A completed seek lands on the nearest keyframe rather than on the exact
/// offset, so a seek is accepted with a looser bound than it is requested with.
const SETTLED_SEEK_TOLERANCE_SECONDS: f64 = 0.5;

/// A clip's end timestamp is exclusive, so the playhead stops this far short of
/// it to stay on a moment the clip actually contains.
const CLIP_END_MARGIN_SECONDS: f64 = 1.0;

/// Keeps a final reported time inside its source without moving it visibly.
const SOURCE_END_EPSILON_SECONDS: f64 = 0.001;

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
            if (video.current_time() - requested_offset).abs() < SEEK_TOLERANCE_SECONDS {
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
                    if (video.current_time() - requested_offset).abs() >= SEEK_TOLERANCE_SECONDS {
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
            if (video.current_time() - requested_offset).abs() >= SETTLED_SEEK_TOLERANCE_SECONDS {
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
                    selected_time
                        .set(playback_time.min(source.end_time - CLIP_END_MARGIN_SECONDS));
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
            let final_time = activity
                .end_time
                .min(source.end_time - SOURCE_END_EPSILON_SECONDS);
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
            let last_playable_time =
                (clip.range.end_time - CLIP_END_MARGIN_SECONDS).max(clip.range.start_time);
            requested_time.clamp(clip.range.start_time, last_playable_time)
        })
        .min_by(|left, right| {
            (left - requested_time)
                .abs()
                .total_cmp(&(right - requested_time).abs())
        })
        .expect("recording timeline requires at least one clip")
}

/// Positions the span `span_start..span_end` within the window
/// `range_start..range_end` as CSS percentages.
fn timeline_segment_style(
    span_start: f64,
    span_end: f64,
    range_start: f64,
    range_end: f64,
) -> String {
    let duration = range_end - range_start;
    let left = (span_start - range_start) / duration * 100.0;
    let width = (span_end - span_start) / duration * 100.0;
    format!("left: {left:.4}%; width: {width:.4}%")
}

fn review_timeline_bounds(
    review_start: f64,
    review_end: Option<f64>,
    range_start: f64,
    range_end: f64,
) -> Option<(f64, f64)> {
    /// Frigate reports an instantaneous review as a zero-length span, which
    /// would compute to a zero-width marker the reader cannot see or click.
    const MINIMUM_MARKER_SECONDS: f64 = 1.0;

    let review_end = review_end.unwrap_or(range_end);
    if review_start >= range_end || review_end < range_start {
        return None;
    }
    let start_time = review_start.max(range_start);
    let end_time = review_end
        .min(range_end)
        .max((start_time + MINIMUM_MARKER_SECONDS).min(range_end));
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct TimelineActivity {
    start_time: f64,
    end_time: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            timeline_segment_style(125.0, 150.0, 100.0, 200.0),
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
