use crate::plane::Plane;

/// The bounding rectangle of one outer blob and twice the area its boundary encloses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contour {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub twice_area: u64,
}

/// Returns the outer contour of every blob open to the background, last discovered first.
#[must_use]
pub fn external_contours(mask: Plane<'_>) -> Vec<Contour> {
    let shape = mask.shape();
    let (Some(stride), Some(rows)) = (shape.width().checked_add(2), shape.height().checked_add(2))
    else {
        return Vec::new();
    };
    let Some(cells) = stride.checked_mul(rows) else {
        return Vec::new();
    };
    let mut grid = vec![0u8; cells];
    for (row, line) in mask.data().chunks(shape.width()).enumerate() {
        let base = (row + 1) * stride + 1;
        for (col, &byte) in line.iter().enumerate() {
            grid[base + col] = u8::from(byte != 0);
        }
    }
    let outside = flood_outside(&grid, cells, stride);
    // steps for directions 0 east, 1 north-east, 2 north, 3 north-west, 4 west, 5 south-west, 6 south, 7 south-east
    let add: [usize; 8] = [1, 1, 0, 0, 0, stride, stride, stride + 1];
    let sub: [usize; 8] = [0, stride, stride, stride + 1, 1, 1, 0, 0];
    let mut contours = Vec::new();
    let mut seen = vec![false; cells];
    for row in 1..=shape.height() {
        for col in 1..=shape.width() {
            let start = row * stride + col;
            if grid[start] == 0 || seen[start] {
                continue;
            }
            let mut blob = Blob {
                min_row: row,
                max_row: row,
                min_col: col,
                max_col: col,
                open_to_outside: false,
            };
            seen[start] = true;
            let mut stack = vec![start];
            while let Some(here) = stack.pop() {
                let here_row = here / stride;
                let here_col = here % stride;
                blob.min_row = blob.min_row.min(here_row);
                blob.max_row = blob.max_row.max(here_row);
                blob.min_col = blob.min_col.min(here_col);
                blob.max_col = blob.max_col.max(here_col);
                for direction in 0..8 {
                    let next = here + add[direction] - sub[direction];
                    if grid[next] == 1 {
                        if !seen[next] {
                            seen[next] = true;
                            stack.push(next);
                        }
                    } else if outside[next] {
                        blob.open_to_outside = true;
                    }
                }
            }
            if blob.open_to_outside {
                contours.push(Contour {
                    x: blob.min_col - 1,
                    y: blob.min_row - 1,
                    width: blob.max_col - blob.min_col + 1,
                    height: blob.max_row - blob.min_row + 1,
                    twice_area: trace(&grid, cells, stride, start, &add, &sub),
                });
            }
        }
    }
    contours.reverse();
    contours
}

struct Blob {
    min_row: usize,
    max_row: usize,
    min_col: usize,
    max_col: usize,
    open_to_outside: bool,
}

fn flood_outside(grid: &[u8], cells: usize, stride: usize) -> Vec<bool> {
    let mut outside = vec![false; cells];
    outside[0] = true;
    let mut stack = vec![0usize];
    while let Some(here) = stack.pop() {
        let col = here % stride;
        for candidate in [
            here.checked_sub(stride),
            here.checked_add(stride).filter(|next| *next < cells),
            if col > 0 { Some(here - 1) } else { None },
            if col + 1 < stride {
                Some(here + 1)
            } else {
                None
            },
        ] {
            if let Some(next) = candidate
                && grid[next] == 0
                && !outside[next]
            {
                outside[next] = true;
                stack.push(next);
            }
        }
    }
    outside
}

fn trace(
    grid: &[u8],
    cells: usize,
    stride: usize,
    start: usize,
    add: &[usize; 8],
    sub: &[usize; 8],
) -> u64 {
    let mut first_direction = None;
    let mut direction = 4usize;
    for _ in 0..7 {
        direction = direction.wrapping_sub(1) & 7;
        let probe = start + add[direction] - sub[direction];
        if grid[probe] == 1 {
            first_direction = Some(direction);
            break;
        }
    }
    let Some(first) = first_direction else {
        return 0;
    };
    let partner = start + add[first] - sub[first];
    let bound = 8usize
        .checked_mul(cells)
        .and_then(|steps| steps.checked_add(64))
        .unwrap_or(usize::MAX);
    let mut here = start;
    let mut direction = first;
    let mut twice: i128 = 0;
    let mut steps = 0usize;
    loop {
        steps += 1;
        if steps > bound {
            break;
        }
        let mut search = direction;
        let mut next = here;
        while search < 15 {
            search += 1;
            next = here + add[search & 7] - sub[search & 7];
            if grid[next] == 1 {
                break;
            }
        }
        direction = search & 7;
        twice += crossing(here, next, stride);
        if next == start && here == partner {
            break;
        }
        here = next;
        direction = (direction + 4) & 7;
    }
    u64::try_from(twice.abs()).unwrap_or(u64::MAX)
}

fn crossing(from: usize, to: usize, stride: usize) -> i128 {
    let x1 = i128::try_from(from % stride).unwrap_or_default();
    let y1 = i128::try_from(from / stride).unwrap_or_default();
    let x2 = i128::try_from(to % stride).unwrap_or_default();
    let y2 = i128::try_from(to / stride).unwrap_or_default();
    x1 * y2 - x2 * y1
}
