use crate::plane::{Image, Plane};

/// Weight of the two neighbours of the centre pixel in the reference 3-tap blur kernel.
pub const GAUSSIAN_OUTER_WEIGHT: f64 = 0.274_068_619_061_197;

/// Weight of the centre pixel in the reference 3-tap blur kernel, one normalisation with the outer weight.
pub const GAUSSIAN_CENTRE_WEIGHT: f64 = 0.451_862_761_877_606_05;

/// Blurs with the reference 3-tap kernel at sigma 1 and radius 1: a vertical pass then a horizontal pass, each stored in 8 bits before the next reads it.
#[must_use]
pub fn gaussian_blur(src: Plane<'_>) -> Image {
    let shape = src.shape();
    let levels: Vec<f64> = (0..=255_u8).map(f64::from).collect();
    let mut first = Image::zeroed(shape);
    blur_axis(
        src.data(),
        first.data_mut(),
        shape.height(),
        shape.width(),
        1,
        shape.width(),
        &levels,
    );
    let mut blurred = Image::zeroed(shape);
    blur_axis(
        first.data(),
        blurred.data_mut(),
        shape.width(),
        shape.height(),
        shape.width(),
        1,
        &levels,
    );
    blurred
}

fn blur_axis(
    src: &[u8],
    dst: &mut [u8],
    length: usize,
    lines: usize,
    line_stride: usize,
    axis_stride: usize,
    levels: &[f64],
) {
    for line in 0..lines {
        let start = line * line_stride;
        for index in 0..length {
            let here = start + index * axis_stride;
            let previous = start + index.saturating_sub(1) * axis_stride;
            let next = start + (index + 1).min(length - 1) * axis_stride;
            let centre_term = f64::from(src[here]) * GAUSSIAN_CENTRE_WEIGHT;
            let neighbour_term =
                (f64::from(src[previous]) + f64::from(src[next])) * GAUSSIAN_OUTER_WEIGHT;
            dst[here] = truncated(levels, centre_term + neighbour_term);
        }
    }
}

fn truncated(levels: &[f64], value: f64) -> u8 {
    let at_or_below = levels.partition_point(|&level| level <= value);
    u8::try_from(at_or_below.saturating_sub(1)).unwrap_or(255)
}
