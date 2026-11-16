use crate::crop::Region;
use crate::regions::{cluster_region, truncated, within_domain};
use crate::shape::Shape;

const AREA_RATIO: f64 = 0.05;
const REGION_FRACTION: f64 = 0.1;
const DIST_SPREAD: f64 = 1.1;

/// Frigate's `box_inside`: `inner` inside `outer`, edges inclusive.
#[must_use]
pub const fn box_inside(outer: Region, inner: Region) -> bool {
    inner.x1 >= outer.x1 && inner.y1 >= outer.y1 && inner.x2 <= outer.x2 && inner.y2 <= outer.y2
}

/// Frigate's `area`: `(x2 - x1 + 1) * (y2 - y1 + 1)`; `None` off the `regions` domain.
#[must_use]
pub fn box_area(b: Region) -> Option<i64> {
    if !within_domain(i64::from(b.x1))
        || !within_domain(i64::from(b.y1))
        || !within_domain(i64::from(b.x2))
        || !within_domain(i64::from(b.y2))
    {
        return None;
    }
    let width = i64::from(b.x2)
        .checked_sub(i64::from(b.x1))?
        .checked_add(1)?;
    let height = i64::from(b.y2)
        .checked_sub(i64::from(b.y1))?
        .checked_add(1)?;
    width.checked_mul(height)
}

/// Frigate's `get_cluster_boundary`; `None` off the `regions` domain or when a step does not fit.
#[must_use]
pub fn cluster_boundary(b: Region, floor: usize) -> Option<Region> {
    let floor_size = i64::try_from(floor).ok()?;
    if !within_domain(floor_size)
        || !within_domain(i64::from(b.x1))
        || !within_domain(i64::from(b.y1))
        || !within_domain(i64::from(b.x2))
        || !within_domain(i64::from(b.y2))
    {
        return None;
    }
    let box_width = b.x2.checked_sub(b.x1)?;
    let box_height = b.y2.checked_sub(b.y1)?;
    let area_magnitude = (f64::from(box_width) * f64::from(box_height)).abs();
    let max_region_area = area_magnitude / REGION_FRACTION;
    let floor_width = i32::try_from(floor).ok()?;
    let root_size = truncated(max_region_area.sqrt())?;
    let max_region_size = root_size.max(floor_width);
    let half_width = f64::from(box_width) / 2.0;
    let half_height = f64::from(box_height) / 2.0;
    let spread_width = half_width * DIST_SPREAD;
    let spread_height = half_height * DIST_SPREAD;
    let reach_width = truncated_signed(f64::from(max_region_size) - spread_width)?;
    let reach_height = truncated_signed(f64::from(max_region_size) - spread_height)?;
    let col_centre = half_width + f64::from(b.x1);
    let row_centre = half_height + f64::from(b.y1);
    let left = col_centre - f64::from(reach_width);
    let right = col_centre + f64::from(reach_width);
    let top = row_centre - f64::from(reach_height);
    let bottom = row_centre + f64::from(reach_height);
    Some(Region {
        x1: truncated_signed(left)?,
        y1: truncated_signed(top)?,
        x2: truncated_signed(right)?,
        y2: truncated_signed(bottom)?,
    })
}

/// Frigate's `get_cluster_candidates`, its clusters in first-member order.
#[must_use]
pub fn cluster_candidates(frame: Shape, floor: usize, boxes: &[Region]) -> Option<Vec<Vec<usize>>> {
    let frame_width = i64::try_from(frame.width()).ok()?;
    let frame_height = i64::try_from(frame.height()).ok()?;
    let floor_size = i64::try_from(floor).ok()?;
    if !within_domain(frame_width) || !within_domain(frame_height) || !within_domain(floor_size) {
        return None;
    }
    for b in boxes {
        if !within_domain(i64::from(b.x1))
            || !within_domain(i64::from(b.y1))
            || !within_domain(i64::from(b.x2))
            || !within_domain(i64::from(b.y2))
        {
            return None;
        }
    }
    let mut used: Vec<bool> = vec![false; boxes.len()];
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for (seed, seed_box) in boxes.iter().enumerate() {
        if used.get(seed).copied().unwrap_or(false) {
            continue;
        }
        let boundary = cluster_boundary(*seed_box, floor)?;
        let mut cluster = vec![seed];
        *used.get_mut(seed)? = true;
        for (candidate, candidate_box) in boxes.iter().enumerate() {
            if used.get(candidate).copied().unwrap_or(false)
                || !box_inside(boundary, *candidate_box)
            {
                continue;
            }
            cluster.push(candidate);
            if joins(frame, floor, &cluster, boxes)? {
                *used.get_mut(candidate)? = true;
            } else {
                cluster.pop();
            }
        }
        clusters.push(cluster);
    }
    Some(clusters)
}

fn joins(frame: Shape, floor: usize, potential: &[usize], boxes: &[Region]) -> Option<bool> {
    let region = cluster_region(frame, floor, potential, boxes)?;
    let beyond_floor = i64::from(region.x2) - i64::from(region.x1) > i64::try_from(floor).ok()?;
    if !beyond_floor {
        return Some(true);
    }
    let region_area = f64_area(region)?;
    for &index in potential {
        let member = boxes.get(index)?;
        if f64_area(*member)? / region_area < AREA_RATIO {
            return Some(false);
        }
    }
    Some(true)
}

fn f64_area(b: Region) -> Option<f64> {
    let width = i32::try_from(i64::from(b.x2) - i64::from(b.x1) + 1).ok()?;
    let height = i32::try_from(i64::from(b.y2) - i64::from(b.y1) + 1).ok()?;
    Some(f64::from(width) * f64::from(height))
}

fn truncated_signed(value: f64) -> Option<i32> {
    if value < 0.0 {
        truncated(-value)?.checked_neg()
    } else {
        truncated(value)
    }
}
