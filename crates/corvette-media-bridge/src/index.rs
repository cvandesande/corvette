//! The recording index: the sole `SQLite` writer, the per-camera sequence counter, and the range queries playback asks.

use crate::reviews;
use crate::supervise::log_event;
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

pub const PIECE_CHANNEL_CAPACITY: usize = 64;

const SCHEMA_VERSION: &str = "2";
const MIGRATABLE_SCHEMA_VERSION: &str = "1";

pub const ALERT_EVENT_CUTOFF_MS: i64 = 40_000;
pub const DETECTION_EVENT_CUTOFF_MS: i64 = 30_000;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS pieces (
    camera TEXT NOT NULL,
    seq INTEGER NOT NULL,
    start_ts INTEGER NOT NULL,
    end_ts INTEGER NOT NULL,
    duration_s REAL NOT NULL,
    path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    codec TEXT NOT NULL,
    upload_state TEXT NOT NULL DEFAULT 'local',
    PRIMARY KEY (camera, seq)
);
CREATE INDEX IF NOT EXISTS pieces_camera_start ON pieces(camera, start_ts);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    camera TEXT NOT NULL,
    track_id TEXT NOT NULL,
    label TEXT NOT NULL,
    start_ts INTEGER NOT NULL,
    end_ts INTEGER,
    last_motion_ts INTEGER NOT NULL,
    first_motion INTEGER,
    box_json TEXT NOT NULL,
    zones_json TEXT NOT NULL,
    UNIQUE(camera, track_id, start_ts)
);
CREATE TABLE IF NOT EXISTS reviews (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    camera TEXT NOT NULL,
    severity TEXT NOT NULL CHECK (severity IN ('alert', 'detection')),
    start_ts INTEGER NOT NULL,
    end_ts INTEGER,
    last_activity_ts INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS reviews_camera_end ON reviews(camera, end_ts);
CREATE TABLE IF NOT EXISTS motion (
    camera TEXT NOT NULL,
    second INTEGER NOT NULL,
    UNIQUE(camera, second)
);
";

#[derive(Debug, Clone)]
pub struct PieceRow {
    pub camera: String,
    pub seq: u64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub duration_s: f64,
    pub path: String,
    pub size_bytes: u64,
    pub codec: String,
}

/// One event row write, applied on the index writer's channel by the singleton writer task.
#[derive(Debug, Clone)]
pub enum EventWrite {
    Open {
        camera: String,
        track_id: String,
        label: String,
        start_ts: i64,
        first_motion_ts: i64,
        last_motion_ts: i64,
        box_json: String,
        zones_json: String,
    },
    Refresh {
        camera: String,
        track_id: String,
        start_ts: i64,
        last_motion_ts: i64,
        box_json: String,
        zones_json: String,
    },
    Close {
        camera: String,
        track_id: String,
        start_ts: i64,
        end_ts: i64,
    },
}

#[derive(Debug, Clone)]
pub struct ReviewActivity {
    pub camera: String,
    pub activity_ts: i64,
    pub is_alert: bool,
}

#[derive(Debug)]
pub enum IndexWrite {
    Piece(PieceRow),
    Event(EventWrite),
    Review(ReviewActivity),
    Sweep {
        now_ms: i64,
        alert_labels: HashMap<String, Vec<String>>,
    },
}

#[derive(Debug, Clone)]
pub struct EventRow {
    pub camera: String,
    pub track_id: String,
    pub label: String,
    pub start_ts: i64,
    pub end_ts: Option<i64>,
    pub last_motion_ts: i64,
    pub first_motion: Option<i64>,
}

#[derive(Debug)]
pub enum IndexError {
    Open {
        source: rusqlite::Error,
    },
    Setup {
        operation: &'static str,
        source: rusqlite::Error,
    },
    Pragma {
        operation: &'static str,
    },
    Version {
        found: String,
    },
    UnknownCamera {
        camera: String,
    },
    MetaValue {
        key: String,
    },
    Query {
        operation: &'static str,
        camera: String,
        source: rusqlite::Error,
    },
    OutOfRange {
        operation: &'static str,
        camera: String,
    },
    MissingRow {
        camera: String,
        seq: u64,
    },
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open { source } => write!(f, "opening the recording index failed: {source}"),
            Self::Setup { operation, source } => {
                write!(f, "recording index setup {operation} failed: {source}")
            }
            Self::Pragma { operation } => {
                write!(f, "recording index {operation} did not take effect")
            }
            Self::Version { found } => write!(
                f,
                "recording index schema_version is {found}, expected the version this build writes"
            ),
            Self::UnknownCamera { camera } => {
                write!(f, "next_seq camera {camera} is not known to the index")
            }
            Self::MetaValue { key } => {
                write!(f, "recording index meta key {key} does not hold an integer")
            }
            Self::Query {
                operation,
                camera,
                source,
            } => write!(f, "{operation} camera {camera} failed: {source}"),
            Self::OutOfRange { operation, camera } => write!(
                f,
                "{operation} camera {camera} carries a value outside the SQLite integer range"
            ),
            Self::MissingRow { camera, seq } => write!(
                f,
                "remove: the recording index holds no piece row for camera {camera} seq {seq}"
            ),
        }
    }
}

impl std::error::Error for IndexError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source } | Self::Setup { source, .. } | Self::Query { source, .. } => {
                Some(source)
            }
            Self::Version { .. }
            | Self::UnknownCamera { .. }
            | Self::MetaValue { .. }
            | Self::Pragma { .. }
            | Self::OutOfRange { .. }
            | Self::MissingRow { .. } => None,
        }
    }
}

fn query(operation: &'static str, camera: &str, source: rusqlite::Error) -> IndexError {
    IndexError::Query {
        operation,
        camera: camera.to_owned(),
        source,
    }
}

const fn setup(operation: &'static str, source: rusqlite::Error) -> IndexError {
    IndexError::Setup { operation, source }
}

fn sql_int(operation: &'static str, camera: &str, value: u64) -> Result<i64, IndexError> {
    i64::try_from(value).map_err(|_| IndexError::OutOfRange {
        operation,
        camera: camera.to_owned(),
    })
}

fn int_column(
    row: &Row<'_>,
    index: usize,
    operation: &'static str,
    camera: &str,
) -> Result<u64, IndexError> {
    let raw = row
        .get::<_, i64>(index)
        .map_err(|source| query(operation, camera, source))?;
    u64::try_from(raw).map_err(|_| IndexError::OutOfRange {
        operation,
        camera: camera.to_owned(),
    })
}

fn insert_review(
    conn: &Connection,
    camera: &str,
    severity: &str,
    at_ms: i64,
) -> Result<(), IndexError> {
    conn.execute(
        "INSERT INTO reviews (camera, severity, start_ts, last_activity_ts) VALUES (?1, ?2, ?3, ?3)",
        params![camera, severity, at_ms],
    )
    .map(|_| ())
    .map_err(|source| query("review open", camera, source))
}

fn piece_seconds_key(camera: &str) -> String {
    format!("piece_seconds:{camera}")
}

fn last_seq_key(camera: &str) -> String {
    format!("last_seq:{camera}")
}

fn read_meta(
    conn: &Connection,
    key: &str,
    operation: &'static str,
) -> Result<Option<String>, IndexError> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|source| setup(operation, source))
}

fn upsert_meta(
    conn: &Connection,
    key: &str,
    value: &str,
    operation: &'static str,
) -> Result<(), IndexError> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|source| setup(operation, source))?;
    Ok(())
}

fn parse_meta_u64(stored: Option<String>, key: &str) -> Result<u64, IndexError> {
    stored.map_or_else(
        || Ok(0),
        |text| {
            text.parse::<u64>().map_err(|_| IndexError::MetaValue {
                key: key.to_owned(),
            })
        },
    )
}

fn row_to_piece(
    row: &Row<'_>,
    operation: &'static str,
    camera: &str,
) -> Result<PieceRow, IndexError> {
    Ok(PieceRow {
        camera: row
            .get(0)
            .map_err(|source| query(operation, camera, source))?,
        seq: int_column(row, 1, operation, camera)?,
        start_ms: row
            .get(2)
            .map_err(|source| query(operation, camera, source))?,
        end_ms: row
            .get(3)
            .map_err(|source| query(operation, camera, source))?,
        duration_s: row
            .get(4)
            .map_err(|source| query(operation, camera, source))?,
        path: row
            .get(5)
            .map_err(|source| query(operation, camera, source))?,
        size_bytes: int_column(row, 6, operation, camera)?,
        codec: row
            .get(7)
            .map_err(|source| query(operation, camera, source))?,
    })
}

/// Hands out piece sequence numbers from the counter the [`Index`] writer keeps, so other threads share one source of truth.
#[derive(Clone, Debug)]
pub struct SeqAllocator {
    last_seq: Arc<Mutex<HashMap<String, u64>>>,
}

impl SeqAllocator {
    /// # Errors
    pub fn next(&self, camera: &str) -> Result<u64, IndexError> {
        let mut last_seq = self.last_seq.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(last) = last_seq.get_mut(camera) else {
            return Err(IndexError::UnknownCamera {
                camera: camera.to_owned(),
            });
        };
        *last += 1;
        let seq = *last;
        drop(last_seq);
        Ok(seq)
    }
}

pub struct Index {
    conn: Connection,
    last_seq: Arc<Mutex<HashMap<String, u64>>>,
}

impl std::fmt::Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Index")
            .field("last_seq", &self.last_seq)
            .finish_non_exhaustive()
    }
}

impl Index {
    /// # Errors
    /// Returns an error when the file cannot be opened or set up, a WAL or synchronous setting did not take effect, or its recorded schema version is neither this build's version nor the version that upgrades to it.
    pub fn open(path: &Path, cameras: &[(String, u32)]) -> Result<Self, IndexError> {
        let conn = Connection::open(path).map_err(|source| IndexError::Open { source })?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .map_err(|source| setup("pragmas", source))?;
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(|source| setup("journal mode", source))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(IndexError::Pragma {
                operation: "journal mode",
            });
        }
        let synchronous: i64 = conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .map_err(|source| setup("synchronous", source))?;
        if synchronous != 2 {
            return Err(IndexError::Pragma {
                operation: "synchronous",
            });
        }
        conn.execute_batch(SCHEMA)
            .map_err(|source| setup("schema", source))?;

        let stored_version = read_meta(&conn, "schema_version", "schema version")?;
        match stored_version.as_deref() {
            Some(SCHEMA_VERSION) => {}
            Some(MIGRATABLE_SCHEMA_VERSION) => {
                // The new tables are additive, so the schema batch above already created whatever the stored version lacked.
                upsert_meta(&conn, "schema_version", SCHEMA_VERSION, "schema version")?;
            }
            Some(found) => {
                return Err(IndexError::Version {
                    found: found.to_owned(),
                });
            }
            None => upsert_meta(&conn, "schema_version", SCHEMA_VERSION, "schema version")?,
        }

        for (camera, piece_seconds) in cameras {
            upsert_meta(
                &conn,
                &piece_seconds_key(camera),
                &piece_seconds.to_string(),
                "piece seconds",
            )?;
        }

        let mut last_seq = HashMap::new();
        for (camera, _) in cameras {
            let key = last_seq_key(camera);
            let stored = read_meta(&conn, &key, "last seq")?;
            let value = parse_meta_u64(stored, &key)?;
            last_seq.insert(camera.clone(), value);
        }
        Ok(Self {
            conn,
            last_seq: Arc::new(Mutex::new(last_seq)),
        })
    }

    /// # Errors
    pub fn next_seq(&mut self, camera: &str) -> Result<u64, IndexError> {
        self.allocator().next(camera)
    }

    /// Returns a handle that allocates the same per-camera sequence numbers as [`Self::next_seq`].
    #[must_use]
    pub fn allocator(&self) -> SeqAllocator {
        SeqAllocator {
            last_seq: Arc::clone(&self.last_seq),
        }
    }

    /// # Errors
    pub fn insert(&mut self, row: &PieceRow) -> Result<(), IndexError> {
        let seq_sql = sql_int("insert", &row.camera, row.seq)?;
        let size_sql = sql_int("insert", &row.camera, row.size_bytes)?;
        let tx = self
            .conn
            .transaction()
            .map_err(|source| query("insert", &row.camera, source))?;
        tx.execute(
            "INSERT INTO pieces (camera, seq, start_ts, end_ts, duration_s, path, size_bytes, codec)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                row.camera,
                seq_sql,
                row.start_ms,
                row.end_ms,
                row.duration_s,
                row.path,
                size_sql,
                row.codec
            ],
        )
        .map_err(|source| query("insert", &row.camera, source))?;

        let key = last_seq_key(&row.camera);
        let stored: Option<String> = tx
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| query("insert", &row.camera, source))?;
        let stored_seq = match stored {
            None => 0,
            Some(text) => text
                .parse::<u64>()
                .map_err(|_| IndexError::MetaValue { key: key.clone() })?,
        };
        let newest = stored_seq.max(row.seq);
        upsert_meta(&tx, &key, &newest.to_string(), "insert")?;
        tx.commit()
            .map_err(|source| query("insert", &row.camera, source))?;

        let mut memory = self.last_seq.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = memory.entry(row.camera.clone()).or_insert(0);
        *entry = (*entry).max(row.seq);
        drop(memory);
        Ok(())
    }

    /// # Errors
    /// Returns an error when the row is not present or cannot be deleted; the `last_seq` meta is not lowered.
    pub fn remove(&mut self, camera: &str, seq: u64) -> Result<(), IndexError> {
        let seq_sql = sql_int("remove", camera, seq)?;
        let tx = self
            .conn
            .transaction()
            .map_err(|source| query("remove", camera, source))?;
        let deleted = tx
            .execute(
                "DELETE FROM pieces WHERE camera = ?1 AND seq = ?2",
                params![camera, seq_sql],
            )
            .map_err(|source| query("remove", camera, source))?;
        if deleted == 0 {
            return Err(IndexError::MissingRow {
                camera: camera.to_owned(),
                seq,
            });
        }
        tx.commit()
            .map_err(|source| query("remove", camera, source))?;
        Ok(())
    }

    /// # Errors
    pub fn all_pieces(&self) -> Result<Vec<PieceRow>, IndexError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT camera, seq, start_ts, end_ts, duration_s, path, size_bytes, codec
                 FROM pieces
                 ORDER BY camera, seq",
            )
            .map_err(|source| query("all pieces", "", source))?;
        let mut rows = statement
            .query([])
            .map_err(|source| query("all pieces", "", source))?;
        let mut pieces = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|source| query("all pieces", "", source))?
        {
            pieces.push(row_to_piece(row, "all pieces", "")?);
        }
        Ok(pieces)
    }

    /// # Errors
    pub fn overlapping(
        &self,
        camera: &str,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<PieceRow>, IndexError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT camera, seq, start_ts, end_ts, duration_s, path, size_bytes, codec
                 FROM pieces
                 WHERE camera = ?1 AND end_ts > ?2 AND start_ts < ?3
                 ORDER BY start_ts",
            )
            .map_err(|source| query("overlapping", camera, source))?;
        let mut rows = statement
            .query(params![camera, from_ms, to_ms])
            .map_err(|source| query("overlapping", camera, source))?;
        let mut pieces = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|source| query("overlapping", camera, source))?
        {
            pieces.push(row_to_piece(row, "overlapping", camera)?);
        }
        Ok(pieces)
    }

    /// # Errors
    pub fn coverage(
        &self,
        camera: &str,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<(i64, i64)>, IndexError> {
        let pieces = self.overlapping(camera, from_ms, to_ms)?;
        let mut ranges: Vec<(i64, i64)> = Vec::new();
        for piece in pieces {
            match ranges.last_mut() {
                Some((_, end)) if piece.start_ms <= *end => *end = (*end).max(piece.end_ms),
                _ => ranges.push((piece.start_ms, piece.end_ms)),
            }
        }
        Ok(ranges)
    }

    /// Applies one event row write; a refresh or close for a row that is missing or already closed changes nothing.
    ///
    /// # Errors
    pub fn apply_event_write(&mut self, write: &EventWrite) -> Result<(), IndexError> {
        let outcome = match write {
            EventWrite::Open {
                camera,
                track_id,
                label,
                start_ts,
                first_motion_ts,
                last_motion_ts,
                box_json,
                zones_json,
            } => self.conn.execute(
                "INSERT INTO events (camera, track_id, label, start_ts, first_motion, last_motion_ts, box_json, zones_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(camera, track_id, start_ts) DO NOTHING",
                params![
                    camera,
                    track_id,
                    label,
                    start_ts,
                    first_motion_ts,
                    last_motion_ts,
                    box_json,
                    zones_json
                ],
            ),
            EventWrite::Refresh {
                camera,
                track_id,
                start_ts,
                last_motion_ts,
                box_json,
                zones_json,
            } => self.conn.execute(
                "UPDATE events SET last_motion_ts = MAX(last_motion_ts, ?4), box_json = ?5, zones_json = ?6
                 WHERE camera = ?1 AND track_id = ?2 AND start_ts = ?3 AND end_ts IS NULL",
                params![camera, track_id, start_ts, last_motion_ts, box_json, zones_json],
            ),
            EventWrite::Close {
                camera,
                track_id,
                start_ts,
                end_ts,
            } => self.conn.execute(
                "UPDATE events SET end_ts = ?4 WHERE camera = ?1 AND track_id = ?2 AND start_ts = ?3 AND end_ts IS NULL",
                params![camera, track_id, start_ts, end_ts],
            ),
        };
        outcome.map_err(|source| query("event write", "", source))?;
        Ok(())
    }

    /// Closes open events whose last motion is older than their cutoff — the label cutoff for camera alert labels, the other cutoff otherwise — and returns how many closed.
    ///
    /// # Errors
    pub fn close_stale_events(
        &mut self,
        now_ms: i64,
        alert_labels: &HashMap<String, Vec<String>>,
    ) -> Result<usize, IndexError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT camera, track_id, start_ts, label, last_motion_ts FROM events WHERE end_ts IS NULL",
            )
            .map_err(|source| query("sweep select", "", source))?;
        let mut rows = statement
            .query([])
            .map_err(|source| query("sweep select", "", source))?;
        let mut closing = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|source| query("sweep select", "", source))?
        {
            let camera: String = row
                .get(0)
                .map_err(|source| query("sweep select", "", source))?;
            let track_id: String = row
                .get(1)
                .map_err(|source| query("sweep select", "", source))?;
            let start_ts: i64 = row
                .get(2)
                .map_err(|source| query("sweep select", "", source))?;
            let label: String = row
                .get(3)
                .map_err(|source| query("sweep select", "", source))?;
            let last_motion_ts: i64 = row
                .get(4)
                .map_err(|source| query("sweep select", "", source))?;
            let is_alert = alert_labels
                .get(&camera)
                .is_some_and(|labels| labels.contains(&label));
            let cutoff = if is_alert {
                ALERT_EVENT_CUTOFF_MS
            } else {
                DETECTION_EVENT_CUTOFF_MS
            };
            if now_ms - last_motion_ts > cutoff {
                closing.push((camera, track_id, start_ts, last_motion_ts + cutoff));
            }
        }
        let mut closed = 0;
        for (camera, track_id, start_ts, end_ts) in closing {
            self.conn
                .execute(
                    "UPDATE events SET end_ts = ?4 WHERE camera = ?1 AND track_id = ?2 AND start_ts = ?3 AND end_ts IS NULL",
                    params![camera, track_id, start_ts, end_ts],
                )
                .map_err(|source| query("sweep close", &camera, source))?;
            closed += 1;
        }
        Ok(closed)
    }

    /// Applies one event activity to the camera's review aggregation: extend the open item, upgrade and extend, close it and open a new one, or open the first item.
    ///
    /// # Errors
    pub fn apply_review_activity(&mut self, activity: &ReviewActivity) -> Result<(), IndexError> {
        let open = self
            .conn
            .query_row(
                "SELECT id, severity, last_activity_ts FROM reviews WHERE camera = ?1 AND end_ts IS NULL",
                params![activity.camera],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
            )
            .optional()
            .map_err(|source| query("review lookup", &activity.camera, source))?;
        let severity = reviews::severity_for(activity.is_alert);
        match open {
            None => insert_review(&self.conn, &activity.camera, severity, activity.activity_ts),
            Some((id, item_severity, last_activity_ts)) => {
                let open_item = reviews::OpenReview {
                    id,
                    severity: item_severity,
                    last_activity_ts,
                };
                match reviews::on_open(&open_item, activity.activity_ts, activity.is_alert) {
                    reviews::AggregationAction::Extend { upgrade_to_alert } => {
                        let statement = if upgrade_to_alert {
                            "UPDATE reviews SET severity = 'alert', last_activity_ts = MAX(last_activity_ts, ?2) WHERE id = ?1 AND end_ts IS NULL"
                        } else {
                            "UPDATE reviews SET last_activity_ts = MAX(last_activity_ts, ?2) WHERE id = ?1 AND end_ts IS NULL"
                        };
                        self.conn
                            .execute(statement, params![id, activity.activity_ts])
                            .map(|_| ())
                            .map_err(|source| query("review extend", &activity.camera, source))
                    }
                    reviews::AggregationAction::CloseThenOpen { closed_end_ts } => {
                        let tx = self
                            .conn
                            .transaction()
                            .map_err(|source| query("review split", &activity.camera, source))?;
                        tx.execute(
                            "UPDATE reviews SET end_ts = ?2 WHERE id = ?1 AND end_ts IS NULL",
                            params![id, closed_end_ts],
                        )
                        .map_err(|source| query("review split", &activity.camera, source))?;
                        insert_review(&tx, &activity.camera, severity, activity.activity_ts)?;
                        tx.commit()
                            .map_err(|source| query("review split", &activity.camera, source))
                    }
                }
            }
        }
    }

    /// Closes open review items whose severity cutoff has strictly passed before `now_ms`, each at its own rule instant, and returns how many closed.
    ///
    /// # Errors
    pub fn close_stale_reviews(&mut self, now_ms: i64) -> Result<usize, IndexError> {
        let mut statement = self
            .conn
            .prepare("SELECT id, severity, last_activity_ts FROM reviews WHERE end_ts IS NULL")
            .map_err(|source| query("review sweep select", "", source))?;
        let mut rows = statement
            .query([])
            .map_err(|source| query("review sweep select", "", source))?;
        let mut closing = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|source| query("review sweep select", "", source))?
        {
            let id: i64 = row
                .get(0)
                .map_err(|source| query("review sweep select", "", source))?;
            let severity: String = row
                .get(1)
                .map_err(|source| query("review sweep select", "", source))?;
            let last_activity_ts: i64 = row
                .get(2)
                .map_err(|source| query("review sweep select", "", source))?;
            let open_item = reviews::OpenReview {
                id,
                severity,
                last_activity_ts,
            };
            if let Some(end_ts) = reviews::sweep_close_ts(&open_item, now_ms) {
                closing.push((id, end_ts));
            }
        }
        let mut closed = 0;
        for (id, end_ts) in closing {
            closed += self
                .conn
                .execute(
                    "UPDATE reviews SET end_ts = ?2 WHERE id = ?1 AND end_ts IS NULL",
                    params![id, end_ts],
                )
                .map_err(|source| query("review sweep close", "", source))?;
        }
        Ok(closed)
    }

    /// The count of events still open, the restart recovery number.
    ///
    /// # Errors
    pub fn open_event_count(&self) -> Result<i64, IndexError> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE end_ts IS NULL",
                [],
                |row| row.get(0),
            )
            .map_err(|source| query("open events", "", source))
    }

    /// The events of `camera`, oldest row first.
    ///
    /// # Errors
    pub fn events(&self, camera: &str) -> Result<Vec<EventRow>, IndexError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT camera, track_id, label, start_ts, end_ts, last_motion_ts, first_motion
                 FROM events WHERE camera = ?1 ORDER BY id",
            )
            .map_err(|source| query("events", camera, source))?;
        let mut rows = statement
            .query(params![camera])
            .map_err(|source| query("events", camera, source))?;
        let mut events = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|source| query("events", camera, source))?
        {
            events.push(EventRow {
                camera: row
                    .get(0)
                    .map_err(|source| query("events", camera, source))?,
                track_id: row
                    .get(1)
                    .map_err(|source| query("events", camera, source))?,
                label: row
                    .get(2)
                    .map_err(|source| query("events", camera, source))?,
                start_ts: row
                    .get(3)
                    .map_err(|source| query("events", camera, source))?,
                end_ts: row
                    .get(4)
                    .map_err(|source| query("events", camera, source))?,
                last_motion_ts: row
                    .get(5)
                    .map_err(|source| query("events", camera, source))?,
                first_motion: row
                    .get(6)
                    .map_err(|source| query("events", camera, source))?,
            });
        }
        Ok(events)
    }
}

fn apply_write(index: &mut Index, write: IndexWrite) {
    match write {
        IndexWrite::Piece(row) => {
            if index.insert(&row).is_err() {
                eprintln!(
                    "corvette-media-bridge: index insert failed camera={} seq={}",
                    row.camera, row.seq
                );
            }
        }
        IndexWrite::Event(event) => {
            if let Err(source) = index.apply_event_write(&event) {
                eprintln!("corvette-media-bridge: event write failed: {source}");
            }
        }
        IndexWrite::Review(activity) => {
            if let Err(source) = index.apply_review_activity(&activity) {
                eprintln!("corvette-media-bridge: review write failed: {source}");
            }
        }
        IndexWrite::Sweep {
            now_ms,
            alert_labels,
        } => {
            match index.close_stale_events(now_ms, &alert_labels) {
                Err(source) => {
                    eprintln!("corvette-media-bridge: event cutoff sweep failed: {source}");
                }
                Ok(0) => {}
                Ok(closed) => log_event("events", "cutoff-close", format_args!("closed={closed}")),
            }
            match index.close_stale_reviews(now_ms) {
                Err(source) => {
                    eprintln!("corvette-media-bridge: review cutoff sweep failed: {source}");
                }
                Ok(0) => {}
                Ok(closed) => log_event("reviews", "cutoff-close", format_args!("closed={closed}")),
            }
        }
    }
}

/// Hands the index to a writer thread fed by the returned sender; it returns the index once the sender is dropped and the channel drains.
pub fn spawn_writer(
    index: Index,
    capacity: usize,
) -> (
    std::sync::mpsc::SyncSender<IndexWrite>,
    std::thread::JoinHandle<Index>,
) {
    let (sender, receiver) = std::sync::mpsc::sync_channel(capacity);
    let handle = std::thread::spawn(move || {
        let mut index = index;
        while let Ok(write) = receiver.recv() {
            apply_write(&mut index, write);
        }
        index
    });
    (sender, handle)
}

pub const EXPIRY_PASS_INTERVAL_MS: i64 = 3_600_000;
const PRESSURE_PASS_INTERVAL_MS: i64 = 60_000;
const EXPIRY_TICK: Duration = Duration::from_mins(1);

pub type FreeSpaceReader = Box<dyn Fn(&Path) -> std::io::Result<u64> + Send>;

/// Storage root, per-camera `retention_days`, dry-run switch, injected clock and injected free-space reader for the hourly expiry pass and the pressure pass.
pub struct ExpiryWiring {
    pub root: PathBuf,
    pub retention_days: HashMap<String, u32>,
    pub enforce: bool,
    pub clock: Box<dyn Fn() -> i64 + Send>,
    pub recording_cameras: usize,
    pub free_space: FreeSpaceReader,
}

impl std::fmt::Debug for ExpiryWiring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExpiryWiring")
            .field("root", &self.root)
            .field("retention_days", &self.retention_days)
            .field("enforce", &self.enforce)
            .field("clock", &"injected")
            .field("recording_cameras", &self.recording_cameras)
            .field("free_space", &"injected")
            .finish()
    }
}

/// Runs [`spawn_writer`]'s loop on one thread and additionally runs [`crate::expiry::run_pass`] hourly and [`crate::pressure::run_pressure_pass`] at the pressure cadence there, so inserts and passes never overlap; `wiring.clock` is the passes' only clock read.
pub fn spawn_writer_with_expiry(
    index: Index,
    capacity: usize,
    wiring: ExpiryWiring,
) -> (
    std::sync::mpsc::SyncSender<IndexWrite>,
    std::thread::JoinHandle<Index>,
) {
    let (sender, receiver) = std::sync::mpsc::sync_channel(capacity);
    let handle = std::thread::spawn(move || {
        let mut index = index;
        let mut next_pass_ms = (wiring.clock)().saturating_add(EXPIRY_PASS_INTERVAL_MS);
        let mut next_pressure_ms = (wiring.clock)();
        let mut free_unreadable = false;
        loop {
            match receiver.recv_timeout(EXPIRY_TICK) {
                Ok(write) => apply_write(&mut index, write),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            let now_ms = (wiring.clock)();
            if now_ms >= next_pass_ms {
                next_pass_ms = now_ms.saturating_add(EXPIRY_PASS_INTERVAL_MS);
                if let Err(error) = crate::expiry::run_pass(
                    &wiring.root,
                    &mut index,
                    now_ms,
                    &wiring.retention_days,
                    wiring.enforce,
                ) {
                    eprintln!("corvette-media-bridge: expiry pass failed: {error}");
                }
            }
            if now_ms >= next_pressure_ms {
                next_pressure_ms = now_ms.saturating_add(PRESSURE_PASS_INTERVAL_MS);
                match (wiring.free_space)(&wiring.root) {
                    Err(source) => {
                        if !free_unreadable {
                            log_event("storage", "pressure-free-unreadable", source.kind());
                            free_unreadable = true;
                        }
                    }
                    Ok(free_bytes) => {
                        if free_unreadable {
                            log_event("storage", "pressure-free-readable", "");
                            free_unreadable = false;
                        }
                        if let Err(error) = crate::pressure::run_pressure_pass(
                            &wiring.root,
                            &mut index,
                            now_ms,
                            &wiring.retention_days,
                            wiring.recording_cameras,
                            free_bytes,
                            wiring.enforce,
                        ) {
                            eprintln!("corvette-media-bridge: pressure pass failed: {error}");
                        }
                    }
                }
            }
        }
        index
    });
    (sender, handle)
}
