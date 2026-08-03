//! Browser access to the Frigate API boundary.

use corvette_api::{
    Camera, Event, FrigateConfig, PreviewClip, RecordingSegment, ReviewEvent, ReviewSegment,
};
use gloo_net::http::Request;
use std::collections::BTreeMap;

const CONFIG_PATH: &str = "/api/config";
const EVENTS_PATH: &str = "/api/events?limit=10&sort=date_desc&has_clip=1";

/// Fetches the enabled cameras visible in Frigate's current configuration.
// Browser fetch futures are confined to the WASM thread and cannot implement Send.
#[allow(clippy::future_not_send)]
// The parent component is the only caller across this private module boundary.
#[allow(clippy::redundant_pub_crate)]
pub(super) async fn fetch_cameras() -> Result<Vec<Camera>, String> {
    let response = Request::get(CONFIG_PATH)
        .header("X-CACHE-BYPASS", "1")
        .send()
        .await
        .map_err(|error| format!("request {CONFIG_PATH}: {error}"))?;

    if !response.ok() {
        return Err(format!(
            "request {CONFIG_PATH}: server returned HTTP {}",
            response.status()
        ));
    }

    let config = response
        .json::<FrigateConfig>()
        .await
        .map_err(|error| format!("decode {CONFIG_PATH} response: {error}"))?;
    Ok(config.enabled_cameras())
}

/// Fetches the 10 most recent playable events in descending start-time order.
// Browser fetch futures are confined to the WASM thread and cannot implement Send.
#[allow(clippy::future_not_send)]
// The parent component is the only caller across this private module boundary.
#[allow(clippy::redundant_pub_crate)]
pub(super) async fn fetch_events() -> Result<Vec<Event>, String> {
    let response = Request::get(EVENTS_PATH)
        .send()
        .await
        .map_err(|error| format!("request {EVENTS_PATH}: {error}"))?;

    if !response.ok() {
        return Err(format!(
            "request {EVENTS_PATH}: server returned HTTP {}",
            response.status()
        ));
    }

    response
        .json::<Vec<Event>>()
        .await
        .map_err(|error| format!("decode {EVENTS_PATH} response: {error}"))
}

/// Fetches review activity within the dashboard's recent time window.
// Browser fetch futures are confined to the WASM thread and cannot implement Send.
#[allow(clippy::future_not_send)]
// The parent component is the only caller across this private module boundary.
#[allow(clippy::redundant_pub_crate)]
pub(super) async fn fetch_reviews(after: f64, before: f64) -> Result<Vec<ReviewEvent>, String> {
    let path = format!("/api/review?cameras=all&after={after}&before={before}&limit=1000");
    let response = Request::get(&path)
        .send()
        .await
        .map_err(|error| format!("request {path}: {error}"))?;

    if !response.ok() {
        return Err(format!(
            "request {path}: server returned HTTP {}",
            response.status()
        ));
    }

    response
        .json::<Vec<ReviewEvent>>()
        .await
        .map_err(|error| format!("decode {path} response: {error}"))
}

/// Fetches review activity for one camera and timestamp range.
// Browser fetch futures are confined to the WASM thread and cannot implement Send.
#[allow(clippy::future_not_send)]
// The parent component is the only caller across this private module boundary.
#[allow(clippy::redundant_pub_crate)]
pub(super) async fn fetch_review_activity(
    camera: &str,
    after: f64,
    before: f64,
) -> Result<Vec<ReviewSegment>, String> {
    if camera.is_empty() {
        return Ok(Vec::new());
    }

    let camera = js_sys::encode_uri_component(camera);
    let path = format!("/api/review?cameras={camera}&after={after}&before={before}&limit=1000");
    let response = Request::get(&path)
        .send()
        .await
        .map_err(|error| format!("request {path}: {error}"))?;

    if !response.ok() {
        return Err(format!(
            "request {path}: server returned HTTP {}",
            response.status()
        ));
    }

    response
        .json::<Vec<ReviewSegment>>()
        .await
        .map_err(|error| format!("decode {path} response: {error}"))
}

/// Fetches the local dates with retained recordings for one camera.
// Browser fetch futures are confined to the WASM thread and cannot implement Send.
#[allow(clippy::future_not_send)]
// The parent component is the only caller across this private module boundary.
#[allow(clippy::redundant_pub_crate)]
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
    let response = Request::get(&path)
        .send()
        .await
        .map_err(|error| format!("request {path}: {error}"))?;

    if !response.ok() {
        return Err(format!(
            "request {path}: server returned HTTP {}",
            response.status()
        ));
    }

    response
        .json::<BTreeMap<String, bool>>()
        .await
        .map_err(|error| format!("decode {path} response: {error}"))
}

/// Fetches retained media segments for one camera and timestamp range.
// Browser fetch futures are confined to the WASM thread and cannot implement Send.
#[allow(clippy::future_not_send)]
// The parent component is the only caller across this private module boundary.
#[allow(clippy::redundant_pub_crate)]
pub(super) async fn fetch_recording_segments(
    camera: &str,
    after: f64,
    before: f64,
) -> Result<Vec<RecordingSegment>, String> {
    let camera = js_sys::encode_uri_component(camera);
    let path = format!("/api/{camera}/recordings?after={after}&before={before}");
    let response = Request::get(&path)
        .send()
        .await
        .map_err(|error| format!("request {path}: {error}"))?;

    if !response.ok() {
        return Err(format!(
            "request {path}: server returned HTTP {}",
            response.status()
        ));
    }

    response
        .json::<Vec<RecordingSegment>>()
        .await
        .map_err(|error| format!("decode {path} response: {error}"))
}

/// Fetches Frigate's low-resolution preview videos for timeline scrubbing.
// Browser fetch futures are confined to the WASM thread and cannot implement Send.
#[allow(clippy::future_not_send)]
// The parent component is the only caller across this private module boundary.
#[allow(clippy::redundant_pub_crate)]
pub(super) async fn fetch_preview_clips(
    camera: &str,
    after: f64,
    before: f64,
) -> Result<Vec<PreviewClip>, String> {
    let camera = js_sys::encode_uri_component(camera);
    let path = format!("/api/preview/{camera}/start/{after}/end/{before}");
    let response = Request::get(&path)
        .send()
        .await
        .map_err(|error| format!("request {path}: {error}"))?;

    if response.status() == 404 {
        return Ok(Vec::new());
    }
    if !response.ok() {
        return Err(format!(
            "request {path}: server returned HTTP {}",
            response.status()
        ));
    }

    response
        .json::<Vec<PreviewClip>>()
        .await
        .map_err(|error| format!("decode {path} response: {error}"))
}
