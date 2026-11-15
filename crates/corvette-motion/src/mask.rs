use crate::plane::Image;
use crate::polygon::fill_polygons;
use crate::shape::Shape;
use crate::shape::motion_shape;
use crate::shrink::shrink_keep_mask;

/// Returns the motion-frame keep mask of a camera frame: a picture of shape `motion_shape(frame, frame_height)` whose 0 bytes mark the pixels to ignore and whose 255 bytes mark the pixels to keep.
///
/// Returns `None` when `motion_shape`, `fill_polygons` or `shrink_keep_mask` returns `None`.
#[must_use]
pub fn motion_keep_mask(
    frame: Shape,
    frame_height: usize,
    polygons: &[Vec<(f64, f64)>],
) -> Option<Image> {
    let motion = motion_shape(frame, frame_height)?;
    let filled = fill_polygons(frame, polygons)?;
    shrink_keep_mask(filled.plane(), motion)
}
