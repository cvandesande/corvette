//! Starts one operating-system child process, watches it, restarts it with a doubling delay, and kills it when the guard is dropped.
#![forbid(unsafe_code)]

mod backoff;
mod candidate_stage;
mod clock;
mod engine;
mod ffmpeg;
mod object_stage;
mod reader;
mod record;
mod region_stage;
mod replay;
mod replay_segment;
mod segment;
mod source;
mod supervised;

pub use backoff::{BACKOFF_INITIAL, BACKOFF_MAX, Backoff, DEFAULT_POLICY, HEALTHY_RUN, Policy};
pub use candidate_stage::{CandidateError, CandidateStage, Infer};
pub use clock::FrameClock;
pub use engine::{EngineError, LabelError, Labels, WorkerClient};
pub use ffmpeg::{Decode, FrameSpec, Input, MAX_FRAME_BYTES, SpecError, decode_command};
pub use object_stage::{FrameObjects, ObjectStage, ObjectStageError, TrackedObject};
pub use reader::{Frame, FrameReader, ReadError};
pub use record::{RecordError, RecordHeader, RecordWriter};
pub use region_stage::{RegionStage, RegionStageError, RegionStageSettings, RegionTensor};
pub use replay::{ReplayError, ReplayPipeline};
pub use replay_segment::{Area, ReplayConfig, ReplaySegmentError, ReplaySummary, replay_segment};
pub use segment::{SegmentError, SegmentName};
pub use source::{FrameSource, STREAM_QUEUE_CAPACITY, SourceError};
pub use supervised::{Status, Supervised};

pub(crate) fn log_event(unit: &str, event: &str, detail: impl std::fmt::Display) {
    eprintln!("corvette-detect unit=\"{unit}\" event={event} detail=\"{detail}\"");
}
