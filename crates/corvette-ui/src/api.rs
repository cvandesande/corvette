//! Browser access to the Frigate API boundary.
//!
//! Every request reports failure as a `String` naming the path and what went
//! wrong, for display to the reader. Two absences are reported as an empty
//! result instead of an error, and only these two: a camera that has not been
//! chosen yet, and Frigate's `404` for a range it has generated no preview
//! videos for. Every other unsuccessful status is an error.

// Browser fetch futures are confined to the WASM thread and cannot implement Send.
#![allow(clippy::future_not_send)]
// `pub` here would trip the workspace's denied `unreachable_pub`, since this
// module is private; `pub(super)` is what remains, and it trips this lint for
// the same reason. Neither visibility satisfies both lints.
#![allow(clippy::redundant_pub_crate)]

use corvette_api::{
    Camera, Event, FrigateConfig, MotionActivity, PreviewClip, RecordingSegment, ReviewEvent,
    ReviewSegment,
};
use gloo_net::http::{Request, RequestBuilder};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;

const CONFIG_PATH: &str = "/api/config";
const EVENTS_PATH: &str = "/api/events?limit=10&sort=date_desc&has_clip=1";
const REVIEW_LIMIT: u32 = 1_000;

/// Sends `request` and decodes its JSON body, reporting `404` as an absent
/// resource and naming `path` in any failure.
///
/// `path` is the request's own path; it is passed separately because the
/// builder does not expose the URL it was created with.
async fn fetch_json_or_absent<T: DeserializeOwned>(
    request: RequestBuilder,
    path: &str,
) -> Result<Option<T>, String> {
    let response = request
        .send()
        .await
        .map_err(|error| format!("request {path}: {error}"))?;

    if response.status() == 404 {
        return Ok(None);
    }
    if !response.ok() {
        return Err(format!(
            "request {path}: server returned HTTP {}",
            response.status()
        ));
    }

    response
        .json::<T>()
        .await
        .map(Some)
        .map_err(|error| format!("decode {path} response: {error}"))
}

/// Sends `request` and decodes its JSON body, naming `path` in any failure.
async fn fetch_json<T: DeserializeOwned>(request: RequestBuilder, path: &str) -> Result<T, String> {
    fetch_json_or_absent(request, path)
        .await?
        .ok_or_else(|| format!("request {path}: server returned HTTP 404"))
}

/// Fetches the enabled cameras visible in Frigate's current configuration.
pub(super) async fn fetch_cameras() -> Result<Vec<Camera>, String> {
    let config: FrigateConfig = fetch_json(
        Request::get(CONFIG_PATH).header("X-CACHE-BYPASS", "1"),
        CONFIG_PATH,
    )
    .await?;
    Ok(config.enabled_cameras())
}

/// Fetches the 10 most recent playable events in descending start-time order.
pub(super) async fn fetch_events() -> Result<Vec<Event>, String> {
    fetch_json(Request::get(EVENTS_PATH), EVENTS_PATH).await
}

/// Fetches review activity across every camera within a timestamp range.
pub(super) async fn fetch_reviews(after: f64, before: f64) -> Result<Vec<ReviewEvent>, String> {
    let path =
        format!("/api/review?cameras=all&after={after}&before={before}&limit={REVIEW_LIMIT}");
    fetch_json(Request::get(&path), &path).await
}

/// Fetches significant recording motion within a timestamp range.
pub(super) async fn fetch_motion_activity(
    after: f64,
    before: f64,
) -> Result<Vec<MotionActivity>, String> {
    const MOTION_BUCKET_SECONDS: u32 = 30;

    let path = format!(
        "/api/review/activity/motion?cameras=all&after={after}&before={before}&scale={MOTION_BUCKET_SECONDS}"
    );
    fetch_json(Request::get(&path), &path).await
}

/// Fetches review activity for one camera and timestamp range, or nothing when
/// no camera has been chosen.
pub(super) async fn fetch_review_activity(
    camera: &str,
    after: f64,
    before: f64,
) -> Result<Vec<ReviewSegment>, String> {
    if camera.is_empty() {
        return Ok(Vec::new());
    }

    let camera = js_sys::encode_uri_component(camera);
    let path =
        format!("/api/review?cameras={camera}&after={after}&before={before}&limit={REVIEW_LIMIT}");
    fetch_json(Request::get(&path), &path).await
}

/// Fetches the local dates with retained recordings for one camera, or nothing
/// when no camera has been chosen.
pub(super) async fn fetch_recording_days(
    camera: &str,
    timezone: &str,
) -> Result<BTreeMap<String, bool>, String> {
    if camera.is_empty() {
        return Ok(BTreeMap::new());
    }

    let camera = js_sys::encode_uri_component(camera);
    let timezone = js_sys::encode_uri_component(timezone);
    let path = format!("/api/recordings/summary?cameras={camera}&timezone={timezone}");
    fetch_json(Request::get(&path), &path).await
}

/// Fetches retained media segments for one camera and timestamp range.
pub(super) async fn fetch_recording_segments(
    camera: &str,
    after: f64,
    before: f64,
) -> Result<Vec<RecordingSegment>, String> {
    let camera = js_sys::encode_uri_component(camera);
    let path = format!("/api/{camera}/recordings?after={after}&before={before}");
    fetch_json(Request::get(&path), &path).await
}

/// Fetches Frigate's low-resolution preview videos for timeline scrubbing.
///
/// A range Frigate has generated no previews for answers `404`, which is
/// reported as no previews rather than as a failure.
pub(super) async fn fetch_preview_clips(
    camera: &str,
    after: f64,
    before: f64,
) -> Result<Vec<PreviewClip>, String> {
    let camera = js_sys::encode_uri_component(camera);
    let path = format!("/api/preview/{camera}/start/{after}/end/{before}");
    Ok(fetch_json_or_absent(Request::get(&path), &path)
        .await?
        .unwrap_or_default())
}
