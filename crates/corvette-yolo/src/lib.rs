//! Model input for the detection worker: the channel-first f32 plan built from a region crop.

pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 640;
pub const CHANNELS: usize = 3;
pub const RGB_BYTES: usize = WIDTH * HEIGHT * CHANNELS;
pub const TENSOR_ELEMENTS: usize = RGB_BYTES;
pub const SCALE: f32 = 255.0;

/// Row-major RGB bytes become three channel planes, each byte divided by `SCALE`.
#[must_use]
pub fn normalize(rgb: &[u8; RGB_BYTES]) -> Vec<f32> {
    let plane = HEIGHT * WIDTH;
    let mut tensor = vec![0.0; TENSOR_ELEMENTS];
    for (pixel, pixel_rgb) in rgb.chunks_exact(CHANNELS).enumerate() {
        for (channel, &byte) in pixel_rgb.iter().enumerate() {
            tensor[channel * plane + pixel] = f32::from(byte) / SCALE;
        }
    }
    tensor
}
