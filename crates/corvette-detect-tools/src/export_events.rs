//! Reads a saved events response and writes the answer-key document it compares against.

use std::io::Read;
use std::io::Write;
use std::process::ExitCode;

use serde_json::Value;

use crate::Failure;
use crate::args::Flags;
use crate::motion_detect::{open_input, open_output};

const EXPORT_FLAGS: &[&str] = &["in", "out", "camera", "label"];

struct EventRow {
    id: String,
    camera: String,
    label: String,
    start: f64,
    end: Option<f64>,
    zones: Vec<String>,
    top_score: Option<f64>,
    score: Option<f64>,
    box_values: Option<[f64; 4]>,
    has_clip: bool,
}

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let flags = Flags::parse(args, EXPORT_FLAGS).ok_or(Failure::Usage)?;
    let path = flags.required("in")?;
    let out = flags.required("out")?;
    let camera = flags.get("camera");
    let label = flags.get("label");
    let mut input = open_input(path)?;
    let mut text = String::new();
    input
        .read_to_string(&mut text)
        .map_err(|error| Failure::Input(format!("cannot read {path}: {error}")))?;
    let answer: Value = serde_json::from_str(&text)
        .map_err(|error| Failure::Input(format!("input is not valid JSON: {error}")))?;
    let items = answer
        .as_array()
        .ok_or_else(|| Failure::Input(String::from("input is not a JSON array")))?;
    let mut rows: Vec<EventRow> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let row = read_event(item, index)?;
        if camera.is_some_and(|wanted| row.camera != wanted) {
            continue;
        }
        if label.is_some_and(|wanted| row.label != wanted) {
            continue;
        }
        rows.push(row);
    }
    // total_cmp keeps the order total and so deterministic; JSON holds no NaN.
    rows.sort_by(|left, right| {
        left.start
            .total_cmp(&right.start)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut body = String::from("{\"schema\": 1, \"source\": \"frigate-events\", \"events\": [");
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            body.push_str(", ");
        }
        body.push_str(&event_text(row)?);
    }
    body.push_str("]}\n");
    let mut output = open_output(out)?;
    output
        .write_all(body.as_bytes())
        .map_err(|error| Failure::Input(format!("cannot write {out}: {error}")))?;
    Ok(ExitCode::SUCCESS)
}

fn read_event(item: &Value, index: usize) -> Result<EventRow, Failure> {
    let end = match item.get("end_time") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_f64()
                .ok_or_else(|| not_type(index, "end_time", "a number or null"))?,
        ),
    };
    let data = item.get("data").filter(|value| !value.is_null());
    Ok(EventRow {
        id: required_text(item, index, "id")?,
        camera: required_text(item, index, "camera")?,
        label: required_text(item, index, "label")?,
        start: required_number(item, index, "start_time")?,
        end,
        zones: read_zones(item, index)?,
        top_score: data
            .and_then(|value| value.get("top_score"))
            .and_then(Value::as_f64),
        score: data
            .and_then(|value| value.get("score"))
            .and_then(Value::as_f64),
        box_values: read_box(data, index)?,
        has_clip: item
            .get("has_clip")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn required_text(item: &Value, index: usize, key: &str) -> Result<String, Failure> {
    let value = item.get(key).ok_or_else(|| missing(index, key))?;
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| not_type(index, key, "a string"))
}

fn required_number(item: &Value, index: usize, key: &str) -> Result<f64, Failure> {
    let value = item.get(key).ok_or_else(|| missing(index, key))?;
    value
        .as_f64()
        .ok_or_else(|| not_type(index, key, "a number"))
}

fn read_zones(item: &Value, index: usize) -> Result<Vec<String>, Failure> {
    let Some(value) = item.get("zones") else {
        return Ok(Vec::new());
    };
    let entries = value
        .as_array()
        .ok_or_else(|| not_type(index, "zones", "an array"))?;
    entries
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| not_type(index, "zones", "an array of strings"))
        })
        .collect()
}

fn read_box(data: Option<&Value>, index: usize) -> Result<Option<[f64; 4]>, Failure> {
    let wrong = || not_type(index, "box", "an array of four numbers");
    match data.and_then(|value| value.get("box")) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let parts = value.as_array().ok_or_else(wrong)?;
            let edges: &[Value] = parts.as_slice();
            let [x1, y1, x2, y2]: &[Value; 4] = edges.try_into().map_err(|_| wrong())?;
            let edge = |value: &Value| value.as_f64().ok_or_else(wrong);
            Ok(Some([edge(x1)?, edge(y1)?, edge(x2)?, edge(y2)?]))
        }
    }
}

fn event_text(row: &EventRow) -> Result<String, Failure> {
    // A parsed number re-encoded by serde_json is its shortest round-trip decimal.
    let zones = row
        .zones
        .iter()
        .map(|zone| json_quote(zone))
        .collect::<Result<Vec<String>, Failure>>()?
        .join(", ");
    let end = optional_number_text(row.end)?;
    let top_score = optional_number_text(row.top_score)?;
    let score = optional_number_text(row.score)?;
    let box_text = match row.box_values {
        Some(edges) => {
            let parts = edges
                .iter()
                .map(|edge| json_number(*edge))
                .collect::<Result<Vec<String>, Failure>>()?
                .join(", ");
            format!("[{parts}]")
        }
        None => String::from("null"),
    };
    Ok(format!(
        "{{\"id\": {id}, \"camera\": {camera}, \"label\": {label}, \"start\": {start}, \
         \"end\": {end}, \"zones\": [{zones}], \"top_score\": {top_score}, \"score\": {score}, \
         \"box\": {box_text}, \"has_clip\": {has_clip}}}",
        id = json_quote(&row.id)?,
        camera = json_quote(&row.camera)?,
        label = json_quote(&row.label)?,
        start = json_number(row.start)?,
        has_clip = serde_json::to_string(&row.has_clip).map_err(|error| encode_error(&error))?,
    ))
}

fn optional_number_text(value: Option<f64>) -> Result<String, Failure> {
    value.map_or_else(|| Ok(String::from("null")), json_number)
}

fn json_quote(text: &str) -> Result<String, Failure> {
    serde_json::to_string(text).map_err(|error| encode_error(&error))
}

fn json_number(value: f64) -> Result<String, Failure> {
    serde_json::to_string(&value).map_err(|error| encode_error(&error))
}

fn encode_error(error: &serde_json::Error) -> Failure {
    Failure::Input(format!("cannot encode a value: {error}"))
}

fn missing(index: usize, key: &str) -> Failure {
    Failure::Input(format!("event {index}: {key} is missing"))
}

fn not_type(index: usize, key: &str, want: &str) -> Failure {
    Failure::Input(format!("event {index}: {key} is not {want}"))
}
