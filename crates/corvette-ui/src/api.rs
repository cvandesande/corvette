//! Browser access to the Frigate API boundary.

use corvette_api::{Camera, Event, FrigateConfig, ReviewSegment};
use gloo_net::http::Request;

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
