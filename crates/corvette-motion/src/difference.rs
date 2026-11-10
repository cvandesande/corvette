use crate::plane::{Image, Plane};

/// Returns the per-pixel absolute difference of two grey pictures, or `None` unless they have equal shapes.
#[must_use]
pub fn absolute_difference(a: Plane<'_>, b: Plane<'_>) -> Option<Image> {
    if a.shape() != b.shape() {
        return None;
    }
    let mut difference = Image::zeroed(a.shape());
    let pairs = a.data().iter().zip(b.data());
    for (out, (left, right)) in difference.data_mut().iter_mut().zip(pairs) {
        *out = left.abs_diff(*right);
    }
    Some(difference)
}

/// Returns a black and white picture: 255 where the source byte is strictly above `level`, 0 elsewhere.
#[must_use]
pub fn threshold_above(src: Plane<'_>, level: u8) -> Image {
    let mut mask = Image::zeroed(src.shape());
    for (out, &pixel) in mask.data_mut().iter_mut().zip(src.data()) {
        *out = if pixel > level { 255 } else { 0 };
    }
    mask
}
