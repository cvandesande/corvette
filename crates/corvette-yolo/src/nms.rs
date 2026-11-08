use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub rect: Rect,
    pub score: f32,
}

fn std_max(a: f64, b: f64) -> f64 {
    if a < b { b } else { a }
}
fn std_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

const fn round_f64_to_f32(value: f64) -> f32 {
    let bits = value.to_bits();
    let sign = ((bits >> 32) & 0x8000_0000) as u32;
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let mant = bits & 0x000f_ffff_ffff_ffff;
    if exponent == 0x7ff {
        let q = if mant == 0 {
            0
        } else {
            0x40_0000 | ((mant >> 29) & 0x3f_ffff) as u32
        };
        return f32::from_bits(sign | 0x7f80_0000 | q);
    }
    if exponent == 0 {
        return f32::from_bits(sign);
    }
    let normal = exponent > 896;
    let shift = if normal { 29 } else { 926 - exponent };
    if shift >= 54 {
        return f32::from_bits(sign);
    }
    let exp_base = if normal { exponent - 896 } else { 0 };
    let carry_bit = if normal { 24u32 } else { 23 };
    let sig = (1u64 << 52) | mant;
    let mut m = sig >> shift;
    let dropped = sig & ((1u64 << shift) - 1);
    let half = 1u64 << (shift - 1);
    if dropped > half || (dropped == half && (m & 1) == 1) {
        m += 1;
    }
    let extra = if m >= (1u64 << carry_bit) { 1 } else { 0 };
    let result = exp_base + extra;
    if result >= 255 {
        return f32::from_bits(sign | 0x7f80_0000);
    }
    let drop = if normal { extra } else { 0 };
    let fraction = ((m >> drop.cast_unsigned()) & 0x7f_ffff) as u32;
    f32::from_bits(sign | result.cast_unsigned() << 23 | fraction)
}

fn overlap(a: &Rect, b: &Rect) -> f32 {
    let left = std_max(a.x, b.x);
    let top = std_max(a.y, b.y);
    let right = std_min(a.x + a.width, b.x + b.width);
    let bottom = std_min(a.y + a.height, b.y + b.height);
    let inter_w = right - left;
    let inter_h = bottom - top;
    let intersection = if inter_w > 0.0 && inter_h > 0.0 {
        inter_w * inter_h
    } else {
        0.0
    };
    let area_a = a.width * a.height;
    let area_b = b.width * b.height;
    let union = area_a + area_b - intersection;
    1.0f32 - round_f64_to_f32(1.0 - intersection / union)
}

/// Keep the best box of each cluster of overlapping candidates.
///
/// Reproduces the double (`Rect2d`) overload of `OpenCV` 4.11.0 `NMSBoxes`: `nms.inl.hpp` and `nms.cpp` in `modules/dnn/src`.
#[must_use]
pub fn nms_boxes(candidates: &[Candidate], score_threshold: f32, nms_threshold: f32) -> Vec<usize> {
    let mut scored: Vec<(f32, usize)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.score > score_threshold)
        .map(|(index, candidate)| (candidate.score, index))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
    let mut kept: Vec<usize> = Vec::new();
    for &(_, index) in &scored {
        let mut keep = true;
        for &kept_index in &kept {
            let overlap = overlap(&candidates[index].rect, &candidates[kept_index].rect);
            keep = overlap <= nms_threshold;
            if !keep {
                break;
            }
        }
        if keep {
            kept.push(index);
        }
    }
    kept
}
