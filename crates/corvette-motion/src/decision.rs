use crate::contours::Contour;
use crate::shape::Shape;
use crate::shape::motion_shape;

const QUIET_FRACTION: f64 = 0.05;
const QUIET_BOX_LIMIT: usize = 4;
const CALIBRATION_WEIGHT: f64 = 0.2;
const PERSISTED_FRAMES: u32 = 10;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionSettings {
    pub threshold: u8,
    pub lightning_threshold: f64,
    pub improve_contrast: bool,
    pub contour_area: u32,
    pub frame_alpha: f64,
    pub frame_height: usize,
}

impl Default for MotionSettings {
    fn default() -> Self {
        Self {
            threshold: 30,
            lightning_threshold: 0.8,
            improve_contrast: true,
            contour_area: 10,
            frame_alpha: 0.01,
            frame_height: 100,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MotionBox {
    pub x1: usize,
    pub y1: usize,
    pub x2: usize,
    pub y2: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub boxes: Vec<MotionBox>,
    pub motion_fraction: f64,
    pub calibrating: bool,
    pub average_weight: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct MotionDecider {
    settings: MotionSettings,
    shape: Shape,
    resize_factor: f64,
    motion_pixels: u32,
    min_twice_area: u64,
    calibrating: bool,
    motion_frame_count: u32,
}

impl MotionDecider {
    #[must_use]
    pub fn new(settings: MotionSettings, frame: Shape) -> Option<Self> {
        let shape = motion_shape(frame, settings.frame_height)?;
        let twice_pixels = u32::try_from(shape.pixels().checked_mul(2)?).ok()?;
        if !settings.frame_alpha.is_finite()
            || settings.frame_alpha < 0.0
            || settings.frame_alpha > 1.0
        {
            return None;
        }
        if !settings.lightning_threshold.is_finite() {
            return None;
        }
        let frame_height = u32::try_from(frame.height()).ok()?;
        let setting_height = u32::try_from(settings.frame_height).ok()?;
        Some(Self {
            resize_factor: f64::from(frame_height) / f64::from(setting_height),
            motion_pixels: twice_pixels / 2,
            min_twice_area: 2 * u64::from(settings.contour_area),
            settings,
            shape,
            calibrating: true,
            motion_frame_count: 0,
        })
    }

    #[must_use]
    pub const fn shape(&self) -> Shape {
        self.shape
    }

    #[must_use]
    pub const fn calibrating(&self) -> bool {
        self.calibrating
    }

    /// Contours are expected inside the motion shape; a contour outside it gives defined but meaningless saturating output.
    pub fn decide(&mut self, contours: &[Contour]) -> Decision {
        let mut total_twice_area: u64 = 0;
        let mut boxes: Vec<MotionBox> = Vec::new();
        for contour in contours {
            total_twice_area = total_twice_area.saturating_add(contour.twice_area);
            if contour.twice_area > self.min_twice_area {
                boxes.push(self.scaled_box(contour));
            }
        }
        let total = u32::try_from(total_twice_area).unwrap_or(u32::MAX);
        let motion_fraction = (f64::from(total) / 2.0) / f64::from(self.motion_pixels);
        if motion_fraction < QUIET_FRACTION && boxes.len() <= QUIET_BOX_LIMIT {
            self.calibrating = false;
        }
        if self.calibrating || motion_fraction > self.settings.lightning_threshold {
            self.calibrating = true;
        }
        let weight = if self.calibrating {
            CALIBRATION_WEIGHT
        } else {
            self.settings.frame_alpha
        };
        let average_weight = if boxes.is_empty() {
            self.motion_frame_count = 0;
            Some(weight)
        } else {
            self.motion_frame_count = self.motion_frame_count.saturating_add(1);
            (self.motion_frame_count >= PERSISTED_FRAMES).then_some(weight)
        };
        Decision {
            boxes,
            motion_fraction,
            calibrating: self.calibrating,
            average_weight,
        }
    }

    fn scaled_box(&self, contour: &Contour) -> MotionBox {
        let x = scaled_length(contour.x, self.resize_factor);
        let y = scaled_length(contour.y, self.resize_factor);
        let x2 = scaled_length(contour.x.saturating_add(contour.width), self.resize_factor);
        let y2 = scaled_length(contour.y.saturating_add(contour.height), self.resize_factor);
        MotionBox {
            x1: x,
            y1: y,
            x2,
            y2,
        }
    }
}

fn coord(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}

fn scaled_length(length: usize, factor: f64) -> usize {
    trunc_to_whole(coord(length) * factor)
}

/// The largest whole number `n` with `f64::from(n) <= product`, saturating at `u32::MAX`.
fn trunc_to_whole(product: f64) -> usize {
    if product >= f64::from(u32::MAX) || product.is_nan() {
        return usize::try_from(u32::MAX).unwrap_or(usize::MAX);
    }
    let mut lo: u32 = 0;
    let mut hi: u32 = u32::MAX;
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if f64::from(mid) <= product {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    usize::try_from(lo).unwrap_or(usize::MAX)
}
