//! Replays one recorded segment file through the region, candidate, object and record stages into a record writer.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use corvette_motion::{MotionSettings, Shape, SizeGrid};
use corvette_track::{IdSource, ObjectFilter, ObjectFilters, TrackerSettings, ZoneSet};

use crate::backoff::DEFAULT_POLICY;
use crate::candidate_stage::CandidateStage;
use crate::clock::FrameClock;
use crate::engine::{EngineError, Labels, WorkerClient};
use crate::ffmpeg::{Decode, FrameSpec, Input, SpecError, decode_command};
use crate::object_stage::ObjectStage;
use crate::record::{RecordHeader, RecordWriter};
use crate::region_stage::{RegionStage, RegionStageSettings};
use crate::replay::{ReplayError, ReplayPipeline};
use crate::segment::{SegmentError, SegmentName};
use crate::source::{FrameSource, SourceError};

#[derive(Clone, Debug)]
pub struct ReplayConfig {
    pub camera: String,
    pub frame_width: u32,
    pub frame_height: u32,
    pub fps: u32,
    pub socket: PathBuf,
    pub deadline: Duration,
    pub labels: Labels,
    pub confirm_threshold: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplaySummary {
    pub frames: u64,
}

#[derive(Debug)]
pub enum ReplaySegmentError {
    Segment(SegmentError),
    Source(SourceError),
    Spawn(SpecError),
    Worker(EngineError),
    Stage(ReplayError),
    Io(String),
    NoFrames,
}

impl From<ReplayError> for ReplaySegmentError {
    fn from(error: ReplayError) -> Self {
        Self::Stage(error)
    }
}

impl fmt::Display for ReplaySegmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Segment(error) => write!(f, "replay segment name: {error}"),
            Self::Source(error) => write!(f, "replay frame source: {error}"),
            Self::Spawn(error) => write!(f, "replay frame spec: {error}"),
            Self::Worker(error) => write!(f, "replay worker: {error}"),
            Self::Stage(error) => write!(f, "{error}"),
            Self::Io(detail) => write!(f, "replay: {detail}"),
            Self::NoFrames => f.write_str("replay: the segment decoded no frames"),
        }
    }
}

impl std::error::Error for ReplaySegmentError {}

/// Replays one segment through the stages; blocks until the decoded frames end, flushes `out` before returning, and must not be called from inside a Tokio runtime, where it panics.
/// # Errors
/// The failing step's variant; `Io` covers a missing segment, a refused shape, a failed runtime build and a failed flush (a refused frame size or rate is `Spawn`), and `NoFrames` covers a segment that decoded no frame (the header is already written).
pub fn replay_segment<W: Write>(
    segment: &Path,
    config: &ReplayConfig,
    out: W,
) -> Result<ReplaySummary, ReplaySegmentError> {
    let file_name = segment
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            ReplaySegmentError::Io(format!("{} has no usable file name", segment.display()))
        })?;
    let name = SegmentName::parse(file_name).map_err(ReplaySegmentError::Segment)?;
    if !segment.is_file() {
        return Err(ReplaySegmentError::Io(format!(
            "{} is not a file",
            segment.display()
        )));
    }
    let spec = FrameSpec::new(
        config.frame_width,
        config.frame_height,
        config.fps,
        Decode::Cpu,
    )
    .map_err(ReplaySegmentError::Spawn)?;
    let engine = WorkerClient::connect(&config.socket, config.deadline)
        .map_err(ReplaySegmentError::Worker)?;
    let frame_shape = Shape::new(
        usize::try_from(config.frame_width).unwrap_or(0),
        usize::try_from(config.frame_height).unwrap_or(0),
    )
    .ok_or_else(|| ReplaySegmentError::Io(String::from("frame shape refused the settings")))?;
    // Frigate defaults the deployment runs: floor 320, 640 model, "person" tracked with the default filter.
    let regions = RegionStage::new(
        RegionStageSettings {
            motion: MotionSettings::default(),
            floor: 320,
            grid: SizeGrid::empty(),
            model: Shape::new(640, 640)
                .ok_or_else(|| ReplaySegmentError::Io(String::from("model shape refused")))?,
            motion_enabled: true,
        },
        frame_shape,
        None,
    )
    .ok_or_else(|| ReplaySegmentError::Io(String::from("region stage refused the settings")))?;
    let mut filters = BTreeMap::new();
    filters.insert("person".to_owned(), ObjectFilter::default());
    let candidates = CandidateStage::new(
        config.labels.clone(),
        ObjectFilters::new(vec!["person".to_owned()], filters),
        config.frame_width,
        config.frame_height,
    )
    .ok_or_else(|| ReplaySegmentError::Io(String::from("candidate stage refused the settings")))?;
    let tracker = TrackerSettings {
        width: i32::try_from(config.frame_width).unwrap_or(0),
        height: i32::try_from(config.frame_height).unwrap_or(0),
        min_initialized: config.fps / 2,
        max_disappeared: config.fps.saturating_mul(5),
        stationary_threshold: u64::from(config.fps).saturating_mul(10),
        max_frames_default: None,
        max_frames_overrides: Vec::new(),
        own_labels: Vec::new(),
    };
    let mut ids = IdSource::default();
    let objects = ObjectStage::new(
        tracker,
        &mut ids,
        ZoneSet::new(Vec::new()),
        config.confirm_threshold,
    )
    .ok_or_else(|| ReplaySegmentError::Io(String::from("object stage refused the settings")))?;
    let header = RecordHeader {
        camera: config.camera.clone(),
        segment: file_name.to_owned(),
        frame_width: config.frame_width,
        frame_height: config.frame_height,
        detect_fps: f64::from(config.fps),
    };
    let writer = RecordWriter::new(out, &header)
        .map_err(|error| ReplaySegmentError::Stage(ReplayError::Record(error)))?;
    let pipeline = ReplayPipeline::new(regions, candidates, objects, engine, writer);
    let clock = FrameClock::replay(name.start(), spec);
    let path = segment.to_path_buf();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| ReplaySegmentError::Io(format!("build the runtime: {error}")))?;
    let frames = runtime.block_on(async move {
        let source = FrameSource::spawn(
            String::from("decode"),
            DEFAULT_POLICY,
            spec,
            clock,
            move || decode_command("ffmpeg", &Input::Segment { path: path.clone() }, spec),
        );
        pump_frames(source, pipeline).await
    })?;
    Ok(ReplaySummary { frames })
}

async fn pump_frames<W: Write>(
    mut source: FrameSource,
    mut pipeline: ReplayPipeline<'_, WorkerClient, W>,
) -> Result<u64, ReplaySegmentError> {
    let mut frames: u64 = 0;
    let mut failure: Option<ReplaySegmentError> = None;
    loop {
        match source.next_frame().await {
            Ok(frame) => {
                if let Err(error) = pipeline.step(frame.time.as_secs_f64(), &frame.data) {
                    failure = Some(error.into());
                    break;
                }
                frames = frames.saturating_add(1);
            }
            Err(SourceError::Ended) => break,
            Err(error) => {
                failure = Some(ReplaySegmentError::Source(error));
                break;
            }
        }
    }
    // Only dropping the source stops the supervisor; otherwise it restarts the command and re-decodes the file.
    drop(source);
    let mut out = pipeline.finish();
    let flushed = out
        .flush()
        .map_err(|error| ReplaySegmentError::Io(format!("flush the record: {error}")));
    if let Some(error) = failure {
        return Err(error);
    }
    let counted = if frames == 0 {
        Err(ReplaySegmentError::NoFrames)
    } else {
        Ok(frames)
    };
    flushed.and(counted)
}
