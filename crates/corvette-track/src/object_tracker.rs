// Per-frame object tracker: frigate/track/norfair_tracker.py 261-636 at v0.17.2 on a static camera; deregister removes one track, not the tracker's whole live list; the median box is the ordinary median; the stationary decision lives at the tracker site as Stationary::observe and the strict comparison of the next layer is out of scope.

use crate::matcher::NanDistance;
use crate::stationary::{Stationary, StationaryThresholds, Verdict};
use crate::track::{IdSource, Observation, TrackConfig};
use crate::tracker::{Sighting, Tracker};

const OWN_LABELS: [&str; 2] = ["car", "license_plate"];
const PAST_DETECTIONS_LENGTH: u32 = 4;
const CAR_R: f64 = 3.4;
const CAR_Q: f64 = 0.03;
const PLATE_R: f64 = 2.5;
const PLATE_Q: f64 = 0.05;
const DEFAULT_R: f64 = 3.4;
const DEFAULT_Q: f64 = 0.1;
const CAR_THRESHOLD: f64 = 2.5;
const PLATE_THRESHOLD: f64 = 3.75;
const DEFAULT_THRESHOLD: f64 = 2.5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackerSettings {
    pub width: i32,
    pub height: i32,
    pub min_initialized: u32,
    pub max_disappeared: u32,
    pub stationary_threshold: u64,
    pub max_frames_default: Option<u64>,
    pub max_frames_overrides: Vec<(String, u64)>,
    pub own_labels: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameDetection {
    pub label: String,
    pub score: f64,
    pub edges: [i32; 4],
    pub area: i64,
    pub ratio: f64,
    pub region: Vec<[i32; 4]>,
}

#[derive(Clone, Debug)]
struct Sight {
    score: f64,
    edges: [i32; 4],
    area: i64,
    ratio: f64,
    region: Vec<[i32; 4]>,
    frame_time: f64,
    centroid: [i32; 2],
}

#[derive(Debug)]
struct Entry {
    label: String,
    score: f64,
    edges: [i32; 4],
    area: i64,
    ratio: f64,
    region: Vec<[i32; 4]>,
    frame_time: f64,
    centroid: [i32; 2],
    estimate: [i32; 4],
    estimate_velocity: [f64; 4],
    start_time: f64,
    score_history: Vec<f64>,
    disappeared: u64,
    global_id: u64,
    stationary: Stationary,
}

#[derive(Debug)]
pub struct ObjectView<'a> {
    pub label: &'a str,
    pub score: f64,
    pub edges: [i32; 4],
    pub area: i64,
    pub ratio: f64,
    pub region: &'a [[i32; 4]],
    pub frame_time: f64,
    pub centroid: [i32; 2],
    pub estimate: [i32; 4],
    pub estimate_velocity: [f64; 4],
    pub start_time: f64,
    pub motionless: u64,
    pub position_changes: u64,
    pub score_history: &'a [f64],
    pub envelope: [f64; 4],
    pub history: &'a [[i32; 4]],
    pub disappeared: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackerState {
    pub name: String,
    pub tracks: Vec<(Option<u64>, i64, bool)>,
}

#[derive(Debug)]
struct ActiveTrack {
    global_id: u64,
    label: String,
    sight: Sight,
    estimate: [i32; 4],
    estimate_velocity: [f64; 4],
    past_scores: Vec<f64>,
    past_boxes: Vec<[i32; 4]>,
}

#[derive(Debug)]
pub struct ObjectTracker<'a> {
    settings: TrackerSettings,
    id_source: &'a mut IdSource,
    own: Vec<(String, Tracker<Sight>)>,
    fallback: Tracker<Sight>,
    entries: Vec<Entry>,
    untracked: Vec<[i32; 4]>,
}

fn unsigned_truncated(value: f64) -> Option<i32> {
    if !(0.0..2_147_483_648.0).contains(&value) {
        return None;
    }
    let mut low: i64 = 0;
    let mut high: i64 = 2_147_483_648;
    while high - low > 1 {
        let middle = low + (high - low) / 2;
        let candidate = f64::from(i32::try_from(middle).ok()?);
        if candidate <= value {
            low = middle;
        } else {
            high = middle;
        }
    }
    i32::try_from(low).ok()
}

fn trunc_to_i32(value: f64) -> i32 {
    let truncated = value.trunc();
    if truncated.is_nan() {
        return i32::MIN;
    }
    if truncated < 0.0 {
        unsigned_truncated(-truncated)
            .and_then(i32::checked_neg)
            .unwrap_or(i32::MIN)
    } else {
        unsigned_truncated(truncated).unwrap_or(i32::MAX)
    }
}

fn clamped_estimate(estimate: [f64; 4], width: i32, height: i32) -> [i32; 4] {
    let [left, top, right, bottom] = estimate.map(trunc_to_i32);
    [
        left.max(0),
        top.max(0),
        right.min(width - 1),
        bottom.min(height - 1),
    ]
}

fn centroid_of(first: i32, second: i32) -> i32 {
    let half = f64::midpoint(f64::from(first), f64::from(second));
    trunc_to_i32(half)
}

impl<'a> ObjectTracker<'a> {
    #[must_use]
    pub fn new(settings: TrackerSettings, ids: &'a mut IdSource) -> Option<Self> {
        let car = TrackConfig::new(
            settings.max_disappeared,
            settings.min_initialized,
            PAST_DETECTIONS_LENGTH,
            CAR_R,
            CAR_Q,
        )?;
        let plate = TrackConfig::new(
            settings.max_disappeared,
            settings.min_initialized,
            PAST_DETECTIONS_LENGTH,
            PLATE_R,
            PLATE_Q,
        )?;
        let default = TrackConfig::new(
            settings.max_disappeared,
            settings.min_initialized,
            PAST_DETECTIONS_LENGTH,
            DEFAULT_R,
            DEFAULT_Q,
        )?;
        let mut own: Vec<(String, Tracker<Sight>)> = Vec::new();
        for label in OWN_LABELS {
            if !settings.own_labels.iter().any(|tracked| tracked == label) {
                continue;
            }
            let tracker = match label {
                "car" => Tracker::new(car, CAR_THRESHOLD),
                "license_plate" => Tracker::new(plate, PLATE_THRESHOLD),
                _ => continue,
            };
            own.push((label.to_owned(), tracker));
        }
        let fallback = Tracker::new(default, DEFAULT_THRESHOLD);
        Some(Self {
            settings,
            id_source: ids,
            own,
            fallback,
            entries: Vec::new(),
            untracked: Vec::new(),
        })
    }

    /// # Errors
    pub fn match_and_update(
        &mut self,
        frame_time: f64,
        detections: Vec<FrameDetection>,
    ) -> Result<(), NanDistance> {
        let mut input_boxes: Vec<[i32; 4]> = Vec::with_capacity(detections.len());
        let mut groups: Vec<(String, Vec<Sighting<Sight>>)> = Vec::new();
        for detection in detections {
            input_boxes.push(detection.edges);
            let [left, top, right, bottom] = detection.edges;
            let sighting = Sighting {
                label: detection.label.clone(),
                observation: Observation {
                    edges: detection.edges,
                    data: Sight {
                        score: detection.score,
                        edges: detection.edges,
                        area: detection.area,
                        ratio: detection.ratio,
                        region: detection.region,
                        frame_time,
                        centroid: [centroid_of(left, right), centroid_of(top, bottom)],
                    },
                },
            };
            if let Some((_, group)) = groups
                .iter_mut()
                .find(|(label, _)| *label == detection.label)
            {
                group.push(sighting);
            } else {
                groups.push((detection.label.clone(), vec![sighting]));
            }
        }
        for (label, tracker) in &mut self.own {
            let sightings = take_group(&mut groups, label);
            tracker.update(&mut *self.id_source, sightings)?;
        }
        let mut default_sightings = Vec::new();
        for (_, group) in groups {
            default_sightings.extend(group);
        }
        self.fallback
            .update(&mut *self.id_source, default_sightings)?;
        self.run_tail(frame_time);
        let tracked: Vec<[i32; 4]> = self.entries.iter().map(|entry| entry.edges).collect();
        self.untracked = input_boxes
            .into_iter()
            .filter(|box_edges| !tracked.contains(box_edges))
            .collect();
        Ok(())
    }

    /// # Errors
    pub fn update_frame_times(&mut self, frame_time: f64) -> Result<(), NanDistance> {
        let dets: Vec<FrameDetection> = self
            .entries
            .iter()
            .filter(|entry| entry.disappeared == 0)
            .map(|entry| FrameDetection {
                label: entry.label.clone(),
                score: entry.score,
                edges: entry.edges,
                area: entry.area,
                ratio: entry.ratio,
                region: entry.region.clone(),
            })
            .collect();
        self.match_and_update(frame_time, dets)
    }

    pub fn objects(&self) -> impl Iterator<Item = ObjectView<'_>> {
        self.entries.iter().map(|entry| ObjectView {
            label: &entry.label,
            score: entry.score,
            edges: entry.edges,
            area: entry.area,
            ratio: entry.ratio,
            region: &entry.region,
            frame_time: entry.frame_time,
            centroid: entry.centroid,
            estimate: entry.estimate,
            estimate_velocity: entry.estimate_velocity,
            start_time: entry.start_time,
            motionless: entry.stationary.motionless_count(),
            position_changes: entry.stationary.position_changes(),
            score_history: &entry.score_history,
            envelope: entry.stationary.envelope(),
            history: entry.stationary.history(),
            disappeared: entry.disappeared,
        })
    }

    #[must_use]
    pub fn untracked_boxes(&self) -> &[[i32; 4]] {
        &self.untracked
    }

    #[must_use]
    pub const fn map_entries(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn tracker_states(&self) -> Vec<TrackerState> {
        let mut rows: Vec<TrackerState> = self
            .own
            .iter()
            .map(|(label, tracker)| TrackerState {
                name: label.clone(),
                tracks: state_of(tracker),
            })
            .collect();
        rows.push(TrackerState {
            name: "default".to_owned(),
            tracks: state_of(&self.fallback),
        });
        rows
    }

    fn run_tail(&mut self, frame_time: f64) {
        let mut actives: Vec<ActiveTrack> = Vec::new();
        for (_, tracker) in &self.own {
            collect_actives(
                tracker,
                self.settings.width,
                self.settings.height,
                &mut actives,
            );
        }
        collect_actives(
            &self.fallback,
            self.settings.width,
            self.settings.height,
            &mut actives,
        );
        let mut active_ids: Vec<u64> = Vec::with_capacity(actives.len());
        for active in actives {
            active_ids.push(active.global_id);
            let Some(index) = self
                .entries
                .iter()
                .position(|entry| entry.global_id == active.global_id)
            else {
                self.register(&active);
                continue;
            };
            if active.sight.frame_time.to_bits() == frame_time.to_bits() {
                let thresholds = StationaryThresholds::for_label(&active.label);
                let max_frames = self.max_frames_for(&active.label);
                let verdict = match self.entries.get_mut(index) {
                    Some(entry) => {
                        entry.disappeared = 0;
                        entry.stationary.observe(
                            active.sight.edges,
                            &thresholds,
                            self.settings.stationary_threshold,
                            max_frames,
                        )
                    }
                    None => continue,
                };
                if verdict == Verdict::Expired {
                    self.deregister(active.global_id);
                } else if let Some(entry) = self.entries.get_mut(index) {
                    entry.score = active.sight.score;
                    entry.edges = active.sight.edges;
                    entry.area = active.sight.area;
                    entry.ratio = active.sight.ratio;
                    entry.region.clone_from(&active.sight.region);
                    entry.frame_time = active.sight.frame_time;
                    entry.centroid = active.sight.centroid;
                    entry.estimate = active.estimate;
                    entry.estimate_velocity = active.estimate_velocity;
                }
            } else if let Some(entry) = self.entries.get_mut(index) {
                entry.disappeared = entry.disappeared.saturating_add(1);
                let [left, top, right, bottom] = active.estimate;
                if left < right && top < bottom {
                    entry.estimate = active.estimate;
                }
            }
        }
        let expired: Vec<u64> = self
            .entries
            .iter()
            .map(|entry| entry.global_id)
            .filter(|global_id| !active_ids.contains(global_id))
            .collect();
        for global_id in expired {
            self.deregister(global_id);
        }
    }

    fn register(&mut self, active: &ActiveTrack) {
        let stationary = Stationary::new(
            &active.past_boxes,
            self.settings.width,
            self.settings.height,
        );
        self.entries.push(Entry {
            label: active.label.clone(),
            score: active.sight.score,
            edges: active.sight.edges,
            area: active.sight.area,
            ratio: active.sight.ratio,
            region: active.sight.region.clone(),
            frame_time: active.sight.frame_time,
            centroid: active.sight.centroid,
            estimate: active.estimate,
            estimate_velocity: active.estimate_velocity,
            start_time: active.sight.frame_time,
            score_history: active.past_scores.clone(),
            disappeared: 0,
            global_id: active.global_id,
            stationary,
        });
    }

    fn deregister(&mut self, global_id: u64) {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.global_id == global_id)
        else {
            return;
        };
        let label = match self.entries.get(index) {
            Some(entry) => entry.label.clone(),
            None => return,
        };
        self.entries.remove(index);
        if self.max_frames_for(&label).is_some() {
            self.tracker_for_mut(&label).remove(global_id);
        }
    }

    fn tracker_for_mut(&mut self, label: &str) -> &mut Tracker<Sight> {
        match self.own.iter_mut().find(|(name, _)| name == label) {
            Some((_, tracker)) => tracker,
            None => &mut self.fallback,
        }
    }

    fn max_frames_for(&self, label: &str) -> Option<u64> {
        self.settings
            .max_frames_overrides
            .iter()
            .find(|(name, _)| name == label)
            .map(|(_, frames)| *frames)
            .or(self.settings.max_frames_default)
    }
}

fn take_group(
    groups: &mut Vec<(String, Vec<Sighting<Sight>>)>,
    label: &str,
) -> Vec<Sighting<Sight>> {
    groups
        .iter()
        .position(|(name, _)| name == label)
        .map_or_else(Vec::new, |position| groups.remove(position).1)
}

fn state_of(tracker: &Tracker<Sight>) -> Vec<(Option<u64>, i64, bool)> {
    tracker
        .tracks()
        .iter()
        .map(|track| (track.id(), track.hit_counter(), track.is_initializing()))
        .collect()
}

fn collect_actives(
    tracker: &Tracker<Sight>,
    width: i32,
    height: i32,
    actives: &mut Vec<ActiveTrack>,
) {
    for track in tracker.active_tracks() {
        let Some(global_id) = track.id() else {
            continue;
        };
        actives.push(ActiveTrack {
            global_id,
            label: track.label().to_owned(),
            sight: track.last_detection().data.clone(),
            estimate: clamped_estimate(track.estimate(), width, height),
            estimate_velocity: track.velocity(),
            past_scores: track
                .past_detections()
                .iter()
                .map(|(_, past)| past.data.score)
                .collect(),
            past_boxes: track
                .past_detections()
                .iter()
                .map(|(_, past)| past.data.edges)
                .collect(),
        });
    }
}
