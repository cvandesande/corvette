use crate::crop::Region;
use crate::shape::Shape;

const DOMAIN_LIMIT: i64 = 1 << 20;

/// The multiplier Frigate applies to the longest edge: `1.35` around a cluster, `2` for motion, `1` for the startup regions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Margin {
    Cluster,
    Exact,
    Motion,
}

impl Margin {
    const fn multiplier(self) -> f64 {
        match self {
            Self::Cluster => 1.35,
            Self::Exact => 1.0,
            Self::Motion => 2.0,
        }
    }
}

#[must_use]
pub fn min_region_size(model: Shape) -> usize {
    let largest = model.width().max(model.height());
    if largest >= 320 {
        return 320;
    }
    if largest.is_multiple_of(4) {
        return largest;
    }
    largest.div_ceil(4) * 4
}

/// Returns `None` when a frame side, the floor or a box coordinate has magnitude above `1 << 20`.
#[must_use]
pub fn calculate_region(
    frame: Shape,
    bounds: Region,
    floor: usize,
    margin: Margin,
) -> Option<Region> {
    let frame_width = i64::try_from(frame.width()).ok()?;
    let frame_height = i64::try_from(frame.height()).ok()?;
    let model_size = i64::try_from(floor).ok()?;
    let xmin = i64::from(bounds.x1);
    let ymin = i64::from(bounds.y1);
    let xmax = i64::from(bounds.x2);
    let ymax = i64::from(bounds.y2);
    if frame_width > DOMAIN_LIMIT
        || frame_height > DOMAIN_LIMIT
        || model_size > DOMAIN_LIMIT
        || !within_domain(xmin)
        || !within_domain(ymin)
        || !within_domain(xmax)
        || !within_domain(ymax)
    {
        return None;
    }
    let longest = i32::try_from((xmax - xmin).max(ymax - ymin)).ok()?;
    let product = f64::from(longest) * margin.multiplier();
    let quartered = product / 4.0;
    let floored = quartered.floor();
    let sized = floored * 4.0;
    let size = if sized < 0.0 {
        model_size
    } else {
        i64::from(truncated(sized)?).max(model_size)
    };
    let mut x_offset = (xmax + xmin - size) / 2;
    if x_offset < 0 {
        x_offset = 0;
    } else if x_offset > frame_width - size {
        x_offset = (frame_width - size).max(0);
    }
    let mut y_offset = (ymax + ymin - size) / 2;
    if y_offset < 0 {
        y_offset = 0;
    } else if y_offset > frame_height - size {
        y_offset = (frame_height - size).max(0);
    }
    Some(Region {
        x1: i32::try_from(x_offset).ok()?,
        y1: i32::try_from(y_offset).ok()?,
        x2: i32::try_from(x_offset + size).ok()?,
        y2: i32::try_from(y_offset + size).ok()?,
    })
}

/// A Frigate bounding box as four floats; the box may be inverted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// Returns `None` when a frame side or the floor exceeds `1 << 20`, or a bound is non-finite or above `1 << 20` in magnitude.
#[must_use]
pub fn calculate_region_f64(
    frame: Shape,
    bounds: Bounds,
    floor: usize,
    margin: Margin,
) -> Option<Region> {
    let frame_width = i64::try_from(frame.width()).ok()?;
    let frame_height = i64::try_from(frame.height()).ok()?;
    let floor_size = i32::try_from(floor).ok()?;
    if frame_width > DOMAIN_LIMIT
        || frame_height > DOMAIN_LIMIT
        || i64::from(floor_size) > DOMAIN_LIMIT
    {
        return None;
    }
    let bound_limit = f64::from(1_u32 << 20);
    if !bounds.x1.is_finite()
        || bounds.x1.abs() > bound_limit
        || !bounds.y1.is_finite()
        || bounds.y1.abs() > bound_limit
        || !bounds.x2.is_finite()
        || bounds.x2.abs() > bound_limit
        || !bounds.y2.is_finite()
        || bounds.y2.abs() > bound_limit
    {
        return None;
    }
    let x_extent = bounds.x2 - bounds.x1;
    let y_extent = bounds.y2 - bounds.y1;
    let longest = f64::max(x_extent, y_extent);
    let scaled = longest * margin.multiplier();
    let quartered = scaled / 4.0;
    let floored = quartered.floor();
    let sized = floored * 4.0;
    let size = truncated_signed(sized)?.max(floor_size);
    let half_size = f64::from(size) / 2.0;
    let x_half = x_extent / 2.0;
    let x_mid = x_half + bounds.x1;
    let x_raw = x_mid - half_size;
    let mut x_offset = i64::from(truncated_signed(x_raw)?);
    if x_offset < 0 {
        x_offset = 0;
    } else if x_offset > frame_width - i64::from(size) {
        x_offset = (frame_width - i64::from(size)).max(0);
    }
    let y_half = y_extent / 2.0;
    let y_mid = y_half + bounds.y1;
    let y_raw = y_mid - half_size;
    let mut y_offset = i64::from(truncated_signed(y_raw)?);
    if y_offset < 0 {
        y_offset = 0;
    } else if y_offset > frame_height - i64::from(size) {
        y_offset = (frame_height - i64::from(size)).max(0);
    }
    Some(Region {
        x1: i32::try_from(x_offset).ok()?,
        y1: i32::try_from(y_offset).ok()?,
        x2: i32::try_from(x_offset + i64::from(size)).ok()?,
        y2: i32::try_from(y_offset + i64::from(size)).ok()?,
    })
}

/// Returns `None` when an index does not address `boxes` or the frame or floor leave `calculate_region`'s domain.
#[must_use]
pub fn cluster_region(
    frame: Shape,
    floor: usize,
    cluster: &[usize],
    boxes: &[Region],
) -> Option<Region> {
    let mut min_x = i64::try_from(frame.width()).ok()?;
    let mut min_y = i64::try_from(frame.height()).ok()?;
    let mut max_x: i64 = 0;
    let mut max_y: i64 = 0;
    for &index in cluster {
        let picked = boxes.get(index)?;
        min_x = min_x.min(i64::from(picked.x1));
        min_y = min_y.min(i64::from(picked.y1));
        max_x = max_x.max(i64::from(picked.x2));
        max_y = max_y.max(i64::from(picked.y2));
    }
    let bounds = Region {
        x1: i32::try_from(min_x).ok()?,
        y1: i32::try_from(min_y).ok()?,
        x2: i32::try_from(max_x).ok()?,
        y2: i32::try_from(max_y).ok()?,
    };
    calculate_region(frame, bounds, floor, Margin::Cluster)
}

pub(crate) const fn within_domain(value: i64) -> bool {
    value.abs() <= DOMAIN_LIMIT
}

pub(crate) fn truncated_signed(value: f64) -> Option<i32> {
    if value < 0.0 {
        truncated(-value)?.checked_neg()
    } else {
        truncated(value)
    }
}

/// Returns `None` unless `value` is finite, non-negative and below `2_147_483_648.0`.
pub(crate) fn truncated(value: f64) -> Option<i32> {
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
