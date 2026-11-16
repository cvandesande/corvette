use crate::colour::i420_to_rgb;
use crate::crop::{Region, RegionError, crop_pad_i420};
use crate::plane::Plane;
use crate::resize_rgb::resize_linear_rgb;
use crate::shape::Shape;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TensorError {
    Region(RegionError),
    ModelShape,
}

impl From<RegionError> for TensorError {
    fn from(error: RegionError) -> Self {
        Self::Region(error)
    }
}

/// The RGB bytes of `region` at `model` size, as Frigate's `create_tensor_input` returns them without the leading axis.
/// # Errors
/// `Region` when the crop or the conversion fails; `ModelShape` when the resize rejects the model's size.
pub fn region_tensor(
    frame: Plane<'_>,
    region: Region,
    model: Shape,
) -> Result<Vec<u8>, TensorError> {
    let canvas = crop_pad_i420(frame, region)?;
    let rgb = i420_to_rgb(canvas.plane())?;
    let side = canvas.shape().width();
    if side == model.width() && side == model.height() {
        return Ok(rgb);
    }
    let square = Shape::new(side, side).ok_or(TensorError::ModelShape)?;
    resize_linear_rgb(&rgb, square, model).ok_or(TensorError::ModelShape)
}
