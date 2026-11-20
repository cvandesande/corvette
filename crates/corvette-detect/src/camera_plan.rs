//! Turns one camera's configuration into a plan: checked once at startup, rebuilt fresh after every runner failure.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use corvette_motion::{
    Image, MotionSettings, Shape, SizeGrid, fill_polygons, motion_shape, shrink_keep_mask,
};
use corvette_track::{
    IdSource, ObjectFilter, ObjectFilters, TrackerSettings, Zone, ZoneSet, polygon_from_relative,
};
use tokio::process::Command;

use crate::candidate_stage::CandidateStage;
use crate::clock::FrameClock;
use crate::config::{CameraConfig, Config};
use crate::engine::Labels;
use crate::ffmpeg::{Decode, FrameSpec, Input, SpecError, decode_command};
use crate::live_pipeline::LivePipeline;
use crate::object_stage::ObjectStage;
use crate::region_stage::{RegionStage, RegionStageSettings};
use crate::replay_segment::Area;
use crate::shared_link::SharedInfer;

#[derive(Debug)]
pub enum PlanError {
    Labels(String),
    Spec(SpecError),
    Area(String),
    Stage(&'static str),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Labels(detail) => write!(f, "camera plan labels: {detail}"),
            Self::Spec(error) => write!(f, "camera plan frame spec: {error}"),
            Self::Area(detail) => write!(f, "camera plan area: {detail}"),
            Self::Stage(stage) => write!(f, "camera plan: the {stage} stage refused the settings"),
        }
    }
}

impl std::error::Error for PlanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spec(error) => Some(error),
            Self::Labels(_) | Self::Area(_) | Self::Stage(_) => None,
        }
    }
}

#[derive(Debug)]
pub struct CameraPlan {
    name: String,
    spec: FrameSpec,
    labels: Labels,
    confirm_threshold: f64,
    area: Option<(Zone, Image)>,
}

impl CameraPlan {
    /// # Errors
    /// Unreadable or malformed labels are `Labels` naming the path, a refused frame size or rate is `Spec`, a refused area polygon or mask is `Area`, and a stage constructor refusing the settings is `Stage`; the trial build at the end of this function is what surfaces a constructor refusal here rather than on a later restart.
    pub fn new(config: &Config, camera: &CameraConfig) -> Result<Self, PlanError> {
        let path = &config.labels;
        let text = std::fs::read_to_string(path)
            .map_err(|error| PlanError::Labels(format!("{}: {error}", path.display())))?;
        let labels = Labels::parse(&text)
            .map_err(|error| PlanError::Labels(format!("{}: {error}", path.display())))?;
        let spec = FrameSpec::new(camera.width, camera.height, camera.fps, Decode::Cpu)
            .map_err(PlanError::Spec)?;
        let frame = Shape::new(
            usize::try_from(camera.width).unwrap_or(0),
            usize::try_from(camera.height).unwrap_or(0),
        )
        .ok_or(SpecError::Size)
        .map_err(PlanError::Spec)?;
        let area = match &camera.area {
            None => None,
            Some(area) => Some(
                Self::area_parts(area, frame, MotionSettings::default().frame_height)
                    .map_err(PlanError::Area)?,
            ),
        };
        let mut ids = IdSource::default();
        Self::build_stages(
            camera.width,
            camera.height,
            camera.fps,
            &labels,
            camera.confirm_threshold,
            area.as_ref(),
            &mut ids,
        )
        .map_err(PlanError::Stage)?;
        Ok(Self {
            name: camera.name.clone(),
            spec,
            labels,
            confirm_threshold: camera.confirm_threshold,
            area,
        })
    }

    /// # Errors
    /// A stage constructor refusing the settings returns `Stage`; these are the refusals the trial build already surfaced for this plan.
    pub fn pipeline<'a>(
        &self,
        ids: &'a mut IdSource,
        infer: SharedInfer,
    ) -> Result<LivePipeline<'a, SharedInfer>, PlanError> {
        let (regions, candidates, objects) = Self::build_stages(
            self.spec.width(),
            self.spec.height(),
            self.spec.fps(),
            &self.labels,
            self.confirm_threshold,
            self.area.as_ref(),
            ids,
        )
        .map_err(PlanError::Stage)?;
        Ok(LivePipeline::new(regions, candidates, objects, infer))
    }

    #[must_use]
    pub const fn spec(&self) -> FrameSpec {
        self.spec
    }

    #[must_use]
    pub const fn clock(&self) -> FrameClock {
        FrameClock::live()
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn command_factory(
        &self,
        ffmpeg: PathBuf,
        restream_address: String,
    ) -> impl FnMut() -> Command + Send + 'static {
        let name = self.name.clone();
        let spec = self.spec;
        move || {
            let url = format!("rtsp://{restream_address}/{name}_sub");
            decode_command(&ffmpeg, &Input::Live { url }, spec)
        }
    }

    // Frigate defaults the deployment runs: floor 320, a 640x640 model, "person" tracked with the default filter, tracker timings from the camera rate; the failing stage names itself.
    pub(crate) fn build_stages<'a>(
        width: u32,
        height: u32,
        fps: u32,
        labels: &Labels,
        confirm_threshold: f64,
        area: Option<&(Zone, Image)>,
        ids: &'a mut IdSource,
    ) -> Result<(RegionStage, CandidateStage, ObjectStage<'a>), &'static str> {
        let frame = Shape::new(
            usize::try_from(width).unwrap_or(0),
            usize::try_from(height).unwrap_or(0),
        )
        .ok_or("region")?;
        let keep = area.map(|(_, mask)| mask.plane());
        let regions = RegionStage::new(
            RegionStageSettings {
                motion: MotionSettings::default(),
                floor: 320,
                grid: SizeGrid::empty(),
                model: Shape::new(640, 640).ok_or("region")?,
                motion_enabled: true,
            },
            frame,
            keep,
        )
        .ok_or("region")?;
        let mut filters = BTreeMap::new();
        filters.insert("person".to_owned(), ObjectFilter::default());
        let candidates = CandidateStage::new(
            labels.clone(),
            ObjectFilters::new(vec!["person".to_owned()], filters),
            width,
            height,
        )
        .ok_or("candidate")?;
        let tracker = TrackerSettings {
            width: i32::try_from(width).unwrap_or(0),
            height: i32::try_from(height).unwrap_or(0),
            min_initialized: fps / 2,
            max_disappeared: fps.saturating_mul(5),
            stationary_threshold: u64::from(fps).saturating_mul(10),
            max_frames_default: None,
            max_frames_overrides: Vec::new(),
            own_labels: Vec::new(),
        };
        let zones = match area {
            None => ZoneSet::new(Vec::new()),
            Some((zone, _)) => ZoneSet::new(vec![zone.clone()]),
        };
        let objects = ObjectStage::new(tracker, ids, zones, confirm_threshold).ok_or("object")?;
        Ok((regions, candidates, objects))
    }

    /// Builds the area's zone in frame pixels (Frigate defaults: inertia 3, empty label filter) and its motion keep mask, shaped `motion_shape(frame, motion_height)`, with the polygon grown by `pad_pixels` kept and everything else ignored.
    pub(crate) fn area_parts(
        area: &Area,
        frame: Shape,
        motion_height: usize,
    ) -> Result<(Zone, Image), String> {
        let width = u32::try_from(frame.width()).map_err(|_| POLYGON_REFUSAL.to_owned())?;
        let height = u32::try_from(frame.height()).map_err(|_| POLYGON_REFUSAL.to_owned())?;
        let points: Vec<[f64; 2]> = area
            .polygon
            .iter()
            .map(|point| [point.0, point.1])
            .collect();
        let polygon = polygon_from_relative(&points, width, height)
            .ok_or_else(|| POLYGON_REFUSAL.to_owned())?;
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
        let grown = Image::new(keep, frame).ok_or_else(|| {
            "area mask: the grown plane does not match the frame shape".to_owned()
        })?;
        let mask = shrink_keep_mask(grown.plane(), motion).ok_or_else(|| {
            "area mask: the frame-to-motion shrink refused this pair of shapes".to_owned()
        })?;
        Ok((zone, mask))
    }
}

const AREA_INERTIA: u32 = 3;
const POLYGON_REFUSAL: &str = "area polygon needs at least three finite points inside 0.0..=1.0";

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
