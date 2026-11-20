//! Replays one recorded segment file through the region, candidate, object and record stages into a record writer.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use corvette_motion::{MotionSettings, Shape};
use corvette_track::IdSource;

use crate::backoff::DEFAULT_POLICY;
use crate::camera_plan::CameraPlan;
use crate::clock::FrameClock;
use crate::engine::{EngineError, Labels, WorkerClient};
use crate::ffmpeg::{Decode, FrameSpec, Input, SpecError, decode_command};
use crate::record::{RecordHeader, RecordWriter};
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
    pub area: Option<Area>,
}

/// One camera area: a zone name and its relative `x,y` polygon in `0.0..=1.0`; `pad_pixels` grows the motion keep mask outward by a square dilation.
#[derive(Clone, Debug)]
pub struct Area {
    pub name: String,
    pub polygon: Vec<(f64, f64)>,
    pub pad_pixels: u32,
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
    Area(String),
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
            Self::Area(detail) => write!(f, "replay area: {detail}"),
            Self::NoFrames => f.write_str("replay: the segment decoded no frames"),
        }
    }
}

impl std::error::Error for ReplaySegmentError {}

/// Replays one segment through the stages; blocks until the decoded frames end, flushes `out` before returning, and must not be called from inside a Tokio runtime, where it panics.
/// # Errors
/// The failing step's variant; `Io` covers a missing segment, a refused shape, a failed runtime build and a failed flush (a refused frame size or rate is `Spawn`), and `NoFrames` covers a segment that decoded no frame (the header is already written); a refused area polygon or mask is `Area`.
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
    let motion = MotionSettings::default();
    let area = match &config.area {
        None => None,
        Some(area) => Some(
            CameraPlan::area_parts(area, frame_shape, motion.frame_height)
                .map_err(ReplaySegmentError::Area)?,
        ),
    };
    let mut ids = IdSource::default();
    let (regions, candidates, objects) = CameraPlan::build_stages(
        config.frame_width,
        config.frame_height,
        config.fps,
        &config.labels,
        config.confirm_threshold,
        area.as_ref(),
        &mut ids,
    )
    .map_err(|stage| ReplaySegmentError::Io(format!("{stage} stage refused the settings")))?;
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
