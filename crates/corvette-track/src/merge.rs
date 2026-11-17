//! Frigate's second merge pass over one frame's detections and its box helpers.
//! Reference: `frigate/util/object.py` 495-591 and `frigate/util/image.py` 701-765.

use crate::mapping::{Frame, Geometry, Region};

const EDGE_LIMIT: i64 = 16_777_216;
const SPLIT: i64 = 4_294_967_296;
const AREA_SKIP_RATIO: f64 = 0.05;
const CAR_LIMIT: f64 = 0.8;
const FACE_LIMIT: f64 = 0.5;
const DEFAULT_LIMIT: f64 = 0.9;

#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    label: String,
    score: f32,
    geometry: Geometry,
    region: Region,
}

impl Detection {
    #[must_use]
    pub fn new(label: &str, score: f32, geometry: Geometry, region: Region) -> Option<Self> {
        let edges = box_edges(geometry);
        if edges
            .iter()
            .any(|edge| *edge > EDGE_LIMIT || *edge < -EDGE_LIMIT)
        {
            return None;
        }
        Some(Self {
            label: label.to_string(),
            score,
            geometry,
            region,
        })
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub const fn score(&self) -> f32 {
        self.score
    }

    #[must_use]
    pub const fn geometry(&self) -> Geometry {
        self.geometry
    }

    #[must_use]
    pub const fn region(&self) -> Region {
        self.region
    }
}

const fn box_edges(geometry: Geometry) -> [i64; 4] {
    [
        geometry.x_min,
        geometry.y_min,
        geometry.x_max,
        geometry.y_max,
    ]
}

fn to_f64(value: i64) -> Option<f64> {
    let high = i32::try_from(value.div_euclid(SPLIT)).ok()?;
    let low = u32::try_from(value.rem_euclid(SPLIT)).ok()?;
    let scaled = f64::from(high) * 4_294_967_296.0;
    Some(scaled + f64::from(low))
}

fn consolidation_limit(label: &str) -> f64 {
    match label {
        "car" => CAR_LIMIT,
        "face" => FACE_LIMIT,
        _ => DEFAULT_LIMIT,
    }
}

/// Frigate `image.py` 720: the inclusive `(w + 1) * (h + 1)` of a box.
#[must_use]
pub const fn inclusive_area(edges: [i64; 4]) -> i64 {
    let [x_min, y_min, x_max, y_max] = edges;
    let width = x_max.saturating_sub(x_min).saturating_add(1);
    let height = y_max.saturating_sub(y_min).saturating_add(1);
    width.saturating_mul(height)
}

/// Frigate `image.py` 701-716; touching boxes intersect.
#[must_use]
pub fn intersection(a: [i64; 4], b: [i64; 4]) -> Option<[i64; 4]> {
    let [a0, a1, a2, a3] = a;
    let [b0, b1, b2, b3] = b;
    if a2 < b0 || a0 > b2 || a1 > b3 || a3 < b1 {
        return None;
    }
    Some([a0.max(b0), a1.max(b1), a2.min(b2), a3.min(b3)])
}

#[must_use]
pub const fn clipped(detection: &Detection, frame: Frame) -> bool {
    let geometry = detection.geometry;
    let region = detection.region;
    let [x_min, y_min, x_max, y_max] = box_edges(geometry);
    (region.x_min() > 5 && x_min - region.x_min() <= 5)
        || (region.y_min() > 5 && y_min - region.y_min() <= 5)
        || (frame.width() - region.x_max() > 5 && region.x_max() - x_max <= 5)
        || (frame.height() - region.y_max() > 5 && region.y_max() - y_max <= 5)
}

#[must_use]
pub fn consolidate(detections: Vec<Detection>) -> Vec<Detection> {
    let mut slots: Vec<Option<Detection>> = detections.into_iter().map(Some).collect();
    let mut groups: Vec<(String, Vec<(usize, &Detection)>)> = Vec::new();
    for (index, slot) in slots.iter().enumerate() {
        let Some(detection) = slot else {
            continue;
        };
        match groups.iter().position(|group| group.0 == detection.label) {
            Some(at) => {
                if let Some(group) = groups.get_mut(at) {
                    group.1.push((index, detection));
                }
            }
            None => groups.push((detection.label.clone(), vec![(index, detection)])),
        }
    }
    let mut survivors: Vec<usize> = Vec::new();
    for (_, members) in &groups {
        let mut order: Vec<(usize, &Detection)> = members.clone();
        order.sort_by_key(|entry| entry.1.geometry.area);
        for (position, current) in order.iter().enumerate() {
            let current_box = box_edges(current.1.geometry);
            let current_area = to_f64(inclusive_area(current_box));
            let limit = consolidation_limit(current.1.label());
            let mut dropped = false;
            for to_check in order.iter().skip(position + 1) {
                let other_box = box_edges(to_check.1.geometry);
                let other_area = inclusive_area(other_box);
                // object.py 570: Frigate would divide by zero on a zero-area later box
                if other_area == 0 {
                    continue;
                }
                let (Some(current_f), Some(other_f)) = (current_area, to_f64(other_area)) else {
                    continue;
                };
                if current_f / other_f < AREA_SKIP_RATIO {
                    continue;
                }
                if let Some(inter) = intersection(current_box, other_box) {
                    let inter_f = to_f64(inclusive_area(inter));
                    if inter_f.is_some_and(|value| value / current_f > limit) {
                        dropped = true;
                        break;
                    }
                }
            }
            if !dropped {
                survivors.push(current.0);
            }
        }
    }
    let mut output = Vec::new();
    for index in survivors {
        if let Some(detection) = slots.get_mut(index).and_then(Option::take) {
            output.push(detection);
        }
    }
    output
}
