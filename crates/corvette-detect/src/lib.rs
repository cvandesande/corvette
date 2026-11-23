//! Starts one operating-system child process, watches it, restarts it with a doubling delay, and kills it when the guard is dropped.
#![forbid(unsafe_code)]

mod backoff;
pub mod camera_events;
mod camera_plan;
mod camera_runner;
mod camera_thread;
mod candidate_stage;
mod clock;
mod config;
mod config_change;
mod config_file;
mod config_routes;
mod config_schema;
mod engine;
mod ffmpeg;
mod frame_slot;
mod live_pipeline;
mod object_stage;
mod rate_window;
mod reader;
mod record;
mod region_stage;
mod replay;
mod replay_segment;
mod revision;
mod segment;
mod service_bodies;
mod service_socket;
mod shared_link;
mod source;
mod source_url;
mod supervised;
mod worker_link;

pub use backoff::{BACKOFF_INITIAL, BACKOFF_MAX, Backoff, DEFAULT_POLICY, HEALTHY_RUN, Policy};
pub use camera_plan::{CameraPlan, PlanError, StreamAddress, stream_address};
pub use camera_runner::{
    CameraHandle, CameraRunner, CameraState, CameraStatus, RunnerError, StepEnd,
};
pub use camera_thread::{CameraThreadSettings, CameraThreads, JoinReport, StartError, ThreadEnd};
pub use candidate_stage::{CandidateError, CandidateStage, Infer};
pub use clock::FrameClock;
pub use config::{
    AREA_KEYS, CAMERA_KEYS, CONFIG_KEYS, CONFIRM_MAX, CONFIRM_MIN_EXCLUSIVE, CameraConfig,
    CameraSource, Config, ConfigError, DEFAULT_CONFIRM_THRESHOLD, DEFAULT_PAD_PIXELS,
    DEFAULT_STREAM_PATH, DEFAULT_STREAM_PORT, MAX_DEADLINE_MS, MAX_EDGE, MAX_FPS, MAX_PAD_PIXELS,
    MAX_POLYGON_COORD, MAX_STREAM_PORT, MIN_CAMERAS, MIN_DEADLINE_MS, MIN_EDGE, MIN_FPS,
    MIN_PAD_PIXELS, MIN_POLYGON_COORD, MIN_POLYGON_POINTS, MIN_STREAM_PORT, POINT_COORDINATES,
    REQUIRED_AREA_KEYS, REQUIRED_CAMERA_KEYS, REQUIRED_CONFIG_KEYS, REQUIRED_STREAM_KEYS,
    SOURCE_KEYS, STREAM_KEYS, StreamSource, Violation, convert_item, syntax_error,
};
pub use config_change::classify;
pub use config_file::render;
pub use config_schema::{SCHEMA_ID, SCHEMA_JSON};
pub use engine::{EngineError, LabelError, Labels, WorkerClient};
pub use ffmpeg::{Decode, FrameSpec, Input, MAX_FRAME_BYTES, SpecError, decode_command};
pub use frame_slot::{SlotCloser, SlotEvent, SlotReceiver, SlotSender, frame_slot};
pub use live_pipeline::{FrameOutcome, LiveError, LivePipeline, PipelineCounters};
pub use object_stage::{FrameObjects, ObjectStage, ObjectStageError, TrackedObject};
pub use rate_window::{RateWindow, Rates};
pub use reader::{Frame, FrameReader, ReadError};
pub use record::{RecordError, RecordHeader, RecordWriter, json_string, object_entries};
pub use region_stage::{RegionStage, RegionStageError, RegionStageSettings, RegionTensor};
pub use replay::{ReplayError, ReplayPipeline};
pub use replay_segment::{Area, ReplayConfig, ReplaySegmentError, ReplaySummary, replay_segment};
pub use revision::revision;
pub use segment::{SegmentError, SegmentName};
pub use service_bodies::{
    OFF_STATE, ObjectsAnswer, WorkerReport, objects_answer, off_entry, state_name, status_entry,
};
pub use service_socket::{CameraEntry, ServiceState, serve, serve_with_timeout};
pub use shared_link::{LinkTimings, SharedInfer, SharedLink};
pub use source::{FrameSource, STREAM_QUEUE_CAPACITY, SourceError};
pub use source_url::{StreamAddressError, StreamKind, resolve_stream_url};
pub use supervised::{Status, Supervised};
pub use worker_link::{LinkState, WorkerLink};

/// Lines `log_event` printed this process; in-process stderr cannot be captured otherwise, and the apply audit test reads it here.
#[cfg(test)]
pub(crate) static TEST_LOG: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn log_event(unit: &str, event: &str, detail: impl std::fmt::Display) {
    let line = format!("corvette-detect unit=\"{unit}\" event={event} detail=\"{detail}\"");
    eprintln!("{line}");
    #[cfg(test)]
    TEST_LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(line);
}
