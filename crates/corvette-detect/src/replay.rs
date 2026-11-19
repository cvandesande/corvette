//! Composes the region, candidate, object and record stages for one replay frame: previous frame's object boxes in, one record line out.

use std::fmt;
use std::io::Write;

use corvette_motion::Region;

use crate::candidate_stage::{CandidateError, CandidateStage, Infer};
use crate::object_stage::{ObjectStage, ObjectStageError};
use crate::record::{RecordError, RecordWriter};
use crate::region_stage::{RegionStage, RegionStageError};

#[derive(Debug)]
pub enum ReplayError {
    Region(RegionStageError),
    Candidate(CandidateError),
    Object(ObjectStageError),
    Record(RecordError),
}

impl From<RegionStageError> for ReplayError {
    fn from(error: RegionStageError) -> Self {
        Self::Region(error)
    }
}

impl From<CandidateError> for ReplayError {
    fn from(error: CandidateError) -> Self {
        Self::Candidate(error)
    }
}

impl From<ObjectStageError> for ReplayError {
    fn from(error: ObjectStageError) -> Self {
        Self::Object(error)
    }
}

impl From<RecordError> for ReplayError {
    fn from(error: RecordError) -> Self {
        Self::Record(error)
    }
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Region(error) => write!(f, "replay region stage: {error}"),
            Self::Candidate(error) => write!(f, "replay candidate stage: {error}"),
            Self::Object(error) => write!(f, "replay object stage: {error}"),
            Self::Record(error) => write!(f, "replay record writer: {error}"),
        }
    }
}

impl std::error::Error for ReplayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Region(error) => Some(error),
            Self::Candidate(error) => Some(error),
            Self::Object(error) => Some(error),
            Self::Record(error) => Some(error),
        }
    }
}

pub struct ReplayPipeline<'a, E: Infer, W: Write> {
    regions: RegionStage,
    candidates: CandidateStage,
    objects: ObjectStage<'a>,
    engine: E,
    writer: RecordWriter<W>,
    previous_boxes: Vec<Region>,
}

impl<E: Infer + fmt::Debug, W: Write + fmt::Debug> fmt::Debug for ReplayPipeline<'_, E, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplayPipeline")
            .field("regions", &self.regions)
            .field("candidates", &self.candidates)
            .field("objects", &self.objects)
            .field("engine", &self.engine)
            .field("writer", &self.writer)
            .field("previous_boxes", &self.previous_boxes)
            .finish()
    }
}

impl<'a, E: Infer, W: Write> ReplayPipeline<'a, E, W> {
    #[must_use]
    pub const fn new(
        regions: RegionStage,
        candidates: CandidateStage,
        objects: ObjectStage<'a>,
        engine: E,
        writer: RecordWriter<W>,
    ) -> Self {
        Self {
            regions,
            candidates,
            objects,
            engine,
            writer,
            previous_boxes: Vec::new(),
        }
    }

    /// # Errors
    /// The failing stage's error is returned wrapped in the matching `ReplayError` variant; after one the pipeline must not be stepped again.
    pub fn step(&mut self, frame_time: f64, frame: &[u8]) -> Result<(), ReplayError> {
        let tensors = self.regions.step(frame, &self.previous_boxes)?;
        let detections = self.candidates.detect(&mut self.engine, &tensors)?;
        let frame_objects = self.objects.step(frame_time, detections)?;
        self.writer.write_frame(&frame_objects)?;
        self.previous_boxes = frame_objects.regions();
        Ok(())
    }

    #[must_use]
    pub fn finish(self) -> W {
        self.writer.finish()
    }
}
