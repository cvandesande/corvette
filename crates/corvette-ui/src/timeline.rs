//! The recording timeline and the single player that follows its playhead.

use corvette_api::{PreviewClip, ReviewSegment};
use leptos::prelude::*;

use crate::activity::{severity_class, severity_label};
use crate::local_time::{format_clock_time, format_event_time};
use crate::media::{RecordingClip, RecordingRange};

/// A span of the selection, used both for the whole selection and for the
/// narrower window the track is currently showing.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TimelineView {
    start_time: f64,
    end_time: f64,
}

impl TimelineView {
    fn duration(self) -> f64 {
        self.end_time - self.start_time
    }

    /// Where `time` sits across the span, as 0.0 at its start and 1.0 at its
    /// end. Times outside the span report the nearer edge.
    fn fraction_of(self, time: f64) -> f64 {
        ((time - self.start_time) / self.duration()).clamp(0.0, 1.0)
    }
}

/// The selected window and the playhead every span on the track moves.
#[derive(Clone, Copy)]
struct TimelineControls {
    /// The whole selection, which zooming never leaves.
    bounds: TimelineView,
    /// The part of `bounds` the track currently shows.
    view: RwSignal<TimelineView>,
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

    /// Positions a span against the current view. Reads a signal, so call sites
    /// must be inside a reactive closure for zooming to move the span.
    fn span_style(self, start_time: f64, end_time: f64) -> String {
        let view = self.view.get();
        timeline_segment_style(start_time, end_time, view.start_time, view.end_time)
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
    let bounds = TimelineView {
        start_time: range_start,
        end_time: range_end,
    };
    let controls = TimelineControls {
        bounds,
        view: RwSignal::new(bounds),
        active_activity: RwSignal::new(None::<TimelineActivity>),
        selected_time: RwSignal::new(playable_time(&clips, range_start)),
        uses_preview: RwSignal::new(true),
    };
    let selected_time = controls.selected_time;
    let view_window = controls.view;
    let track = NodeRef::<leptos::html::Div>::new();
    let gestures = TimelineGestures {
        bounds,
        view: view_window,
        track,
        fingers: StoredValue::new(PinchGesture::default()),
    };

    view! {
        <section class="recording-timeline" aria-label="Recording timeline">
            <div class="timeline-heading">
                <h3>"Timeline"</h3>
                <output>{move || format_event_time(selected_time.get())}</output>
            </div>
            <div
                class="timeline-track"
                node_ref=track
                on:wheel=move |event| gestures.wheel(&event)
                on:pointerdown=move |event| gestures.finger_down(&event)
                on:pointermove=move |event| gestures.finger_moved(&event)
                on:pointerup=move |event| gestures.finger_lifted(&event)
                on:pointercancel=move |event| gestures.finger_lifted(&event)
            >
                <AvailabilitySpans clips=clips.clone() controls/>
                <MotionSpans motion_ranges clips=clips.clone() controls/>
                <ReviewMarkers reviews clips=clips.clone() controls/>
                <input
                    type="range"
                    min=move || view_window.get().start_time
                    max=move || view_window.get().end_time
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
            <TimelineScale view=view_window/>
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

/// Turns wheel and two-finger gestures on the track into movements of the view
/// window, and owns the fingers currently down.
///
/// Every gesture is measured against the track's width on screen, so all of
/// these do nothing until the track has been laid out.
#[derive(Clone, Copy)]
struct TimelineGestures {
    bounds: TimelineView,
    view: RwSignal<TimelineView>,
    track: NodeRef<leptos::html::Div>,
    fingers: StoredValue<PinchGesture>,
}

impl TimelineGestures {
    /// Zooms on `ctrl`/`meta` and the wheel, and pans on a horizontal wheel.
    ///
    /// A plain vertical wheel is left alone so that it still scrolls the page.
    fn wheel(self, event: &web_sys::WheelEvent) {
        if event.ctrl_key() || event.meta_key() {
            event.prevent_default();
            self.zoom_about(
                (event.delta_y() * WHEEL_ZOOM_PER_PIXEL).exp(),
                f64::from(event.client_x()),
            );
        } else if event.delta_x() != 0.0 {
            event.prevent_default();
            self.pan_by(-event.delta_x());
        }
    }

    fn finger_down(self, event: &web_sys::PointerEvent) {
        self.fingers.set_value(
            self.fingers
                .get_value()
                .press(event.pointer_id(), f64::from(event.client_x())),
        );
    }

    fn finger_moved(self, event: &web_sys::PointerEvent) {
        let (moved, delta) = self
            .fingers
            .get_value()
            .slide(event.pointer_id(), f64::from(event.client_x()));
        self.fingers.set_value(moved);
        let Some(delta) = delta else {
            return;
        };

        event.prevent_default();
        self.zoom_about(delta.scale, delta.anchor_client_x);
        self.pan_by(delta.pan_pixels);
    }

    fn finger_lifted(self, event: &web_sys::PointerEvent) {
        self.fingers
            .set_value(self.fingers.get_value().release(event.pointer_id()));
    }

    fn zoom_about(self, scale: f64, client_x: f64) {
        let Some(track) = self.laid_out_track() else {
            return;
        };
        let current = self.view.get_untracked();
        let anchor = time_at_client_x(current, track.left(), track.width(), client_x);
        self.view
            .set(zoom_view(current, self.bounds, scale, anchor));
    }

    /// Moves the window against `pan_pixels` of finger travel, so the footage
    /// under the fingers follows them.
    fn pan_by(self, pan_pixels: f64) {
        let Some(track) = self.laid_out_track() else {
            return;
        };
        let current = self.view.get_untracked();
        let seconds = -pan_pixels / track.width() * current.duration();
        self.view.set(pan_view(current, self.bounds, seconds));
    }

    /// The track's box on screen, once it has one to divide screen pixels by.
    fn laid_out_track(self) -> Option<web_sys::DomRect> {
        self.track
            .get_untracked()
            .map(|element| element.get_bounding_client_rect())
            .filter(|track| track.width() > 0.0)
    }
}

/// Names the window the track is showing, so a zoomed view says where it is.
#[component]
fn TimelineScale(view: RwSignal<TimelineView>) -> impl IntoView {
    view! {
        <div class="timeline-scale" aria-hidden="true">
            <span>{move || format_clock_time(view.get().start_time)}</span>
            <span>{move || {
                let view = view.get();
                format_clock_time(view.start_time.midpoint(view.end_time))
            }}</span>
            <span>{move || format_clock_time(view.get().end_time)}</span>
        </div>
    }
}

/// Renders one clickable span per stretch of retained footage.
#[component]
fn AvailabilitySpans(clips: Vec<RecordingClip>, controls: TimelineControls) -> impl IntoView {
    clips
        .into_iter()
        .map(|clip| {
            let start_time = clip.range.start_time;
            let end_time = clip.range.end_time;
            let label = format!("Play from {}", format_event_time(start_time));
            view! { <button
                type="button"
                class="timeline-availability"
                style=move || controls.span_style(start_time, end_time)
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
            let label = format!(
                "Motion recording at {}",
                format_event_time(activity.start_time),
            );
            let click_clips = clips.clone();
            view! { <button
                type="button"
                class="timeline-activity timeline-motion-recording activity-motion"
                style=move || controls.span_style(activity.start_time, activity.end_time)
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
                controls.bounds.start_time,
                controls.bounds.end_time,
            )?;
            let activity = TimelineActivity {
                start_time,
                end_time,
            };
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
                style=move || controls.span_style(start_time, end_time)
                aria-label=label
                on:click=move |_| controls.select(
                    Some(activity),
                    playable_time(&click_clips, start_time),
                )
            ></button> })
        })
        .collect_view()
}

/// The shortest window zooming can produce. A minute across the full track is
/// roughly a tenth of a second per pixel on a phone, which is finer than the
/// playhead's one-second step can address.
const MINIMUM_VIEW_SECONDS: f64 = 60.0;

/// Converts a wheel notch into a zoom factor. A notch is 100 pixels on most
/// platforms, so this makes one notch change the window by about a fifth,
/// small enough that a single flick does not overshoot the span being aimed at.
const WHEEL_ZOOM_PER_PIXEL: f64 = 0.002;

/// Two fingers closer together than this pinch too coarsely to derive a scale
/// from, and at zero would divide by it.
const MINIMUM_PINCH_SPAN_PIXELS: f64 = 8.0;

/// Narrows or widens `view` within `bounds` by `scale`, keeping `anchor_time`
/// under the same point on the track.
///
/// A `scale` below 1.0 zooms in. The result is never shorter than
/// `MINIMUM_VIEW_SECONDS`, never longer than `bounds`, and never outside it.
fn zoom_view(
    view: TimelineView,
    bounds: TimelineView,
    scale: f64,
    anchor_time: f64,
) -> TimelineView {
    let shortest = MINIMUM_VIEW_SECONDS.min(bounds.duration());
    let duration = (view.duration() * scale).clamp(shortest, bounds.duration());
    let anchor_fraction = view.fraction_of(anchor_time);
    positioned_view(
        anchor_fraction.mul_add(-duration, anchor_time),
        duration,
        bounds,
    )
}

/// Moves `view` by `seconds` without changing its duration, stopping at the
/// edges of `bounds`.
fn pan_view(view: TimelineView, bounds: TimelineView, seconds: f64) -> TimelineView {
    positioned_view(view.start_time + seconds, view.duration(), bounds)
}

fn positioned_view(start_time: f64, duration: f64, bounds: TimelineView) -> TimelineView {
    // The clamp is well-formed only while duration fits inside bounds, which is
    // what both callers guarantee before reaching here.
    let start_time = start_time.clamp(bounds.start_time, bounds.end_time - duration);
    TimelineView {
        start_time,
        end_time: start_time + duration,
    }
}

/// Reads the time under a pointer, given the track's position and width on
/// screen.
fn time_at_client_x(view: TimelineView, track_left: f64, track_width: f64, client_x: f64) -> f64 {
    let fraction = ((client_x - track_left) / track_width).clamp(0.0, 1.0);
    fraction.mul_add(view.duration(), view.start_time)
}

/// The fingers of a two-finger gesture, tracked so that spreading them zooms
/// the view and sliding them together moves it.
#[derive(Clone, Copy, Default)]
struct PinchGesture {
    fingers: [Option<TrackedFinger>; 2],
}

#[derive(Clone, Copy)]
struct TrackedFinger {
    pointer_id: i32,
    client_x: f64,
}

/// What one finger's movement changed about a two-finger gesture.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PinchDelta {
    /// Multiplier for the view's duration; below 1.0 the fingers spread apart.
    scale: f64,
    /// How far the fingers travelled together, in screen pixels.
    pan_pixels: f64,
    /// The point on screen the zoom keeps fixed.
    anchor_client_x: f64,
}

impl PinchGesture {
    fn press(mut self, pointer_id: i32, client_x: f64) -> Self {
        let finger = TrackedFinger {
            pointer_id,
            client_x,
        };
        if let Some(slot) = self
            .fingers
            .iter_mut()
            .find(|slot| slot.is_none_or(|tracked| tracked.pointer_id == pointer_id))
        {
            *slot = Some(finger);
        }
        self
    }

    fn tracks(self, pointer_id: i32) -> bool {
        self.fingers
            .iter()
            .any(|slot| slot.is_some_and(|tracked| tracked.pointer_id == pointer_id))
    }

    fn release(mut self, pointer_id: i32) -> Self {
        for slot in &mut self.fingers {
            if slot.is_some_and(|tracked| tracked.pointer_id == pointer_id) {
                *slot = None;
            }
        }
        self
    }

    /// Moves one finger, reporting the zoom and pan the pair now describes.
    ///
    /// Reports nothing until both fingers are down, so a one-finger drag stays
    /// with the playhead rather than moving the view under it.
    fn slide(self, pointer_id: i32, client_x: f64) -> (Self, Option<PinchDelta>) {
        if !self.tracks(pointer_id) {
            return (self, None);
        }

        // A finger is followed even while it is alone on the track, so that a
        // pinch beginning after a playhead drag measures from where that finger
        // now is rather than from where it first landed.
        let previous = self.fingers;
        let moved = self.press(pointer_id, client_x);
        let ([Some(first), Some(second)], [Some(next_first), Some(next_second)]) =
            (previous, moved.fingers)
        else {
            return (moved, None);
        };
        let previous_span = (first.client_x - second.client_x).abs();
        let current_span = (next_first.client_x - next_second.client_x).abs();
        if previous_span < MINIMUM_PINCH_SPAN_PIXELS || current_span < MINIMUM_PINCH_SPAN_PIXELS {
            return (moved, None);
        }

        let current_midpoint = next_first.client_x.midpoint(next_second.client_x);
        (
            moved,
            Some(PinchDelta {
                scale: previous_span / current_span,
                pan_pixels: current_midpoint - first.client_x.midpoint(second.client_x),
                anchor_client_x: current_midpoint,
            }),
        )
    }
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

    /// A week-long selection, the range zoom exists for: a 30-second motion
    /// span is 0.005% of it, which rounds to nothing the reader can hit.
    const WEEK: TimelineView = TimelineView {
        start_time: 0.0,
        end_time: 604_800.0,
    };

    /// Compares view arithmetic well inside a millisecond, which is finer than
    /// the playhead's one-second step or any label derived from it.
    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 0.001,
            "expected {expected}, got {actual}",
        );
    }

    #[test]
    fn zooming_in_keeps_the_anchored_time_under_the_same_point() {
        let zoomed = zoom_view(WEEK, WEEK, 0.5, 302_400.0);

        assert_close(zoomed.duration(), 302_400.0);
        assert_close(zoomed.fraction_of(302_400.0), WEEK.fraction_of(302_400.0));

        let off_centre = zoom_view(WEEK, WEEK, 0.5, 151_200.0);
        assert_close(off_centre.fraction_of(151_200.0), 0.25);
    }

    #[test]
    fn zooming_stops_at_one_minute_and_at_the_whole_selection() {
        let deep = zoom_view(WEEK, WEEK, 0.000_001, 302_400.0);
        assert_close(deep.duration(), MINIMUM_VIEW_SECONDS);

        assert_eq!(zoom_view(deep, WEEK, 1_000_000.0, 302_400.0), WEEK);
    }

    #[test]
    fn a_selection_shorter_than_the_minimum_window_still_zooms_to_itself() {
        let minute = TimelineView {
            start_time: 0.0,
            end_time: 30.0,
        };

        assert_eq!(zoom_view(minute, minute, 0.1, 15.0), minute);
    }

    #[test]
    fn panning_and_zooming_never_leave_the_selection() {
        let window = zoom_view(WEEK, WEEK, 0.25, 302_400.0);

        assert_close(
            pan_view(window, WEEK, -f64::MAX).start_time,
            WEEK.start_time,
        );
        assert_close(pan_view(window, WEEK, f64::MAX).end_time, WEEK.end_time);

        let at_start = zoom_view(WEEK, WEEK, 0.5, WEEK.start_time);
        assert_close(at_start.start_time, WEEK.start_time);
        let at_end = zoom_view(WEEK, WEEK, 0.5, WEEK.end_time);
        assert_close(at_end.end_time, WEEK.end_time);
    }

    #[test]
    fn a_wheel_notch_zooms_by_about_a_fifth_in_each_direction() {
        let out = zoom_view(WEEK, WEEK, (100.0 * WHEEL_ZOOM_PER_PIXEL).exp(), 302_400.0);
        let half = TimelineView {
            start_time: 0.0,
            end_time: 302_400.0,
        };
        let into = zoom_view(half, WEEK, (-100.0 * WHEEL_ZOOM_PER_PIXEL).exp(), 151_200.0);

        // Clamped to the whole selection, which one notch out already exceeds.
        assert_eq!(out, WEEK);
        assert!((into.duration() / half.duration() - 0.818_7).abs() < 0.001);
    }

    #[test]
    fn one_finger_reports_nothing_until_a_second_joins_it() {
        let (dragged, delta) = PinchGesture::default().press(1, 100.0).slide(1, 200.0);
        assert_eq!(delta, None);

        // The lone finger was still followed to 200, so the pinch measures its
        // span from there rather than from where it first landed.
        let (_, delta) = dragged.press(2, 400.0).slide(2, 600.0);
        assert_eq!(
            delta,
            Some(PinchDelta {
                scale: 0.5,
                pan_pixels: 100.0,
                anchor_client_x: 400.0,
            })
        );
    }

    #[test]
    fn spreading_the_fingers_zooms_in_and_pinching_them_zooms_out() {
        let pinched = PinchGesture::default().press(1, 100.0).press(2, 200.0);

        let (_, spread) = pinched.slide(2, 300.0);
        assert_close(spread.expect("two fingers are down").scale, 0.5);

        let (_, squeezed) = pinched.slide(2, 150.0);
        assert_close(squeezed.expect("two fingers are down").scale, 2.0);
    }

    #[test]
    fn fingers_travelling_together_move_the_view_by_what_they_travelled() {
        let pinched = PinchGesture::default().press(1, 100.0).press(2, 200.0);

        // A browser reports one pointer at a time, so a 50-pixel two-finger
        // slide arrives as two events that each move the midpoint half as far.
        let (moved, first_step) = pinched.slide(1, 150.0);
        let (_, second_step) = moved.slide(2, 250.0);

        assert_close(first_step.expect("two fingers are down").pan_pixels, 25.0);
        assert_close(second_step.expect("two fingers are down").pan_pixels, 25.0);
    }

    #[test]
    fn a_lifted_finger_stops_reporting_a_pinch() {
        let (_, delta) = PinchGesture::default()
            .press(1, 100.0)
            .press(2, 200.0)
            .release(2)
            .slide(1, 150.0);

        assert_eq!(delta, None);
    }

    #[test]
    fn a_pointer_the_track_never_saw_does_not_move_the_view() {
        let (_, delta) = PinchGesture::default()
            .press(1, 100.0)
            .press(2, 200.0)
            .slide(9, 400.0);

        assert_eq!(delta, None);
    }

    #[test]
    fn the_time_under_a_pointer_is_read_from_the_track_geometry() {
        let window = TimelineView {
            start_time: 100.0,
            end_time: 200.0,
        };

        assert_close(time_at_client_x(window, 40.0, 400.0, 240.0), 150.0);
        assert_close(time_at_client_x(window, 40.0, 400.0, 0.0), 100.0);
        assert_close(time_at_client_x(window, 40.0, 400.0, 1_000.0), 200.0);
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
