//! Zone membership per object per frame: current zones and zones entered so far.
//!
//! Entry needs inertia consecutive inside frames; outside frames reset; loitering, filters, speed zones out of scope.
//! Rules: `frigate/track/tracked_object.py` 183-298 and `frigate/config/camera/zone.py` 107-171 at v0.17.2.
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    pub name: String,
    pub polygon: Vec<[i32; 2]>,
    pub labels: Vec<String>,
    pub inertia: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneState {
    pub current: Vec<String>,
    pub entered: Vec<String>,
}

#[derive(Debug)]
pub struct ZoneSet {
    zones: Vec<Zone>,
    objects: HashMap<u64, ObjectZones>,
}

#[derive(Debug)]
struct ObjectZones {
    counts: Vec<u32>,
    entered: Vec<String>,
}

impl ZoneSet {
    #[must_use]
    pub fn new(zones: Vec<Zone>) -> Self {
        Self {
            zones,
            objects: HashMap::new(),
        }
    }

    pub fn observe(&mut self, id: u64, label: &str, centroid_x: i32, bottom: i32) -> ZoneState {
        let zone_count = self.zones.len();
        let object = self.objects.entry(id).or_insert_with(|| ObjectZones {
            counts: vec![0; zone_count],
            entered: Vec::new(),
        });
        let mut current: Vec<String> = Vec::new();
        for (zone, count) in self.zones.iter().zip(object.counts.iter_mut()) {
            if !zone.labels.is_empty()
                && !zone.labels.iter().any(|allowed| allowed.as_str() == label)
            {
                continue;
            }
            let inside = point_inside(&zone.polygon, centroid_x, bottom);
            *count = if inside { count.saturating_add(1) } else { 0 };
            if inside && *count >= zone.inertia {
                current.push(zone.name.clone());
                let name = zone.name.clone();
                if !object.entered.iter().any(|seen| seen == &name) {
                    object.entered.push(name);
                }
            }
        }
        ZoneState {
            current,
            entered: object.entered.clone(),
        }
    }

    pub fn forget(&mut self, id: u64) {
        self.objects.remove(&id);
    }
}

#[must_use]
pub fn polygon_from_relative(
    coordinates: &[[f64; 2]],
    width: u32,
    height: u32,
) -> Option<Vec<[i32; 2]>> {
    if coordinates.len() < 3 {
        return None;
    }
    let scale_x = f64::from(width);
    let scale_y = f64::from(height);
    let mut polygon = Vec::with_capacity(coordinates.len());
    for [rx, ry] in coordinates.iter().copied() {
        polygon.push([
            pixel_from_relative(rx, scale_x)?,
            pixel_from_relative(ry, scale_y)?,
        ]);
    }
    Some(polygon)
}

fn pixel_from_relative(relative: f64, scale: f64) -> Option<i32> {
    let truncated = (relative * scale).trunc();
    if !(-2_147_483_648.0..=2_147_483_647.0).contains(&truncated) {
        return None;
    }
    let anchored = truncated.abs() + 4_503_599_627_370_496.0; // 2^52 puts the integer in the mantissa
    let whole = i64::try_from(anchored.to_bits() & 0x000f_ffff_ffff_ffff).ok()?;
    let signed = if truncated < 0.0 { -whole } else { whole };
    i32::try_from(signed).ok()
}

fn point_inside(polygon: &[[i32; 2]], x: i32, y: i32) -> bool {
    let px = i64::from(x);
    let py = i64::from(y);
    let closing = polygon.last().zip(polygon.first());
    let edges = closing
        .into_iter()
        .chain(polygon.iter().zip(polygon.iter().skip(1)));
    let mut inside = false;
    for (a, b) in edges {
        let [ax, ay] = *a;
        let [bx, by] = *b;
        let (ax, ay) = (i64::from(ax), i64::from(ay));
        let (bx, by) = (i64::from(bx), i64::from(by));
        let dx = bx - ax;
        let dy = by - ay;
        let on_edge = dx * (py - ay) == dy * (px - ax)
            && px >= ax.min(bx)
            && px <= ax.max(bx)
            && py >= ay.min(by)
            && py <= ay.max(by);
        if on_edge {
            return true;
        }
        if (ay > py) != (by > py) {
            let left = (px - ax) * dy;
            let crossing = dx * (py - ay);
            inside ^= if dy > 0 {
                left < crossing
            } else {
                left > crossing
            };
        }
    }
    inside
}
