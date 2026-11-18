//! Writes a replay record file: one header line, then one compact JSON object per frame.

use std::fmt::Write as _;
use std::io::Write;

use crate::object_stage::{FrameObjects, TrackedObject};

#[derive(Clone, Debug)]
pub struct RecordHeader {
    pub camera: String,
    pub segment: String,
    pub frame_width: u32,
    pub frame_height: u32,
    pub detect_fps: f64,
}

#[derive(Debug)]
pub enum RecordError {
    Io(String),
    NonFinite(&'static str),
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(detail) => write!(f, "{detail}"),
            Self::NonFinite(field) => write!(f, "record number field {field} is not finite"),
        }
    }
}

impl std::error::Error for RecordError {}

#[derive(Debug)]
pub struct RecordWriter<W: Write> {
    writer: W,
}

impl<W: Write> RecordWriter<W> {
    /// # Errors
    pub fn new(mut writer: W, header: &RecordHeader) -> Result<Self, RecordError> {
        let line = header_line(header)?;
        write_line(&mut writer, "header", &line)?;
        Ok(Self { writer })
    }

    /// Writes one frame line, built whole before it is emitted. After a failed write the writer's state is unspecified and later calls have no defined effect.
    /// # Errors
    pub fn write_frame(&mut self, frame: &FrameObjects) -> Result<(), RecordError> {
        let line = frame_line(frame)?;
        write_line(&mut self.writer, "frame", &line)
    }

    #[must_use]
    pub fn finish(self) -> W {
        self.writer
    }
}

fn write_line<W: Write>(writer: &mut W, what: &str, line: &str) -> Result<(), RecordError> {
    writer
        .write_all(line.as_bytes())
        .map_err(|error| RecordError::Io(format!("write record {what} line: {error}")))
}

fn header_line(header: &RecordHeader) -> Result<String, RecordError> {
    let detect_fps = number(header.detect_fps, "detect_fps")?;
    let mut line = String::new();
    let _ = writeln!(
        line,
        "{{\"schema\":1,\"camera\":{camera},\"segment\":{segment},\"frame_width\":{width},\"frame_height\":{height},\"detect_fps\":{detect_fps}}}",
        camera = json_string(&header.camera),
        segment = json_string(&header.segment),
        width = header.frame_width,
        height = header.frame_height,
    );
    Ok(line)
}

fn frame_line(frame: &FrameObjects) -> Result<String, RecordError> {
    let frame_time = number(frame.frame_time, "frame_time")?;
    let mut objects: Vec<String> = Vec::with_capacity(frame.objects.len());
    for object in &frame.objects {
        objects.push(object_line(object)?);
    }
    let ended = json_array(&frame.ended);
    let mut line = String::new();
    let _ = writeln!(
        line,
        "{{\"frame_time\":{frame_time},\"objects\":[{}],\"ended\":[{ended}]}}",
        objects.join(","),
    );
    Ok(line)
}

fn object_line(object: &TrackedObject) -> Result<String, RecordError> {
    let score = number(object.score, "score")?;
    let first_seen = number(object.first_seen, "first_seen")?;
    let last_seen = number(object.last_seen, "last_seen")?;
    let mut box_edges: Vec<String> = Vec::with_capacity(object.bounds.len());
    for edge in object.bounds {
        box_edges.push(number(edge, "box")?);
    }
    let box_text = box_edges.join(",");
    let zones_now = json_array(&object.zones_now);
    let zones_entered = json_array(&object.zones_entered);
    let state = if object.stationary {
        "stationary"
    } else {
        "moving"
    };
    let mut line = String::new();
    let _ = write!(
        line,
        "{{\"id\":{id},\"label\":{label},\"score\":{score},\"confirmed\":{confirmed},\"box\":[{box_text}],\"first_seen\":{first_seen},\"last_seen\":{last_seen},\"zones_now\":[{zones_now}],\"zones_entered\":[{zones_entered}],\"state\":\"{state}\"}}",
        id = json_string(&object.id),
        label = json_string(&object.label),
        confirmed = object.confirmed,
    );
    Ok(line)
}

fn json_array(values: &[String]) -> String {
    values
        .iter()
        .map(|value| json_string(value))
        .collect::<Vec<String>>()
        .join(",")
}

fn json_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            control @ '\0'..='\u{1f}' => {
                let _ = write!(quoted, "\\u{:04x}", u32::from(control));
            }
            plain => quoted.push(plain),
        }
    }
    quoted.push('"');
    quoted
}

fn number(value: f64, field: &'static str) -> Result<String, RecordError> {
    if !value.is_finite() {
        return Err(RecordError::NonFinite(field));
    }
    let text = format!("{value}");
    if text.contains('.') {
        return Ok(text);
    }
    Ok(format!("{text}.0"))
}
