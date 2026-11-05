//! Classification, stats reading and since-tracking for `GET /corvette/api/v1/cameras/status`.

use crate::config::get_body;
use corvette_api::CameraEntry;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;

// The whole stats answer measured on the current backing is 13653 bytes.
const STATS_LIMIT: usize = 4 * 1024 * 1024;

/// A run of failed picture fetches turns a live camera `stale` past this many seconds.
const STALE_AFTER_SECONDS: f64 = 60.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CameraState {
    Live,
    OffByChoice,
    NoPicture,
    Stale,
}

impl CameraState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::OffByChoice => "off_by_choice",
            Self::NoPicture => "no_picture",
            Self::Stale => "stale",
        }
    }
}

/// One camera's current run of failed picture fetches.
#[derive(Debug)]
struct StillFailure {
    failing_since: f64,
    last_failure: f64,
}

#[derive(Debug, Default)]
pub(crate) struct StatusState {
    observed: Mutex<HashMap<String, (CameraState, f64)>>,
    still_failures: Mutex<HashMap<String, StillFailure>>,
}

impl StatusState {
    pub(crate) fn observe(&self, name: &str, state: CameraState, now: f64) -> f64 {
        let mut observed = self.observed.lock().expect("status state table");
        match observed.get_mut(name) {
            Some(stored) if stored.0 == state => stored.1,
            Some(stored) => {
                *stored = (state, now);
                now
            }
            None => {
                observed.insert(name.to_owned(), (state, now));
                now
            }
        }
    }

    /// A delivered picture ends the camera's run of failed fetches.
    pub(crate) fn still_delivered(&self, name: &str) {
        self.still_failures
            .lock()
            .expect("still failure records")
            .remove(name);
    }

    pub(crate) fn still_failed(&self, name: &str, now: f64) {
        let mut failures = self.still_failures.lock().expect("still failure records");
        match failures.get_mut(name) {
            Some(record) => record.last_failure = now,
            None => {
                failures.insert(
                    name.to_owned(),
                    StillFailure {
                        failing_since: now,
                        last_failure: now,
                    },
                );
            }
        }
    }

    /// True while a run of failures began over a minute ago and a failure is recent.
    pub(crate) fn still_failing(&self, name: &str, now: f64) -> bool {
        self.still_failures
            .lock()
            .expect("still failure records")
            .get(name)
            .is_some_and(|record| {
                now - record.failing_since > STALE_AFTER_SECONDS
                    && now - record.last_failure <= STALE_AFTER_SECONDS
            })
    }
}

pub(crate) fn classify(entry: &CameraEntry, fps: &HashMap<String, f64>) -> CameraState {
    if !entry.enabled {
        return CameraState::OffByChoice;
    }
    match fps.get(&entry.name) {
        Some(camera_fps) if *camera_fps > 0.0 => CameraState::Live,
        _ => CameraState::NoPicture,
    }
}

pub(crate) async fn fetch_camera_fps(addr: &str) -> Result<HashMap<String, f64>, String> {
    let body = get_body(addr, "/api/stats", STATS_LIMIT, "stats")
        .await
        .map_err(|err| err.to_string())?;
    parse_stats(&body)
}

fn parse_stats(body: &[u8]) -> Result<HashMap<String, f64>, String> {
    let answer: Value = serde_json::from_slice(body)
        .map_err(|err| format!("stats answer is not valid JSON: {err}"))?;
    let cameras = answer
        .get("cameras")
        .and_then(Value::as_object)
        .ok_or_else(|| "stats answer has no cameras object".to_owned())?;
    let mut fps_by_name = HashMap::with_capacity(cameras.len());
    for (name, camera) in cameras {
        let fps = camera
            .get("camera_fps")
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("stats camera {name:?} has no numeric camera_fps"))?;
        fps_by_name.insert(name.clone(), fps);
    }
    Ok(fps_by_name)
}

pub(crate) fn status_body(
    entries: &[CameraEntry],
    fps: &HashMap<String, f64>,
    status_state: &StatusState,
    now: f64,
) -> Vec<u8> {
    let cameras: Vec<Value> = entries
        .iter()
        .map(|entry| {
            let mut state = classify(entry, fps);
            if state == CameraState::Live && status_state.still_failing(&entry.name, now) {
                state = CameraState::Stale;
            }
            json!({
                "name": entry.name,
                "state": state.as_str(),
                "since": status_state.observe(&entry.name, state, now),
            })
        })
        .collect();
    // The answer is strings and numbers only; serializing it cannot fail.
    serde_json::to_vec(&json!({ "cameras": cameras })).expect("status body serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera_entry(name: &str, enabled: bool) -> CameraEntry {
        CameraEntry {
            name: name.to_owned(),
            display_name: name.to_owned(),
            order: 0,
            detect_width: 704,
            detect_height: 576,
            enabled,
        }
    }

    #[test]
    fn classify_calls_a_switched_off_camera_off_by_choice_even_with_frames() {
        let mut fps = HashMap::new();
        fps.insert("living".to_owned(), 5.0);
        assert_eq!(
            classify(&camera_entry("living", false), &fps),
            CameraState::OffByChoice
        );
    }

    #[test]
    fn classify_calls_an_enabled_camera_with_frames_live() {
        let mut fps = HashMap::new();
        fps.insert("front".to_owned(), 5.1);
        assert_eq!(
            classify(&camera_entry("front", true), &fps),
            CameraState::Live
        );
    }

    #[test]
    fn classify_calls_an_enabled_camera_absent_from_the_stats_no_picture() {
        assert_eq!(
            classify(&camera_entry("back yard", true), &HashMap::new()),
            CameraState::NoPicture
        );
    }

    #[test]
    fn classify_calls_an_enabled_camera_at_a_zero_frame_rate_no_picture() {
        let mut fps = HashMap::new();
        fps.insert("back".to_owned(), 0.0);
        assert_eq!(
            classify(&camera_entry("back", true), &fps),
            CameraState::NoPicture
        );
    }

    #[test]
    fn parse_stats_accepts_a_cameras_object_of_numeric_frame_rates() {
        let body =
            br#"{"cameras":{"front":{"camera_fps":5.1},"back":{"camera_fps":0}},"service":{"uptime":100}}"#;
        let fps = parse_stats(body).expect("accepted");
        assert_eq!(fps.get("front"), Some(&5.1));
        assert_eq!(fps.get("back"), Some(&0.0));
        assert_eq!(fps.len(), 2);
    }

    #[test]
    fn parse_stats_refuses_bodies_without_usable_camera_frame_rates() {
        for body in [
            b"not json at all".as_slice(),
            b"[1,2,3]".as_slice(),
            b"null".as_slice(),
            br#"{"service":{"uptime":100}}"#.as_slice(),
            br#"{"cameras":"none"}"#.as_slice(),
            br#"{"cameras":{"front":{}}}"#.as_slice(),
            br#"{"cameras":{"front":{"camera_fps":"5.1"}}}"#.as_slice(),
            br#"{"cameras":{"front":{"camera_fps":null}}}"#.as_slice(),
        ] {
            assert!(parse_stats(body).is_err(), "body {body:?} must be refused");
        }
    }

    #[test]
    fn the_table_keeps_a_holding_state_and_restarts_a_changed_one() {
        let table = StatusState::default();
        let first = table.observe("front", CameraState::Live, 100.0);
        assert_eq!(Value::from(first), json!(100.0));
        let holding = table.observe("front", CameraState::Live, 130.0);
        assert_eq!(Value::from(holding), json!(100.0));
        let changed = table.observe("front", CameraState::NoPicture, 160.0);
        assert_eq!(Value::from(changed), json!(160.0));
        let holds_new = table.observe("front", CameraState::NoPicture, 200.0);
        assert_eq!(Value::from(holds_new), json!(160.0));
    }

    #[test]
    fn the_table_tracks_each_camera_independently() {
        let table = StatusState::default();
        table.observe("front", CameraState::Live, 100.0);
        let back_first = table.observe("back", CameraState::NoPicture, 150.0);
        assert_eq!(Value::from(back_first), json!(150.0));
        let back_again = table.observe("back", CameraState::NoPicture, 200.0);
        assert_eq!(Value::from(back_again), json!(150.0));
        let front_still = table.observe("front", CameraState::Live, 200.0);
        assert_eq!(Value::from(front_still), json!(100.0));
    }

    #[test]
    fn still_failing_is_false_without_a_record() {
        let state = StatusState::default();
        assert!(!state.still_failing("front", 1000.0));
    }

    #[test]
    fn a_run_of_failures_turns_stale_just_after_its_first_minute() {
        let state = StatusState::default();
        state.still_failed("front", 1000.0);
        state.still_failed("front", 1060.0);
        // now 1060.0 is exactly 60 s after the run began, not over it.
        assert!(!state.still_failing("front", 1060.0));
        assert!(state.still_failing("front", 1060.1));
    }

    #[test]
    fn a_delivered_picture_clears_the_record() {
        let state = StatusState::default();
        state.still_failed("front", 1000.0);
        state.still_failed("front", 1070.0);
        assert!(state.still_failing("front", 1070.0));
        state.still_delivered("front");
        assert!(!state.still_failing("front", 1080.0));
    }

    #[test]
    fn a_lone_failure_after_a_quiet_time_starts_a_new_run() {
        let state = StatusState::default();
        state.still_failed("front", 1000.0);
        state.still_delivered("front");
        state.still_failed("front", 5000.0);
        assert!(!state.still_failing("front", 5000.0));
    }

    #[test]
    fn evidence_older_than_a_minute_stops_counting() {
        let state = StatusState::default();
        state.still_failed("front", 1000.0);
        state.still_failed("front", 1070.0);
        // At now 1130.0 the last failure is exactly 60 s old and still counts.
        assert!(state.still_failing("front", 1130.0));
        assert!(!state.still_failing("front", 1131.0));
    }

    #[test]
    fn the_records_track_each_camera_independently() {
        let state = StatusState::default();
        state.still_failed("front", 1000.0);
        state.still_failed("front", 1070.0);
        state.still_failed("back", 1000.0);
        state.still_failed("back", 1070.0);
        assert!(state.still_failing("front", 1070.0));
        state.still_delivered("back");
        assert!(!state.still_failing("back", 1080.0));
        assert!(state.still_failing("front", 1080.0));
    }

    #[test]
    fn status_body_turns_live_into_stale_and_leaves_the_other_states() {
        let status_state = StatusState::default();
        let entries = [
            camera_entry("front", true),
            camera_entry("back", true),
            camera_entry("living", false),
        ];
        let mut fps = HashMap::new();
        fps.insert("front".to_owned(), 5.1);
        fps.insert("back".to_owned(), 0.0);
        fps.insert("living".to_owned(), 5.0);
        for camera in ["front", "back", "living"] {
            status_state.still_failed(camera, 1000.0);
            status_state.still_failed(camera, 1070.0);
        }
        let answer: Value =
            serde_json::from_slice(&status_body(&entries, &fps, &status_state, 1070.0))
                .expect("status body is JSON");
        assert_eq!(
            answer["cameras"][0],
            json!({"name": "front", "state": "stale", "since": 1070.0})
        );
        assert_eq!(answer["cameras"][1]["state"], "no_picture");
        assert_eq!(answer["cameras"][2]["state"], "off_by_choice");
    }
}
