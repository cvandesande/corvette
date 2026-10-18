//! Shared wire contracts used by the browser and the future Corvette service.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The subset of Frigate's configuration response needed for camera discovery.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct FrigateConfig {
    cameras: BTreeMap<String, FrigateCamera>,
    /// Absent `auth` or `roles` deserializes to an empty map, so every role
    /// sees no cameras: an unreadable authorization section fails closed.
    #[serde(default)]
    auth: FrigateAuth,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
struct FrigateAuth {
    /// An empty camera list for a role means every camera.
    #[serde(default)]
    roles: BTreeMap<String, Vec<String>>,
}

impl FrigateConfig {
    /// Enabled cameras in their configured dashboard order.
    #[must_use]
    pub fn enabled_camera_entries(self) -> Vec<CameraEntry> {
        Self::camera_entries_in_dashboard_order(self.cameras)
    }

    /// Returns the enabled cameras `role` may see, in [`FrigateConfig::enabled_camera_entries`] order.
    ///
    /// A role absent from `auth.roles` sees nothing, as does the empty role of
    /// a request with no `Remote-Role`; a disabled camera is never returned, even when listed.
    #[must_use]
    pub fn camera_entries_for_role(self, role: &str) -> Vec<CameraEntry> {
        let Self { cameras, auth } = self;
        let entries = Self::camera_entries_in_dashboard_order(cameras);
        let enabled_names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
        let allowed = allowed_cameras(role, &auth.roles, &enabled_names);
        entries
            .into_iter()
            .filter(|entry| allowed.contains(&entry.name.as_str()))
            .collect()
    }

    fn camera_entries_in_dashboard_order(
        cameras: BTreeMap<String, FrigateCamera>,
    ) -> Vec<CameraEntry> {
        Self::enabled_in_dashboard_order(cameras)
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
    pub name: String,
    pub display_name: String,
    pub width: u32,
    pub height: u32,
    order: i32,
}

/// A camera as returned by Corvette's own camera list API.
///
/// Detection sizes are Frigate's own values, verbatim and unscaled.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct CameraEntry {
    /// Stable camera name used in API paths.
    pub name: String,
    pub display_name: String,
    pub order: i32,
    pub detect_width: u32,
    pub detect_height: u32,
}

impl From<CameraEntry> for Camera {
    fn from(entry: CameraEntry) -> Self {
        Self {
            name: entry.name,
            display_name: entry.display_name,
            width: entry.detect_width,
            height: entry.detect_height,
            order: entry.order,
        }
    }
}

/// The response body of Corvette's camera list route.
///
/// An object rather than a bare array so later fields can be added without a breaking change.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct CameraList {
    /// The cameras the caller's role may see, in dashboard order.
    pub cameras: Vec<CameraEntry>,
}

/// The response body of Corvette's health route; both probe results are `ok` in a `200` answer.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct Health {
    pub database: String,
    pub config: String,
}

/// Error payload returned by Corvette's own API routes.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ErrorBody {
    /// Stable machine code for callers to branch on, such as `unauthorized`.
    pub code: String,
    pub message: String,
}

/// Returns the camera names `role` may access in `all_cameras` order: an absent
/// role gets none, an empty role list every camera.
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

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub camera: String,
    pub label: String,
    pub sub_label: Option<String>,
    pub start_time: f64,
    pub end_time: Option<f64>,
    pub zones: Vec<String>,
    pub has_clip: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ReviewSegment {
    pub camera: String,
    pub start_time: f64,
    pub end_time: Option<f64>,
    pub severity: ReviewSeverity,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ReviewEvent {
    pub id: String,
    pub camera: String,
    pub start_time: f64,
    pub end_time: Option<f64>,
    pub severity: ReviewSeverity,
    pub thumb_path: String,
    pub data: ReviewEventData,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct ReviewEventData {
    #[serde(default)]
    pub audio: Vec<String>,
    #[serde(default)]
    pub detections: Vec<String>,
    #[serde(default)]
    pub objects: Vec<String>,
    #[serde(default)]
    pub sub_labels: Vec<String>,
    #[serde(default)]
    pub zones: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct MotionActivity {
    pub start_time: f64,
    pub motion: f64,
    pub camera: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct RecordingSegment {
    pub start_time: f64,
    pub end_time: f64,
    #[serde(default)]
    pub motion: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PreviewClip {
    pub camera: String,
    pub src: String,
    #[serde(rename = "type")]
    pub media_type: String,
    pub start: f64,
    pub end: f64,
}

/// Frigate's review activity classifications, in ascending order of importance.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    SignificantMotion,
    Detection,
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

/// The body of `GET /corvette/api/v1/activity`: every field is always written.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ActivityResponse {
    pub after: f64,
    pub before: f64,
    pub motion_bucket_seconds: u32,
    pub reviews: Vec<ActivityReview>,
    pub motion: Vec<MotionBucket>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ActivityReview {
    pub id: String,
    pub camera: String,
    pub severity: ReviewSeverity,
    pub start: f64,
    pub end: Option<f64>,
    pub objects: Vec<String>,
    pub zones: Vec<String>,
    pub detections: Vec<String>,
    pub sub_labels: Vec<String>,
    pub audio: Vec<String>,
    pub media: ReviewMedia,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ReviewMedia {
    pub thumbnail: Option<MediaLink>,
    pub clip: Option<MediaLink>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct MediaLink {
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct MotionBucket {
    /// Bucket start as Unix seconds, a multiple of `motion_bucket_seconds`.
    pub start: i64,
    pub level: i64,
    pub cameras: Vec<String>,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
struct FrigateDetect {
    width: u32,
    height: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

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
            .enabled_camera_entries()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["east", "west"]);
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
    fn camera_from_entry_maps_order_and_detection_size_to_camera_fields() {
        let camera = Camera::from(CameraEntry {
            name: "front_door".to_owned(),
            display_name: "Front door".to_owned(),
            order: 10,
            detect_width: 640,
            detect_height: 480,
        });

        assert_eq!(
            camera,
            Camera {
                name: "front_door".to_owned(),
                display_name: "Front door".to_owned(),
                width: 640,
                height: 480,
                order: 10,
            }
        );
    }

    const ROLE_CAMERA_CONFIG: &str = r#"{
        "auth": {
            "roles": {
                "admin": [],
                "limited": ["front_door", "garage", "cellar"]
            }
        },
        "cameras": {
            "side_door": {
                "enabled": true,
                "friendly_name": "Side door",
                "ui": {"order": 20},
                "detect": {"width": 1920, "height": 1080}
            },
            "garage": {
                "enabled": false,
                "friendly_name": "Garage",
                "ui": {"order": 0},
                "detect": {"width": 1280, "height": 720}
            },
            "front_door": {
                "enabled": true,
                "friendly_name": null,
                "ui": {"order": 10},
                "detect": {"width": 640, "height": 480}
            }
        }
    }"#;

    fn role_entries(config: &str, role: &str) -> Vec<CameraEntry> {
        serde_json::from_str::<FrigateConfig>(config)
            .expect("Frigate config should deserialize")
            .camera_entries_for_role(role)
    }

    #[test]
    fn camera_entries_for_role_grants_nothing_to_a_role_absent_from_roles() {
        assert!(role_entries(ROLE_CAMERA_CONFIG, "auditor").is_empty());
    }

    #[test]
    fn camera_entries_for_role_grants_every_enabled_camera_to_an_empty_list_role() {
        assert_eq!(
            role_entries(ROLE_CAMERA_CONFIG, "admin")
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["front_door", "side_door"],
        );
    }

    #[test]
    fn camera_entries_for_role_keeps_dashboard_order_for_a_listed_role() {
        let config = r#"{
            "auth": { "roles": { "viewer": ["side_door", "east", "west"] } },
            "cameras": {
                "west": {
                    "enabled": true,
                    "friendly_name": null,
                    "ui": {"order": 1},
                    "detect": {"width": 1920, "height": 1080}
                },
                "east": {
                    "enabled": true,
                    "friendly_name": null,
                    "ui": {"order": 1},
                    "detect": {"width": 1920, "height": 1080}
                },
                "side_door": {
                    "enabled": true,
                    "friendly_name": null,
                    "ui": {"order": 10},
                    "detect": {"width": 640, "height": 480}
                }
            }
        }"#;

        assert_eq!(
            role_entries(config, "viewer")
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["east", "west", "side_door"],
        );
    }

    #[test]
    fn camera_entries_for_role_drops_listed_names_that_are_disabled_or_unknown() {
        assert_eq!(
            role_entries(ROLE_CAMERA_CONFIG, "limited")
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["front_door"],
        );
    }

    #[test]
    fn camera_entries_for_role_grants_nothing_when_the_config_has_no_auth() {
        assert!(role_entries(MULTI_CAMERA_CONFIG, "admin").is_empty());
        assert!(role_entries(MULTI_CAMERA_CONFIG, "").is_empty());
    }

    #[test]
    fn camera_entries_for_role_grants_nothing_when_auth_has_no_roles() {
        let entries = serde_json::from_str::<FrigateConfig>(
            r#"{
                "auth": { "cookie_secret": "x", "restored": false },
                "cameras": {
                    "front": {
                        "enabled": true,
                        "friendly_name": null,
                        "ui": {"order": 0},
                        "detect": {"width": 640, "height": 480}
                    }
                }
            }"#,
        )
        .expect("Frigate config should deserialize")
        .camera_entries_for_role("admin");
        assert!(entries.is_empty());
    }

    #[test]
    fn camera_list_json_round_trips_as_the_route_object() {
        let list = CameraList {
            cameras: role_entries(ROLE_CAMERA_CONFIG, "admin"),
        };

        let json = serde_json::to_string(&list).expect("camera list should serialize");
        assert_eq!(
            serde_json::from_str::<CameraList>(&json).expect("camera list should deserialize"),
            list
        );
        assert!(json.starts_with(r#"{"cameras":["#));
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
    fn health_json_round_trips_with_the_rust_field_names() {
        let health = Health {
            database: "ok".to_owned(),
            config: "ok".to_owned(),
        };

        let json = serde_json::to_string(&health).expect("health body should serialize");
        assert_eq!(json, r#"{"database":"ok","config":"ok"}"#);
        assert_eq!(
            serde_json::from_str::<Health>(&json).expect("health body should deserialize"),
            health
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

    /// Whole-number `f64` fields are written with `.0`.
    const ACTIVITY_RESPONSE_JSON: &str = r#"{
        "after": 1760227200.0, "before": 1760313600.0, "motion_bucket_seconds": 30,
        "reviews": [
          {
            "id": "1760230000.5-abc123", "camera": "front", "severity": "alert",
            "start": 1760230000.5, "end": 1760230042.0,
            "objects": ["person"], "zones": ["driveway"], "detections": ["1760230001.2-def456"],
            "sub_labels": ["alice"], "audio": [],
            "media": {
              "thumbnail": {"url": "/clips/review/thumb-front-1760230000.5-abc123.webp"},
              "clip": {"url": "/api/front/start/1760230000.5/end/1760230042.0/clip.mp4"}
            }
          },
          {
            "id": "1760231000.0-xyz789", "camera": "back", "severity": "significant_motion",
            "start": 1760231000.0, "end": null,
            "objects": [], "zones": [], "detections": [], "sub_labels": [], "audio": ["bark"],
            "media": {"thumbnail": null, "clip": null}
          }
        ],
        "motion": [{"start": 1760230020, "level": 14, "cameras": ["back", "front"]}]
    }"#;

    fn documented_activity_response() -> ActivityResponse {
        ActivityResponse {
            after: 1_760_227_200.0,
            before: 1_760_313_600.0,
            motion_bucket_seconds: 30,
            reviews: vec![
                ActivityReview {
                    id: "1760230000.5-abc123".to_owned(),
                    camera: "front".to_owned(),
                    severity: ReviewSeverity::Alert,
                    start: 1_760_230_000.5,
                    end: Some(1_760_230_042.0),
                    objects: vec!["person".to_owned()],
                    zones: vec!["driveway".to_owned()],
                    detections: vec!["1760230001.2-def456".to_owned()],
                    sub_labels: vec!["alice".to_owned()],
                    audio: vec![],
                    media: ReviewMedia {
                        thumbnail: Some(MediaLink {
                            url: "/clips/review/thumb-front-1760230000.5-abc123.webp".to_owned(),
                        }),
                        clip: Some(MediaLink {
                            url: "/api/front/start/1760230000.5/end/1760230042.0/clip.mp4"
                                .to_owned(),
                        }),
                    },
                },
                ActivityReview {
                    id: "1760231000.0-xyz789".to_owned(),
                    camera: "back".to_owned(),
                    severity: ReviewSeverity::SignificantMotion,
                    start: 1_760_231_000.0,
                    end: None,
                    objects: vec![],
                    zones: vec![],
                    detections: vec![],
                    sub_labels: vec![],
                    audio: vec!["bark".to_owned()],
                    media: ReviewMedia {
                        thumbnail: None,
                        clip: None,
                    },
                },
            ],
            motion: vec![MotionBucket {
                start: 1_760_230_020,
                level: 14,
                cameras: vec!["back".to_owned(), "front".to_owned()],
            }],
        }
    }

    #[test]
    fn activity_response_serializes_to_the_documented_shape() {
        let response = documented_activity_response();

        assert_eq!(
            serde_json::to_value(&response).expect("activity response should serialize"),
            serde_json::from_str::<serde_json::Value>(ACTIVITY_RESPONSE_JSON)
                .expect("documented activity body should parse")
        );
    }

    #[test]
    fn activity_response_round_trips() {
        let response = documented_activity_response();

        assert_eq!(
            serde_json::from_str::<ActivityResponse>(ACTIVITY_RESPONSE_JSON)
                .expect("documented activity body should deserialize"),
            response
        );
    }
}
