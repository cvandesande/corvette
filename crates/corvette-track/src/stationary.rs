// Rules: frigate/track/norfair_tracker.py 324-479, util/object.py 339-358, util/image.py 701-749 and track/stationary_classifier.py 14-80 at v0.17.2.
// The median is the ordinary median: an even count averages the two middle boxes edge by edge.
// The envelope is the numpy linear-interpolation 15th and 85th percentiles of the position samples.

use crate::merge::inclusive_area;

const SAMPLE_CAP: usize = 10;
const LOW_QUANTILE: f64 = 15.0 / 100.0;
const HIGH_QUANTILE: f64 = 85.0 / 100.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StationaryThresholds {
    known_active_iou: f64,
    stationary_check_iou: f64,
    active_check_iou: f64,
    max_history: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Alive,
    Expired,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stationary {
    history: Vec<[i32; 4]>,
    samples: Vec<[i32; 4]>,
    envelope: [f64; 4],
    motionless: u64,
    changes: u64,
}

impl StationaryThresholds {
    #[must_use]
    pub fn for_label(label: &str) -> Self {
        let mut thresholds = Self {
            known_active_iou: 0.2,
            stationary_check_iou: 0.6,
            active_check_iou: 0.9,
            max_history: 10,
        };
        match label {
            "bbq_grill" | "package" | "waste_bin" => thresholds.known_active_iou = 0.0,
            "bicycle" | "boat" | "car" | "motorcycle" | "tractor" | "truck" => {
                thresholds.active_check_iou = 0.75;
            }
            "license_plate" => {
                thresholds.known_active_iou = 0.9;
                thresholds.stationary_check_iou = 0.9;
                thresholds.max_history = 4;
            }
            _ => {}
        }
        thresholds
    }

    #[must_use]
    pub const fn known_active_iou(&self) -> f64 {
        self.known_active_iou
    }

    #[must_use]
    pub const fn stationary_check_iou(&self) -> f64 {
        self.stationary_check_iou
    }

    #[must_use]
    pub const fn active_check_iou(&self) -> f64 {
        self.active_check_iou
    }

    #[must_use]
    pub const fn max_history(&self) -> usize {
        self.max_history
    }
}

fn to_f64_box(edges: [i32; 4]) -> [f64; 4] {
    [
        f64::from(edges[0]),
        f64::from(edges[1]),
        f64::from(edges[2]),
        f64::from(edges[3]),
    ]
}

fn mean_box(history: &[[i32; 4]]) -> [f64; 4] {
    let mut sums = [0.0_f64; 4];
    for edges in history {
        sums[0] += f64::from(edges[0]);
        sums[1] += f64::from(edges[1]);
        sums[2] += f64::from(edges[2]);
        sums[3] += f64::from(edges[3]);
    }
    let count = f64::from(u32::try_from(history.len()).unwrap_or(u32::MAX));
    [
        sums[0] / count,
        sums[1] / count,
        sums[2] / count,
        sums[3] / count,
    ]
}

fn half_mean(lower: i32, upper: i32) -> f64 {
    let sum = f64::from(lower) + f64::from(upper);
    sum / 2.0
}

fn median_box(history: &[[i32; 4]]) -> [f64; 4] {
    let mut boxes = history.to_vec();
    boxes.sort_by_key(|edges| {
        inclusive_area([
            i64::from(edges[0]),
            i64::from(edges[1]),
            i64::from(edges[2]),
            i64::from(edges[3]),
        ])
    });
    let count = boxes.len();
    let middle = count / 2;
    if count == 0 {
        return [0.0; 4];
    }
    if count % 2 == 1 {
        let pick = boxes.get(middle).copied().unwrap_or([0; 4]);
        return to_f64_box(pick);
    }
    let lower = boxes.get(middle - 1).copied().unwrap_or([0; 4]);
    let upper = boxes.get(middle).copied().unwrap_or([0; 4]);
    [
        half_mean(lower[0], upper[0]),
        half_mean(lower[1], upper[1]),
        half_mean(lower[2], upper[2]),
        half_mean(lower[3], upper[3]),
    ]
}

fn intersection_over_union(first: [f64; 4], second: [f64; 4]) -> f64 {
    if first[2] < second[0] || first[0] > second[2] || first[1] > second[3] || first[3] < second[1]
    {
        return 0.0;
    }
    let left = first[0].max(second[0]);
    let top = first[1].max(second[1]);
    let right = first[2].min(second[2]);
    let bottom = first[3].min(second[3]);
    let inter_width = (right - left + 1.0).max(0.0);
    let inter_height = (bottom - top + 1.0).max(0.0);
    let inter = inter_width * inter_height;
    if inter <= 0.0 {
        return 0.0;
    }
    let first_area = (first[2] - first[0] + 1.0) * (first[3] - first[1] + 1.0);
    let second_area = (second[2] - second[0] + 1.0) * (second[3] - second[1] + 1.0);
    inter / (first_area + second_area - inter)
}

fn percentile_of(samples: &[[i32; 4]], edge: usize, quantile: f64) -> f64 {
    let mut values: Vec<i32> = samples.iter().map(|box_edges| box_edges[edge]).collect();
    values.sort_unstable();
    if values.is_empty() {
        return 0.0;
    }
    let top = u32::try_from(values.len() - 1).unwrap_or(u32::MAX);
    let last = f64::from(values.last().copied().unwrap_or(0));
    let virtual_index = f64::from(top) * quantile;
    if virtual_index >= f64::from(top) {
        return last;
    }
    let mut previous_index: usize = 0;
    let mut candidate: usize = 1;
    while candidate < values.len()
        && f64::from(u32::try_from(candidate).unwrap_or(u32::MAX)) <= virtual_index
    {
        previous_index = candidate;
        candidate += 1;
    }
    let previous = f64::from(values.get(previous_index).copied().unwrap_or(0));
    let next = f64::from(values.get(previous_index + 1).copied().unwrap_or(0));
    let gamma = virtual_index - f64::from(u32::try_from(previous_index).unwrap_or(u32::MAX));
    let diff = next - previous;
    if gamma >= 0.5 {
        let rest = 1.0 - gamma;
        let correction = diff * rest;
        next - correction
    } else {
        let advance = diff * gamma;
        previous + advance
    }
}

fn envelope_of(samples: &[[i32; 4]]) -> [f64; 4] {
    [
        percentile_of(samples, 0, LOW_QUANTILE),
        percentile_of(samples, 1, LOW_QUANTILE),
        percentile_of(samples, 2, HIGH_QUANTILE),
        percentile_of(samples, 3, HIGH_QUANTILE),
    ]
}

impl Stationary {
    #[must_use]
    pub fn new(initial: &[[i32; 4]], frame_width: i32, frame_height: i32) -> Self {
        Self {
            history: initial.to_vec(),
            samples: initial.to_vec(),
            envelope: [0.0, 0.0, f64::from(frame_width), f64::from(frame_height)],
            motionless: 0,
            changes: 0,
        }
    }

    pub fn observe(
        &mut self,
        edges: [i32; 4],
        thresholds: &StationaryThresholds,
        after: u64,
        max_frames: Option<u64>,
    ) -> Verdict {
        let stationary = self.motionless >= after;
        self.history.push(edges);
        if self.history.len() > thresholds.max_history() {
            let excess = self.history.len() - thresholds.max_history();
            self.history.drain(..excess);
        }
        let current = to_f64_box(edges);
        let average = mean_box(&self.history);
        let average_iou = intersection_over_union(current, average);
        if average_iou < thresholds.known_active_iou() {
            return self.moved(edges, stationary);
        }
        let limit = if stationary {
            thresholds.stationary_check_iou()
        } else {
            thresholds.active_check_iou()
        };
        if average_iou < limit {
            let median = median_box(&self.history);
            let median_iou = intersection_over_union(self.envelope, median);
            if median_iou < limit {
                return self.moved(edges, stationary);
            }
        }
        if self.samples.len() < SAMPLE_CAP {
            self.samples.push(edges);
            self.envelope = envelope_of(&self.samples);
        }
        self.motionless += 1;
        if let Some(cap) = max_frames
            && self.motionless.saturating_sub(after) > cap
        {
            return Verdict::Expired;
        }
        Verdict::Alive
    }

    fn moved(&mut self, edges: [i32; 4], stationary: bool) -> Verdict {
        if self.changes == 0 || stationary {
            self.changes += 1;
        }
        self.motionless = 0;
        self.samples = vec![edges];
        self.envelope = to_f64_box(edges);
        self.history.clear();
        Verdict::Alive
    }

    #[must_use]
    pub const fn motionless_count(&self) -> u64 {
        self.motionless
    }

    #[must_use]
    pub const fn position_changes(&self) -> u64 {
        self.changes
    }

    #[must_use]
    pub const fn envelope(&self) -> [f64; 4] {
        self.envelope
    }

    #[must_use]
    pub const fn position_samples(&self) -> usize {
        self.samples.len()
    }

    #[must_use]
    pub fn history(&self) -> &[[i32; 4]] {
        &self.history
    }
}
