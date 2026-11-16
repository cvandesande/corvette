use crate::plane::{Image, Plane};
use crate::shape::Shape;

const CANVAS_FRAME_FACTOR: i64 = 4;
const CANVAS_MIN_LIMIT: i64 = 1024;

/// A region as Frigate emits it: `x1` and `y1` inclusive, `x2` and `y2` exclusive, `x1` and `y1` may be negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionError {
    FrameShape,
    CanvasTooSmall,
    CanvasTooLarge,
    OutsideFrame,
    WiderThanCanvas,
    CanvasShape,
}

struct Rect {
    y0: i64,
    y1: i64,
    x0: i64,
    x1: i64,
}

impl Rect {
    const fn new(y0: i64, x0: i64, y1: i64, x1: i64) -> Self {
        Self { y0, y1, x0, x1 }
    }

    const fn fits_inside(&self, rows: i64, cols: i64) -> bool {
        self.y0 >= 0
            && self.x0 >= 0
            && self.y1 <= rows
            && self.x1 <= cols
            && self.y1 >= self.y0
            && self.x1 >= self.x0
    }

    fn first_byte(&self, cols: i64, row: i64) -> Option<usize> {
        let byte = (self.y0 + row).checked_mul(cols)?.checked_add(self.x0)?;
        usize::try_from(byte).ok()
    }

    fn width(&self) -> Option<usize> {
        usize::try_from(self.x1 - self.x0).ok()
    }
}

struct Source<'a> {
    data: &'a [u8],
    rows: i64,
    cols: i64,
}

struct Target<'a> {
    data: &'a mut [u8],
    rows: i64,
    cols: i64,
}

struct CopyPlan {
    y_src: Rect,
    u_left_src: Rect,
    u_right_src: Rect,
    v_left_src: Rect,
    v_right_src: Rect,
    y_dst: Rect,
    u_left_dst: Rect,
    u_right_dst: Rect,
    v_left_dst: Rect,
    v_right_dst: Rect,
}

const fn copy_plan(
    clamped: &Rect,
    height: i64,
    cols: i64,
    side: i64,
    over_x: i64,
    over_y: i64,
) -> CopyPlan {
    let uv_width = cols / 2;
    let uv_height = height / 4;
    let uv_x_offset = clamped.x0 / 2;
    let uv_y_offset = clamped.y0 / 4;
    let uv_crop_width = (clamped.x1 - clamped.x0) / 2;
    let uv_crop_height = (clamped.y1 - clamped.y0) / 4;
    let uv_over_x = over_x / 2;
    let uv_over_y = over_y / 4;
    let u_top = height + uv_y_offset;
    let v_top = height + uv_height + uv_y_offset;
    let u_top_dst = side + uv_over_y;
    let v_top_dst = side + side / 4 + uv_over_y;
    CopyPlan {
        y_src: Rect::new(
            clamped.y0,
            clamped.x0,
            clamped.y0 + uv_crop_height * 4,
            clamped.x0 + uv_crop_width * 2,
        ),
        u_left_src: Rect::new(
            u_top,
            uv_x_offset,
            u_top + uv_crop_height,
            uv_x_offset + uv_crop_width,
        ),
        u_right_src: Rect::new(
            u_top,
            uv_width + uv_x_offset,
            u_top + uv_crop_height,
            uv_width + uv_x_offset + uv_crop_width,
        ),
        v_left_src: Rect::new(
            v_top,
            uv_x_offset,
            v_top + uv_crop_height,
            uv_x_offset + uv_crop_width,
        ),
        v_right_src: Rect::new(
            v_top,
            uv_width + uv_x_offset,
            v_top + uv_crop_height,
            uv_width + uv_x_offset + uv_crop_width,
        ),
        y_dst: Rect::new(
            over_y,
            over_x,
            over_y + uv_crop_height * 4,
            over_x + uv_crop_width * 2,
        ),
        u_left_dst: Rect::new(
            u_top_dst,
            uv_over_x,
            u_top_dst + uv_crop_height,
            uv_over_x + uv_crop_width,
        ),
        u_right_dst: Rect::new(
            u_top_dst,
            side / 2 + uv_over_x,
            u_top_dst + uv_crop_height,
            side / 2 + uv_over_x + uv_crop_width,
        ),
        v_left_dst: Rect::new(
            v_top_dst,
            uv_over_x,
            v_top_dst + uv_crop_height,
            uv_over_x + uv_crop_width,
        ),
        v_right_dst: Rect::new(
            v_top_dst,
            side / 2 + uv_over_x,
            v_top_dst + uv_crop_height,
            side / 2 + uv_over_x + uv_crop_width,
        ),
    }
}

fn copy_rect(source: &Source<'_>, src: &Rect, target: &mut Target<'_>, dst: &Rect) -> bool {
    // Row spans copy row by row; a rect outside its buffer fails like numpy's clipped copy.
    if !src.fits_inside(source.rows, source.cols) || !dst.fits_inside(target.rows, target.cols) {
        return false;
    }
    if src.y1 <= src.y0 {
        return true;
    }
    let Some(width) = src.width() else {
        return false;
    };
    for row in 0..src.y1 - src.y0 {
        let (Some(from), Some(to)) = (
            src.first_byte(source.cols, row),
            dst.first_byte(target.cols, row),
        ) else {
            return false;
        };
        let (Some(src_bytes), Some(dst_bytes)) = (
            source.data.get(from..from + width),
            target.data.get_mut(to..to + width),
        ) else {
            return false;
        };
        dst_bytes.copy_from_slice(src_bytes);
    }
    true
}

/// The square canvas that Frigate's `yuv_crop_and_resize` builds for `region` from the packed I420 `frame`: side `size`, rows `size + size / 2`.
///
/// # Errors
/// - `FrameShape` when the frame row count is not a multiple of 3; `CanvasTooSmall` when `size` is below 4 or no such canvas can be built; `CanvasTooLarge` when `size` exceeds four times the frame's longer side (at least 1024).
/// - `OutsideFrame` when the region clipped to the frame has no area; `WiderThanCanvas` when a clipped crop or one of its copies does not fit the canvas.
pub fn crop_pad_i420(frame: Plane<'_>, region: Region) -> Result<Image, RegionError> {
    let cols = i64::try_from(frame.shape().width()).unwrap_or(i64::MAX);
    let rows = i64::try_from(frame.shape().height()).unwrap_or(i64::MAX);
    if rows % 3 != 0 {
        return Err(RegionError::FrameShape);
    }
    let height = rows / 3 * 2;
    let canvas_size = (i64::from(region.y2) - i64::from(region.y1))
        .div_euclid(4)
        .saturating_mul(4);
    if canvas_size < 4 {
        return Err(RegionError::CanvasTooSmall);
    }
    let limit = CANVAS_FRAME_FACTOR
        .saturating_mul(cols.max(height))
        .max(CANVAS_MIN_LIMIT);
    if canvas_size > limit {
        return Err(RegionError::CanvasTooLarge);
    }
    let side = usize::try_from(canvas_size).map_err(|_| RegionError::CanvasTooLarge)?;
    let side64 = i64::try_from(side).unwrap_or(i64::MAX);
    let canvas_rows = side
        .checked_add(side / 2)
        .ok_or(RegionError::CanvasTooSmall)?;
    let shape = Shape::new(side, canvas_rows).ok_or(RegionError::CanvasTooSmall)?;

    let clamped = Rect::new(
        i64::from(region.y1).max(0),
        i64::from(region.x1).max(0),
        i64::from(region.y2).min(height),
        i64::from(region.x2).min(cols),
    );
    if clamped.y0 >= clamped.y1 || clamped.x0 >= clamped.x1 {
        return Err(RegionError::OutsideFrame);
    }
    if (clamped.x1 - clamped.x0) / 2 * 2 > side64 {
        return Err(RegionError::WiderThanCanvas);
    }

    let over_x = i64::from(region.x1).min(0).abs();
    let over_y = i64::from(region.y1).min(0).abs();
    let plan = copy_plan(&clamped, height, cols, side64, over_x, over_y);

    let mut canvas = vec![128u8; shape.pixels()];
    let y_area_end = side.checked_mul(side).ok_or(RegionError::CanvasTooSmall)?;
    let Some(y_area) = canvas.get_mut(..y_area_end) else {
        return Err(RegionError::CanvasTooSmall);
    };
    y_area.fill(16);

    let source = Source {
        data: frame.data(),
        rows,
        cols,
    };
    let mut target = Target {
        data: canvas.as_mut_slice(),
        rows: side64 + side64 / 2,
        cols: side64,
    };
    for (src_rect, dst_rect) in [
        (&plan.y_src, &plan.y_dst),
        (&plan.u_left_src, &plan.u_left_dst),
        (&plan.u_right_src, &plan.u_right_dst),
        (&plan.v_left_src, &plan.v_left_dst),
        (&plan.v_right_src, &plan.v_right_dst),
    ] {
        if !copy_rect(&source, src_rect, &mut target, dst_rect) {
            return Err(RegionError::WiderThanCanvas);
        }
    }
    Image::new(canvas, shape).ok_or(RegionError::CanvasTooSmall)
}
