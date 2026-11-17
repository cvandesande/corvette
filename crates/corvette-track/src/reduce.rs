//! Frigate's reduce pass: NMS over label groups, then the consolidation.

use crate::mapping::Frame;
use crate::merge::{Detection, clipped, consolidate, to_f64};
use corvette_yolo::{Candidate, Rect, nms_boxes};

const SCORE_FLOOR: f32 = 0.5;
const CLIPPED_CONFIDENCE: f32 = 0.6;
const CAR_NMS_LIMIT: f32 = 0.6;
const DEFAULT_NMS_LIMIT: f32 = 0.4;

fn nms_limit(label: &str) -> f32 {
    match label {
        "car" => CAR_NMS_LIMIT,
        _ => DEFAULT_NMS_LIMIT,
    }
}

fn rect_of(detection: &Detection) -> Option<Rect> {
    let geometry = detection.geometry();
    let (Some(x), Some(y)) = (to_f64(geometry.x_min), to_f64(geometry.y_min)) else {
        return None;
    };
    let (Some(width), Some(height)) = (
        to_f64(geometry.x_max - geometry.x_min),
        to_f64(geometry.y_max - geometry.y_min),
    ) else {
        return None;
    };
    Some(Rect {
        x,
        y,
        width,
        height,
    })
}

/// object.py 501-538 run per label group in first-seen order, then consolidated.
#[must_use]
pub fn reduce_detections(detections: Vec<Detection>, frame: Frame) -> Vec<Detection> {
    let mut slots: Vec<Option<Detection>> = detections.into_iter().map(Some).collect();
    let mut groups: Vec<(String, Vec<(usize, &Detection)>)> = Vec::new();
    for (index, slot) in slots.iter().enumerate() {
        let Some(detection) = slot else { continue };
        match groups.iter().position(|group| group.0 == detection.label()) {
            Some(at) => {
                if let Some(group) = groups.get_mut(at) {
                    group.1.push((index, detection));
                }
            }
            None => groups.push((detection.label().to_string(), vec![(index, detection)])),
        }
    }
    let mut survivors: Vec<usize> = Vec::new();
    for (label, members) in &groups {
        let mut items: Vec<(usize, Candidate)> = Vec::new();
        for &(index, detection) in members {
            let Some(rect) = rect_of(detection) else {
                continue;
            };
            let score = if clipped(detection, frame) {
                CLIPPED_CONFIDENCE
            } else {
                detection.score()
            };
            items.push((index, Candidate { rect, score }));
        }
        let candidates: Vec<Candidate> = items.iter().map(|entry| entry.1).collect();
        for position in nms_boxes(&candidates, SCORE_FLOOR, nms_limit(label)) {
            if let Some(&(index, _)) = items.get(position) {
                survivors.push(index);
            }
        }
    }
    let mut reduced: Vec<Detection> = Vec::new();
    for index in survivors {
        if let Some(detection) = slots.get_mut(index).and_then(Option::take) {
            reduced.push(detection);
        }
    }
    consolidate(reduced)
}
