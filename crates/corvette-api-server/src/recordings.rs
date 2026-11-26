//! Parsing and index reads for `GET /corvette/api/v1/recordings` and the status route's recording flag.

use crate::activity::{check_range, parse_parameters, parse_timestamp};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Number, Value, json};
use std::path::Path;

pub const INDEX_PATH_ENV: &str = "CORVETTE_API_INDEX_PATH";

/// The index path when the env var is unset: the bridge writes `<root>/index.sqlite`, and the root has no built-in default.
pub const DEFAULT_INDEX_PATH: &str = "/media/corvette/index.sqlite";

pub(crate) const RECORDING_PIECE_MULTIPLE: i64 = 3;

const PIECES_OVER_SQL: &str = "SELECT start_ts, end_ts FROM pieces \
     WHERE camera = ?1 AND start_ts < ?2 AND end_ts > ?3 ORDER BY start_ts";

const RECORDING_EXISTS_SQL: &str =
    "SELECT EXISTS(SELECT 1 FROM pieces WHERE camera = ?1 AND end_ts BETWEEN ?2 AND ?3)";

fn piece_seconds_key(camera: &str) -> String {
    format!("piece_seconds:{camera}")
}

const fn recording_window_ms(piece_seconds: i64) -> i64 {
    RECORDING_PIECE_MULTIPLE
        .saturating_mul(piece_seconds)
        .saturating_mul(1000)
}

#[derive(Debug)]
pub(crate) enum IndexError {
    /// The `SQLite` reason is deliberately not carried out: no answer or log line may quote it.
    Database,
    PieceSecondsValue,
}

impl From<rusqlite::Error> for IndexError {
    fn from(_source: rusqlite::Error) -> Self {
        Self::Database
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RecordingsQuery {
    pub(crate) camera: String,
    pub(crate) start: f64,
    pub(crate) end: f64,
}

impl RecordingsQuery {
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

/// Request Unix seconds to the index's milliseconds; out-of-range bounds saturate rather than wrap.
pub(crate) fn to_millis(seconds: f64) -> i64 {
    format!("{:.0}", (seconds * 1000.0).round())
        .parse::<i64>()
        .unwrap_or(i64::MAX)
}

/// The index's milliseconds as the exact Unix-seconds JSON number the response carries.
pub(crate) fn seconds_number(millis: i64) -> Value {
    let text = format!("{}.{:03}", millis.div_euclid(1000), millis.rem_euclid(1000));
    Value::Number(
        text.parse::<Number>()
            .expect("millisecond text is a JSON number"),
    )
}

/// Sorted ranges of one camera merge where they touch or overlap; a positive gap starts a new range.
pub(crate) fn merge_ranges(pieces: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for &(start, end) in pieces {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

pub(crate) fn overlapping_ranges(
    connection: &Connection,
    camera: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<(i64, i64)>, IndexError> {
    let mut statement = connection.prepare(PIECES_OVER_SQL)?;
    let mut rows = statement.query(params![camera, end_ms, start_ms])?;
    let mut clipped: Vec<(i64, i64)> = Vec::new();
    while let Some(row) = rows.next()? {
        let start: i64 = row.get(0)?;
        let end: i64 = row.get(1)?;
        clipped.push((start.max(start_ms), end.min(end_ms)));
    }
    Ok(merge_ranges(&clipped))
}

pub(crate) fn ranges_body(camera: &str, ranges: &[(i64, i64)]) -> Vec<u8> {
    let entries: Vec<Value> = ranges
        .iter()
        .map(|(start_ms, end_ms)| {
            json!({
                "camera": camera,
                "start": seconds_number(*start_ms),
                "end": seconds_number(*end_ms),
            })
        })
        .collect();
    serde_json::to_vec(&json!({ "ranges": entries })).expect("recordings body serializes")
}

pub(crate) fn recording_states(
    index_path: &Path,
    cameras: &[String],
    now_seconds: f64,
) -> Result<Vec<bool>, IndexError> {
    let connection = crate::database::open_read_only(index_path)?;
    let now_ms = to_millis(now_seconds);
    cameras
        .iter()
        .map(|camera| recording_now(&connection, camera, now_ms))
        .collect()
}

fn recording_now(connection: &Connection, camera: &str, now_ms: i64) -> Result<bool, IndexError> {
    let stored: Option<String> = connection
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![piece_seconds_key(camera)],
            |row| row.get(0),
        )
        .optional()?;
    let Some(text) = stored else {
        return Ok(false);
    };
    let piece_seconds = text
        .parse::<i64>()
        .map_err(|_| IndexError::PieceSecondsValue)?;
    let window = recording_window_ms(piece_seconds.max(0));
    connection
        .query_row(
            RECORDING_EXISTS_SQL,
            params![
                camera,
                now_ms.saturating_sub(window),
                now_ms.saturating_add(window)
            ],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_joins_touching_and_overlapping_pieces_and_splits_on_a_gap() {
        assert_eq!(merge_ranges(&[]), []);
        assert_eq!(merge_ranges(&[(10, 20)]), [(10, 20)]);
        assert_eq!(merge_ranges(&[(10, 20), (20, 30)]), [(10, 30)]);
        assert_eq!(merge_ranges(&[(10, 25), (20, 30)]), [(10, 30)]);
        assert_eq!(merge_ranges(&[(10, 40), (15, 20)]), [(10, 40)]);
        assert_eq!(merge_ranges(&[(10, 20), (21, 30)]), [(10, 20), (21, 30)]);
    }

    #[test]
    fn the_conversions_round_trip_whole_milliseconds() {
        assert_eq!(seconds_number(1_764_000_000).to_string(), "1764000.0");
        assert_eq!(seconds_number(1).to_string(), "0.001");
        assert_eq!(to_millis(1_764_000.0), 1_764_000_000);
        assert_eq!(to_millis(0.0015), 2);
        assert_eq!(to_millis(f64::MAX), i64::MAX);
    }

    #[test]
    fn the_window_saturates_rather_than_wraps() {
        assert_eq!(recording_window_ms(10), 30_000);
        assert_eq!(recording_window_ms(i64::MAX), i64::MAX);
        assert_eq!(recording_window_ms(0), 0);
    }
}
