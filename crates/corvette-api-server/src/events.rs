//! Review-item reads for `GET /corvette/api/v1/events` and `/events/<id>` from the read-only index.

use crate::activity::{
    check_range, padded_clip_end, padded_clip_start, parse_parameters, parse_timestamp,
    playlist_link,
};
use crate::recordings::{seconds_number, to_millis};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

const REVIEW_ROW_SQL: &str =
    "SELECT camera, severity, start_ts, end_ts, last_activity_ts FROM reviews WHERE id = ?";

// `start_ts + 1` keeps a one-frame event visible under half-open overlap.
const REVIEW_EVENTS_SQL: &str = "SELECT label, box_json, zones_json, first_motion FROM events \
     WHERE camera = ?1 AND start_ts < ?3 \
     AND MAX(COALESCE(end_ts, last_motion_ts), start_ts + 1) > ?2 \
     ORDER BY start_ts, id";

const CLIP_EXISTS_SQL: &str =
    "SELECT EXISTS(SELECT 1 FROM pieces WHERE camera = ?1 AND start_ts < ?2 AND end_ts > ?3)";

pub(crate) fn review_id_segment(path: &str) -> Option<&str> {
    let prefix = format!("{}events/", crate::API_PREFIX);
    let rest = path.strip_prefix(prefix.as_str())?;
    (!rest.is_empty() && !rest.contains('/')).then_some(rest)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EventsQuery {
    pub(crate) after: f64,
    pub(crate) before: f64,
    pub(crate) cameras: Option<Vec<String>>,
    pub(crate) severities: Vec<String>,
}

impl EventsQuery {
    /// Parses the raw query of an events-list request; an error names the failing part.
    pub(crate) fn parse(query: &str) -> Result<Self, String> {
        let params = parse_parameters(query, &["after", "before", "cameras", "severity"])?;
        let after = params
            .get("after")
            .map(String::as_str)
            .ok_or_else(|| "missing query parameter `after`".to_owned())
            .and_then(|value| parse_timestamp("after", value))?;
        let before = params
            .get("before")
            .map(String::as_str)
            .ok_or_else(|| "missing query parameter `before`".to_owned())
            .and_then(|value| parse_timestamp("before", value))?;
        check_range("after", "before", after, before)?;

        let cameras = params.get("cameras").map(|value| {
            value
                .split(',')
                .filter(|camera| !camera.is_empty())
                .map(str::to_owned)
                .collect()
        });

        let mut severities: Vec<String> = match params.get("severity") {
            None => vec!["alert".to_owned(), "detection".to_owned()],
            Some(value) => {
                let mut selected = Vec::new();
                for piece in value.split(',') {
                    match piece {
                        "" => {}
                        "alert" | "detection" => selected.push(piece.to_owned()),
                        other => return Err(format!("`severity` does not accept `{other}`")),
                    }
                }
                if selected.is_empty() {
                    return Err("`severity` must name `alert`, `detection`, or both".to_owned());
                }
                selected.sort();
                selected.dedup();
                selected
            }
        };
        severities.sort();

        Ok(Self {
            after,
            before,
            cameras,
            severities,
        })
    }
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<&str>>()
        .join(", ")
}

fn seconds(millis: i64) -> f64 {
    seconds_number(millis)
        .as_f64()
        .expect("millisecond text is a JSON number")
}

fn clip_value(
    connection: &Connection,
    camera: &str,
    start_ms: i64,
    end_ms: Option<i64>,
    now_seconds: f64,
) -> rusqlite::Result<Value> {
    let now_ms = to_millis(now_seconds);
    let (overlap_end_ms, url_end_seconds) = match end_ms {
        Some(end) => (end, padded_clip_end(seconds(end), now_seconds)),
        None if now_ms > start_ms => (now_ms, now_seconds),
        None => return Ok(Value::Null),
    };
    let has_footage: bool = connection.query_row(
        CLIP_EXISTS_SQL,
        params![camera, overlap_end_ms, start_ms],
        |row| row.get(0),
    )?;
    if !has_footage {
        return Ok(Value::Null);
    }
    Ok(json!(playlist_link(
        camera,
        padded_clip_start(seconds(start_ms)),
        url_end_seconds,
    )))
}

fn review_value(
    connection: &Connection,
    id: i64,
    camera: &str,
    severity: &str,
    start_ms: i64,
    end_ms: Option<i64>,
    now_seconds: f64,
) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": id,
        "camera": camera,
        "severity": severity,
        "start": seconds_number(start_ms),
        "end": end_ms.map_or(Value::Null, seconds_number),
        "media": {"clip": clip_value(connection, camera, start_ms, end_ms, now_seconds)?},
    }))
}

fn stored_json(column: usize, text: &str) -> rusqlite::Result<Value> {
    serde_json::from_str(text).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(err.to_string())),
        )
    })
}

/// The list route's body: reviews overlapping the half-open `[after, before)` window,
/// ordered by `start` then `id` like the activity route orders its reviews; the
/// error is a read failure or a corrupt stored JSON column.
pub(crate) fn list_body(
    connection: &Connection,
    query: &EventsQuery,
    cameras: &[String],
    now_seconds: f64,
) -> rusqlite::Result<Vec<u8>> {
    let reviews = reviews_values(
        connection,
        cameras,
        query.after,
        query.before,
        Some(&query.severities),
        now_seconds,
    )?;
    Ok(body_with("reviews", &reviews))
}

fn reviews_values(
    connection: &Connection,
    cameras: &[String],
    after_seconds: f64,
    before_seconds: f64,
    severities: Option<&[String]>,
    now_seconds: f64,
) -> rusqlite::Result<Vec<Value>> {
    let severity_filter = severities.map_or_else(String::new, |severities| {
        format!(" AND severity IN ({})", placeholders(severities.len()))
    });
    let sql = format!(
        "SELECT id, camera, severity, start_ts, end_ts FROM reviews \
         WHERE camera IN ({}) AND start_ts < ? AND (end_ts IS NULL OR end_ts > ? \
         OR start_ts >= ?){severity_filter} ORDER BY start_ts, id",
        placeholders(cameras.len()),
    );
    let mut parameters: Vec<SqlValue> = cameras
        .iter()
        .map(|camera| SqlValue::Text(camera.clone()))
        .collect();
    parameters.extend([
        SqlValue::Integer(to_millis(before_seconds)),
        SqlValue::Integer(to_millis(after_seconds)),
        SqlValue::Integer(to_millis(after_seconds)),
    ]);
    if let Some(severities) = severities {
        parameters.extend(
            severities
                .iter()
                .map(|severity| SqlValue::Text(severity.clone())),
        );
    }

    let mut reviews = Vec::new();
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query(rusqlite::params_from_iter(parameters.iter()))?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let camera: String = row.get(1)?;
        let severity: String = row.get(2)?;
        let start_ms: i64 = row.get(3)?;
        let end_ms: Option<i64> = row.get(4)?;
        reviews.push(review_value(
            connection,
            id,
            &camera,
            &severity,
            start_ms,
            end_ms,
            now_seconds,
        )?);
    }
    Ok(reviews)
}

pub(crate) fn reviews_in_range(
    connection: &Connection,
    cameras: &[String],
    after_seconds: f64,
    before_seconds: f64,
    now_seconds: f64,
) -> rusqlite::Result<Vec<Value>> {
    reviews_values(
        connection,
        cameras,
        after_seconds,
        before_seconds,
        None,
        now_seconds,
    )
}

pub(crate) fn empty_list_body() -> Vec<u8> {
    body_with("reviews", &[])
}

fn body_with(key: &str, entries: &[Value]) -> Vec<u8> {
    serde_json::to_vec(&json!({ key: entries })).expect("events body serializes")
}

/// One review plus the events whose active span overlaps its half-open window, from the
/// times rather than a stored link; `None` is not-found, the error a read or corrupt-JSON failure.
pub(crate) fn detail_body(
    connection: &Connection,
    id: i64,
    cameras: &[String],
    now_seconds: f64,
) -> rusqlite::Result<Option<Vec<u8>>> {
    let row = connection.query_row(REVIEW_ROW_SQL, params![id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, i64>(4)?,
        ))
    });
    let Some((camera, severity, start_ms, end_ms, last_activity_ms)) = row.optional()? else {
        return Ok(None);
    };
    if !cameras.contains(&camera) {
        return Ok(None);
    }
    let review_end_ms = end_ms.unwrap_or(last_activity_ms);
    let mut events = Vec::new();
    let mut statement = connection.prepare(REVIEW_EVENTS_SQL)?;
    let mut rows = statement.query(params![camera, start_ms, review_end_ms])?;
    while let Some(row) = rows.next()? {
        let label: String = row.get(0)?;
        let box_json: String = row.get(1)?;
        let zones_json: String = row.get(2)?;
        let first_motion: Option<i64> = row.get(3)?;
        events.push(json!({
            "label": label,
            "box": stored_json(1, &box_json)?,
            "zones": stored_json(2, &zones_json)?,
            "first_motion": first_motion.map_or(Value::Null, seconds_number),
        }));
    }
    let mut review = review_value(
        connection,
        id,
        &camera,
        &severity,
        start_ms,
        end_ms,
        now_seconds,
    )?;
    review["events"] = Value::Array(events);
    Ok(Some(
        serde_json::to_vec(&review).expect("review detail serializes"),
    ))
}
