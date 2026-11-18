//! Matches answer-key events against replayed track records for the comparison report.

pub(crate) mod report;

use std::cmp::Ordering;
use std::collections::HashSet;
use std::io::Read;
use std::process::ExitCode;

use serde_json::Value;

use crate::Failure;
use crate::args::Flags;
use crate::motion_detect::{open_input, open_output};

const COMPARE_FLAGS: &[&str] = &["events", "out"];
const OPEN_END: f64 = f64::INFINITY;

struct Event {
    id: String,
    camera: String,
    label: String,
    start: f64,
    end: Option<f64>,
    zones: Vec<String>,
}

struct Track {
    record: usize,
    path: String,
    id: String,
    camera: String,
    label: String,
    start: f64,
    end: Option<f64>,
    zones: Vec<String>,
    first_frame: f64,
    last_frame: f64,
}

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let (records, rest) = take_records(args)?;
    let flags = Flags::parse(&rest, COMPARE_FLAGS).ok_or(Failure::Usage)?;
    let key_path = flags.required("events")?;
    let out = flags.required("out")?;
    let events = read_events(key_path)?;
    let mut tracks: Vec<Track> = Vec::new();
    for (record, path) in records.iter().enumerate() {
        let text = read_file(path)?;
        tracks.extend(read_tracks(record, path, &text)?);
    }
    let pairs = match_events(&events, &tracks);
    let body = report::render(&events, &tracks, &pairs);
    let mut output = open_output(out)?;
    std::io::Write::write_all(&mut output, body.as_bytes())
        .map_err(|error| Failure::Input(format!("cannot write {out}: {error}")))?;
    Ok(ExitCode::SUCCESS)
}

fn take_records(args: &[String]) -> Result<(Vec<String>, Vec<String>), Failure> {
    let mut records: Vec<String> = Vec::new();
    let mut rest: Vec<String> = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--record" {
            records.push(iter.next().ok_or(Failure::Usage)?.clone());
        } else {
            rest.push(arg.clone());
        }
    }
    if records.is_empty() {
        return Err(Failure::Usage);
    }
    Ok((records, rest))
}

fn read_file(path: &str) -> Result<String, Failure> {
    let mut input = open_input(path)?;
    let mut text = String::new();
    input
        .read_to_string(&mut text)
        .map_err(|error| Failure::Input(format!("cannot read {path}: {error}")))?;
    Ok(text)
}

fn read_events(path: &str) -> Result<Vec<Event>, Failure> {
    let text = read_file(path)?;
    let key: Value = serde_json::from_str(&text)
        .map_err(|_| Failure::Input(format!("{path}: key is not valid JSON")))?;
    if key.get("schema").and_then(Value::as_i64) != Some(1) {
        return Err(Failure::Input(format!("{path}: schema is not 1")));
    }
    let items = key
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| Failure::Input(format!("{path}: events is not an array")))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| read_event(item, index))
        .collect()
}

fn read_event(item: &Value, index: usize) -> Result<Event, Failure> {
    let note = |problem: String| Failure::Input(format!("event {index}: {problem}"));
    let end = item
        .get("end")
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| note(String::from("end is not a number or null")))
        })
        .transpose()?;
    Ok(Event {
        id: field_text(item, "id").map_err(note)?,
        camera: field_text(item, "camera").map_err(note)?,
        label: field_text(item, "label").map_err(note)?,
        start: field_number(item, "start").map_err(note)?,
        end,
        zones: zone_set(item.get("zones")).map_err(note)?,
    })
}

fn read_tracks(record: usize, path: &str, text: &str) -> Result<Vec<Track>, Failure> {
    let body = text.strip_suffix('\n').unwrap_or(text);
    let mut lines = body.split('\n').enumerate();
    let Some((_, header_text)) = lines.next().filter(|(_, text)| !text.is_empty()) else {
        return Err(Failure::Input(format!("{path}: line 1: no header line")));
    };
    let header = json_object(header_text)
        .map_err(|problem| Failure::Input(format!("{path}: line 1: header {problem}")))?;
    if header.get("schema").and_then(Value::as_i64) != Some(1) {
        return Err(Failure::Input(format!("{path}: line 1: schema is not 1")));
    }
    let camera = header
        .get("camera")
        .and_then(Value::as_str)
        .ok_or_else(|| Failure::Input(format!("{path}: line 1: camera is missing")))?;
    let mut tracks: Vec<Track> = Vec::new();
    for (position, line_text) in lines {
        read_frame(record, path, camera, position + 1, line_text, &mut tracks)?;
    }
    Ok(tracks)
}

fn read_frame(
    record: usize,
    path: &str,
    camera: &str,
    lineno: usize,
    line_text: &str,
    tracks: &mut Vec<Track>,
) -> Result<(), Failure> {
    let frame_note =
        |problem: String| Failure::Input(format!("{path}: line {lineno}: frame {problem}"));
    let field_note = |problem: String| Failure::Input(format!("{path}: line {lineno}: {problem}"));
    let object_note = |object: usize, problem: String| {
        Failure::Input(format!("{path}: line {lineno}: object {object}: {problem}"))
    };
    let frame = json_object(line_text).map_err(frame_note)?;
    let frame_time = field_number(&frame, "frame_time").map_err(field_note)?;
    let empty: Vec<Value> = Vec::new();
    let objects = match frame.get("objects") {
        None => &empty[..],
        Some(Value::Array(items)) => items,
        Some(_) => return Err(field_note(String::from("objects is not an array"))),
    };
    for (object_index, object) in objects.iter().enumerate() {
        let note = |problem: String| object_note(object_index, problem);
        let id = field_text(object, "id").map_err(note)?;
        let label = field_text(object, "label").map_err(note)?;
        let confirmed = field_bool(object, "confirmed").map_err(note)?;
        let start = field_number(object, "first_seen").map_err(note)?;
        let zones = zone_set(object.get("zones_entered")).map_err(note)?;
        let mut known = false;
        for track in tracks.iter_mut() {
            if track.id == id {
                track.label.clone_from(&label);
                track.zones.clone_from(&zones);
                track.last_frame = frame_time;
                known = true;
            }
        }
        if known || !confirmed {
            continue;
        }
        tracks.push(Track {
            record,
            path: path.to_string(),
            camera: camera.to_string(),
            id,
            label,
            start,
            end: None,
            zones,
            first_frame: frame_time,
            last_frame: frame_time,
        });
    }
    let ended = match frame.get("ended") {
        None => &empty[..],
        Some(Value::Array(items)) => items,
        Some(_) => return Err(field_note(String::from("ended is not an array"))),
    };
    for entry in ended {
        let ended_id = entry
            .as_str()
            .ok_or_else(|| field_note(String::from("ended is not an array of strings")))?;
        for track in tracks.iter_mut() {
            if track.id == ended_id && track.end.is_none() {
                track.end = Some(frame_time);
            }
        }
    }
    Ok(())
}

fn json_object(line_text: &str) -> Result<Value, String> {
    let value: Value =
        serde_json::from_str(line_text).map_err(|_| String::from("is not valid JSON"))?;
    match value {
        Value::Object(_) => Ok(value),
        _ => Err(String::from("is not a JSON object")),
    }
}

fn field_text(item: &Value, key: &str) -> Result<String, String> {
    item.get(key).map_or_else(
        || Err(format!("{key} is missing")),
        |value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{key} is not a string"))
        },
    )
}

fn field_number(item: &Value, key: &str) -> Result<f64, String> {
    item.get(key).map_or_else(
        || Err(format!("{key} is missing")),
        |value| {
            value
                .as_f64()
                .ok_or_else(|| format!("{key} is not a number"))
        },
    )
}

fn field_bool(item: &Value, key: &str) -> Result<bool, String> {
    item.get(key).map_or_else(
        || Err(format!("{key} is missing")),
        |value| {
            value
                .as_bool()
                .ok_or_else(|| format!("{key} is not a boolean"))
        },
    )
}

fn zone_set(value: Option<&Value>) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let entries = value
        .as_array()
        .ok_or_else(|| String::from("zones are not an array"))?;
    let mut names: Vec<&str> = Vec::new();
    for entry in entries {
        names.push(entry.as_str().ok_or("zones are not an array of strings")?);
    }
    names.sort_unstable();
    names.dedup();
    Ok(names.into_iter().map(str::to_owned).collect())
}

fn match_events(events: &[Event], tracks: &[Track]) -> Vec<(usize, usize)> {
    let mut used: HashSet<usize> = HashSet::new();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (event_index, event) in events.iter().enumerate() {
        let event_end = event.end.unwrap_or(OPEN_END);
        let mut best: Option<(usize, &Track, f64)> = None;
        for (track_index, track) in tracks.iter().enumerate() {
            if used.contains(&track_index)
                || track.camera != event.camera
                || track.label != event.label
            {
                continue;
            }
            let track_end = track.end.unwrap_or(OPEN_END);
            let overlap = track_end.min(event_end) - track.start.max(event.start);
            if overlap.partial_cmp(&0.0) != Some(Ordering::Greater) {
                continue;
            }
            let chosen = match best {
                None => true,
                Some((_, current, current_overlap)) => {
                    takes_over(track, current, overlap, current_overlap)
                }
            };
            if chosen {
                best = Some((track_index, track, overlap));
            }
        }
        if let Some((track_index, _, _)) = best {
            used.insert(track_index);
            pairs.push((event_index, track_index));
        }
    }
    pairs
}

// A larger overlap wins; total_cmp keeps the order total, then earlier start, record order, id bytes.
fn takes_over(
    candidate: &Track,
    current: &Track,
    candidate_overlap: f64,
    current_overlap: f64,
) -> bool {
    current_overlap
        .total_cmp(&candidate_overlap)
        .then_with(|| candidate.start.total_cmp(&current.start))
        .then_with(|| candidate.record.cmp(&current.record))
        .then_with(|| candidate.id.cmp(&current.id))
        .is_lt()
}
