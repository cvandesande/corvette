use crate::clusters::{box_inside, cluster_candidates};
use crate::crop::Region;
use crate::grid::{SizeGrid, cluster_region_from_grid, startup_regions};
use crate::regions::cluster_region;
use crate::shape::Shape;

/// Returns the regions Frigate's frame loop shows the detector for one frame: one per object cluster, then one per cluster of the motion boxes outside those regions, then the startup regions; `None` when any step leaves its domain, with no partial list.
#[must_use]
pub fn build_regions(
    frame: Shape,
    floor: usize,
    grid: &SizeGrid,
    object_boxes: &[Region],
    motion_boxes: &[Region],
    motion_enabled: bool,
    startup: bool,
) -> Option<Vec<Region>> {
    let mut regions = Vec::new();
    for cluster in cluster_candidates(frame, floor, object_boxes)? {
        regions.push(cluster_region(frame, floor, &cluster, object_boxes)?);
    }
    if motion_enabled {
        let standalone: Vec<Region> = motion_boxes
            .iter()
            .copied()
            .filter(|b| !regions.iter().any(|region| box_inside(*region, *b)))
            .collect();
        if !standalone.is_empty() {
            for cluster in cluster_candidates(frame, floor, &standalone)? {
                regions.push(cluster_region_from_grid(
                    frame,
                    floor,
                    &cluster,
                    &standalone,
                    grid,
                )?);
            }
        }
    }
    if startup {
        regions.extend(startup_regions(frame, floor, grid)?);
    }
    Some(regions)
}

/// A camera's region composition state: the floor and size grid, plus the startup scan that runs on the first successful build; a failed build leaves the scan pending.
#[derive(Debug)]
pub struct RegionBuilder {
    floor: usize,
    grid: SizeGrid,
    startup_pending: bool,
}

impl RegionBuilder {
    #[must_use]
    pub const fn new(floor: usize, grid: SizeGrid) -> Self {
        Self {
            floor,
            grid,
            startup_pending: true,
        }
    }

    pub fn build(
        &mut self,
        frame: Shape,
        object_boxes: &[Region],
        motion_boxes: &[Region],
        motion_enabled: bool,
    ) -> Option<Vec<Region>> {
        let built = build_regions(
            frame,
            self.floor,
            &self.grid,
            object_boxes,
            motion_boxes,
            motion_enabled,
            self.startup_pending,
        );
        if built.is_some() {
            self.startup_pending = false;
        }
        built
    }
}
