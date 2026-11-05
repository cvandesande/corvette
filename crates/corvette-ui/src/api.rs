//! Failure and empty-result rules: `docs/design/api-contracts.md`.

// Browser fetch futures live on the WASM thread and cannot implement `Send`.
#![allow(clippy::future_not_send)]

use crate::activity::MOTION_BUCKET_SECONDS;
use corvette_api::{
    ActivityResponse, ActivityReview, Camera, CameraEntry, CameraList, Event, MediaLink,
    MotionActivity, PreviewClip, RecordingSegment, ReviewEvent, ReviewSegment,
};
use gloo_net::http::{Request, RequestBuilder};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;

const CAMERAS_PATH: &str = "/corvette/api/v1/cameras";
const ACTIVITY_PATH: &str = "/corvette/api/v1/activity";
const CLIP_LINK_PATH: &str = "/corvette/api/v1/clip-link";
const EVENTS_PATH: &str = "/api/events?limit=10&sort=date_desc&has_clip=1";
const REVIEW_LIMIT: u32 = 1_000;

// The request builder exposes no URL; callers pass the path separately.
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

async fn fetch_json<T: DeserializeOwned>(request: RequestBuilder, path: &str) -> Result<T, String> {
    fetch_json_or_absent(request, path)
        .await?
        .ok_or_else(|| format!("request {path}: server returned HTTP 404"))
}

pub(super) async fn fetch_camera_entries() -> Result<Vec<CameraEntry>, String> {
    let list: CameraList = fetch_json(Request::get(CAMERAS_PATH), CAMERAS_PATH).await?;
    Ok(list.cameras)
}

pub(super) async fn fetch_cameras() -> Result<Vec<Camera>, String> {
    Ok(fetch_camera_entries()
        .await?
        .into_iter()
        .map(Camera::from)
        .collect())
}

pub(super) async fn fetch_events() -> Result<Vec<Event>, String> {
    fetch_json(Request::get(EVENTS_PATH), EVENTS_PATH).await
}

pub(super) async fn fetch_reviews(after: f64, before: f64) -> Result<Vec<ReviewEvent>, String> {
    let path =
        format!("/api/review?cameras=all&after={after}&before={before}&limit={REVIEW_LIMIT}");
    fetch_json(Request::get(&path), &path).await
}

pub(super) async fn fetch_motion_activity(
    after: f64,
    before: f64,
) -> Result<Vec<MotionActivity>, String> {
    let path = format!(
        "/api/review/activity/motion?cameras=all&after={after}&before={before}&scale={MOTION_BUCKET_SECONDS}"
    );
    fetch_json(Request::get(&path), &path).await
}

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

pub(super) async fn fetch_activity_reviews(
    camera: &str,
    after: f64,
    before: f64,
) -> Result<Vec<ActivityReview>, String> {
    // The request builder appends `&` to a url that already has a query, and this route refuses the empty segment.
    let after = after.to_string();
    let before = before.to_string();
    let request = Request::get(ACTIVITY_PATH).query([
        ("cameras", camera),
        ("after", after.as_str()),
        ("before", before.as_str()),
        ("include", "reviews"),
    ]);
    let response: ActivityResponse = fetch_json(request, ACTIVITY_PATH).await?;
    Ok(response.reviews)
}

pub(super) async fn fetch_clip_link(
    camera: &str,
    start: f64,
    end: f64,
) -> Result<Option<MediaLink>, String> {
    let start = start.to_string();
    let end = end.to_string();
    let request = Request::get(CLIP_LINK_PATH).query([
        ("camera", camera),
        ("start", start.as_str()),
        ("end", end.as_str()),
    ]);
    fetch_json_or_absent(request, CLIP_LINK_PATH).await
}

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

pub(super) async fn fetch_recording_segments(
    camera: &str,
    after: f64,
    before: f64,
) -> Result<Vec<RecordingSegment>, String> {
    let camera = js_sys::encode_uri_component(camera);
    let path = format!("/api/{camera}/recordings?after={after}&before={before}");
    fetch_json(Request::get(&path), &path).await
}

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
