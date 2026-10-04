//! Shared wire contracts used by the browser and the future Corvette service.

use serde::{Deserialize, Serialize};
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
        Self::enabled_in_dashboard_order(self.cameras)
            .into_iter()
            .map(|(name, camera)| Camera {
                display_name: camera.friendly_name.unwrap_or_else(|| name.clone()),
                name,
                order: camera.ui.order,
                width: camera.detect.width,
                height: camera.detect.height,
            })
            .collect()
    }

    /// Returns the same cameras and order as [`FrigateConfig::enabled_cameras`],
    /// as the wire entries served by Corvette's own camera list (issue #4).
    #[must_use]
    pub fn enabled_camera_entries(self) -> Vec<CameraEntry> {
        Self::enabled_in_dashboard_order(self.cameras)
            .into_iter()
            .map(|(name, camera)| CameraEntry {
                display_name: camera.friendly_name.unwrap_or_else(|| name.clone()),
                name,
                order: camera.ui.order,
                detect_width: camera.detect.width,
                detect_height: camera.detect.height,
            })
            .collect()
    }

    /// Enabled cameras sorted into dashboard order: `ui.order`, then name.
    fn enabled_in_dashboard_order(
        cameras: BTreeMap<String, FrigateCamera>,
    ) -> Vec<(String, FrigateCamera)> {
        let mut cameras = cameras
            .into_iter()
            .filter(|(_, camera)| camera.enabled)
            .collect::<Vec<_>>();
        cameras.sort_by(|(left_name, left), (right_name, right)| {
            left.ui
                .order
                .cmp(&right.ui.order)
                .then_with(|| left_name.cmp(right_name))
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
    /// Width in pixels of the stream Frigate runs detection on, unscaled.
    pub width: u32,
    /// Height in pixels of the stream Frigate runs detection on, unscaled.
    pub height: u32,
    order: i32,
}

/// A camera as returned by Corvette's own camera list API (issue #4).
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct CameraEntry {
    /// Stable camera name used in API paths.
    pub name: String,
    /// Human-readable label supplied by Frigate.
    pub display_name: String,
    /// Configured dashboard order; lower values sort first.
    pub order: i32,
    /// Width in pixels of the stream Frigate runs detection on, unscaled.
    pub detect_width: u32,
    /// Height in pixels of the stream Frigate runs detection on, unscaled.
    pub detect_height: u32,
}

/// Error payload returned by Corvette's own API routes (issue #4).
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ErrorBody {
    /// Stable machine code for callers to branch on, such as `unauthorized`.
    pub code: String,
    /// Human-readable description of the failure.
    pub message: String,
}

/// Returns the camera names `role` may access in `all_cameras` order, following
/// Frigate's `User.get_allowed_cameras` rule (issue #4).
#[must_use]
pub fn allowed_cameras<'a>(
    role: &str,
    roles: &BTreeMap<String, Vec<String>>,
    all_cameras: &'a [String],
) -> Vec<&'a str> {
    let Some(allowed) = roles.get(role) else {
        return Vec::new();
    };
    if allowed.is_empty() {
        return all_cameras.iter().map(String::as_str).collect();
    }
    all_cameras
        .iter()
        .filter(|name| allowed.iter().any(|listed| listed == *name))
        .map(String::as_str)
        .collect()
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
    /// Camera that observed the segment.
    pub camera: String,
    /// Segment start as Unix seconds.
    pub start_time: f64,
    /// Segment end as Unix seconds, or `None` while activity continues.
    pub end_time: Option<f64>,
    /// Highest activity classification reached by the segment.
    pub severity: ReviewSeverity,
}

/// A review entry displayed in the recent activity feed.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ReviewEvent {
    /// Stable review identifier.
    pub id: String,
    /// Camera that observed the activity.
    pub camera: String,
    /// Activity start as Unix seconds.
    pub start_time: f64,
    /// Activity end as Unix seconds, or `None` while activity continues.
    pub end_time: Option<f64>,
    /// Highest activity classification reached by the review.
    pub severity: ReviewSeverity,
    /// Frigate media path for the review thumbnail.
    pub thumb_path: String,
    /// Labels and zones collected during the review.
    pub data: ReviewEventData,
}

/// Labels and zones attached to a Frigate review entry.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct ReviewEventData {
    /// Detected audio labels.
    #[serde(default)]
    pub audio: Vec<String>,
    /// Identifiers of the tracked-object events this review was built from.
    ///
    /// Empty for a review with no tracked object, such as an audio-only one.
    #[serde(default)]
    pub detections: Vec<String>,
    /// Detected object labels.
    #[serde(default)]
    pub objects: Vec<String>,
    /// Recognition labels assigned to detected objects.
    #[serde(default)]
    pub sub_labels: Vec<String>,
    /// Zones entered during the review.
    #[serde(default)]
    pub zones: Vec<String>,
}

/// A time bucket returned by Frigate's motion-activity endpoint.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct MotionActivity {
    /// Bucket start as Unix seconds.
    pub start_time: f64,
    /// Highest motion percentage recorded within the bucket.
    pub motion: f64,
    /// Comma-separated cameras that recorded motion within the bucket.
    pub camera: String,
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

/// A low-resolution video generated by Frigate for timeline scrubbing.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PreviewClip {
    /// Camera that produced the preview.
    pub camera: String,
    /// Authenticated media path for the preview video.
    pub src: String,
    /// Preview media MIME type.
    #[serde(rename = "type")]
    pub media_type: String,
    /// Preview start as Unix seconds.
    pub start: f64,
    /// Preview end as Unix seconds.
    pub end: f64,
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
    detect: FrigateDetect,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct FrigateCameraUi {
    order: i32,
}

/// The subset of Frigate's `detect` sub-config needed for tile aspect ratio.
///
/// Frigate resolves both fields from the camera's stream before serving
/// `/api/config` (`frigate/config/config.py`), so a running deployment's
/// response always carries concrete values even though Frigate's own model
/// allows either to be unset in raw user configuration.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
struct FrigateDetect {
    width: u32,
    height: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One enabled camera with a label, one enabled without, one disabled.
    const MULTI_CAMERA_CONFIG: &str = r#"{
        "version": "0.17",
        "cameras": {
            "side_door": {
                "enabled": true,
                "friendly_name": "Side door",
                "ui": {"order": 20, "dashboard": true},
                "detect": {"width": 1920, "height": 1080, "fps": 5},
                "ffmpeg": {"inputs": []}
            },
            "garage": {
                "enabled": false,
                "friendly_name": "Garage",
                "ui": {"order": 0, "dashboard": true},
                "detect": {"width": 1280, "height": 720, "fps": 5}
            },
            "front_door": {
                "enabled": true,
                "friendly_name": null,
                "ui": {"order": 10, "dashboard": true},
                "detect": {"width": 640, "height": 480, "fps": 5}
            }
        }
    }"#;

    #[test]
    fn enabled_cameras_are_ordered_and_ignore_unneeded_config() {
        let config: FrigateConfig =
            serde_json::from_str(MULTI_CAMERA_CONFIG).expect("Frigate config should deserialize");

        assert_eq!(
            config.enabled_cameras(),
            vec![
                Camera {
                    name: "front_door".to_owned(),
                    display_name: "front_door".to_owned(),
                    width: 640,
                    height: 480,
                    order: 10,
                },
                Camera {
                    name: "side_door".to_owned(),
                    display_name: "Side door".to_owned(),
                    width: 1920,
                    height: 1080,
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
                    "west": {
                        "enabled": true,
                        "friendly_name": "West",
                        "ui": {"order": 1},
                        "detect": {"width": 1920, "height": 1080}
                    },
                    "east": {
                        "enabled": true,
                        "friendly_name": "East",
                        "ui": {"order": 1},
                        "detect": {"width": 1920, "height": 1080}
                    }
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
    fn camera_entries_match_enabled_camera_names_and_order() {
        let entries = serde_json::from_str::<FrigateConfig>(MULTI_CAMERA_CONFIG)
            .expect("Frigate config should deserialize")
            .enabled_camera_entries();
        let cameras = serde_json::from_str::<FrigateConfig>(MULTI_CAMERA_CONFIG)
            .expect("Frigate config should deserialize")
            .enabled_cameras();

        assert_eq!(
            entries.iter().map(|entry| &entry.name).collect::<Vec<_>>(),
            cameras
                .iter()
                .map(|camera| &camera.name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn camera_entries_fall_back_to_camera_name_and_omit_disabled_cameras() {
        let entries = serde_json::from_str::<FrigateConfig>(MULTI_CAMERA_CONFIG)
            .expect("Frigate config should deserialize")
            .enabled_camera_entries();

        assert_eq!(
            entries,
            vec![
                CameraEntry {
                    name: "front_door".to_owned(),
                    display_name: "front_door".to_owned(),
                    order: 10,
                    detect_width: 640,
                    detect_height: 480,
                },
                CameraEntry {
                    name: "side_door".to_owned(),
                    display_name: "Side door".to_owned(),
                    order: 20,
                    detect_width: 1920,
                    detect_height: 1080,
                },
            ]
        );
    }

    #[test]
    fn camera_entry_json_round_trips_with_the_rust_field_names() {
        let entry = CameraEntry {
            name: "front_door".to_owned(),
            display_name: "Front door".to_owned(),
            order: 10,
            detect_width: 640,
            detect_height: 480,
        };

        let json = serde_json::to_string(&entry).expect("camera entry should serialize");
        assert_eq!(
            json,
            r#"{"name":"front_door","display_name":"Front door","order":10,"detect_width":640,"detect_height":480}"#
        );
        assert_eq!(
            serde_json::from_str::<CameraEntry>(&json).expect("camera entry should deserialize"),
            entry
        );
    }

    #[test]
    fn error_body_json_round_trips_with_the_rust_field_names() {
        let body = ErrorBody {
            code: "unauthorized".to_owned(),
            message: "The role may not see this camera.".to_owned(),
        };

        let json = serde_json::to_string(&body).expect("error body should serialize");
        assert_eq!(
            json,
            r#"{"code":"unauthorized","message":"The role may not see this camera."}"#
        );
        assert_eq!(
            serde_json::from_str::<ErrorBody>(&json).expect("error body should deserialize"),
            body
        );
    }

    #[test]
    fn allowed_cameras_grants_nothing_to_a_role_absent_from_roles() {
        let roles = BTreeMap::from([("viewer".to_owned(), vec!["front_door".to_owned()])]);
        let all = vec!["front_door".to_owned(), "side_door".to_owned()];

        assert!(allowed_cameras("admin", &roles, &all).is_empty());
    }

    #[test]
    fn allowed_cameras_grants_every_camera_for_an_empty_role_list() {
        let roles = BTreeMap::from([("admin".to_owned(), vec![])]);
        let all = vec![
            "front_door".to_owned(),
            "side_door".to_owned(),
            "back_yard".to_owned(),
        ];

        assert_eq!(
            allowed_cameras("admin", &roles, &all),
            ["front_door", "side_door", "back_yard"]
        );
    }

    #[test]
    fn allowed_cameras_returns_listed_cameras_in_all_cameras_order() {
        let roles = BTreeMap::from([(
            "viewer".to_owned(),
            vec!["side_door".to_owned(), "front_door".to_owned()],
        )]);
        let all = vec![
            "front_door".to_owned(),
            "back_yard".to_owned(),
            "side_door".to_owned(),
        ];

        assert_eq!(
            allowed_cameras("viewer", &roles, &all),
            ["front_door", "side_door"]
        );
    }

    #[test]
    fn allowed_cameras_ignores_listed_cameras_that_do_not_exist() {
        let roles = BTreeMap::from([(
            "viewer".to_owned(),
            vec!["front_door".to_owned(), "cellar".to_owned()],
        )]);
        let all = vec!["front_door".to_owned(), "back_yard".to_owned()];

        assert_eq!(allowed_cameras("viewer", &roles, &all), ["front_door"]);
    }

    #[test]
    fn allowed_cameras_lists_a_camera_once_despite_duplicate_role_entries() {
        let roles = BTreeMap::from([(
            "viewer".to_owned(),
            vec!["front_door".to_owned(), "front_door".to_owned()],
        )]);
        let all = vec!["front_door".to_owned(), "back_yard".to_owned()];

        assert_eq!(allowed_cameras("viewer", &roles, &all), ["front_door"]);
    }

    #[test]
    fn allowed_cameras_is_empty_without_any_cameras_in_every_branch() {
        let roles = BTreeMap::from([
            ("admin".to_owned(), vec![]),
            ("viewer".to_owned(), vec!["front_door".to_owned()]),
        ]);

        assert!(allowed_cameras("admin", &roles, &[]).is_empty());
        assert!(allowed_cameras("viewer", &roles, &[]).is_empty());
        assert!(allowed_cameras("ghost", &roles, &[]).is_empty());
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

        assert_eq!(review.camera, "front");
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

    #[test]
    fn motion_activity_deserializes_frigate_time_bucket() {
        let activity: MotionActivity = serde_json::from_str(
            r#"{
                "start_time": 1785770490,
                "motion": 100.0,
                "camera": "back,front"
            }"#,
        )
        .expect("Frigate motion activity should deserialize");

        assert!((activity.start_time - 1_785_770_490.0).abs() < f64::EPSILON);
        assert!((activity.motion - 100.0).abs() < f64::EPSILON);
        assert_eq!(activity.camera, "back,front");
    }

    #[test]
    fn preview_clip_deserializes_frigate_media_contract() {
        let preview: PreviewClip = serde_json::from_str(
            r#"{
                "camera": "front",
                "src": "/clips/previews/front/1785764400.0.mp4",
                "type": "video/mp4",
                "start": 1785764400.0,
                "end": 1785768000.0
            }"#,
        )
        .expect("Frigate preview clip should deserialize");

        assert_eq!(preview.camera, "front");
        assert_eq!(preview.media_type, "video/mp4");
        assert_eq!(preview.src, "/clips/previews/front/1785764400.0.mp4");
    }
}
