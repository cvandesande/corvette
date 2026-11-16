//! Replays the box file of `motion-detect` through `RegionBuilder` as the detector's squares.

use std::io::{BufWriter, Read, Write};
use std::process::ExitCode;

use corvette_motion::{Region, RegionBuilder, Shape, SizeGrid, min_region_size};

use crate::Failure;
use crate::args::Flags;
use crate::motion_detect::{open_input, open_output, shape_from};

const REGION_FLAGS: &[&str] = &["boxes", "width", "height", "out", "model-size"];
const MODEL_MIN: u32 = 1;
const MODEL_MAX: u32 = 8192;
const MODEL_DEFAULT: u32 = 640;

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let flags = Flags::parse(args, REGION_FLAGS).ok_or(Failure::Usage)?;
    let frame = shape_from(&flags)?;
    let model = flags.bounded("model-size", MODEL_MIN, MODEL_MAX, MODEL_DEFAULT)?;
    let model = usize::try_from(model).map_err(|_| Failure::Usage)?;
    let model_shape = Shape::new(model, model).ok_or(Failure::Usage)?;
    let floor = min_region_size(model_shape);
    let path = flags.required("boxes")?;
    let out = flags.required("out")?;
    let mut input = open_input(path)?;
    let mut text = String::new();
    input
        .read_to_string(&mut text)
        .map_err(|error| Failure::Input(format!("cannot read {path}: {error}")))?;
    if text.is_empty() {
        return Err(Failure::Input(String::from(
            "line 1: the boxes file is empty",
        )));
    }
    let body = text
        .strip_suffix('\n')
        .map_or(text.as_str(), |stripped| stripped);
    let mut writer = BufWriter::new(open_output(out)?);
    let mut builder = RegionBuilder::new(floor, SizeGrid::empty());
    let pictures = body.split('\n').count();
    let mut with_regions = 0_usize;
    let mut region_total = 0_usize;
    for (position, line) in body.split('\n').enumerate() {
        let lineno = position + 1;
        let (index, calibrating, boxes) = three_fields(line, lineno)?;
        let number = digits(index)
            .ok_or_else(|| Failure::Input(format!("line {lineno}: index {index} is not digits")))?;
        if number != position {
            return Err(Failure::Input(format!(
                "line {lineno}: index {number} is not {position}"
            )));
        }
        let calibrating = match calibrating {
            "0" => false,
            "1" => true,
            other => {
                return Err(Failure::Input(format!(
                    "line {lineno}: calibrating {other} is not 0 or 1"
                )));
            }
        };
        let motion = parse_boxes(boxes, lineno)?;
        let mut squares = builder
            .build(frame, &[], &motion, !calibrating)
            .ok_or_else(|| {
                Failure::Input(format!(
                    "picture {number}: the boxes are outside the supported range"
                ))
            })?;
        squares.sort_by_key(|square| (square.x1, square.y1, square.x2, square.y2));
        if !squares.is_empty() {
            with_regions += 1;
        }
        region_total += squares.len();
        let text_line = format!("{number}\t{}\n", squares_text(&squares));
        writer
            .write_all(text_line.as_bytes())
            .map_err(|error| Failure::Input(format!("cannot write a line: {error}")))?;
    }
    writer
        .flush()
        .map_err(|error| Failure::Input(format!("cannot finish writing {out}: {error}")))?;
    writeln!(
        std::io::stdout(),
        "pictures {pictures}, with regions {with_regions}, regions {region_total}"
    )
    .map_err(|error| Failure::Input(format!("cannot write the summary: {error}")))?;
    Ok(ExitCode::SUCCESS)
}

fn three_fields(line: &str, lineno: usize) -> Result<(&str, &str, &str), Failure> {
    let fields: Vec<&str> = line.split('\t').collect();
    let [index, calibrating, boxes] = fields.as_slice() else {
        let message = format!(
            "line {lineno}: expected 3 tab-separated fields, found {}",
            fields.len()
        );
        return Err(Failure::Input(message));
    };
    Ok((index, calibrating, boxes))
}

fn digits(text: &str) -> Option<usize> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<usize>().ok()
}

fn parse_boxes(text: &str, lineno: usize) -> Result<Vec<Region>, Failure> {
    if text == "-" {
        return Ok(Vec::new());
    }
    text.split(';')
        .map(|group| region_from(group, lineno))
        .collect()
}

fn region_from(text: &str, lineno: usize) -> Result<Region, Failure> {
    let parts: Vec<&str> = text.split(',').collect();
    let [x1, y1, x2, y2] = parts.as_slice() else {
        let message = format!(
            "line {lineno}: a box has {} numbers, expected 4",
            parts.len()
        );
        return Err(Failure::Input(message));
    };
    Ok(Region {
        x1: coordinate(x1, lineno)?,
        y1: coordinate(y1, lineno)?,
        x2: coordinate(x2, lineno)?,
        y2: coordinate(y2, lineno)?,
    })
}

fn coordinate(text: &str, lineno: usize) -> Result<i32, Failure> {
    let value = digits(text)
        .ok_or_else(|| Failure::Input(format!("line {lineno}: coordinate {text} is not digits")))?;
    i32::try_from(value)
        .map_err(|_| Failure::Input(format!("line {lineno}: coordinate {text} is out of range")))
}

fn squares_text(squares: &[Region]) -> String {
    if squares.is_empty() {
        return String::from("-");
    }
    squares
        .iter()
        .map(|square| format!("{},{},{},{}", square.x1, square.y1, square.x2, square.y2))
        .collect::<Vec<_>>()
        .join(";")
}
