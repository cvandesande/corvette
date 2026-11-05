//! Query parsing and the recordings check for `GET /corvette/api/v1/clip-link`.

use crate::activity::{RECORDING_OVERLAP_SQL, check_range, parse_parameters, parse_timestamp};
use rusqlite::params;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ClipLinkQuery {
    pub(crate) camera: String,
    pub(crate) start: f64,
    pub(crate) end: f64,
}

impl ClipLinkQuery {
    pub(crate) fn parse(query: &str) -> Result<Self, String> {
        let params = parse_parameters(query, &["camera", "start", "end"])?;
        let camera = params
            .get("camera")
            .map(String::as_str)
            .ok_or_else(|| "missing query parameter `camera`".to_owned())?;
        if camera.is_empty() {
            return Err("`camera` must name one camera".to_owned());
        }
        let start = params
            .get("start")
            .map(String::as_str)
            .ok_or_else(|| "missing query parameter `start`".to_owned())
            .and_then(|value| parse_timestamp("start", value))?;
        let end = params
            .get("end")
            .map(String::as_str)
            .ok_or_else(|| "missing query parameter `end`".to_owned())
            .and_then(|value| parse_timestamp("end", value))?;
        check_range("start", "end", start, end)?;
        Ok(Self {
            camera: camera.to_owned(),
            start,
            end,
        })
    }
}

pub(crate) fn has_recorded_footage(
    connection: &rusqlite::Connection,
    camera: &str,
    start: f64,
    end: f64,
) -> rusqlite::Result<bool> {
    // The statement's placeholders bind `end` to `start_time` and `start` to
    // `end_time`: a recording overlaps when it starts before the range ends.
    connection.query_row(RECORDING_OVERLAP_SQL, params![camera, end, start], |row| {
        row.get(0)
    })
}
