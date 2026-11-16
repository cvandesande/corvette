//! Runs `corvette_motion::MotionDetector` over a raw luma file, one text line per picture.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::process::ExitCode;

use corvette_motion::{
    Decision, Image, MotionDetector, MotionSettings, Plane, Shape, motion_keep_mask,
};

use crate::Failure;
use crate::args::Flags;

const MOTION_FLAGS: &[&str] = &[
    "luma",
    "width",
    "height",
    "out",
    "threshold",
    "contour-area",
    "improve-contrast",
    "lightning-threshold",
    "frame-alpha",
    "frame-height",
    "mask",
];

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let flags = Flags::parse(args, MOTION_FLAGS).ok_or(Failure::Usage)?;
    let settings = settings_from(&flags)?;
    let shape = shape_from(&flags)?;
    let mask = mask_from(&flags, shape, settings.frame_height)?;
    let mut detector = MotionDetector::new(settings, shape, mask.as_ref().map(Image::plane))
        .ok_or_else(|| {
            Failure::Input(format!(
                "no detector for a {} by {} picture with these settings",
                shape.width(),
                shape.height()
            ))
        })?;
    let luma = flags.required("luma")?;
    let out = flags.required("out")?;
    let file = open_input(luma)?;
    let bytes = file
        .metadata()
        .map_err(|error| Failure::Input(format!("cannot inspect {luma}: {error}")))?
        .len();
    let picture_bytes = u64::try_from(shape.pixels())
        .map_err(|_| Failure::Input(String::from("a picture is too large")))?;
    if bytes == 0 || bytes % picture_bytes != 0 {
        return Err(Failure::Input(format!(
            "{luma} is {bytes} bytes, a whole picture is {picture_bytes} bytes"
        )));
    }
    let pictures = usize::try_from(bytes / picture_bytes)
        .map_err(|_| Failure::Input(String::from("too many pictures")))?;
    let mut reader = BufReader::new(file);
    let mut writer = BufWriter::new(open_output(out)?);
    let (with_boxes, calibrating) =
        replay(&mut reader, &mut writer, &mut detector, shape, pictures)?;
    writer
        .flush()
        .map_err(|error| Failure::Input(format!("cannot finish writing {out}: {error}")))?;
    writeln!(
        std::io::stdout(),
        "pictures {pictures}, with boxes {with_boxes}, calibrating {calibrating}"
    )
    .map_err(|error| Failure::Input(format!("cannot write the summary: {error}")))?;
    Ok(ExitCode::SUCCESS)
}

fn settings_from(flags: &Flags) -> Result<MotionSettings, Failure> {
    let mut settings = MotionSettings::default();
    if let Some(text) = flags.get("threshold") {
        settings.threshold = text.parse::<u8>().map_err(|_| Failure::Usage)?;
    }
    if let Some(text) = flags.get("contour-area") {
        settings.contour_area = text.parse::<u32>().map_err(|_| Failure::Usage)?;
    }
    if let Some(text) = flags.get("improve-contrast") {
        settings.improve_contrast = match text {
            "1" => true,
            "0" => false,
            _ => return Err(Failure::Usage),
        };
    }
    if let Some(text) = flags.get("lightning-threshold") {
        settings.lightning_threshold = finite_number(text)?;
    }
    if let Some(text) = flags.get("frame-alpha") {
        settings.frame_alpha = finite_number(text)?;
    }
    if let Some(text) = flags.get("frame-height") {
        settings.frame_height = text.parse::<usize>().map_err(|_| Failure::Usage)?;
    }
    Ok(settings)
}

fn finite_number(text: &str) -> Result<f64, Failure> {
    text.parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or(Failure::Usage)
}

fn shape_from(flags: &Flags) -> Result<Shape, Failure> {
    let width = side(flags, "width")?;
    let height = side(flags, "height")?;
    Shape::new(width, height)
        .ok_or_else(|| Failure::Input(format!("no picture shape for {width} by {height}")))
}

fn side(flags: &Flags, name: &str) -> Result<usize, Failure> {
    flags
        .required(name)?
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or(Failure::Usage)
}

fn mask_from(flags: &Flags, shape: Shape, frame_height: usize) -> Result<Option<Image>, Failure> {
    let Some(text) = flags.get("mask") else {
        return Ok(None);
    };
    let mut numbers: Vec<f64> = Vec::new();
    for part in text.split(',') {
        numbers.push(part.parse::<f64>().map_err(|_| Failure::Usage)?);
    }
    if numbers.len() < 6 || numbers.len() % 2 == 1 {
        return Err(Failure::Usage);
    }
    let mut polygon: Vec<(f64, f64)> = Vec::with_capacity(numbers.len() / 2);
    for pair in numbers.chunks_exact(2) {
        if let [x, y] = pair {
            polygon.push((*x, *y));
        }
    }
    let keep = motion_keep_mask(shape, frame_height, std::slice::from_ref(&polygon))
        .ok_or_else(|| Failure::Input(format!("the --mask value is refused: {text}")))?;
    Ok(Some(keep))
}

pub(crate) fn open_input(path: &str) -> Result<File, Failure> {
    File::open(path).map_err(|error| Failure::Input(format!("cannot read {path}: {error}")))
}

pub(crate) fn open_output(path: &str) -> Result<File, Failure> {
    File::create(path).map_err(|error| Failure::Input(format!("cannot write {path}: {error}")))
}

fn replay(
    reader: &mut impl Read,
    writer: &mut impl Write,
    detector: &mut MotionDetector,
    shape: Shape,
    pictures: usize,
) -> Result<(usize, usize), Failure> {
    let mut picture = vec![0u8; shape.pixels()];
    let mut with_boxes = 0_usize;
    let mut calibrating = 0_usize;
    for index in 0..pictures {
        reader
            .read_exact(&mut picture)
            .map_err(|error| Failure::Input(format!("cannot read picture {index}: {error}")))?;
        let plane = Plane::new(&picture, shape)
            .ok_or_else(|| Failure::Input(format!("picture {index} does not match its shape")))?;
        let decision = detector
            .feed(plane)
            .ok_or_else(|| Failure::Input(format!("picture {index} has the wrong shape")))?;
        if !decision.boxes.is_empty() {
            with_boxes += 1;
        }
        if decision.calibrating {
            calibrating += 1;
        }
        writer
            .write_all(line_for(index, &decision).as_bytes())
            .map_err(|error| Failure::Input(format!("cannot write a line: {error}")))?;
    }
    Ok((with_boxes, calibrating))
}

fn line_for(index: usize, decision: &Decision) -> String {
    let boxes = if decision.boxes.is_empty() {
        String::from("-")
    } else {
        decision
            .boxes
            .iter()
            .map(|one| format!("{},{},{},{}", one.x1, one.y1, one.x2, one.y2))
            .collect::<Vec<_>>()
            .join(";")
    };
    let calibrating = u8::from(decision.calibrating);
    format!("{index}\t{calibrating}\t{boxes}\n")
}
