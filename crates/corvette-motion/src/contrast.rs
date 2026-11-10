use crate::blur::{level_table, truncated};
use crate::plane::Plane;

const LOWER_QUANTILE: f64 = 4.0_f64 / 100.0;
const UPPER_QUANTILE: f64 = 96.0_f64 / 100.0;

/// The 4th and 96th percentile of the picture's values, interpolated linearly between the two nearest sorted values and truncated to 8 bits; `None` exactly when the pixel count exceeds `u32::MAX`.
#[must_use]
pub fn contrast_bounds(src: Plane<'_>) -> Option<(u8, u8)> {
    let count = u32::try_from(src.shape().pixels()).ok()?;
    if count == 1 {
        let only = *src.data().first()?;
        return Some((only, only));
    }
    let mut counts = [0_u32; 256];
    for byte in src.data() {
        counts[usize::from(*byte)] += 1;
    }
    let levels = level_table();
    let lower = percentile(&counts, count, LOWER_QUANTILE);
    let upper = percentile(&counts, count, UPPER_QUANTILE);
    Some((truncated(&levels, lower), truncated(&levels, upper)))
}

fn percentile(counts: &[u32; 256], count: u32, quantile: f64) -> f64 {
    let virtual_index = f64::from(count - 1) * quantile;
    let k = floor_below(virtual_index, count - 1);
    let (a, b) = sorted_pair(counts, k, k + 1);
    let gamma = virtual_index - f64::from(k);
    let diff = f64::from(b) - f64::from(a);
    // The two expressions are algebraically equal but round differently, so the halfway choice is evaluated as given.
    if gamma >= 0.5 {
        let scaled = diff * (1.0 - gamma);
        f64::from(b) - scaled
    } else {
        let scaled = diff * gamma;
        f64::from(a) + scaled
    }
}

/// The largest k in `0..=count_minus_one - 1`; the caller gives a `virtual_index` of at least 0 and below `count_minus_one`.
fn floor_below(virtual_index: f64, count_minus_one: u32) -> u32 {
    let mut low = 0_u32;
    let mut high = count_minus_one;
    while low < high {
        let mid = low + (high - low) / 2;
        if f64::from(mid) <= virtual_index {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low - 1
}

fn sorted_pair(counts: &[u32; 256], lower: u32, upper: u32) -> (u8, u8) {
    let mut seen = 0_u32;
    let mut pair = (0_u8, 0_u8);
    let mut found_lower = false;
    for (value, count) in counts.iter().enumerate() {
        seen += count;
        if !found_lower && seen > lower {
            pair.0 = u8::try_from(value).unwrap_or(255);
            found_lower = true;
        }
        if seen > upper {
            pair.1 = u8::try_from(value).unwrap_or(255);
            break;
        }
    }
    pair
}
