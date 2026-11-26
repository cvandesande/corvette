use std::fmt;

use corvette_motion::{
    MotionBox, MotionDetector, MotionSettings, Plane, Region, Shape, SizeGrid, TensorError,
    build_regions, region_tensor,
};

#[derive(Clone, Debug, PartialEq)]
pub struct RegionStageSettings {
    pub motion: MotionSettings,
    pub floor: usize,
    pub grid: SizeGrid,
    pub model: Shape,
    pub motion_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionTensor {
    pub region: Region,
    pub tensor: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionStageError {
    WrongFrameLength { expected: usize, got: usize },
    Tensor(TensorError),
}

impl From<TensorError> for RegionStageError {
    fn from(error: TensorError) -> Self {
        Self::Tensor(error)
    }
}

impl fmt::Display for RegionStageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongFrameLength { expected, got } => {
                write!(
                    f,
                    "region stage frame: expected {expected} bytes, got {got}"
                )
            }
            Self::Tensor(error) => write!(f, "region stage tensor: {error:?}"),
        }
    }
}

impl std::error::Error for RegionStageError {}

#[derive(Debug)]
pub struct RegionStage {
    detector: MotionDetector,
    floor: usize,
    grid: SizeGrid,
    frame: Shape,
    model: Shape,
    motion_enabled: bool,
    startup: bool,
    motion_triggered: bool,
}

impl RegionStage {
    /// Returns the stage, or `None` exactly where the motion detector's own constructor refuses the settings and frame.
    #[must_use]
    pub fn new(
        settings: RegionStageSettings,
        frame: Shape,
        keep_mask: Option<Plane<'_>>,
    ) -> Option<Self> {
        let detector = MotionDetector::new(settings.motion, frame, keep_mask)?;
        Some(Self {
            detector,
            floor: settings.floor,
            grid: settings.grid,
            frame,
            model: settings.model,
            motion_enabled: settings.motion_enabled,
            startup: true,
            motion_triggered: false,
        })
    }

    /// Steps one I420 picture (`width * height * 3 / 2` bytes, Y plane first) through motion, region building and the region tensors, returning one per region in the builder's order; a frame the detector answers with no decision contributes no motion boxes, which happens only for a plane of another shape.
    ///
    /// # Errors
    /// `WrongFrameLength` when the buffer is not `width * height * 3 / 2` bytes; `Tensor` when a region's crop, conversion or resize refuses.
    pub fn step(
        &mut self,
        frame: &[u8],
        object_boxes: &[Region],
    ) -> Result<Vec<RegionTensor>, RegionStageError> {
        let rows = self.frame.height() * 3 / 2;
        let expected = self.frame.width() * rows;
        let Some(picture) =
            Shape::new(self.frame.width(), rows).and_then(|shape| Plane::new(frame, shape))
        else {
            return Err(RegionStageError::WrongFrameLength {
                expected,
                got: frame.len(),
            });
        };
        let motion_boxes = match picture
            .data()
            .get(..self.frame.pixels())
            .and_then(|luma| Plane::new(luma, self.frame))
            .and_then(|luma| self.detector.feed(luma))
        {
            Some(decision) => decision.boxes.into_iter().map(motion_region).collect(),
            None => Vec::new(),
        };
        self.motion_triggered = !motion_boxes.is_empty();
        let startup = std::mem::replace(&mut self.startup, false);
        let regions = build_regions(
            self.frame,
            self.floor,
            &self.grid,
            object_boxes,
            &motion_boxes,
            self.motion_enabled,
            startup,
        )
        .unwrap_or_default();
        let mut tensors = Vec::with_capacity(regions.len());
        for region in regions {
            tensors.push(RegionTensor {
                region,
                tensor: region_tensor(picture, region, self.model)?,
            });
        }
        Ok(tensors)
    }

    #[must_use]
    pub const fn motion_triggered(&self) -> bool {
        self.motion_triggered
    }
}

fn motion_region(motion: MotionBox) -> Region {
    Region {
        x1: i32::try_from(motion.x1).unwrap_or(i32::MAX),
        y1: i32::try_from(motion.y1).unwrap_or(i32::MAX),
        x2: i32::try_from(motion.x2).unwrap_or(i32::MAX),
        y2: i32::try_from(motion.y2).unwrap_or(i32::MAX),
    }
}
