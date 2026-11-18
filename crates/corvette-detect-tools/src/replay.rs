//! Replays one recorded segment through the detection pipeline into a record file.

use std::fs;
use std::io::BufWriter;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use corvette_detect::{Area, Labels, ReplayConfig, SegmentName, replay_segment};

use crate::Failure;
use crate::args::Flags;
use crate::motion_detect::open_output;

const REPLAY_FLAGS: &[&str] = &[
    "socket",
    "labels",
    "in",
    "out",
    "camera",
    "width",
    "height",
    "fps",
    "deadline-ms",
    "confirm",
    "area",
    "area-name",
    "area-pad",
];
const MIN_EDGE: u32 = 16;
const MAX_EDGE: u32 = 8192;
const MIN_FPS: u32 = 1;
const MAX_FPS: u32 = 60;
const MIN_DEADLINE_MS: u32 = 1;
const MAX_DEADLINE_MS: u32 = 600_000;
const DEFAULT_DEADLINE_MS: u32 = 10_000;
const DEFAULT_CONFIRM: f64 = 0.7;
const MIN_AREA_VALUES: usize = 6;
const DEFAULT_AREA_NAME: &str = "area";
const MAX_AREA_PAD: u32 = 200;
const DEFAULT_AREA_PAD: u32 = 16;

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let flags = Flags::parse(args, REPLAY_FLAGS).ok_or(Failure::Usage)?;
    let socket = flags.required("socket")?;
    let labels_path = flags.required("labels")?;
    let segment = flags.required("in")?;
    let out = flags.required("out")?;
    let camera = match flags.get("camera") {
        Some(camera) => camera.to_owned(),
        None => camera_from_file_name(segment)?,
    };
    let frame_width = flags.bounded("width", MIN_EDGE, MAX_EDGE, 704)?;
    let frame_height = flags.bounded("height", MIN_EDGE, MAX_EDGE, 576)?;
    let fps = flags.bounded("fps", MIN_FPS, MAX_FPS, 5)?;
    let deadline_ms = flags.bounded(
        "deadline-ms",
        MIN_DEADLINE_MS,
        MAX_DEADLINE_MS,
        DEFAULT_DEADLINE_MS,
    )?;
    let confirm_threshold = parse_confirm(flags.get("confirm"))?;
    let area = parse_area(&flags)?;
    let labels_text = fs::read_to_string(labels_path)
        .map_err(|error| Failure::Input(format!("cannot read {labels_path}: {error}")))?;
    let labels = Labels::parse(&labels_text)
        .map_err(|error| Failure::Input(format!("cannot parse {labels_path}: {error}")))?;
    let config = ReplayConfig {
        camera,
        frame_width,
        frame_height,
        fps,
        socket: PathBuf::from(socket),
        deadline: Duration::from_millis(u64::from(deadline_ms)),
        labels,
        confirm_threshold,
        area,
    };
    refuse_out_is_input(out, segment)?;
    let writer = BufWriter::new(open_output(out)?);
    replay_segment(Path::new(segment), &config, writer)
        .map_err(|error| Failure::Input(format!("{error}")))?;
    Ok(ExitCode::SUCCESS)
}

// Identity, not spelling: dev and ino catch a symlink or hard link to the input too.
fn refuse_out_is_input(out: &str, segment: &str) -> Result<(), Failure> {
    let (Ok(out_meta), Ok(in_meta)) = (fs::metadata(out), fs::metadata(segment)) else {
        return Ok(());
    };
    if out_meta.dev() == in_meta.dev() && out_meta.ino() == in_meta.ino() {
        return Err(Failure::Input(format!("--out is the input file {out}")));
    }
    Ok(())
}

fn camera_from_file_name(path: &str) -> Result<String, Failure> {
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Failure::Input(format!("{path} has no usable file name")))?;
    SegmentName::parse(name)
        .map(|parsed| parsed.camera().to_owned())
        .map_err(|error| Failure::Input(format!("{path}: segment name: {error}")))
}

fn parse_confirm(text: Option<&str>) -> Result<f64, Failure> {
    let Some(text) = text else {
        return Ok(DEFAULT_CONFIRM);
    };
    text.parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0 && *value <= 1.0)
        .ok_or(Failure::Usage)
}

fn parse_area(flags: &Flags) -> Result<Option<Area>, Failure> {
    let Some(text) = flags.get("area") else {
        if flags.get("area-name").is_some() || flags.get("area-pad").is_some() {
            return Err(Failure::Usage);
        }
        return Ok(None);
    };
    let name = flags
        .get("area-name")
        .map_or_else(|| DEFAULT_AREA_NAME.to_owned(), str::to_owned);
    let pad_pixels = flags.bounded("area-pad", 0, MAX_AREA_PAD, DEFAULT_AREA_PAD)?;
    let parsed: Result<Vec<f64>, _> = text.split(',').map(str::parse::<f64>).collect();
    let values = parsed
        .ok()
        .filter(|values| {
            values.len() >= MIN_AREA_VALUES
                && values.len().is_multiple_of(2)
                && values
                    .iter()
                    .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        })
        .ok_or(Failure::Usage)?;
    let mut polygon: Vec<(f64, f64)> = Vec::new();
    for pair in values.chunks_exact(2) {
        let [x, y] = pair else {
            return Err(Failure::Usage);
        };
        polygon.push((*x, *y));
    }
    Ok(Some(Area {
        name,
        polygon,
        pad_pixels,
    }))
}
