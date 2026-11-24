//! Composes the region, candidate and object stages for one live frame: previous frame's object boxes in, this frame's tracked objects out, an inference loss reported as an outcome rather than as an empty object list.

use std::fmt;

use corvette_motion::Region;

use crate::candidate_stage::{CandidateError, CandidateStage, Infer};
use crate::object_stage::{FrameObjects, ObjectStage, ObjectStageError};
use crate::region_stage::{RegionStage, RegionStageError};

#[derive(Debug)]
pub enum LiveError {
    Region(RegionStageError),
    Candidate(CandidateError),
    Object(ObjectStageError),
}

impl From<RegionStageError> for LiveError {
    fn from(error: RegionStageError) -> Self {
        Self::Region(error)
    }
}

impl From<CandidateError> for LiveError {
    fn from(error: CandidateError) -> Self {
        Self::Candidate(error)
    }
}

impl From<ObjectStageError> for LiveError {
    fn from(error: ObjectStageError) -> Self {
        Self::Object(error)
    }
}

impl fmt::Display for LiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Region(error) => write!(f, "live region stage: {error}"),
            Self::Candidate(error) => write!(f, "live candidate stage: {error}"),
            Self::Object(error) => write!(f, "live object stage: {error}"),
        }
    }
}

impl std::error::Error for LiveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Region(error) => Some(error),
            Self::Candidate(error) => Some(error),
            Self::Object(error) => Some(error),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FrameOutcome {
    Objects(FrameObjects),
    InferenceUnavailable,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PipelineCounters {
    pub frames_in: u64,
    pub frames_tracked: u64,
    pub frames_inference_failed: u64,
    pub inferences_run: u64,
    pub frames_without_region: u64,
}

pub struct LivePipeline<'a, E: Infer> {
    regions: RegionStage,
    candidates: CandidateStage,
    objects: ObjectStage<'a>,
    engine: E,
    previous_boxes: Vec<Region>,
    counters: PipelineCounters,
}

impl<E: Infer + fmt::Debug> fmt::Debug for LivePipeline<'_, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LivePipeline")
            .field("regions", &self.regions)
            .field("candidates", &self.candidates)
            .field("objects", &self.objects)
            .field("engine", &self.engine)
            .field("previous_boxes", &self.previous_boxes)
            .field("counters", &self.counters)
            .finish()
    }
}

impl<'a, E: Infer> LivePipeline<'a, E> {
    #[must_use]
    pub const fn new(
        regions: RegionStage,
        candidates: CandidateStage,
        objects: ObjectStage<'a>,
        engine: E,
    ) -> Self {
        Self {
            regions,
            candidates,
            objects,
            engine,
            previous_boxes: Vec::new(),
            counters: PipelineCounters {
                frames_in: 0,
                frames_tracked: 0,
                frames_inference_failed: 0,
                inferences_run: 0,
                frames_without_region: 0,
            },
        }
    }

    /// Steps one frame through the region, candidate and object stages; a candidate engine error returns `Ok(InferenceUnavailable)` and leaves the tracker, `previous_boxes` and every other counter untouched, any other stage error returns `Err` and after one the pipeline must not be stepped again. Tracks do not age while inference is unavailable, because the object stage is not stepped on such a frame.
    ///
    /// # Errors
    /// The failing stage's error wrapped in the matching `LiveError` variant; an inference failure is not an error, it is `FrameOutcome::InferenceUnavailable`.
    pub fn step(&mut self, frame_time: f64, frame: &[u8]) -> Result<FrameOutcome, LiveError> {
        self.counters.frames_in = self.counters.frames_in.saturating_add(1);
        let tensors = self.regions.step(frame, &self.previous_boxes)?;
        if tensors.is_empty() {
            self.counters.frames_without_region =
                self.counters.frames_without_region.saturating_add(1);
        } else {
            self.counters.inferences_run = self
                .counters
                .inferences_run
                .saturating_add(u64::try_from(tensors.len()).unwrap_or(u64::MAX));
        }
        let detections = match self.candidates.detect(&mut self.engine, &tensors) {
            Ok(detections) => detections,
            Err(CandidateError::Engine(_)) => {
                self.counters.frames_inference_failed =
                    self.counters.frames_inference_failed.saturating_add(1);
                return Ok(FrameOutcome::InferenceUnavailable);
            }
            Err(error) => return Err(LiveError::Candidate(error)),
        };
        let frame_objects = self.objects.step(frame_time, detections)?;
        self.previous_boxes = frame_objects.regions();
        self.counters.frames_tracked = self.counters.frames_tracked.saturating_add(1);
        Ok(FrameOutcome::Objects(frame_objects))
    }

    #[must_use]
    pub const fn counters(&self) -> PipelineCounters {
        self.counters
    }
}
