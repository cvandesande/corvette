//! The object stage of the replay: drives the tracker and the zone set for one frame and returns the live objects and the ids that retired, the pair a record frame line holds.

use std::collections::HashMap;
use std::fmt;

use corvette_motion::Region;
use corvette_track::object_tracker::ObjectView;
use corvette_track::{
    FrameDetection, IdSource, NanDistance, ObjectTracker, Scores, TrackerSettings, ZoneSet,
};

#[derive(Clone, Debug, PartialEq)]
pub struct TrackedObject {
    pub id: String,
    pub label: String,
    pub score: f64,
    pub confirmed: bool,
    pub bounds: [f64; 4],
    /// The tracker's box in frame pixels, `[x1, y1, x2, y2]`.
    pub edges: [i32; 4],
    pub first_seen: f64,
    pub last_seen: f64,
    pub zones_now: Vec<String>,
    pub zones_entered: Vec<String>,
    pub stationary: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameObjects {
    pub frame_time: f64,
    pub objects: Vec<TrackedObject>,
    pub ended: Vec<String>,
    pub motion_triggered: bool,
}

impl FrameObjects {
    #[must_use]
    pub fn regions(&self) -> Vec<Region> {
        self.objects
            .iter()
            .map(|object| {
                let [x1, y1, x2, y2] = object.edges;
                Region { x1, y1, x2, y2 }
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectStageError {
    NanDistance(NanDistance),
}

impl fmt::Display for ObjectStageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NanDistance(error) => write!(f, "object stage tracker: {error}"),
        }
    }
}

impl std::error::Error for ObjectStageError {}

#[derive(Debug)]
pub struct ObjectStage<'a> {
    tracker: ObjectTracker<'a>,
    zones: ZoneSet,
    settings: TrackerSettings,
    confirm_threshold: f64,
    scores: HashMap<u64, Scores>,
    previous_ids: Vec<u64>,
}

impl<'a> ObjectStage<'a> {
    pub fn new(
        settings: TrackerSettings,
        ids: &'a mut IdSource,
        zones: ZoneSet,
        confirm_threshold: f64,
    ) -> Option<Self> {
        let tracker = ObjectTracker::new(settings.clone(), ids)?;
        Some(Self {
            tracker,
            zones,
            settings,
            confirm_threshold,
            scores: HashMap::new(),
            previous_ids: Vec::new(),
        })
    }

    /// Steps one frame: live objects in tracker order, kept-while-unseen included ("one entry per live tracked object"); `score` the median of its last ten scores, `confirmed` its sticky flag, an unseen frame counting 0.0.
    /// # Errors
    pub fn step(
        &mut self,
        frame_time: f64,
        detections: Vec<FrameDetection>,
    ) -> Result<FrameObjects, ObjectStageError> {
        self.tracker
            .match_and_update(frame_time, detections)
            .map_err(ObjectStageError::NanDistance)?;
        let views: Vec<ObjectView<'_>> = self.tracker.objects().collect();
        let width = f64::from(self.settings.width);
        let height = f64::from(self.settings.height);
        let mut objects: Vec<TrackedObject> = Vec::with_capacity(views.len());
        let mut current_ids: Vec<u64> = Vec::with_capacity(views.len());
        for view in &views {
            let zones_state =
                self.zones
                    .observe(view.id, view.label, view.centroid[0], view.edges[3]);
            let seen = view.disappeared == 0;
            let scores = self
                .scores
                .entry(view.id)
                .or_insert_with(|| Scores::new(&[]));
            scores.update(seen.then_some(view.score), self.confirm_threshold);
            let [x1, y1, x2, y2] = view.edges;
            objects.push(TrackedObject {
                id: view.id.to_string(),
                label: view.label.to_owned(),
                score: scores.computed(),
                confirmed: scores.confirmed(),
                bounds: [
                    f64::from(x1) / width,
                    f64::from(y1) / height,
                    f64::from(x2) / width,
                    f64::from(y2) / height,
                ],
                edges: view.edges,
                first_seen: view.start_time,
                last_seen: view.frame_time,
                zones_now: zones_state.current,
                zones_entered: zones_state.entered,
                stationary: view.motionless >= self.settings.stationary_threshold,
            });
            current_ids.push(view.id);
        }
        let mut ended: Vec<String> = Vec::new();
        for id in &self.previous_ids {
            if !current_ids.contains(id) {
                ended.push(id.to_string());
                self.zones.forget(*id);
                self.scores.remove(id);
            }
        }
        self.previous_ids = current_ids;
        Ok(FrameObjects {
            frame_time,
            objects,
            ended,
            motion_triggered: false,
        })
    }
}
