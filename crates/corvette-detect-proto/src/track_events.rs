//! One newline-terminated line per track frame on the track-event subscriber socket: a version byte, a kind byte and tab-separated fields.

use std::fmt;

pub const LINE_VERSION: u8 = b'1';
const SEP: u8 = b'\t';
const LINE_END: u8 = b'\n';
const ZONE_SEP: u8 = b',';
const KIND_START: u8 = b's';
const KIND_UPDATE: u8 = b'u';
const KIND_END: u8 = b'e';
const NO_VALUE: &str = "-";
const INFO_FIELDS: usize = 10;
const END_FIELDS: usize = 2;
const HEX: &[u8; 16] = b"0123456789ABCDEF";

#[derive(Clone, Debug, PartialEq)]
pub struct TrackInfo {
    pub camera: String,
    pub track_id: String,
    pub label: String,
    pub bounds: [f64; 4],
    pub zones: Vec<String>,
    pub first_motion: Option<f64>,
    pub last_seen: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TrackFrame {
    Start(TrackInfo),
    Update(TrackInfo),
    End { camera: String, track_id: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineError {
    Empty,
    Version(u8),
    Shape,
    Kind(u8),
    Fields { wanted: usize, found: usize },
    Number(&'static str),
    Escape,
    Control(u8),
}

impl fmt::Display for LineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "line is shorter than the version, separator and kind"),
            Self::Version(byte) => write!(f, "unknown track line version {byte}"),
            Self::Shape => {
                write!(
                    f,
                    "line does not separate the version or kind byte with a tab"
                )
            }
            Self::Kind(byte) => write!(f, "unknown track line kind byte {byte}"),
            Self::Fields { wanted, found } => {
                write!(f, "line has {found} fields, expected {wanted}")
            }
            Self::Number(field) => write!(f, "field {field} is not a number"),
            Self::Escape => write!(
                f,
                "percent escape is not two hex digits or the result is not UTF-8"
            ),
            Self::Control(byte) => {
                write!(f, "raw control byte {byte} outside an escape")
            }
        }
    }
}

impl std::error::Error for LineError {}

#[must_use]
pub fn encode_track_line(frame: &TrackFrame) -> Vec<u8> {
    let mut line = vec![LINE_VERSION, SEP];
    match frame {
        TrackFrame::Start(info) => {
            line.push(KIND_START);
            push_info(info, &mut line);
        }
        TrackFrame::Update(info) => {
            line.push(KIND_UPDATE);
            push_info(info, &mut line);
        }
        TrackFrame::End { camera, track_id } => {
            line.push(KIND_END);
            push_field(camera.as_bytes(), &mut line);
            push_field(track_id.as_bytes(), &mut line);
        }
    }
    line.push(LINE_END);
    line
}

/// # Errors
/// `LineError` on a wrong version or kind byte, a wrong field count, a field that is not a number, or a broken percent escape.
pub fn decode_track_line(line: &[u8]) -> Result<TrackFrame, LineError> {
    let head = line.get(..4).ok_or(LineError::Empty)?;
    if head[0] != LINE_VERSION {
        return Err(LineError::Version(head[0]));
    }
    if head[1] != SEP || head[3] != SEP {
        return Err(LineError::Shape);
    }
    let fields: Vec<&[u8]> = line[4..].split(|byte| *byte == SEP).collect();
    match head[2] {
        KIND_START => Ok(TrackFrame::Start(decode_info(&fields)?)),
        KIND_UPDATE => Ok(TrackFrame::Update(decode_info(&fields)?)),
        KIND_END => {
            if fields.len() != END_FIELDS {
                return Err(LineError::Fields {
                    wanted: END_FIELDS,
                    found: fields.len(),
                });
            }
            Ok(TrackFrame::End {
                camera: unescape(fields[0])?,
                track_id: unescape(fields[1])?,
            })
        }
        other => Err(LineError::Kind(other)),
    }
}

fn push_info(info: &TrackInfo, line: &mut Vec<u8>) {
    push_field(info.camera.as_bytes(), line);
    push_field(info.track_id.as_bytes(), line);
    push_field(info.label.as_bytes(), line);
    for bound in info.bounds {
        push_field(bound.to_string().as_bytes(), line);
    }
    let mut zones = Vec::new();
    for zone in &info.zones {
        if !zones.is_empty() {
            zones.push(ZONE_SEP);
        }
        escaped(zone.as_bytes(), &mut zones);
    }
    // The zones buffer is already escaped, so this separator is written directly instead of re-escaping.
    line.push(SEP);
    line.extend_from_slice(&zones);
    match info.first_motion {
        Some(time) => push_field(time.to_string().as_bytes(), line),
        None => push_field(NO_VALUE.as_bytes(), line),
    }
    push_field(info.last_seen.to_string().as_bytes(), line);
}

fn decode_info(fields: &[&[u8]]) -> Result<TrackInfo, LineError> {
    if fields.len() != INFO_FIELDS {
        return Err(LineError::Fields {
            wanted: INFO_FIELDS,
            found: fields.len(),
        });
    }
    Ok(TrackInfo {
        camera: unescape(fields[0])?,
        track_id: unescape(fields[1])?,
        label: unescape(fields[2])?,
        bounds: [
            number("x1", fields[3])?,
            number("y1", fields[4])?,
            number("x2", fields[5])?,
            number("y2", fields[6])?,
        ],
        zones: decode_zones(fields[7])?,
        first_motion: if fields[8] == NO_VALUE.as_bytes() {
            None
        } else {
            Some(number("first_motion", fields[8])?)
        },
        last_seen: number("last_seen", fields[9])?,
    })
}

fn decode_zones(field: &[u8]) -> Result<Vec<String>, LineError> {
    if field.is_empty() {
        return Ok(Vec::new());
    }
    field
        .split(|byte| *byte == ZONE_SEP)
        .map(unescape)
        .collect()
}

fn number(name: &'static str, field: &[u8]) -> Result<f64, LineError> {
    std::str::from_utf8(field)
        .map_err(|_| LineError::Number(name))?
        .parse::<f64>()
        .map_err(|_| LineError::Number(name))
}

fn push_field(field: &[u8], line: &mut Vec<u8>) {
    line.push(SEP);
    escaped(field, line);
}

fn escaped(field: &[u8], out: &mut Vec<u8>) {
    for &byte in field {
        match byte {
            b'%' => out.extend_from_slice(b"%25"),
            SEP => out.extend_from_slice(b"%09"),
            b',' => out.extend_from_slice(b"%2C"),
            b'\n' => out.extend_from_slice(b"%0A"),
            b'\r' => out.extend_from_slice(b"%0D"),
            0x20..=0x7e => out.push(byte),
            other => {
                out.push(b'%');
                out.push(HEX[usize::from(other >> 4)]);
                out.push(HEX[usize::from(other & 0x0f)]);
            }
        }
    }
}

fn unescape(field: &[u8]) -> Result<String, LineError> {
    let mut bytes = Vec::with_capacity(field.len());
    let mut index = 0;
    while index < field.len() {
        if !(0x20..=0x7e).contains(&field[index]) {
            return Err(LineError::Control(field[index]));
        }
        if field[index] != b'%' {
            bytes.push(field[index]);
            index += 1;
            continue;
        }
        let hex = field.get(index + 1..index + 3).ok_or(LineError::Escape)?;
        let hex = std::str::from_utf8(hex).map_err(|_| LineError::Escape)?;
        let byte = u8::from_str_radix(hex, 16).map_err(|_| LineError::Escape)?;
        bytes.push(byte);
        index += 3;
    }
    String::from_utf8(bytes).map_err(|_| LineError::Escape)
}
