//! The "All cameras" grid on `/recordings`: one tile per camera, packed by
//! `monitor_layout::pack_tiles` (issue #20's aspect-ratio-aware algorithm,
//! reused verbatim) and re-packed on resize via a `ResizeObserver` -- D-2(b)
//! in `.agents/issue-22/DESIGN-all-cameras-scrubber.md`. Unlike `monitor.rs`'s
//! own wall, this page is ordinary in-flow content, not a fixed kiosk
//! viewport, so its own container's size can change after mount and the
//! layout has to follow it.
//!
//! Issue #22 item R3 added real playback: one shared `selected_time`/
//! `uses_preview` pair drives every tile's own, unmodified
//! `crate::timeline::TimelinePlayer` (D-1(c)) -- N independent per-camera
//! state machines, never a single shared player, and never a cross-tile
//! barrier (D-1's rejected option (b)). Each tile owns its own recording
//! segments fetch and its own "not yet settled" signal.
//!
//! Item R4 adds the per-tile no-footage representation D-3(d) chose: a
//! camera with no retained recordings anywhere in the loaded range gets an
//! explicit placeholder, and a camera with a real gap at the shared scrub
//! position (but clips elsewhere in range) gets a dimmed still frame at its
//! own nearest playable time, labeled with its signed offset from the shared
//! scrub position. Both are recomputed reactively as `selected_time` moves,
//! never by snapping the shared signal itself (D-3's rejected option (c)).
//!
//! Resolved 2026-09-06 (`.agents/issue-22/PLAN-all-cameras-scrubber.md`'s
//! STOP-AND-ASK gates, `REVIEW-R4.md`): a camera whose `clips` list is empty
//! but whose `previews` list is not gets the SAME gap treatment as above --
//! a dimmed nearest-frame preview and offset label -- rather than the plain
//! no-recordings placeholder, since Frigate prunes recording segments and
//! preview intervals independently per `retain.mode`
//! (`frigate/record/cleanup.py:150-260` at `v0.17.2`) and a preview
//! outliving its camera's last recording clip is a routine outcome, not an
//! anomaly. See `AllCameraTileContent`'s own doc comment for the full branch
//! and `nearest_preview`, this module's previews-only mirror of
//! `crate::timeline::playable_time`.
//!
//! Issue #25 item S2 hoists each tile's own `recording_clips` fetch
//! (`fetch_recording_segments` reduced through `recording_media`) up into
//! `AllCamerasGrid`, which now owns one `LocalResource` per camera and passes
//! each tile its own resource by index -- `AllCameraTile` no longer creates
//! its own. Once every camera's own resource has resolved, `AllCamerasGrid`
//! folds every `Ok(media)` through `merge_recording_media` (D-4(a)'s true
//! interval union for `clips`, D-6(a)'s plain concatenation for
//! `motion_ranges`) into one shared union track. S3 renders that union
//! through `crate::timeline::RecordingScrubber` as a sibling of this grid's
//! own tiles; this item only builds the hoisted fetch and the merge itself.

use corvette_api::{Camera, PreviewClip, ReviewSegment};
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, ResizeObserver, ResizeObserverEntry};

use crate::local_time::format_event_time;
use crate::media::{RecordingMedia, RecordingRange, merge_recording_media, recording_media};
use crate::monitor_layout::{TileRect, pack_tiles};
use crate::shell::{Status, StatusGlyph};
use crate::timeline::{TimelinePlayer, clip_at_time, playable_time};

/// Placeholder camera name for merged availability spans that cannot
/// honestly name one real camera (D-4(a)'s own named cost, D-5(b)'s "discard
/// it" resolution) -- the same sentinel `recordings.rs`'s own private
/// `ALL_CAMERAS` constant uses for "every camera at once" (`recordings.rs`).
/// Not imported from there: that constant is private to `recordings.rs`, and
/// G-12 confirms no consumer of a merged clip (`AvailabilitySpans`,
/// `MotionSpans`, `ReviewMarkers`) ever reads it back, so it is not worth a
/// visibility change to share one literal.
const MERGED_CAMERA_PLACEHOLDER: &str = "all";

/// Renders every configured camera for the loaded range
/// (`start_time`/`end_time`; the range's own `camera` field is always `"all"`
/// and carries nothing a tile needs), reusing `cameras` and `preview_clips`
/// (`RecordingBrowser`'s already-fetched resources -- D-5's own cost analysis
/// for `cameras`, and F-13 for `preview_clips` already answering every
/// camera at once under `camera == "all"`) rather than fetching either again.
///
/// Owns the one `selected_time`/`uses_preview` pair every tile's own
/// `TimelinePlayer` is driven by -- the "one scrub position driving N tiles"
/// mechanism D-1(c) describes.
#[component]
pub(crate) fn AllCamerasPlayback(
    start_time: f64,
    end_time: f64,
    cameras: LocalResource<Result<Vec<Camera>, String>>,
    preview_clips: LocalResource<Result<Vec<PreviewClip>, String>>,
    /// `RecordingBrowser`'s already-fetched, already-combined "All" mode
    /// reviews resource (`recordings.rs`'s `RecordingContext.review_activity`,
    /// G-6) -- forwarded straight through with no transformation (D-6(a)),
    /// never re-fetched or merged here. Nothing renders it yet (that is
    /// issue #25's own S3); this item only threads it through.
    review_activity: LocalResource<Result<Vec<ReviewSegment>, String>>,
) -> impl IntoView {
    let detail = format!(
        "{} – {}",
        format_event_time(start_time),
        format_event_time(end_time),
    );
    let selected_time = RwSignal::new(start_time);
    let uses_preview = RwSignal::new(true);

    view! {
        <div class="playback-heading"><h2>"All cameras"</h2><p>{detail}</p></div>
        <div class="all-cameras-scrub">
            <input
                type="range"
                min=start_time
                max=end_time
                step="1"
                prop:value=move || selected_time.get()
                aria-label="All-cameras playhead"
                on:input=move |event| {
                    let requested_time = event_target_value(&event)
                        .parse::<f64>()
                        .unwrap_or(start_time);
                    uses_preview.set(true);
                    selected_time.set(requested_time);
                }
            />
        </div>
        {move || match (cameras.get(), preview_clips.get()) {
            (None, _) | (_, None) => view! {
                <Status heading="Loading cameras" detail="Connecting to the Frigate API." glyph=StatusGlyph::Camera/>
            }.into_any(),
            (Some(Err(error)), _) | (_, Some(Err(error))) => view! {
                <Status heading="Cameras unavailable" detail=error glyph=StatusGlyph::Camera/>
            }.into_any(),
            (Some(Ok(cameras)), _) if cameras.is_empty() => view! {
                <Status heading="No cameras configured" detail="Add or enable a camera in Frigate, then reload this page." glyph=StatusGlyph::Camera/>
            }.into_any(),
            (Some(Ok(cameras)), Some(Ok(previews))) => view! {
                <AllCamerasGrid
                    cameras
                    previews
                    start_time
                    end_time
                    selected_time
                    uses_preview
                    review_activity
                />
            }.into_any(),
        }}
    }
}

/// One tile per camera, packed into `.all-cameras-grid`'s own real pixel box
/// on mount and again whenever that box resizes (D-2(b)) -- the resize-aware
/// counterpart to `monitor.rs`'s `MonitorGrid`, which deliberately does not
/// recompute on resize (that wall targets a fixed-size display loaded once).
///
/// Also owns every camera's own `recording_clips` fetch (hoisted here from
/// `AllCameraTile`, D-3(a)) and folds them, once every one has resolved,
/// through `merge_recording_media` into the shared union track issue #25's
/// own `RecordingScrubber` renders (S3) -- no per-camera resource ever
/// publishes its own resolved result back up through anything other than
/// this component's own `merged_media` memo, keeping D-3(a)'s own named
/// virtue (no child-to-parent publish of an async-resolved result) intact
/// for both the fetches and their merge.
#[component]
fn AllCamerasGrid(
    cameras: Vec<Camera>,
    previews: Vec<PreviewClip>,
    start_time: f64,
    end_time: f64,
    selected_time: RwSignal<f64>,
    uses_preview: RwSignal<bool>,
    /// Forwarded straight through from `AllCamerasPlayback`, unread by
    /// anything in this item -- issue #25's S3 renders it through the shared
    /// `RecordingScrubber`.
    review_activity: LocalResource<Result<Vec<ReviewSegment>, String>>,
) -> impl IntoView {
    let grid_container = NodeRef::<html::Div>::new();
    let camera_dims: Vec<(u32, u32)> = cameras
        .iter()
        .map(|camera| (camera.width, camera.height))
        .collect();
    let layout = RwSignal::new(Vec::<TileRect>::new());
    // Non-`Send`/non-`Sync` browser objects live behind this; `on_cleanup`
    // still requires a `Send + Sync` closure regardless of target -- see
    // `monitor.rs`'s own `session` field for the identical rationale.
    let resize_watcher = StoredValue::new_local(None::<GridResizeWatcher>);

    // Not read yet -- issue #25's S3 renders this through the shared
    // `RecordingScrubber`. Renaming the parameter itself to silence the
    // "unused" warning would force every call site off its current
    // `review_activity` shorthand (see `AllCamerasPlayback`'s own
    // `<AllCamerasGrid .. review_activity />`), so this is a plain
    // acknowledgment instead.
    let _ = &review_activity;

    // One `LocalResource` per camera, in the same order as `cameras`,
    // moved here verbatim from `AllCameraTile`'s own former `LocalResource`
    // (D-3(a)). `AllCameraTile` now takes its own resource as an incoming
    // prop instead of creating it.
    let recording_media_resources: Vec<LocalResource<Result<RecordingMedia, String>>> = cameras
        .iter()
        .map(|camera| {
            let camera_name = camera.name.clone();
            LocalResource::new(move || {
                let camera_name = camera_name.clone();
                async move {
                    crate::api::fetch_recording_segments(&camera_name, start_time, end_time)
                        .await
                        .map(|segments| {
                            let range = RecordingRange {
                                camera: camera_name,
                                start_time,
                                end_time,
                            };
                            recording_media(&range, segments)
                        })
                }
            })
        })
        .collect();

    // Waits for every camera's own resource to report `Some(_)` (`Ok` or
    // `Err` -- an errored camera contributes nothing to the merge, the same
    // tolerance `AllCameraTileContent`'s own per-tile error branch already
    // shows) before folding them into one union track, rather than merging
    // incrementally as each one resolves -- a plain-`Vec` prop change on
    // `RecordingScrubber` would otherwise re-run its whole component
    // function and reset its own pinch/zoom state on every straggling
    // camera's resolution during initial load. Every resource's own `.get()`
    // is read unconditionally on each run (never short-circuited) so this
    // memo keeps tracking every one of them as a reactive dependency, even
    // while some are still `None`.
    // Not read yet -- issue #25's S3 wires this into `RecordingScrubber`.
    let _merged_media: Memo<Option<RecordingMedia>> = {
        let resources = recording_media_resources.clone();
        Memo::new(move |_| {
            let statuses = resources.iter().map(LocalResource::get).collect::<Vec<_>>();
            if statuses.iter().any(Option::is_none) {
                return None;
            }
            let resolved = statuses
                .into_iter()
                .filter_map(|status| status.and_then(Result::ok))
                .collect();
            Some(merge_recording_media(MERGED_CAMERA_PLACEHOLDER, resolved))
        })
    };

    Effect::new(move |_| {
        let Some(container) = grid_container.get() else {
            return;
        };
        let bounds = container.get_bounding_client_rect();
        if bounds.width() > 0.0 && bounds.height() > 0.0 {
            layout.set(pack_tiles(&camera_dims, bounds.width(), bounds.height()));
        }
        resize_watcher.set_value(GridResizeWatcher::start(
            &container,
            camera_dims.clone(),
            layout,
        ));
    });

    on_cleanup(move || {
        if let Some(Some(watcher)) = resize_watcher.try_update_value(Option::take) {
            watcher.stop();
        }
    });

    view! {
        <div class="all-cameras-grid" node_ref=grid_container aria-label="All cameras">
            {cameras.into_iter().enumerate().map(|(index, camera)| {
                let camera_previews = previews
                    .iter()
                    .filter(|preview| preview.camera == camera.name)
                    .cloned()
                    .collect::<Vec<_>>();
                let media = recording_media_resources[index];
                view! {
                    <AllCameraTile
                        camera=camera
                        index=index
                        layout=layout
                        media
                        previews=camera_previews
                        selected_time
                        uses_preview
                    />
                }
            }).collect_view()}
        </div>
    }
}

/// Keeps a `ResizeObserver` and its JS-bound callback alive for exactly as
/// long as `.all-cameras-grid` stays mounted, re-running `pack_tiles`
/// against the container's own new content box on every observed resize.
/// `monitor_layout::pack_tiles` itself is untouched -- this only adds a new
/// caller and a new trigger to re-call it.
struct GridResizeWatcher {
    observer: ResizeObserver,
    _on_resize: Closure<dyn FnMut(js_sys::Array)>,
}

impl GridResizeWatcher {
    fn start(
        container: &Element,
        camera_dims: Vec<(u32, u32)>,
        layout: RwSignal<Vec<TileRect>>,
    ) -> Option<Self> {
        let on_resize = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
            let Some(entry) = entries.get(0).dyn_into::<ResizeObserverEntry>().ok() else {
                return;
            };
            let rect = entry.content_rect();
            if rect.width() <= 0.0 || rect.height() <= 0.0 {
                return;
            }
            layout.set(pack_tiles(&camera_dims, rect.width(), rect.height()));
        });
        let observer = ResizeObserver::new(on_resize.as_ref().unchecked_ref()).ok()?;
        observer.observe(container);
        Some(Self {
            observer,
            _on_resize: on_resize,
        })
    }

    fn stop(self) {
        self.observer.disconnect();
    }
}

/// Renders `rect`'s placement as an inline `style` value -- duplicated from
/// `monitor.rs`'s identical `tile_placement_style` rather than hoisted into
/// `monitor_layout.rs` itself (plan's own "Implementation-level choices" #4).
fn tile_placement_style(rect: TileRect) -> String {
    format!(
        "left: {:.4}px; top: {:.4}px; width: {:.4}px; height: {:.4}px",
        rect.x, rect.y, rect.width, rect.height
    )
}

/// One live tile: renders the existing single-camera `TimelinePlayer`
/// (`crate::timeline`, unmodified) against `media`, following the shared
/// `selected_time`/`uses_preview` signals `AllCamerasPlayback` owns -- the
/// "one scrub position driving N tiles" mechanism D-1(c) describes.
/// `active_activity` is this tile's own, permanently-`None` signal: "All"
/// mode never plays a clicked activity through to its end (Implementation-level
/// choice 2), so nothing here needs it shared across tiles.
///
/// `media` is this camera's own `recording_clips` resource, hoisted up into
/// (and created by) `AllCamerasGrid` (issue #25 S2, D-3(a)) rather than
/// fetched here -- `/{camera}/recordings` has no bulk form (F-12), so
/// `AllCamerasGrid` still creates one `LocalResource` per camera, just no
/// longer inside this component.
///
/// A camera with nothing at all retained in the loaded range, a camera with
/// previews but no clips, or a real gap at the shared scrub position, gets
/// R4's own placeholder (`AllCameraTileContent`) instead of `TimelinePlayer`
/// -- see that component's own doc comment for the full branch and D-3(d)'s
/// no-footage representation. The "not yet settled" overlay (D-1(c)) only
/// applies once a `TimelinePlayer` actually renders, so its own signal is
/// created fresh inside `AllCameraTileContent`'s live branch rather than
/// here.
#[component]
fn AllCameraTile(
    camera: Camera,
    index: usize,
    layout: RwSignal<Vec<TileRect>>,
    media: LocalResource<Result<RecordingMedia, String>>,
    previews: Vec<PreviewClip>,
    selected_time: RwSignal<f64>,
    uses_preview: RwSignal<bool>,
) -> impl IntoView {
    let Camera {
        name: camera_name,
        display_name,
        ..
    } = camera;
    let recording_clips = media;
    let content_camera_name = camera_name.clone();

    view! {
        <div
            class="all-cameras-tile"
            data-camera=camera_name
            style=move || {
                tile_placement_style(layout.get().get(index).copied().unwrap_or_default())
            }
        >
            <h2>{display_name}</h2>
            {
                let camera_name = content_camera_name;
                move || match recording_clips.get() {
                    None => ().into_any(),
                    Some(Err(error)) => view! {
                        <p class="all-cameras-tile-error" role="alert">{error}</p>
                    }.into_any(),
                    Some(Ok(media)) => view! {
                        <AllCameraTileContent
                            media
                            previews=previews.clone()
                            camera_name=camera_name.clone()
                            selected_time
                            uses_preview
                        />
                    }.into_any(),
                }
            }
        </div>
    }
}

/// Branches on this camera's own retained clips (and, since 2026-09-06's
/// resolved STOP-AND-ASK gate, its previews) against the shared scrub
/// position, per D-3(d):
///
/// - `clips` and `previews` both empty (nothing retained for this camera
///   anywhere in the loaded range): an explicit "no recordings" placeholder.
/// - `clips` empty but `previews` non-empty: the SAME dimmed-preview +
///   signed-offset-label treatment as the ordinary gap case below, sourced
///   from `nearest_preview` (this module's own previews-only mirror of
///   `playable_time`) instead, since `playable_time` panics via
///   `.expect(...)` on an empty `clips` slice
///   (`crate::timeline::playable_time`, unmodified, per this item's Scope
///   guard). Resolved 2026-09-06
///   (`.agents/issue-22/PLAN-all-cameras-scrubber.md`'s STOP-AND-ASK gates,
///   `REVIEW-R4.md`): Frigate prunes recording segments and preview
///   intervals independently per `retain.mode`
///   (`frigate/record/cleanup.py:150-260` at `v0.17.2`), so a preview
///   outliving its camera's last recording clip is a routine outcome of a
///   non-`all` retain mode, not an anomaly -- there genuinely is a usable
///   preview frame here, so it is shown rather than hidden behind the plain
///   no-recordings placeholder.
/// - `clips` non-empty but a real gap at the shared scrub position
///   (`clip_at_time` finds nothing, but `clips` is non-empty so
///   `playable_time` cannot panic): a dimmed still frame at the nearest
///   playable time (`RecordingRange::poster_url`), labeled with its signed
///   offset from the shared scrub position.
/// - otherwise: `TimelinePlayer`, unmodified, exactly as R3 built it, with
///   its own fresh "not yet settled" signal and overlay.
///
/// The clips-based gap/live branch is a `Memo` over `clip_at_time`'s own
/// boolean result, not a plain reactive closure, so `TimelinePlayer` is
/// mounted once per live span and is not torn down and rebuilt on every
/// scrub tick that stays inside the same span -- only `Memo`'s
/// change-detected transitions between "gap" and "live" remount it. The
/// previews-only branch has no such transition to guard (it is the only
/// view this camera ever renders, for its own lifetime, once `clips` is
/// known empty), so it is a plain reactive closure. Both gap-style
/// branches' offset labels still update on every tick while showing, since
/// they read `selected_time` directly.
#[component]
fn AllCameraTileContent(
    media: RecordingMedia,
    previews: Vec<PreviewClip>,
    camera_name: String,
    selected_time: RwSignal<f64>,
    uses_preview: RwSignal<bool>,
) -> impl IntoView {
    if media.clips.is_empty() {
        if previews.is_empty() {
            return view! {
                <div class="all-cameras-tile-no-recordings" role="status">
                    <p>"No recordings for this camera in this range."</p>
                </div>
            }
            .into_any();
        }

        return view! {
            {move || {
                let Some((preview, nearest_time)) =
                    nearest_preview(&previews, selected_time.get())
                else {
                    // Structurally unreachable: this branch only renders
                    // when `previews` is confirmed non-empty above.
                    return ().into_any();
                };
                let offset_seconds = nearest_time - selected_time.get();
                // Requests the actual low-res preview video Frigate already
                // retained, paused at the nearest playable instant via the
                // standard Media Fragments URI temporal dimension
                // (https://www.w3.org/TR/media-frags/#naming-time) -- the
                // same "encode the moment in the URL" idiom
                // `RecordingRange::poster_url`/`clip_url` already use, with
                // no new JS seek wiring. Frigate's own recordings-snapshot
                // endpoint (`GET /{camera}/recordings/{frame_time}/
                // snapshot.{format}`, v0.17.2) cannot be reused here: it
                // looks up its frame in the `Recordings` table and 404s
                // when this camera has no retained recording clip at all.
                let offset_in_preview = (nearest_time - preview.start).max(0.0);
                let video_src = format!("{}#t={offset_in_preview:.3}", preview.src);
                view! {
                    <div class="all-cameras-tile-gap">
                        <video
                            class="all-cameras-tile-gap-preview"
                            src=video_src
                            muted
                            playsinline
                            preload="auto"
                        ></video>
                        <p class="all-cameras-tile-gap-offset">
                            "No recording at this exact time (nearest "
                            {format_gap_offset(offset_seconds)}
                            ")"
                        </p>
                    </div>
                }
                .into_any()
            }}
        }
        .into_any();
    }

    let clips = media.clips;
    let is_gap = Memo::new({
        let clips = clips.clone();
        move |_| clip_at_time(&clips, selected_time.get()).is_none()
    });

    view! {
        {move || if is_gap.get() {
            let nearest_time = playable_time(&clips, selected_time.get());
            let offset_seconds = nearest_time - selected_time.get();
            let poster = RecordingRange {
                camera: camera_name.clone(),
                start_time: nearest_time,
                end_time: nearest_time,
            }.poster_url();
            view! {
                <div class="all-cameras-tile-gap">
                    <img class="all-cameras-tile-gap-preview" src=poster alt="" />
                    <p class="all-cameras-tile-gap-offset">
                        "No recording at this exact time (nearest "
                        {format_gap_offset(offset_seconds)}
                        ")"
                    </p>
                </div>
            }.into_any()
        } else {
            let settled = RwSignal::new(false);
            view! {
                <TimelinePlayer
                    clips=clips.clone()
                    previews=previews.clone()
                    activities=Vec::new()
                    active_activity=RwSignal::new(None)
                    selected_time
                    uses_preview
                    settled=settled
                />
                {move || (!settled.get()).then(|| view! {
                    <div class="all-cameras-tile-unsettled" aria-hidden="true"></div>
                })}
            }.into_any()
        }}
    }
    .into_any()
}

/// Renders a signed, whole-second offset from the shared scrub position to a
/// gapped tile's own nearest playable frame, e.g. `"+12s"` when the nearest
/// footage is later than the scrub position, `"-8s"` when it is earlier.
fn format_gap_offset(offset_seconds: f64) -> String {
    format!("{offset_seconds:+.0}s")
}

/// A preview's own `end` is exclusive (mirrors `playback_source`'s own
/// preview match, `crate::timeline`'s `preview.start <= time && time <
/// preview.end`), so the nearest-preview computation below stays this far
/// short of it -- the previews-only counterpart of `crate::timeline`'s own
/// `CLIP_END_MARGIN_SECONDS`. Not reused directly: that constant is private
/// to `timeline.rs`, and this item's Scope guard forbids editing that module.
const PREVIEW_END_MARGIN_SECONDS: f64 = 1.0;

/// Mirrors `crate::timeline::playable_time`'s own logic and shape
/// (unmodified, per this item's Scope guard), but reasons over
/// `&[PreviewClip]` instead of `&[RecordingClip]` -- there is no
/// previews-vs-clips adapter in `timeline.rs`, and `playable_time` itself
/// panics via `.expect(...)` on an empty `clips` slice, so it cannot be
/// called for a camera whose `clips` list is empty. Callers only invoke this
/// once `previews` is confirmed non-empty (`AllCameraTileContent`'s own
/// guard); `None` here would only mean `previews` was itself empty.
///
/// Returns the nearest preview that covers, or is closest to, `requested_time`,
/// paired with the playable instant within it -- the exact `requested_time`
/// itself when some preview already covers it, otherwise the closest instant
/// clamped inside a preview's own `start..end - PREVIEW_END_MARGIN_SECONDS`
/// span, same tie-breaking as `playable_time` (`total_cmp` on absolute
/// distance).
fn nearest_preview(previews: &[PreviewClip], requested_time: f64) -> Option<(PreviewClip, f64)> {
    if let Some(preview) = previews
        .iter()
        .find(|preview| preview.start <= requested_time && requested_time < preview.end)
    {
        return Some((preview.clone(), requested_time));
    }

    previews
        .iter()
        .map(|preview| {
            let last_playable_time = (preview.end - PREVIEW_END_MARGIN_SECONDS).max(preview.start);
            (
                preview.clone(),
                requested_time.clamp(preview.start, last_playable_time),
            )
        })
        .min_by(|(_, left), (_, right)| {
            (left - requested_time)
                .abs()
                .total_cmp(&(right - requested_time).abs())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(camera: &str, start: f64, end: f64) -> PreviewClip {
        PreviewClip {
            camera: camera.to_owned(),
            src: format!("/clips/previews/{camera}/{start}-{end}.mp4"),
            media_type: "video/mp4".to_owned(),
            start,
            end,
        }
    }

    #[test]
    fn nearest_preview_returns_the_requested_time_when_already_covered() {
        let previews = vec![preview("front", 100.0, 200.0)];
        let (found, nearest_time) = nearest_preview(&previews, 150.0).unwrap();
        assert_eq!(found.src, previews[0].src);
        assert!((nearest_time - 150.0).abs() < f64::EPSILON);
    }

    #[test]
    fn nearest_preview_snaps_to_the_nearest_covering_preview_on_a_gap() {
        let previews = vec![
            preview("front", 100.0, 120.0),
            preview("front", 150.0, 180.0),
        ];

        let (found, nearest_time) = nearest_preview(&previews, 140.0).unwrap();
        assert_eq!(found.src, previews[1].src);
        assert!((nearest_time - 150.0).abs() < f64::EPSILON);

        let (found, nearest_time) = nearest_preview(&previews, 125.0).unwrap();
        assert_eq!(found.src, previews[0].src);
        assert!((nearest_time - 119.0).abs() < f64::EPSILON);

        let (found, nearest_time) = nearest_preview(&previews, 180.5).unwrap();
        assert_eq!(found.src, previews[1].src);
        assert!((nearest_time - 179.0).abs() < f64::EPSILON);
    }

    #[test]
    fn nearest_preview_returns_none_for_no_previews() {
        assert!(nearest_preview(&[], 100.0).is_none());
    }
}
