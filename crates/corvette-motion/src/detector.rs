use crate::average::RunningAverage;
use crate::blur::gaussian_blur;
use crate::contours::external_contours;
use crate::contrast_history::ContrastHistory;
use crate::decision::{Decision, MotionDecider, MotionSettings};
use crate::difference::{absolute_difference, threshold_above};
use crate::dilate::dilate_3x3;
use crate::plane::Plane;
use crate::resize::resize_nearest;
use crate::shape::Shape;

/// The deployed pipeline composed: shrink, stretch, masked pixels, blur, difference with the average, threshold, dilation, contours, decisions, then the average moving toward the blurred picture.
#[derive(Clone, Debug)]
pub struct MotionDetector {
    decider: MotionDecider,
    threshold: u8,
    improve_contrast: bool,
    history: ContrastHistory,
    average: RunningAverage,
    frame: Shape,
    motion: Shape,
    masked: Vec<usize>,
}

impl MotionDetector {
    /// Returns the detector, or `None` when the frame and settings give no motion shape or a `keep_mask` has a different shape; in `keep_mask` a byte of 0 marks a masked pixel.
    #[must_use]
    pub fn new(
        settings: MotionSettings,
        frame: Shape,
        keep_mask: Option<Plane<'_>>,
    ) -> Option<Self> {
        let decider = MotionDecider::new(settings, frame)?;
        let motion = decider.shape();
        let masked = match keep_mask {
            Some(mask) if mask.shape() == motion => mask
                .data()
                .iter()
                .enumerate()
                .filter_map(|(index, &byte)| (byte == 0).then_some(index))
                .collect(),
            Some(_) => return None,
            None => Vec::new(),
        };
        Some(Self {
            decider,
            threshold: settings.threshold,
            improve_contrast: settings.improve_contrast,
            history: ContrastHistory::default(),
            average: RunningAverage::new(motion),
            frame,
            motion,
            masked,
        })
    }

    /// Returns the decision for one full grey picture; `None` only when its shape differs from the shape given to `new`, the one refusal, which changes nothing.
    pub fn feed(&mut self, frame: Plane<'_>) -> Option<Decision> {
        if frame.shape() != self.frame {
            return None;
        }
        let mut picture = resize_nearest(frame, self.motion)?;
        if self.improve_contrast {
            picture = self.history.normalise(picture.plane())?;
        }
        for &index in &self.masked {
            picture.data_mut()[index] = 0;
        }
        let blurred = gaussian_blur(picture.plane());
        let difference = absolute_difference(blurred.plane(), self.average.to_image().plane())?;
        let thresholded = threshold_above(difference.plane(), self.threshold);
        let dilated = dilate_3x3(thresholded.plane());
        let contours = external_contours(dilated.plane());
        let decision = self.decider.decide(&contours);
        if let Some(weight) = decision.average_weight {
            self.average.accumulate(blurred.plane(), weight)?;
        }
        Some(decision)
    }

    #[must_use]
    pub const fn calibrating(&self) -> bool {
        self.decider.calibrating()
    }

    #[must_use]
    pub const fn average(&self) -> &RunningAverage {
        &self.average
    }
}
