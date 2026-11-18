use std::fmt;

use corvette_detect_proto::{Row, TENSOR_BYTES};
use corvette_track::{
    Detection, Frame, FrameDetection, ObjectFilters, Region as TrackRegion, reduce_detections,
};

use crate::engine::{EngineError, Labels, WorkerClient};
use crate::region_stage::RegionTensor;

pub trait Infer {
    /// # Errors
    fn infer(&mut self, tensor: &[u8; TENSOR_BYTES]) -> Result<Vec<Row>, EngineError>;
}

impl Infer for WorkerClient {
    fn infer(&mut self, tensor: &[u8; TENSOR_BYTES]) -> Result<Vec<Row>, EngineError> {
        Self::infer(self, tensor)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateError {
    Engine(EngineError),
    WrongTensorLength { expected: usize, got: usize },
    EdgeOutOfRange,
}

impl From<EngineError> for CandidateError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

impl fmt::Display for CandidateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Engine(error) => write!(f, "candidate stage engine: {error}"),
            Self::WrongTensorLength { expected, got } => {
                write!(
                    f,
                    "candidate stage tensor: expected {expected} bytes, got {got}"
                )
            }
            Self::EdgeOutOfRange => {
                write!(
                    f,
                    "candidate stage geometry: a detection edge exceeds the 32-bit tracker input"
                )
            }
        }
    }
}

impl std::error::Error for CandidateError {}

#[derive(Debug)]
pub struct CandidateStage {
    labels: Labels,
    filters: ObjectFilters,
    frame: Frame,
}

impl CandidateStage {
    #[must_use]
    pub fn new(
        labels: Labels,
        filters: ObjectFilters,
        frame_width: u32,
        frame_height: u32,
    ) -> Option<Self> {
        Frame::new(i64::from(frame_width), i64::from(frame_height)).map(|frame| Self {
            labels,
            filters,
            frame,
        })
    }

    /// Infers each region tensor and returns the frame's detections after one whole-frame merge; a row whose motion region is outside the tracking crate's region edge limit is skipped and counted nowhere, and a row whose class id the closed label map does not name is skipped because a closed map has no name to assign.
    /// # Errors
    /// `Engine` when inference fails, and no later region is inferred; `WrongTensorLength` when a tensor's length is not `TENSOR_BYTES`; `EdgeOutOfRange` when a kept edge does not fit `i32`.
    pub fn detect<E: Infer>(
        &self,
        engine: &mut E,
        tensors: &[RegionTensor],
    ) -> Result<Vec<FrameDetection>, CandidateError> {
        let mut detections = Vec::new();
        for item in tensors {
            let tensor = item.tensor.as_slice().try_into().map_err(|_| {
                CandidateError::WrongTensorLength {
                    expected: TENSOR_BYTES,
                    got: item.tensor.len(),
                }
            })?;
            let rows = engine.infer(tensor)?;
            let Some(region) = track_region(item.region) else {
                continue;
            };
            for row in rows {
                if let Some(detection) = self.row_to_detection(&row, region) {
                    detections.push(detection);
                }
            }
        }
        reduce_detections(detections, self.frame)
            .iter()
            .map(frame_detection)
            .collect()
    }

    fn row_to_detection(&self, row: &Row, region: TrackRegion) -> Option<Detection> {
        let label = self.labels.name(row.class_id)?;
        let raw = [row.y_min, row.x_min, row.y_max, row.x_max];
        let geometry = corvette_track::map_box(raw, region, self.frame).ok()?;
        let candidate = geometry.candidate(label, row.score);
        if self.filters.first_rejection(&candidate).is_some() {
            return None;
        }
        Detection::new(label, row.score, geometry, region)
    }
}

fn track_region(region: corvette_motion::Region) -> Option<TrackRegion> {
    TrackRegion::new(
        i64::from(region.x1),
        i64::from(region.y1),
        i64::from(region.x2),
        i64::from(region.y2),
    )
}

fn frame_detection(detection: &Detection) -> Result<FrameDetection, CandidateError> {
    let geometry = detection.geometry();
    let edges = [
        edge_to_i32(geometry.x_min)?,
        edge_to_i32(geometry.y_min)?,
        edge_to_i32(geometry.x_max)?,
        edge_to_i32(geometry.y_max)?,
    ];
    let region = detection.region();
    let region_edges = [
        edge_to_i32(region.x_min())?,
        edge_to_i32(region.y_min())?,
        edge_to_i32(region.x_max())?,
        edge_to_i32(region.y_max())?,
    ];
    Ok(FrameDetection {
        label: detection.label().to_owned(),
        score: f64::from(detection.score()),
        edges,
        area: geometry.area,
        ratio: geometry.ratio,
        region: vec![region_edges],
    })
}

fn edge_to_i32(edge: i64) -> Result<i32, CandidateError> {
    i32::try_from(edge).map_err(|_| CandidateError::EdgeOutOfRange)
}
