use crate::plane::{Image, Plane};
use crate::shape::Shape;

const F64_MAGNITUDE_MASK: u64 = 0x7fff_ffff_ffff_ffff;
const F64_HIDDEN_ONE: u64 = 1 << 52;
const F32_SMALLEST_NORMAL_F64_EXPONENT: u64 = 897;
const F32_SUBNORMAL_SHIFT_BASE: u64 = 926;
const F32_HIDDEN_ONE: u64 = 1 << 23;
const WHOLE_NUMBER_ANCHOR: f32 = 8_388_608.0;

/// One float32 value per pixel, moved toward every new grey picture by a weight.
#[derive(Clone, Debug, PartialEq)]
pub struct RunningAverage {
    values: Vec<f32>,
    shape: Shape,
}

impl RunningAverage {
    #[must_use]
    pub fn new(shape: Shape) -> Self {
        Self {
            values: vec![0.0; shape.pixels()],
            shape,
        }
    }

    #[must_use]
    pub const fn shape(&self) -> Shape {
        self.shape
    }

    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// Moves every value toward `frame` by the weight `alpha`, and returns `None` with the average untouched when `frame` has another shape or `alpha` is NaN, infinite or outside 0.0 through 1.0.
    pub fn accumulate(&mut self, frame: Plane<'_>, alpha: f64) -> Option<()> {
        if frame.shape() != self.shape {
            return None;
        }
        if !(0.0..=1.0).contains(&alpha) {
            return None;
        }
        let weight = round_to_f32(alpha);
        for (mean, &byte) in self.values.iter_mut().zip(frame.data()) {
            *mean = weight.mul_add(f32::from(byte) - *mean, *mean);
        }
        Some(())
    }

    /// Returns the grey picture whose byte is the average's value rounded to the nearest whole number, halves to even, capped at 255.
    #[must_use]
    pub fn to_image(&self) -> Image {
        let mut picture = Image::zeroed(self.shape);
        for (byte, &value) in picture.data_mut().iter_mut().zip(&self.values) {
            *byte = round_to_byte(value);
        }
        picture
    }
}

fn round_to_f32(alpha: f64) -> f32 {
    let bits = alpha.to_bits() & F64_MAGNITUDE_MASK;
    if bits == 0 {
        return 0.0;
    }
    let exponent_field = bits >> 52;
    let significand = F64_HIDDEN_ONE | (bits & (F64_HIDDEN_ONE - 1));
    if exponent_field >= F32_SMALLEST_NORMAL_F64_EXPONENT {
        let dropped = significand & ((1 << 29) - 1);
        let mut kept = significand >> 29;
        let half = 1 << 28;
        if dropped > half || (dropped == half && kept % 2 == 1) {
            kept += 1;
        }
        let f32_bits = ((exponent_field - (F32_SMALLEST_NORMAL_F64_EXPONENT - 1)) << 23)
            + (kept - F32_HIDDEN_ONE);
        f32::from_bits(u32::try_from(f32_bits).unwrap_or(0))
    } else {
        let shift = F32_SUBNORMAL_SHIFT_BASE - exponent_field;
        if shift >= u64::from(u64::BITS) {
            return 0.0;
        }
        let dropped = significand & ((1 << shift) - 1);
        let mut kept = significand >> shift;
        let half = 1 << (shift - 1);
        if dropped > half || (dropped == half && kept % 2 == 1) {
            kept += 1;
        }
        f32::from_bits(u32::try_from(kept).unwrap_or(0))
    }
}

fn round_to_byte(value: f32) -> u8 {
    let whole = value.abs().min(256.0) + WHOLE_NUMBER_ANCHOR;
    u8::try_from(whole.to_bits() & 0x007f_ffff).unwrap_or(255)
}
