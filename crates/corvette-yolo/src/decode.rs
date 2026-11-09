use crate::nms::{Candidate, Rect, nms_boxes};

pub const ANCHORS: usize = 8400;
pub const CLASSES: usize = 46;
pub const BOX_ROWS: usize = 4;
pub const ROWS: usize = BOX_ROWS + CLASSES;
pub const RAW_LEN: usize = ROWS * ANCHORS;
pub const MAX_DETECTIONS: usize = 20;
pub const DEFAULT_SCORE_THRESHOLD: f32 = 0.4;
pub const DEFAULT_NMS_THRESHOLD: f32 = 0.4;

const _: () = assert!(crate::WIDTH == 640 && crate::HEIGHT == 640);
const INPUT_PX: f32 = 640.0;

// `raw` is row-major at `raw[row * ANCHORS + anchor]`: rows 0 to 3 are centre x, centre y, width, height; rows 4 to 49 the class scores.
const ROW_CX: usize = 0;
const ROW_CY: usize = 1;
const ROW_W: usize = 2;
const ROW_H: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detection {
    pub class_id: u32,
    pub score: f32,
    pub y_min: f32,
    pub x_min: f32,
    pub y_max: f32,
    pub x_max: f32,
}

fn corners(raw: &[f32; RAW_LEN], anchor: usize) -> [f32; 4] {
    let cx = raw[ROW_CX * ANCHORS + anchor];
    let cy = raw[ROW_CY * ANCHORS + anchor];
    let w = raw[ROW_W * ANCHORS + anchor];
    let h = raw[ROW_H * ANCHORS + anchor];
    [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0]
}

/// Up to `MAX_DETECTIONS` rows, best score first, corners as fractions of the input.
#[must_use]
pub fn decode(raw: &[f32; RAW_LEN], score_threshold: f32, nms_threshold: f32) -> Vec<Detection> {
    let mut best = vec![f32::MIN; ANCHORS];
    let mut class_of = vec![0u32; ANCHORS];
    for (class, scores) in raw[BOX_ROWS * ANCHORS..].chunks_exact(ANCHORS).enumerate() {
        let class_id = u32::try_from(class).unwrap_or(u32::MAX);
        for (anchor, &score) in scores.iter().enumerate() {
            if best[anchor].is_nan() {
                continue;
            }
            if score.is_nan() {
                best[anchor] = f32::NAN;
            } else if score > best[anchor] {
                best[anchor] = score;
                class_of[anchor] = class_id;
            }
        }
    }
    let anchors: Vec<usize> = (0..ANCHORS)
        .filter(|&anchor| best[anchor] > score_threshold)
        .collect();
    // The suppression call receives the corners (x1, y1, x2, y2) in the x, y, width, height fields because Frigate does.
    let candidates: Vec<Candidate> = anchors
        .iter()
        .map(|&anchor| {
            let [x1, y1, x2, y2] = corners(raw, anchor);
            Candidate {
                rect: Rect {
                    x: f64::from(x1),
                    y: f64::from(y1),
                    width: f64::from(x2),
                    height: f64::from(y2),
                },
                score: best[anchor],
            }
        })
        .collect();
    let kept = nms_boxes(&candidates, score_threshold, nms_threshold);
    kept.iter()
        .take(MAX_DETECTIONS)
        .map(|&kept_index| {
            let anchor = anchors[kept_index];
            let [x1, y1, x2, y2] = corners(raw, anchor);
            Detection {
                class_id: class_of[anchor],
                score: best[anchor],
                y_min: y1 / INPUT_PX,
                x_min: x1 / INPUT_PX,
                y_max: y2 / INPUT_PX,
                x_max: x2 / INPUT_PX,
            }
        })
        .collect()
}
