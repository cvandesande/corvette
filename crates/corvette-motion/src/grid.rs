use crate::crop::Region;
use crate::regions::{
    Bounds, Margin, bounding_region, calculate_region, calculate_region_f64, truncated,
};
use crate::shape::Shape;

pub(crate) const GRID_SIZE: usize = 8;
const GRID_SIZE_F64: f64 = 8.0;

/// One cell of the size grid: the sample count and the mean relative width and standard deviation of the region sizes seen around one place in the picture; `samples` 0 marks an empty cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeCell {
    pub samples: usize,
    pub mean: f64,
    pub std_dev: f64,
}

impl SizeCell {
    pub const EMPTY: Self = Self {
        samples: 0,
        mean: 0.0,
        std_dev: 0.0,
    };
}

/// The grid of typical detection sizes a camera learns, `GRID_SIZE` by `GRID_SIZE` cells indexed `[x][y]`, empty until filled.
#[derive(Clone, Debug, PartialEq)]
pub struct SizeGrid {
    cells: [[SizeCell; GRID_SIZE]; GRID_SIZE],
}

impl Default for SizeGrid {
    fn default() -> Self {
        Self::empty()
    }
}

impl SizeGrid {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            cells: [[SizeCell::EMPTY; GRID_SIZE]; GRID_SIZE],
        }
    }

    /// A grid seeded for the startup scan: the four quadrant cells (2,2),(5,2),(2,5),(5,5) and the four edge-centre cells (3,0),(3,7),(0,3),(7,3) each hold one sample of a `floor`-wide region relative to `frame`, every other cell stays empty.
    #[must_use]
    pub fn seeded(frame: Shape, floor: usize) -> Option<Self> {
        // The mean is frame-width-relative like the learned sizes, so `startup_regions` sizes each region to exactly `floor`.
        let mean =
            f64::from(u32::try_from(floor).ok()?) / f64::from(u32::try_from(frame.width()).ok()?);
        let cell = SizeCell {
            samples: 1,
            mean,
            std_dev: 0.0,
        };
        let mut grid = Self::empty();
        for (x, y) in [
            (2, 2),
            (5, 2),
            (2, 5),
            (5, 5),
            (3, 0),
            (3, 7),
            (0, 3),
            (7, 3),
        ] {
            grid.set(x, y, cell);
        }
        Some(grid)
    }

    pub const fn set(&mut self, x: usize, y: usize, cell: SizeCell) -> bool {
        if x >= GRID_SIZE || y >= GRID_SIZE {
            return false;
        }
        self.cells[x][y] = cell;
        true
    }

    #[must_use]
    pub fn cell(&self, x: usize, y: usize) -> Option<&SizeCell> {
        self.cells.get(x)?.get(y)
    }
}

/// Returns the motion region around `bounds`, grown to the cell's usual size when the region is smaller than it; `None` when the frame, the floor or the bounds leave `calculate_region`'s domain.
#[must_use]
pub fn region_from_grid(
    frame: Shape,
    bounds: Region,
    floor: usize,
    grid: &SizeGrid,
) -> Option<Region> {
    let plain = calculate_region(frame, bounds, floor, Margin::Motion)?;
    let frame_width = i32::try_from(frame.width()).ok()?;
    let frame_height = i32::try_from(frame.height()).ok()?;
    let span_x = plain.x2.min(frame_width) - plain.x1;
    let centroid_x = f64::from(plain.x1) + f64::from(span_x) / 2.0;
    let span_y = plain.y2.min(frame_height) - plain.y1;
    let centroid_y = f64::from(plain.y1) + f64::from(span_y) / 2.0;
    let width_f64 = f64::from(frame_width);
    let height_f64 = f64::from(frame_height);
    let centred_x = centroid_x / width_f64;
    let scaled_x = centred_x * GRID_SIZE_F64;
    let grid_x = usize::try_from(truncated(scaled_x)?).ok()?;
    let centred_y = centroid_y / height_f64;
    let scaled_y = centred_y * GRID_SIZE_F64;
    let grid_y = usize::try_from(truncated(scaled_y)?).ok()?;
    let cell = grid.cell(grid_x, grid_y)?;
    if cell.samples == 0 {
        return Some(plain);
    }
    let calc_size = f64::from(plain.x2 - plain.x1) / width_f64;
    let low = cell.mean - cell.std_dev;
    let high = cell.mean + cell.std_dev;
    if (low..=high).contains(&calc_size) {
        return Some(plain);
    }
    if calc_size > high {
        return Some(plain);
    }
    let size = cell.mean * width_f64;
    let half = size / 2.0;
    let left_x = centroid_x - half;
    let left_y = centroid_y - half;
    let right_x = centroid_x + half;
    let right_y = centroid_y + half;
    calculate_region_f64(
        frame,
        Bounds {
            x1: 0.0_f64.max(left_x),
            y1: 0.0_f64.max(left_y),
            x2: width_f64.min(right_x),
            y2: height_f64.min(right_y),
        },
        floor,
        Margin::Motion,
    )
}

/// Returns the grid-adjusted region around the cluster's bounding box; `None` when an index does not address `boxes` or the frame, the floor or a box leave `calculate_region`'s domain.
#[must_use]
pub fn cluster_region_from_grid(
    frame: Shape,
    floor: usize,
    cluster: &[usize],
    boxes: &[Region],
    grid: &SizeGrid,
) -> Option<Region> {
    let bounds = bounding_region(frame, cluster, boxes)?;
    region_from_grid(frame, bounds, floor, grid)
}

/// Returns one region per grid cell with the most samples, at most `GRID_SIZE` of them, stopping at the first empty cell; `None` when the frame or the floor leave `calculate_region`'s domain or a float bound does not fit.
#[must_use]
pub fn startup_regions(frame: Shape, floor: usize, grid: &SizeGrid) -> Option<Vec<Region>> {
    let width_f64 = f64::from(u32::try_from(frame.width()).ok()?);
    let height_f64 = f64::from(u32::try_from(frame.height()).ok()?);
    let mut cells: Vec<(u8, u8, SizeCell)> = Vec::with_capacity(GRID_SIZE * GRID_SIZE);
    for (x, row) in grid.cells.iter().enumerate() {
        for (y, cell) in row.iter().enumerate() {
            cells.push((u8::try_from(x).ok()?, u8::try_from(y).ok()?, *cell));
        }
    }
    cells.sort_by_key(|&(_, _, cell)| std::cmp::Reverse(cell.samples));
    let cell_width = width_f64 / GRID_SIZE_F64;
    let cell_height = height_f64 / GRID_SIZE_F64;
    let mut regions = Vec::new();
    for (x, y, cell) in cells.into_iter().take(GRID_SIZE) {
        if cell.samples == 0 {
            break;
        }
        let shift_x = 0.5 + f64::from(x);
        let centre_x = cell_width * shift_x;
        let shift_y = 0.5 + f64::from(y);
        let centre_y = cell_height * shift_y;
        let size = cell.mean * width_f64;
        let half = size / 2.0;
        regions.push(calculate_region_f64(
            frame,
            Bounds {
                x1: centre_x - half,
                y1: centre_y - half,
                x2: centre_x + half,
                y2: centre_y + half,
            },
            floor,
            Margin::Exact,
        )?);
    }
    Some(regions)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_SHAPES: [(usize, usize); 2] = [(1920, 1080), (640, 360)]; // 16:9 main stream, small substream

    #[test]
    fn seeded_grid_yields_eight_in_frame_floor_sized_regions() {
        for (width, height) in REAL_SHAPES {
            let frame = Shape::new(width, height).expect("frame shape");
            let grid = SizeGrid::seeded(frame, 320).expect("seed");
            let regions = startup_regions(frame, 320, &grid).expect("startup regions");
            assert_eq!(regions.len(), 8, "shape {width}x{height}");
            for region in &regions {
                assert!(region.x1 >= 0 && region.y1 >= 0, "shape {width}x{height}");
                assert!(
                    region.x2 <= i32::try_from(width).expect("width fits")
                        && region.y2 <= i32::try_from(height).expect("height fits"),
                    "shape {width}x{height}"
                );
                assert_eq!(region.x2 - region.x1, 320, "shape {width}x{height}");
                assert_eq!(region.y2 - region.y1, 320, "shape {width}x{height}");
            }
        }
    }

    #[test]
    fn seeded_grid_yields_no_duplicate_regions() {
        for (width, height) in REAL_SHAPES {
            let frame = Shape::new(width, height).expect("frame shape");
            let grid = SizeGrid::seeded(frame, 320).expect("seed");
            let regions = startup_regions(frame, 320, &grid).expect("startup regions");
            for (index, left) in regions.iter().enumerate() {
                for right in &regions[index + 1..] {
                    assert_ne!(left, right, "shape {width}x{height}");
                }
            }
        }
    }

    #[test]
    fn seeded_grid_yields_the_same_regions_on_every_call() {
        let frame = Shape::new(1920, 1080).expect("frame shape");
        let grid = SizeGrid::seeded(frame, 320).expect("seed");
        let first = startup_regions(frame, 320, &grid).expect("first call");
        let second = startup_regions(frame, 320, &grid).expect("second call");
        assert_eq!(first, second);
    }
}
