//! Read-only access to Frigate's `SQLite` database (issue #4 D5).
//!
//! The database belongs to Frigate and this service must never write it, so
//! every open carries `SQLITE_OPEN_READ_ONLY` and a `mode=ro` URI (issue #4
//! I3). rusqlite is synchronous; callers move a probe onto a blocking thread.

use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;

/// Environment variable naming Frigate's `SQLite` database file.
pub const DB_PATH_ENV: &str = "CORVETTE_API_DB_PATH";

/// The database path used when `CORVETTE_API_DB_PATH` is unset: Frigate's
/// database in the shared config volume (issue #4 D5).
pub const DEFAULT_DB_PATH: &str = "/config/frigate.db";

/// One short read proving the file is a readable Frigate database, not an
/// unrelated or empty file: `reviewsegment` exists (Frigate's own table) and
/// is queryable. An empty table is healthy; a missing table is not.
const PROBE_QUERY: &str = "SELECT 1 FROM reviewsegment LIMIT 1";

/// How long a read waits on Frigate's write lock before giving up: health
/// must answer rather than block, and a lock held past this is a fault.
const BUSY_TIMEOUT: Duration = Duration::from_secs(1);

/// Opens `path` strictly read-only (issue #4 I3). Never use
/// `Connection::open`: it would open the live database read-write.
///
/// # Errors
///
/// Returns the `SQLite` error if the file is missing, unreadable, not a
/// database, or locked past [`BUSY_TIMEOUT`].
pub fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    let uri = read_only_uri(path);
    let connection = Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    Ok(connection)
}

/// Opens `path` read-only and runs [`PROBE_QUERY`], reporting a one-line
/// reason instead of opening the connection for the caller.
///
/// # Errors
///
/// Describes the open or query failure, including `SQLite`'s reason.
pub(crate) fn probe(path: &Path) -> Result<(), String> {
    let connection = open_read_only(path).map_err(|err| format!("open failed: {err}"))?;
    let mut statement = connection
        .prepare(PROBE_QUERY)
        .map_err(|err| format!("probe query failed: {err}"))?;
    // Ok even with zero rows: an empty table still proves this is Frigate's
    // database; only a prepare failure (missing table) reaches the Err arm.
    match statement.query([]) {
        Ok(_) => Ok(()),
        Err(err) => Err(format!("probe query failed: {err}")),
    }
}

/// Builds the `file:` URI for a strictly read-only open. `%`, `?` and `#`
/// percent-encode so a path cannot inject URI parameters, and bytes outside
/// ASCII percent-encode to keep the URI plain ASCII (issue #4 I3).
fn read_only_uri(path: &Path) -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut uri = String::from("file:");
    for byte in path.to_string_lossy().as_bytes() {
        match byte {
            b'%' => uri.push_str("%25"),
            b'?' => uri.push_str("%3F"),
            b'#' => uri.push_str("%23"),
            0x01..=0x1F | 0x7F | 0x80..=0xFF => {
                uri.push('%');
                uri.push(HEX_DIGITS[usize::from(byte >> 4)] as char);
                uri.push(HEX_DIGITS[usize::from(byte & 0x0F)] as char);
            }
            _ => uri.push(char::from(*byte)),
        }
    }
    uri.push_str("?mode=ro");
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique writable temp dir; removed again by the caller's cleanup.
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "corvette-db-test-{}-{tag}-{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn the_uri_encodes_the_parameter_characters_and_leaves_the_rest() {
        let path = Path::new("/db/room 1?x#20%3");
        let uri = read_only_uri(path);
        assert_eq!(uri, "file:/db/room 1%3Fx%2320%253?mode=ro");
        assert!(uri.contains("mode=ro"));
        assert!(!uri.contains("immutable"));
    }

    #[test]
    fn a_write_attempt_is_refused_and_changes_nothing() {
        let dir = temp_dir("write-refused");
        let path = dir.join("frigate.db");
        {
            let writer = Connection::open(&path).expect("create test database");
            writer
                .execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE reviewsegment (id INTEGER);")
                .expect("set up WAL database");
            writer
                .execute("INSERT INTO reviewsegment (id) VALUES (1)", [])
                .expect("seed row");
        }

        let reader = open_read_only(&path).expect("open read-only");
        let write_attempt = reader.execute("INSERT INTO reviewsegment (id) VALUES (2)", []);
        match write_attempt {
            Err(rusqlite::Error::SqliteFailure(err, Some(message))) => {
                assert_eq!(err.code, rusqlite::ErrorCode::ReadOnly);
                assert!(message.to_ascii_lowercase().contains("readonly"));
            }
            other => panic!("a write through a read-only open must fail, got {other:?}"),
        }
        drop(reader);

        let checker = Connection::open(&path).expect("reopen read-write");
        let rows: i64 = checker
            .query_row("SELECT count(*) FROM reviewsegment", [], |row| row.get(0))
            .expect("count rows");
        assert_eq!(rows, 1, "the refused write must have changed nothing");
        drop(checker);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_probe_passes_on_an_empty_reviewsegment_table() {
        let dir = temp_dir("empty-table");
        let path = dir.join("frigate.db");
        {
            let writer = Connection::open(&path).expect("create test database");
            writer
                .execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE reviewsegment (id INTEGER);")
                .expect("set up WAL database");
        }
        assert_eq!(probe(&path), Ok(()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_probe_reports_a_missing_file_without_creating_it() {
        let dir = temp_dir("missing-file");
        let path = dir.join("absent.db");
        let result = probe(&path);
        assert!(result.is_err());
        assert!(!path.exists(), "a probe must not create the database file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
