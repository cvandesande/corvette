use crate::blur::{level_table, truncated};
use crate::contrast::contrast_bounds;
use crate::plane::{Image, Plane};

const LENGTH: usize = 50;

/// The last 50 pairs of contrast bounds that the contrast stretch averages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContrastHistory {
    pairs: [(u8, u8); LENGTH],
    position: usize,
}

impl Default for ContrastHistory {
    fn default() -> Self {
        Self {
            pairs: [(0, 255); LENGTH],
            position: 0,
        }
    }
}

impl ContrastHistory {
    /// Returns the stretched picture, the unchanged picture when the bounds are equal, and `None` exactly when `contrast_bounds` does.
    pub fn normalise(&mut self, src: Plane<'_>) -> Option<Image> {
        let (min, max) = contrast_bounds(src)?;
        if min >= max {
            return Image::new(src.data().to_vec(), src.shape());
        }
        self.pairs[self.position] = (min, max);
        self.position += 1;
        if self.position == LENGTH {
            self.position = 0;
        }
        let mut min_total = 0_u32;
        let mut max_total = 0_u32;
        for &(low, high) in &self.pairs {
            min_total += u32::from(low);
            max_total += u32::from(high);
        }
        let divisor = f64::from(u8::try_from(LENGTH).unwrap_or(u8::MAX));
        let avg_min = f64::from(min_total) / divisor;
        let avg_max = f64::from(max_total) / divisor;
        let levels = level_table();
        let mut outputs = [0_u8; 256];
        for pixel in 0..=255_u8 {
            let x = f64::from(pixel);
            let clamped = if x < avg_min {
                avg_min
            } else if x > avg_max {
                avg_max
            } else {
                x
            };
            let shifted = clamped - avg_min;
            let spread = avg_max - avg_min;
            let scaled = shifted / spread;
            outputs[usize::from(pixel)] = truncated(&levels, scaled * 255.0);
        }
        let mut stretched = Image::zeroed(src.shape());
        for (out, &pixel) in stretched.data_mut().iter_mut().zip(src.data()) {
            *out = outputs[usize::from(pixel)];
        }
        Some(stretched)
    }
}
