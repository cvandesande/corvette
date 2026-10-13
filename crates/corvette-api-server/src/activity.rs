//! Parsing and reads for `GET /corvette/api/v1/activity` (issue #5).

use corvette_api::ReviewSeverity;
use rusqlite::types::{Type, Value};
use rusqlite::{Connection, params, params_from_iter};
use std::collections::BTreeMap;

/// Longest accepted range: 31 days in seconds (issue #5 D-4).
pub const MAX_RANGE_SECONDS: f64 = 2_678_400.0;

/// Validated parameters of one activity query.
#[derive(Clone, Debug, PartialEq)]
pub struct ActivityQuery {
    /// Lower end of the requested half-open time range, in epoch seconds.
    pub after: f64,
    /// Upper end of the range; strictly greater than `after` and at most
    /// [`MAX_RANGE_SECONDS`] seconds after it.
    pub before: f64,
    /// Camera identifiers to restrict to; `None` when the request named none.
    pub cameras: Option<Vec<String>>,
    /// Whether reviews are requested.
    pub include_reviews: bool,
    /// Whether motion buckets are requested.
    pub include_motion: bool,
}

impl ActivityQuery {
    /// Parses the raw query of an activity request: the text after `?`,
    /// without the `?`, and empty when the request has none.
    ///
    /// # Errors
    ///
    /// Returns a message naming the offending parameter when a parameter is
    /// empty, unknown, repeated, missing its value, missing, or outside its
    /// rules, and quotes the failing text when a percent escape is malformed
    /// or decodes to invalid UTF-8.
    pub fn parse(query: &str) -> Result<Self, String> {
        let params = parse_parameters(query)?;
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
        if after >= before {
            return Err("`before` must be greater than `after`".to_owned());
        }
        if before - after > MAX_RANGE_SECONDS {
            return Err(format!(
                "`before` is more than {MAX_RANGE_SECONDS} seconds after `after`"
            ));
        }

        let cameras = params.get("cameras").map(|value| {
            value
                .split(',')
                .filter(|camera| !camera.is_empty())
                .map(str::to_owned)
                .collect()
        });

        let mut include_reviews = true;
        let mut include_motion = true;
        if let Some(value) = params.get("include") {
            include_reviews = false;
            include_motion = false;
            for piece in value.split(',') {
                match piece {
                    "" => {}
                    "reviews" => include_reviews = true,
                    "motion" => include_motion = true,
                    other => return Err(format!("`include` does not accept `{other}`")),
                }
            }
            if !include_reviews && !include_motion {
                return Err("`include` must name `reviews`, `motion`, or both".to_owned());
            }
        }

        Ok(Self {
            after,
            before,
            cameras,
            include_reviews,
            include_motion,
        })
    }
}

/// Splits the query on `&`, decodes each name and value, and rejects empty,
/// unknown, value-less, or repeated parameters.
fn parse_parameters(query: &str) -> Result<BTreeMap<String, String>, String> {
    let mut params = BTreeMap::new();
    if query.is_empty() {
        return Ok(params);
    }
    for segment in query.split('&') {
        if segment.is_empty() {
            return Err("empty query parameter".to_owned());
        }
        let Some((raw_name, raw_value)) = segment.split_once('=') else {
            let name = percent_decode(segment)?;
            return Err(format!("query parameter `{name}` is missing its value"));
        };
        let name = percent_decode(raw_name)?;
        if !matches!(name.as_str(), "after" | "before" | "cameras" | "include") {
            return Err(format!("unknown query parameter `{name}`"));
        }
        if params.contains_key(name.as_str()) {
            return Err(format!("repeated query parameter `{name}`"));
        }
        params.insert(name, percent_decode(raw_value)?);
    }
    Ok(params)
}

/// Parses one timestamp parameter: a finite, non-negative number of seconds.
fn parse_timestamp(name: &str, value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .ok_or_else(|| format!("`{name}` must be a finite number of seconds, at least 0"))
}

/// Expands `%XX` escapes (hex digits in either case) to their bytes and
/// requires the result to be UTF-8. `+` stays `+`: this grammar has no
/// plus-as-space convention.
fn percent_decode(text: &str) -> Result<String, String> {
    let mut decoded = Vec::with_capacity(text.len());
    let mut bytes = text.as_bytes().iter();
    while let Some(&byte) = bytes.next() {
        if byte != b'%' {
            decoded.push(byte);
            continue;
        }
        let (Some(&first), Some(&second)) = (bytes.next(), bytes.next()) else {
            return Err(format!("`{text}` ends in an incomplete percent escape"));
        };
        let (Some(high), Some(low)) = (hex_digit(first), hex_digit(second)) else {
            return Err(format!(
                "`{text}` has a percent escape that is not two hex digits"
            ));
        };
        decoded.push(high * 16 + low);
    }
    String::from_utf8(decoded).map_err(|_| format!("`{text}` is not valid UTF-8"))
}

const fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// One review row as read from Frigate, before media URLs are built.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewRow {
    /// Frigate's review segment identifier.
    pub id: String,
    /// The camera the review belongs to.
    pub camera: String,
    /// The review's activity classification.
    pub severity: ReviewSeverity,
    /// Review start in Unix seconds.
    pub start: f64,
    /// Review end in Unix seconds, `None` while the review is ongoing.
    pub end: Option<f64>,
    /// The thumbnail path as Frigate stores it.
    pub thumb_path: String,
    /// Sorted object classes recorded in the review.
    pub objects: Vec<String>,
    /// Sorted zone names recorded in the review.
    pub zones: Vec<String>,
    /// Sorted identifiers of the tracked-object events recorded in the review.
    pub detections: Vec<String>,
    /// Sorted sub-labels recorded in the review.
    pub sub_labels: Vec<String>,
    /// Sorted audio labels recorded in the review.
    pub audio: Vec<String>,
    /// An ended review with at least one recording of its camera overlapping it.
    pub has_recording: bool,
}

/// Everything one activity request reads from the database.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActivityRows {
    /// Reviews ordered by start time, then id.
    pub reviews: Vec<ReviewRow>,
}

/// Reads from one transaction the reviews of `cameras` that overlap the
/// query's range, each with its clip availability, ordered by start time
/// then id.
///
/// A review overlaps the half-open range `[after, before)` when it starts
/// before `before` and either has no end yet, ends after `after`, or starts
/// at or after `after`. An empty `cameras` list or `include_reviews` false
/// reads nothing, and no review SQL runs.
///
/// # Errors
///
/// [`rusqlite::Error::SqliteFailure`] if the transaction or a query fails,
/// and [`rusqlite::Error::FromSqlConversionFailure`] if a row's `severity`
/// or `data` column holds a value Frigate never writes.
pub fn read_activity(
    connection: &mut Connection,
    query: &ActivityQuery,
    cameras: &[String],
) -> rusqlite::Result<ActivityRows> {
    if !query.include_reviews || cameras.is_empty() {
        return Ok(ActivityRows::default());
    }
    let transaction = connection.transaction()?;
    let reviews = read_reviews(&transaction, query, cameras)?;
    transaction.commit()?;
    Ok(ActivityRows { reviews })
}

fn read_reviews(
    transaction: &rusqlite::Transaction<'_>,
    query: &ActivityQuery,
    cameras: &[String],
) -> rusqlite::Result<Vec<ReviewRow>> {
    let placeholders = cameras
        .iter()
        .map(|_| "?")
        .collect::<Vec<&str>>()
        .join(", ");
    let sql = format!(
        "SELECT id, camera, severity, start_time, end_time, thumb_path, data \
         FROM reviewsegment WHERE camera IN ({placeholders}) \
         AND start_time < ? \
         AND (end_time IS NULL OR end_time > ? OR start_time >= ?) \
         ORDER BY start_time, id"
    );
    let mut parameters: Vec<Value> = cameras
        .iter()
        .map(|camera| Value::Text(camera.clone()))
        .collect();
    parameters.extend([
        Value::Real(query.before),
        Value::Real(query.after),
        Value::Real(query.after),
    ]);

    let mut reviews = transaction
        .prepare(&sql)?
        .query_map(params_from_iter(&parameters), review_from_row)?
        .collect::<rusqlite::Result<Vec<ReviewRow>>>()?;

    let mut overlaps = transaction.prepare(
        "SELECT EXISTS(SELECT 1 FROM recordings \
         WHERE camera = ? AND start_time < ? AND end_time > ?)",
    )?;
    for review in &mut reviews {
        review.has_recording = match review.end {
            None => false,
            // `EXISTS` always returns one row holding 0 or 1, so the value is
            // read rather than asking whether a row came back.
            Some(end) => overlaps
                .query_row(params![review.camera.as_str(), end, review.start], |row| {
                    row.get(0)
                })?,
        };
    }
    Ok(reviews)
}

fn review_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewRow> {
    let id: String = row.get(0)?;
    let camera: String = row.get(1)?;
    let severity_text: String = row.get(2)?;
    let severity = match severity_text.as_str() {
        "alert" => ReviewSeverity::Alert,
        "detection" => ReviewSeverity::Detection,
        "significant_motion" => ReviewSeverity::SignificantMotion,
        other => {
            return Err(text_conversion_error(
                2,
                "severity",
                &format!("unknown value `{other}`"),
            ));
        }
    };
    let start: f64 = row.get(3)?;
    let end: Option<f64> = row.get(4)?;
    let thumb_path: String = row.get(5)?;
    let data_text: String = row.get(6)?;
    let data = parse_review_data(&data_text)
        .map_err(|reason| text_conversion_error(6, "data", &reason))?;
    Ok(ReviewRow {
        id,
        camera,
        severity,
        start,
        end,
        thumb_path,
        objects: data.objects,
        zones: data.zones,
        detections: data.detections,
        sub_labels: data.sub_labels,
        audio: data.audio,
        has_recording: false,
    })
}

/// The five string lists of a review's `data` JSON. A key absent from the
/// JSON gives an empty list, a present key must hold a list of strings, and
/// any other key is ignored.
struct ReviewData {
    objects: Vec<String>,
    zones: Vec<String>,
    detections: Vec<String>,
    sub_labels: Vec<String>,
    audio: Vec<String>,
}

/// # Errors
///
/// Returns a message when `text` is not valid JSON or is not a JSON
/// object, and a message naming the key when a known key is not a list
/// of strings.
fn parse_review_data(text: &str) -> Result<ReviewData, String> {
    let parsed: serde_json::Value = serde_json::from_str(text).map_err(|err| err.to_string())?;
    if !parsed.is_object() {
        return Err("value is not a JSON object".to_owned());
    }
    Ok(ReviewData {
        objects: string_list(&parsed, "objects")?,
        zones: string_list(&parsed, "zones")?,
        detections: string_list(&parsed, "detections")?,
        sub_labels: string_list(&parsed, "sub_labels")?,
        audio: string_list(&parsed, "audio")?,
    })
}

/// One `data` list as sorted strings; an absent key gives an empty list
/// because older rows may lack it.
///
/// # Errors
///
/// Returns a message naming `key` when its value is not a list of strings.
fn string_list(parsed: &serde_json::Value, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = parsed.get(key) else {
        return Ok(Vec::new());
    };
    let mut list = value
        .as_array()
        .ok_or_else(|| format!("`{key}` is not a list"))?
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("`{key}` has a non-string entry"))
        })
        .collect::<Result<Vec<String>, String>>()?;
    list.sort_unstable();
    Ok(list)
}

fn text_conversion_error(column_index: usize, column_name: &str, reason: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column_index,
        Type::Text,
        Box::new(std::io::Error::other(format!("`{column_name}`: {reason}"))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    const REVIEWSEGMENT_DDL: &str = r#"CREATE TABLE "reviewsegment" ("id" VARCHAR(30) NOT NULL PRIMARY KEY, "camera" VARCHAR(20) NOT NULL, "start_time" DATETIME NOT NULL, "end_time" DATETIME, "severity" VARCHAR(30) NOT NULL, "thumb_path" VARCHAR(255) NOT NULL, "data" JSON NOT NULL)"#;
    const RECORDINGS_DDL: &str = r#"CREATE TABLE "recordings" ("id" VARCHAR(30) NOT NULL PRIMARY KEY, "camera" VARCHAR(20) NOT NULL, "path" VARCHAR(255) NOT NULL, "start_time" DATETIME NOT NULL, "end_time" DATETIME NOT NULL, "duration" REAL NOT NULL, "motion" INTEGER)"#;

    /// An in-memory database with the two Frigate tables this read uses.
    fn frigate_database() -> Connection {
        let connection = Connection::open_in_memory().expect("open in-memory database");
        connection
            .execute(REVIEWSEGMENT_DDL, params![])
            .expect("create reviewsegment");
        connection
            .execute(RECORDINGS_DDL, params![])
            .expect("create recordings");
        connection
    }

    fn insert_review(
        connection: &Connection,
        id: &str,
        camera: &str,
        severity: &str,
        start: f64,
        end: Option<f64>,
    ) {
        insert_review_with_data(connection, id, camera, severity, start, end, "{}");
    }

    fn insert_review_with_data(
        connection: &Connection,
        id: &str,
        camera: &str,
        severity: &str,
        start: f64,
        end: Option<f64>,
        data: &str,
    ) {
        connection
            .execute(
                "INSERT INTO reviewsegment \
                 (id, camera, start_time, end_time, severity, thumb_path, data) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![
                    id,
                    camera,
                    start,
                    end,
                    severity,
                    format!("/media/{camera}/{start}.jpg"),
                    data
                ],
            )
            .expect("insert review");
    }

    fn insert_recording(connection: &Connection, id: &str, camera: &str, start: f64, end: f64) {
        connection
            .execute(
                "INSERT INTO recordings \
                 (id, camera, path, start_time, end_time, duration, motion) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![
                    id,
                    camera,
                    format!("/{camera}/{start}.mp4"),
                    start,
                    end,
                    1.0,
                    0
                ],
            )
            .expect("insert recording");
    }

    fn range_query(after: f64, before: f64) -> ActivityQuery {
        ActivityQuery {
            after,
            before,
            cameras: None,
            include_reviews: true,
            include_motion: false,
        }
    }

    fn camera_list(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn review_ids(rows: &ActivityRows) -> Vec<&str> {
        rows.reviews
            .iter()
            .map(|review| review.id.as_str())
            .collect()
    }

    #[test]
    fn range_boundaries_include_only_overlapping_reviews() {
        let mut connection = frigate_database();
        insert_review(
            &connection,
            "before-range",
            "front",
            "alert",
            800.0,
            Some(900.0),
        );
        insert_review(
            &connection,
            "crosses-after",
            "front",
            "alert",
            900.0,
            Some(1_100.0),
        );
        insert_review(
            &connection,
            "ends-at-after",
            "front",
            "alert",
            900.0,
            Some(1_000.0),
        );
        insert_review(
            &connection,
            "zero-length-at-after",
            "front",
            "alert",
            1_000.0,
            Some(1_000.0),
        );
        insert_review(&connection, "ongoing", "front", "detection", 1_500.0, None);
        insert_review(
            &connection,
            "crosses-before",
            "front",
            "alert",
            1_900.0,
            Some(2_100.0),
        );
        insert_review(
            &connection,
            "ongoing-at-before",
            "front",
            "alert",
            2_000.0,
            None,
        );
        insert_review(
            &connection,
            "starts-at-before",
            "front",
            "detection",
            2_000.0,
            Some(2_100.0),
        );
        insert_review(
            &connection,
            "zero-length-at-before",
            "front",
            "alert",
            2_000.0,
            Some(2_000.0),
        );

        let rows = read_activity(
            &mut connection,
            &range_query(1_000.0, 2_000.0),
            &camera_list(&["front"]),
        )
        .expect("read");
        assert_eq!(
            review_ids(&rows),
            [
                "crosses-after",
                "zero-length-at-after",
                "ongoing",
                "crosses-before"
            ]
        );
    }

    #[test]
    fn adjacent_ranges_share_only_overlapping_reviews() {
        let mut connection = frigate_database();
        insert_review(
            &connection,
            "spanning",
            "front",
            "alert",
            1_400.0,
            Some(1_600.0),
        );
        insert_review(
            &connection,
            "zero-length-at-1500",
            "front",
            "alert",
            1_500.0,
            Some(1_500.0),
        );

        let first = read_activity(
            &mut connection,
            &range_query(1_000.0, 1_500.0),
            &camera_list(&["front"]),
        )
        .expect("first range");
        assert_eq!(review_ids(&first), ["spanning"]);
        let second = read_activity(
            &mut connection,
            &range_query(1_500.0, 2_000.0),
            &camera_list(&["front"]),
        )
        .expect("second range");
        assert_eq!(review_ids(&second), ["spanning", "zero-length-at-1500"]);
    }

    #[test]
    fn only_allowed_cameras_are_returned() {
        let mut connection = frigate_database();
        insert_review(
            &connection,
            "row-back",
            "back",
            "alert",
            1_500.0,
            Some(1_600.0),
        );
        insert_review(
            &connection,
            "row-front",
            "front",
            "alert",
            1_500.0,
            Some(1_600.0),
        );
        insert_review(
            &connection,
            "row-side",
            "side",
            "alert",
            1_500.0,
            Some(1_600.0),
        );

        let rows = read_activity(
            &mut connection,
            &range_query(1_000.0, 2_000.0),
            &camera_list(&["front", "side"]),
        )
        .expect("read");
        assert_eq!(review_ids(&rows), ["row-front", "row-side"]);
    }

    #[test]
    fn empty_camera_list_reads_nothing() {
        let mut connection = frigate_database();
        insert_review(
            &connection,
            "front-row",
            "front",
            "alert",
            1_500.0,
            Some(1_600.0),
        );

        let rows =
            read_activity(&mut connection, &range_query(1_000.0, 2_000.0), &[]).expect("read");
        assert!(rows.reviews.is_empty());
    }

    #[test]
    fn equal_start_times_order_by_id() {
        let mut connection = frigate_database();
        insert_review(&connection, "bbb", "front", "alert", 1_500.0, Some(1_600.0));
        insert_review(&connection, "aaa", "front", "alert", 1_500.0, Some(1_600.0));
        insert_review(&connection, "ccc", "front", "alert", 1_500.0, Some(1_600.0));

        let rows = read_activity(
            &mut connection,
            &range_query(1_000.0, 2_000.0),
            &camera_list(&["front"]),
        )
        .expect("read");
        assert_eq!(review_ids(&rows), ["aaa", "bbb", "ccc"]);
    }

    #[test]
    fn review_data_lists_come_back_sorted_and_extra_keys_are_ignored() {
        let mut connection = frigate_database();
        insert_review_with_data(
            &connection,
            "rich",
            "front",
            "alert",
            1_500.0,
            Some(1_600.0),
            r#"{"objects":["cat","bird"],"zones":["driveway","door"],
               "detections":["car","person"],"sub_labels":["zebra","apple"],
               "audio":["siren","beep"],"thumb_time":1550,"metadata":{"x":1}}"#,
        );

        let rows = read_activity(
            &mut connection,
            &range_query(1_000.0, 2_000.0),
            &camera_list(&["front"]),
        )
        .expect("read");
        let review = &rows.reviews[0];
        assert_eq!(review.objects, ["bird", "cat"]);
        assert_eq!(review.zones, ["door", "driveway"]);
        assert_eq!(review.detections, ["car", "person"]);
        assert_eq!(review.sub_labels, ["apple", "zebra"]);
        assert_eq!(review.audio, ["beep", "siren"]);
    }

    #[test]
    fn review_data_without_known_keys_gives_empty_lists() {
        let mut connection = frigate_database();
        insert_review(
            &connection,
            "empty",
            "front",
            "alert",
            1_500.0,
            Some(1_600.0),
        );

        let rows = read_activity(
            &mut connection,
            &range_query(1_000.0, 2_000.0),
            &camera_list(&["front"]),
        )
        .expect("read");
        assert_eq!(
            rows.reviews,
            vec![ReviewRow {
                id: "empty".to_owned(),
                camera: "front".to_owned(),
                severity: ReviewSeverity::Alert,
                start: 1_500.0,
                end: Some(1_600.0),
                thumb_path: "/media/front/1500.jpg".to_owned(),
                objects: vec![],
                zones: vec![],
                detections: vec![],
                sub_labels: vec![],
                audio: vec![],
                has_recording: false,
            }]
        );
    }

    #[test]
    fn invalid_json_in_data_is_a_conversion_error() {
        let mut connection = frigate_database();
        insert_review_with_data(
            &connection,
            "broken",
            "front",
            "alert",
            1_500.0,
            Some(1_600.0),
            "{not json",
        );

        let error = read_activity(
            &mut connection,
            &range_query(1_000.0, 2_000.0),
            &camera_list(&["front"]),
        )
        .unwrap_err();
        assert!(
            matches!(
                error,
                rusqlite::Error::FromSqlConversionFailure(6, Type::Text, _)
            ),
            "{error}"
        );
    }

    #[test]
    fn review_data_with_wrong_shape_is_a_conversion_error() {
        for data in [
            "[]",
            r#"{"objects": "person"}"#,
            r#"{"objects": ["person", 3]}"#,
        ] {
            let mut connection = frigate_database();
            insert_review_with_data(
                &connection,
                "strict",
                "front",
                "alert",
                1_500.0,
                Some(1_600.0),
                data,
            );

            let error = read_activity(
                &mut connection,
                &range_query(1_000.0, 2_000.0),
                &camera_list(&["front"]),
            )
            .unwrap_err();
            assert!(
                matches!(
                    error,
                    rusqlite::Error::FromSqlConversionFailure(6, Type::Text, _)
                ),
                "{data}: {error}"
            );
        }
    }

    #[test]
    fn severity_text_maps_to_variants() {
        let mut connection = frigate_database();
        insert_review(
            &connection,
            "sev-alert",
            "front",
            "alert",
            1_500.0,
            Some(1_600.0),
        );
        insert_review(
            &connection,
            "sev-detection",
            "front",
            "detection",
            1_500.0,
            Some(1_600.0),
        );
        insert_review(
            &connection,
            "sev-motion",
            "front",
            "significant_motion",
            1_500.0,
            Some(1_600.0),
        );

        let rows = read_activity(
            &mut connection,
            &range_query(1_000.0, 2_000.0),
            &camera_list(&["front"]),
        )
        .expect("read");
        assert_eq!(
            rows.reviews
                .iter()
                .map(|review| review.severity)
                .collect::<Vec<_>>(),
            [
                ReviewSeverity::Alert,
                ReviewSeverity::Detection,
                ReviewSeverity::SignificantMotion
            ]
        );
    }

    #[test]
    fn unknown_severity_is_a_conversion_error() {
        let mut connection = frigate_database();
        insert_review(
            &connection,
            "bogus",
            "front",
            "bogus",
            1_500.0,
            Some(1_600.0),
        );

        let error = read_activity(
            &mut connection,
            &range_query(1_000.0, 2_000.0),
            &camera_list(&["front"]),
        )
        .unwrap_err();
        assert!(
            matches!(
                error,
                rusqlite::Error::FromSqlConversionFailure(2, Type::Text, _)
            ),
            "{error}"
        );
    }

    #[test]
    fn has_recording_requires_an_overlapping_recording_of_the_same_camera() {
        let mut connection = frigate_database();
        insert_review(
            &connection,
            "overlap",
            "front",
            "alert",
            1_500.0,
            Some(1_600.0),
        );
        insert_recording(&connection, "rec-overlap", "front", 1_550.0, 1_580.0);
        insert_review(
            &connection,
            "other-camera",
            "back",
            "alert",
            1_700.0,
            Some(1_800.0),
        );
        insert_review(
            &connection,
            "recording-ends-at-start",
            "front",
            "alert",
            1_800.0,
            Some(1_900.0),
        );
        insert_recording(&connection, "rec-before", "front", 1_700.0, 1_800.0);
        insert_review(
            &connection,
            "recording-starts-at-end",
            "side",
            "alert",
            1_500.0,
            Some(1_600.0),
        );
        insert_recording(&connection, "rec-after", "side", 1_600.0, 1_700.0);
        insert_review(&connection, "ongoing", "front", "alert", 1_900.0, None);
        insert_recording(&connection, "rec-ongoing", "front", 1_910.0, 1_920.0);

        let rows = read_activity(
            &mut connection,
            &range_query(1_000.0, 2_000.0),
            &camera_list(&["front", "back", "side"]),
        )
        .expect("read");
        let has_recording = |id: &str| {
            rows.reviews
                .iter()
                .find(|review| review.id == id)
                .unwrap_or_else(|| panic!("review `{id}` missing"))
                .has_recording
        };
        assert!(has_recording("overlap"));
        assert!(!has_recording("other-camera"));
        assert!(!has_recording("recording-ends-at-start"));
        assert!(!has_recording("recording-starts-at-end"));
        assert!(!has_recording("ongoing"));
    }

    #[test]
    fn include_reviews_false_reads_no_reviews() {
        let mut connection = frigate_database();
        insert_review(
            &connection,
            "front-row",
            "front",
            "alert",
            1_500.0,
            Some(1_600.0),
        );

        let query = ActivityQuery {
            include_reviews: false,
            include_motion: true,
            ..range_query(1_000.0, 2_000.0)
        };
        let rows = read_activity(&mut connection, &query, &camera_list(&["front"])).expect("read");
        assert!(rows.reviews.is_empty());
    }

    #[test]
    fn full_query_parses_every_parameter() {
        let query = ActivityQuery::parse(
            "after=1760227200&before=1760313600.5&cameras=front,back&include=reviews",
        )
        .expect("valid");
        assert_eq!(
            query,
            ActivityQuery {
                after: 1_760_227_200.0,
                before: 1_760_313_600.5,
                cameras: Some(vec!["front".to_owned(), "back".to_owned()]),
                include_reviews: true,
                include_motion: false,
            }
        );
    }

    #[test]
    fn absent_cameras_and_include_take_their_defaults() {
        let query = ActivityQuery::parse("after=100&before=200").expect("valid");
        assert_eq!(query.cameras, None);
        assert!(query.include_reviews);
        assert!(query.include_motion);
    }

    #[test]
    fn missing_after_or_before_is_named() {
        let err = ActivityQuery::parse("before=200").unwrap_err();
        assert!(err.contains("`after`"), "{err}");
        let err = ActivityQuery::parse("after=100").unwrap_err();
        assert!(err.contains("`before`"), "{err}");
        let err = ActivityQuery::parse("").unwrap_err();
        assert!(err.contains("`after`"), "{err}");
    }

    #[test]
    fn timestamps_must_be_finite_and_non_negative() {
        for value in ["abc", "inf", "NaN", "-5"] {
            let err = ActivityQuery::parse(&format!("after={value}&before=200")).unwrap_err();
            assert!(err.contains("`after`"), "{value}: {err}");
            let err = ActivityQuery::parse(&format!("after=100&before={value}")).unwrap_err();
            assert!(err.contains("`before`"), "{value}: {err}");
        }
    }

    #[test]
    fn range_requires_after_below_before() {
        let err = ActivityQuery::parse("after=200&before=200").unwrap_err();
        assert!(err.contains("`before`"), "{err}");
        let err = ActivityQuery::parse("after=300&before=200").unwrap_err();
        assert!(err.contains("`before`"), "{err}");
    }

    #[test]
    fn range_is_at_most_31_days() {
        ActivityQuery::parse("after=1000000&before=3678400").expect("exactly 31 days accepted");
        let err = ActivityQuery::parse("after=1000000&before=3678400.001").unwrap_err();
        assert!(err.contains("`before`"), "{err}");
    }

    #[test]
    fn camera_list_is_decoded_split_and_pruned() {
        let query = ActivityQuery::parse("after=100&before=200&cameras=front%2Cback%20yard")
            .expect("valid");
        assert_eq!(
            query.cameras,
            Some(vec!["front".to_owned(), "back yard".to_owned()])
        );
        let query = ActivityQuery::parse("after=100&before=200&cameras=").expect("valid");
        assert_eq!(query.cameras, Some(vec![]));
        let query = ActivityQuery::parse("after=100&before=200&cameras=a,,b").expect("valid");
        assert_eq!(query.cameras, Some(vec!["a".to_owned(), "b".to_owned()]));
    }

    #[test]
    fn include_values_select_their_flags() {
        let query = ActivityQuery::parse("after=100&before=200&include=motion").expect("valid");
        assert!(!query.include_reviews);
        assert!(query.include_motion);
        let query = ActivityQuery::parse("after=100&before=200&include=reviews,motion,reviews")
            .expect("valid");
        assert!(query.include_reviews);
        assert!(query.include_motion);
        let err = ActivityQuery::parse("after=100&before=200&include=").unwrap_err();
        assert!(err.contains("`include`"), "{err}");
        let err = ActivityQuery::parse("after=100&before=200&include=reviews,alerts").unwrap_err();
        assert!(err.contains("`include`"), "{err}");
    }

    #[test]
    fn malformed_segments_are_named() {
        let err = ActivityQuery::parse("after=100&before=200&limit=10").unwrap_err();
        assert!(err.contains("`limit`"), "{err}");
        let err = ActivityQuery::parse("after=100&after=150&before=200").unwrap_err();
        assert!(err.contains("`after`"), "{err}");
        let err = ActivityQuery::parse("after=100&before=200&oops").unwrap_err();
        assert!(err.contains("`oops`"), "{err}");
        let err = ActivityQuery::parse("after=100&before=200&").unwrap_err();
        assert!(err.contains("empty query parameter"), "{err}");
    }

    #[test]
    fn percent_decoding_needs_two_hex_digits() {
        assert_eq!(
            percent_decode("front%2cback%20yard").expect("valid"),
            "front,back yard"
        );
        assert!(percent_decode("%2").is_err());
        assert!(percent_decode("%zz").is_err());
        assert_eq!(percent_decode("a+b").expect("plus kept"), "a+b");
        assert!(percent_decode("%FF").is_err());
        let query = ActivityQuery::parse("%61fter=100&before=200").expect("valid");
        assert_eq!(
            query,
            ActivityQuery {
                after: 100.0,
                before: 200.0,
                cameras: None,
                include_reviews: true,
                include_motion: true,
            }
        );
    }
}
