//! Replays one recorded segment file through the region, candidate, object and record stages into a record writer.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use corvette_motion::{
    Image, MotionSettings, Shape, SizeGrid, fill_polygons, motion_shape, shrink_keep_mask,
};
use corvette_track::{
    IdSource, ObjectFilter, ObjectFilters, TrackerSettings, Zone, ZoneSet, polygon_from_relative,
};

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
            area_parts(area, frame_shape, motion.frame_height).map_err(ReplaySegmentError::Area)?,
        ),
    };
    // Frigate defaults the deployment runs: floor 320, 640 model, "person" tracked with the default filter.
    let regions = RegionStage::new(
        RegionStageSettings {
            motion,
            floor: 320,
            grid: SizeGrid::empty(),
            model: Shape::new(640, 640)
                .ok_or_else(|| ReplaySegmentError::Io(String::from("model shape refused")))?,
            motion_enabled: true,
        },
        frame_shape,
        area.as_ref().map(|(_, mask)| mask.plane()),
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
    let zones = match area {
        None => ZoneSet::new(Vec::new()),
        Some((zone, _)) => ZoneSet::new(vec![zone]),
    };
    let objects = ObjectStage::new(tracker, &mut ids, zones, config.confirm_threshold)
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

const AREA_INERTIA: u32 = 3;
const POLYGON_REFUSAL: &str = "area polygon needs at least three finite points inside 0.0..=1.0";

/// Builds the area's zone in frame pixels (Frigate defaults: inertia 3, empty label filter) and its motion keep mask, shaped `motion_shape(frame, motion_height)`, with the polygon grown by `pad_pixels` kept and everything else ignored.
fn area_parts(area: &Area, frame: Shape, motion_height: usize) -> Result<(Zone, Image), String> {
    let width = u32::try_from(frame.width()).map_err(|_| POLYGON_REFUSAL.to_owned())?;
    let height = u32::try_from(frame.height()).map_err(|_| POLYGON_REFUSAL.to_owned())?;
    let points: Vec<[f64; 2]> = area
        .polygon
        .iter()
        .map(|point| [point.0, point.1])
        .collect();
    let polygon =
        polygon_from_relative(&points, width, height).ok_or_else(|| POLYGON_REFUSAL.to_owned())?;
    let zone = Zone {
        name: area.name.clone(),
        polygon,
        labels: Vec::new(),
        inertia: AREA_INERTIA,
    };
    let motion = motion_shape(frame, motion_height)
        .ok_or_else(|| "area mask: no motion shape for this frame".to_owned())?;
    let filled = fill_polygons(frame, std::slice::from_ref(&area.polygon))
        .ok_or_else(|| POLYGON_REFUSAL.to_owned())?;
    let mut keep: Vec<u8> = filled
        .into_data()
        .into_iter()
        .map(|byte| if byte == 0 { 255 } else { 0 })
        .collect();
    let pad = usize::try_from(area.pad_pixels).map_err(|_| POLYGON_REFUSAL.to_owned())?;
    grow(&mut keep, frame.width(), frame.height(), pad)?;
    let grown = Image::new(keep, frame)
        .ok_or_else(|| "area mask: the grown plane does not match the frame shape".to_owned())?;
    let mask = shrink_keep_mask(grown.plane(), motion).ok_or_else(|| {
        "area mask: the frame-to-motion shrink refused this pair of shapes".to_owned()
    })?;
    Ok((zone, mask))
}

fn grow(keep: &mut [u8], width: usize, height: usize, pad: usize) -> Result<(), String> {
    if pad == 0 || width == 0 || height == 0 {
        return Ok(());
    }
    let outside = "area mask: the plane is shorter than its shape";
    for row in keep.chunks_mut(width) {
        grow_axis(row, pad);
    }
    let mut column: Vec<u8> = vec![0u8; height];
    for x in 0..width {
        for (y, byte) in column.iter_mut().enumerate() {
            *byte = keep
                .get(y * width + x)
                .copied()
                .ok_or_else(|| outside.to_owned())?;
        }
        grow_axis(&mut column, pad);
        for (y, byte) in column.iter().enumerate() {
            *keep
                .get_mut(y * width + x)
                .ok_or_else(|| outside.to_owned())? = *byte;
        }
    }
    Ok(())
}

fn grow_axis(line: &mut [u8], pad: usize) {
    let forward: Vec<bool> = line
        .iter()
        .scan(pad + 1, |since, byte| {
            *since = if *byte == 0 {
                since.saturating_add(1)
            } else {
                0
            };
            Some(*since <= pad)
        })
        .collect();
    let back: Vec<bool> = line
        .iter()
        .rev()
        .scan(pad + 1, |since, byte| {
            *since = if *byte == 0 {
                since.saturating_add(1)
            } else {
                0
            };
            Some(*since <= pad)
        })
        .collect();
    for (byte, seen) in line.iter_mut().zip(forward.iter().zip(back.iter().rev())) {
        *byte = if *seen.0 || *seen.1 { 255 } else { 0 };
    }
}
