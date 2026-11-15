use crate::decision::trunc_to_whole;
use crate::plane::Image;
use crate::shape::Shape;

const SHIFT: u32 = 16;

type Point = (i64, i64);

/// Fill each relative polygon into a frame-sized picture of 255 bytes, zeroing the bytes it covers: `OpenCV`'s fill rule.
///
/// Returns `None` when a side of the frame exceeds 65,535, a polygon has fewer than three points, a coordinate is not finite or outside `0.0..=1.0`, or an edge clip divides by zero.
#[must_use]
pub fn fill_polygons(frame: Shape, polygons: &[Vec<(f64, f64)>]) -> Option<Image> {
    let width = u16::try_from(frame.width()).ok()?;
    let height = u16::try_from(frame.height()).ok()?;
    let (x_scale, y_scale) = (f64::from(width), f64::from(height));
    let mut out = vec![0xFFu8; frame.pixels()];
    for polygon in polygons {
        if polygon.len() < 3 {
            return None;
        }
        let mut points = Vec::with_capacity(polygon.len());
        for &(fx, fy) in polygon {
            if !(0.0..=1.0).contains(&fx) || !(0.0..=1.0).contains(&fy) {
                return None;
            }
            points.push((
                widened(trunc_to_whole(fx * x_scale)),
                widened(trunc_to_whole(fy * y_scale)),
            ));
        }
        fill_one(&points, i64::from(width), i64::from(height), &mut out)?;
    }
    Image::new(out, frame)
}

fn widened(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

struct Edge {
    y0: i64,
    y1: i64,
    x: i64,
    dx: i64,
}

struct Clipped {
    fits: bool,
    p0: Point,
    p1: Point,
}

fn fill_one(points: &[Point], width: i64, height: i64, out: &mut [u8]) -> Option<()> {
    let mut edges: Vec<Edge> = Vec::with_capacity(points.len());
    for index in 0..points.len() {
        let p0 = points[index];
        let p1 = points[(index + 1) % points.len()];
        let (t0, t1) = if inside(p0, width, height) && inside(p1, width, height) {
            draw_line(p0, p1, width, out);
            (p0, p1)
        } else {
            let cut = clip(p0, p1, width, height)?;
            if cut.fits {
                draw_line(cut.p0, cut.p1, width, out);
                (cut.p0, cut.p1)
            } else {
                (p0, p1)
            }
        };
        if p0.1 == p1.1 {
            continue;
        }
        let (py0, py1) = if t0.1 == t1.1 {
            (p0.1, p1.1)
        } else {
            (t0.1, t1.1)
        };
        let dx = ((t1.0 - t0.0) << SHIFT) / (py1 - py0);
        edges.push(if p0.1 < p1.1 {
            Edge {
                y0: p0.1,
                y1: p1.1,
                x: (t0.0 << SHIFT) + (p0.1 - py0) * dx,
                dx,
            }
        } else {
            Edge {
                y0: p1.1,
                y1: p0.1,
                x: (t1.0 << SHIFT) + (p1.1 - py1) * dx,
                dx,
            }
        });
    }
    if edges.len() < 2 {
        return Some(());
    }
    let first = edges.iter().map(|edge| edge.y0).min().unwrap_or(0);
    let last = edges.iter().map(|edge| edge.y1).max().unwrap_or(0) - 1;
    let mut xs: Vec<i64> = Vec::with_capacity(edges.len());
    for y in first..=last.min(height - 1) {
        xs.clear();
        for edge in &edges {
            if edge.y0 <= y && y < edge.y1 {
                xs.push(edge.x + (y - edge.y0) * edge.dx);
            }
        }
        xs.sort_unstable();
        for pair in xs.chunks_exact(2) {
            let left = -((-pair[0]) >> SHIFT);
            let right = pair[1] >> SHIFT;
            if right < 0 || left > width - 1 {
                continue;
            }
            let lo = usize::try_from(y * width + left.max(0)).unwrap_or(usize::MAX);
            let hi = usize::try_from(y * width + right.min(width - 1)).unwrap_or(usize::MAX);
            if let Some(run) = out.get_mut(lo..=hi) {
                run.fill(0);
            }
        }
    }
    Some(())
}

fn inside(point: Point, width: i64, height: i64) -> bool {
    (0..width).contains(&point.0) && (0..height).contains(&point.1)
}

fn clip(p0: Point, p1: Point, width: i64, height: i64) -> Option<Clipped> {
    let right = width - 1;
    let bottom = height - 1;
    let (mut x1, mut y1) = p0;
    let (mut x2, mut y2) = p1;
    let mut c1 = out_code(x1, y1, right, bottom);
    let mut c2 = out_code(x2, y2, right, bottom);
    if c1 & c2 == 0 && c1 | c2 != 0 {
        if c1 & 0xC != 0 {
            let stop = if c1 < 8 { 0 } else { bottom };
            x1 += trunc_div((stop - y1) * (x2 - x1), y2 - y1)?;
            y1 = stop;
            c1 = side_code(x1, right);
        }
        if c2 & 0xC != 0 {
            let stop = if c2 < 8 { 0 } else { bottom };
            x2 += trunc_div((stop - y2) * (x2 - x1), y2 - y1)?;
            y2 = stop;
            c2 = side_code(x2, right);
        }
        if c1 & c2 == 0 && c1 | c2 != 0 {
            if c1 != 0 {
                let stop = if c1 == 1 { 0 } else { right };
                y1 += trunc_div((stop - x1) * (y2 - y1), x2 - x1)?;
                x1 = stop;
                c1 = 0;
            }
            if c2 != 0 {
                let stop = if c2 == 1 { 0 } else { right };
                y2 += trunc_div((stop - x2) * (y2 - y1), x2 - x1)?;
                x2 = stop;
                c2 = 0;
            }
        }
    }
    Some(Clipped {
        fits: c1 | c2 == 0,
        p0: (x1, y1),
        p1: (x2, y2),
    })
}

fn out_code(x: i64, y: i64, right: i64, bottom: i64) -> i64 {
    i64::from(x < 0) + 2 * i64::from(x > right) + 4 * i64::from(y < 0) + 8 * i64::from(y > bottom)
}

fn side_code(x: i64, right: i64) -> i64 {
    i64::from(x < 0) + 2 * i64::from(x > right)
}

fn trunc_div(numerator: i64, denominator: i64) -> Option<i64> {
    (denominator != 0).then(|| numerator / denominator)
}

fn draw_line(from: Point, to: Point, width: i64, out: &mut [u8]) {
    for (x, y) in line_pixels(from, to) {
        let at = usize::try_from(y * width + x).unwrap_or(usize::MAX);
        if let Some(byte) = out.get_mut(at) {
            *byte = 0;
        }
    }
}

fn line_pixels(from: Point, to: Point) -> Vec<Point> {
    let (x1, y1, x2, y2) = if to.0 < from.0 {
        (to.0, to.1, from.0, from.1)
    } else {
        (from.0, from.1, to.0, to.1)
    };
    let dx = x2 - x1;
    let dy = (y2 - y1).abs();
    let step = if y2 >= y1 { 1 } else { -1 };
    if dx == 0 {
        return (0..=dy).map(|k| (x1, y1 + step * k)).collect();
    }
    if dy == 0 {
        return (0..=dx).map(|k| (x1 + k, y1)).collect();
    }
    if dx >= dy {
        (0..=dx)
            .map(|k| (x1 + k, y1 + step * round_up(2 * k * dy - dx, 2 * dx)))
            .collect()
    } else {
        (0..=dy)
            .map(|k| (x1 + round_up(2 * k * dx - dy, 2 * dy), y1 + step * k))
            .collect()
    }
}

const fn round_up(numerator: i64, denominator: i64) -> i64 {
    let whole = numerator / denominator;
    if numerator % denominator > 0 {
        whole + 1
    } else {
        whole
    }
}
