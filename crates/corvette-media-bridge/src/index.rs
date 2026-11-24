//! The recording index: the sole `SQLite` writer, the per-camera sequence counter, and the range queries playback asks.

use rusqlite::{Connection, OptionalExtension, Row, params};
use std::collections::BTreeMap;
use std::path::Path;

pub const PIECE_CHANNEL_CAPACITY: usize = 64;

const SCHEMA_VERSION: &str = "1";

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

pub struct Index {
    conn: Connection,
    last_seq: BTreeMap<String, u64>,
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
    /// Returns an error when the file cannot be opened or set up, a WAL or synchronous setting did not take effect, or its recorded schema version is not the one this build writes.
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
        match stored_version {
            Some(found) if found != SCHEMA_VERSION => return Err(IndexError::Version { found }),
            Some(_) => {}
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

        let mut last_seq = BTreeMap::new();
        for (camera, _) in cameras {
            let key = last_seq_key(camera);
            let stored = read_meta(&conn, &key, "last seq")?;
            let value = parse_meta_u64(stored, &key)?;
            last_seq.insert(camera.clone(), value);
        }
        Ok(Self { conn, last_seq })
    }

    /// # Errors
    /// Returns an error when the camera is not known to this index.
    pub fn next_seq(&mut self, camera: &str) -> Result<u64, IndexError> {
        let last = self
            .last_seq
            .get_mut(camera)
            .ok_or_else(|| IndexError::UnknownCamera {
                camera: camera.to_owned(),
            })?;
        *last += 1;
        Ok(*last)
    }

    /// # Errors
    /// Returns an error when the row cannot be written or repeats a `(camera, seq)` already inserted.
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

        let memory = self.last_seq.entry(row.camera.clone()).or_insert(0);
        *memory = (*memory).max(row.seq);
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
    /// Returns an error when the stored rows cannot be read.
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
}

/// Hands the index to a writer thread fed by the returned sender; it returns the index once the sender is dropped and the channel drains.
pub fn spawn_writer(
    index: Index,
    capacity: usize,
) -> (
    std::sync::mpsc::SyncSender<PieceRow>,
    std::thread::JoinHandle<Index>,
) {
    let (sender, receiver) = std::sync::mpsc::sync_channel(capacity);
    let handle = std::thread::spawn(move || {
        let mut index = index;
        while let Ok(row) = receiver.recv() {
            if index.insert(&row).is_err() {
                eprintln!(
                    "corvette-media-bridge: index insert failed camera={} seq={}",
                    row.camera, row.seq
                );
            }
        }
        index
    });
    (sender, handle)
}
