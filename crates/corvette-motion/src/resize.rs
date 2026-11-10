use crate::plane::{Image, Plane};
use crate::shape::Shape;

/// Copies each destination pixel from the nearest source pixel by `OpenCV`'s f64 index rule.
/// Returns `None` when a side of the source or of `to` exceeds `u32::MAX`.
#[must_use]
pub fn resize_nearest(src: Plane<'_>, to: Shape) -> Option<Image> {
    let src_width = u32::try_from(src.shape().width()).ok()?;
    let src_height = u32::try_from(src.shape().height()).ok()?;
    let dst_width = u32::try_from(to.width()).ok()?;
    let dst_height = u32::try_from(to.height()).ok()?;

    let columns = source_indices(dst_width, src_width);
    let rows = source_indices(dst_height, src_height);

    let src_stride = src.shape().width();
    let mut out = vec![0u8; to.pixels()];
    for (dst_row, &row) in out.chunks_mut(to.width()).zip(&rows) {
        let start = row * src_stride;
        let source_row = &src.data()[start..start + src_stride];
        for (dst_pixel, &column) in dst_row.iter_mut().zip(&columns) {
            *dst_pixel = source_row[column];
        }
    }
    Image::new(out, to)
}

fn source_indices(dst_side: u32, src_side: u32) -> Vec<usize> {
    let inverse = 1.0 / (f64::from(dst_side) / f64::from(src_side));
    (0..dst_side)
        .map(|x| source_index(x, dst_side, src_side, inverse))
        .collect()
}

fn source_index(x: u32, dst_side: u32, src_side: u32, inverse: f64) -> usize {
    let target = f64::from(x) * inverse;
    let guess = u64::from(x) * u64::from(src_side) / u64::from(dst_side);
    let mut k = u32::try_from(guess).unwrap_or(u32::MAX);
    loop {
        if f64::from(k) <= target {
            break;
        }
        k = k.saturating_sub(1);
    }
    loop {
        if f64::from(k) + 1.0 > target {
            break;
        }
        k = k.saturating_add(1);
    }
    // The min mirrors the upstream index tables' clamp to the last source line.
    usize::try_from(k.min(src_side - 1)).unwrap_or(usize::MAX)
}
