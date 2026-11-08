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

fn is_empty(r: &Rect) -> bool {
    r.width <= 0.0 || r.height <= 0.0
}

fn intersection_area(a: &Rect, b: &Rect) -> f64 {
    if is_empty(a) || is_empty(b) {
        return 0.0;
    }
    let (x_min, x_max) = if a.x < b.x { (a, b) } else { (b, a) };
    let (y_min, y_max) = if a.y < b.y { (a, b) } else { (b, a) };
    if (x_min.x < 0.0 && x_min.x + x_min.width < x_max.x)
        || (y_min.y < 0.0 && y_min.y + y_min.height < y_max.y)
    {
        return 0.0;
    }
    let width = std_min(x_min.width - (x_max.x - x_min.x), x_max.width);
    let height = std_min(y_min.height - (y_max.y - y_min.y), y_max.height);
    if width <= 0.0 || height <= 0.0 {
        return 0.0;
    }
    width * height
}

fn jaccard_distance(a: &Rect, b: &Rect) -> f64 {
    let area_a = a.width * a.height;
    let area_b = b.width * b.height;
    if area_a + area_b <= f64::EPSILON {
        return 0.0;
    }
    let overlap_area = intersection_area(a, b);
    1.0 - overlap_area / (area_a + area_b - overlap_area)
}

fn overlap(a: &Rect, b: &Rect) -> f32 {
    1.0f32 - round_f64_to_f32(jaccard_distance(a, b))
}

/// The indices into `candidates` to keep, best score first, equal scores in input order, scores strictly above `score_threshold`.
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
