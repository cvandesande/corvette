use crate::plane::{Image, Plane};

/// Returns the picture grown by one pixel in all eight directions: each byte is the maximum of the 3x3 window centred on it, counting only the pixels inside the picture.
#[must_use]
pub fn dilate_3x3(src: Plane<'_>) -> Image {
    let shape = src.shape();
    let mut rows = Image::zeroed(shape);
    axis_max(
        src.data(),
        rows.data_mut(),
        shape.width(),
        shape.height(),
        shape.width(),
        1,
    );
    let mut grown = Image::zeroed(shape);
    axis_max(
        rows.data(),
        grown.data_mut(),
        shape.height(),
        shape.width(),
        1,
        shape.width(),
    );
    grown
}

fn axis_max(
    src: &[u8],
    dst: &mut [u8],
    length: usize,
    lines: usize,
    line_stride: usize,
    axis_stride: usize,
) {
    for line in 0..lines {
        let start = line * line_stride;
        for index in 0..length {
            let here = start + index * axis_stride;
            let previous = start + index.saturating_sub(1) * axis_stride;
            let next = start + (index + 1).min(length - 1) * axis_stride;
            dst[here] = src[here].max(src[previous]).max(src[next]);
        }
    }
}
