//! Shared wire contracts used by the browser and the future Corvette service.

use serde::Deserialize;
use std::collections::BTreeMap;

/// The subset of Frigate's configuration response needed for camera discovery.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct FrigateConfig {
    cameras: BTreeMap<String, FrigateCamera>,
}

impl FrigateConfig {
    /// Returns enabled cameras in their configured dashboard order.
    #[must_use]
    pub fn enabled_cameras(self) -> Vec<Camera> {
        let mut cameras = self
            .cameras
            .into_iter()
            .filter(|(_, camera)| camera.enabled)
            .map(|(name, camera)| Camera {
                display_name: camera.friendly_name.unwrap_or_else(|| name.clone()),
                name,
                order: camera.ui.order,
            })
            .collect::<Vec<_>>();
        cameras.sort_by(|left, right| {
            left.order
                .cmp(&right.order)
                .then_with(|| left.name.cmp(&right.name))
        });
        cameras
    }
}

/// A camera available to the Corvette interface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Camera {
    /// Stable camera name used in API paths.
    pub name: String,
    /// Human-readable label supplied by Frigate.
    pub display_name: String,
    order: i32,
}

/// An event returned by Frigate's event history endpoint.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Event {
    /// Stable event identifier used in media API paths.
    pub id: String,
    /// Camera that observed the event.
    pub camera: String,
    /// Detected object or audio label.
    pub label: String,
    /// More specific label assigned by recognition or the user.
    pub sub_label: Option<String>,
    /// Event start as Unix seconds.
    pub start_time: f64,
    /// Event end as Unix seconds, or `None` while tracking continues.
    pub end_time: Option<f64>,
    /// Zones entered during the event.
    pub zones: Vec<String>,
    /// Whether Frigate can produce a recording clip for this event.
    pub has_clip: bool,
}

/// A review segment used to summarize activity on the recording calendar.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ReviewSegment {
    /// Segment start as Unix seconds.
    pub start_time: f64,
    /// Segment end as Unix seconds, or `None` while activity continues.
    pub end_time: Option<f64>,
    /// Highest activity classification reached by the segment.
    pub severity: ReviewSeverity,
}

/// A retained media segment returned by Frigate's recordings endpoint.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct RecordingSegment {
    /// Segment start as Unix seconds.
    pub start_time: f64,
    /// Segment end as Unix seconds.
    pub end_time: f64,
    /// Amount of motion Frigate measured in the segment.
    #[serde(default)]
    pub motion: Option<f64>,
}

/// Frigate's ordered review activity classifications.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    /// Recorded motion that did not become a detection.
    SignificantMotion,
    /// A configured object detection that did not become an alert.
    Detection,
    /// An alert-worthy detection.
    Alert,
}

impl ReviewSeverity {
    /// Returns the more important of two activity classifications.
    #[must_use]
    pub const fn highest(self, other: Self) -> Self {
        if self.priority() >= other.priority() {
            self
        } else {
            other
        }
    }

    const fn priority(self) -> u8 {
        match self {
            Self::SignificantMotion => 0,
            Self::Detection => 1,
            Self::Alert => 2,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct FrigateCamera {
    enabled: bool,
    friendly_name: Option<String>,
    ui: FrigateCameraUi,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct FrigateCameraUi {
    order: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_cameras_are_ordered_and_ignore_unneeded_config() {
        let config: FrigateConfig = serde_json::from_str(
            r#"{
                "version": "0.17",
                "cameras": {
                    "side_door": {
                        "enabled": true,
                        "friendly_name": "Side door",
                        "ui": {"order": 20, "dashboard": true},
                        "ffmpeg": {"inputs": []}
                    },
                    "garage": {
                        "enabled": false,
                        "friendly_name": "Garage",
                        "ui": {"order": 0, "dashboard": true}
                    },
                    "front_door": {
                        "enabled": true,
                        "friendly_name": null,
                        "ui": {"order": 10, "dashboard": true}
                    }
                }
            }"#,
        )
        .expect("Frigate config should deserialize");

        assert_eq!(
            config.enabled_cameras(),
            vec![
                Camera {
                    name: "front_door".to_owned(),
                    display_name: "front_door".to_owned(),
                    order: 10,
                },
                Camera {
                    name: "side_door".to_owned(),
                    display_name: "Side door".to_owned(),
                    order: 20,
                },
            ]
        );
    }

    #[test]
    fn cameras_with_equal_order_are_sorted_by_stable_name() {
        let config: FrigateConfig = serde_json::from_str(
            r#"{
                "cameras": {
                    "west": {"enabled": true, "friendly_name": "West", "ui": {"order": 1}},
                    "east": {"enabled": true, "friendly_name": "East", "ui": {"order": 1}}
                }
            }"#,
        )
        .expect("Frigate config should deserialize");

        let names = config
            .enabled_cameras()
            .into_iter()
            .map(|camera| camera.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["east", "west"]);
    }

    #[test]
    fn event_deserializes_from_the_larger_frigate_response() {
        let event: Event = serde_json::from_str(
            r#"{
                "id": "1722690000.25-abc123",
                "camera": "front_door",
                "label": "person",
                "sub_label": null,
                "start_time": 1722690000.25,
                "end_time": 1722690012.75,
                "zones": ["porch"],
                "has_clip": true,
                "has_snapshot": true,
                "data": {"score": 0.83}
            }"#,
        )
        .expect("Frigate event should deserialize");

        assert_eq!(event.id, "1722690000.25-abc123");
        assert_eq!(event.camera, "front_door");
        assert_eq!(event.label, "person");
        assert_eq!(event.sub_label, None);
        assert!((event.start_time - 1_722_690_000.25).abs() < f64::EPSILON);
        assert!(
            (event
                .end_time
                .expect("finished event should have an end time")
                - 1_722_690_012.75)
                .abs()
                < f64::EPSILON
        );
        assert_eq!(event.zones, ["porch"]);
        assert!(event.has_clip);
    }

    #[test]
    fn review_severity_deserializes_and_orders_by_importance() {
        let review: ReviewSegment = serde_json::from_str(
            r#"{
                "id": "review-id",
                "camera": "front",
                "start_time": 1722690000.0,
                "end_time": null,
                "severity": "significant_motion",
                "data": "{}"
            }"#,
        )
        .expect("Frigate review segment should deserialize");

        assert_eq!(review.severity, ReviewSeverity::SignificantMotion);
        assert_eq!(
            review.severity.highest(ReviewSeverity::Detection),
            ReviewSeverity::Detection
        );
        assert_eq!(
            ReviewSeverity::Detection.highest(ReviewSeverity::Alert),
            ReviewSeverity::Alert
        );
    }

    #[test]
    fn recording_segment_ignores_unneeded_metadata() {
        let segment: RecordingSegment = serde_json::from_str(
            r#"{
                "id": "recording-id",
                "start_time": 1722690000.0,
                "end_time": 1722690010.0,
                "duration": 10.0,
                "segment_size": 123456,
                "motion": 4,
                "objects": 1
            }"#,
        )
        .expect("Frigate recording segment should deserialize");

        assert!((segment.start_time - 1_722_690_000.0).abs() < f64::EPSILON);
        assert!((segment.end_time - 1_722_690_010.0).abs() < f64::EPSILON);
        assert!(
            (segment.motion.expect("segment should include motion") - 4.0).abs() < f64::EPSILON
        );
    }
}
