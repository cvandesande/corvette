use corvette_media_bridge::index::{self, Index, IndexError, PieceRow};
use std::path::{Path, PathBuf};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("corvette-index-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("test temp directory created");
    dir
}

fn cleanup(dir: &Path) {
    std::fs::remove_dir_all(dir).expect("test temp directory removed");
}

fn piece(camera: &str, seq: u64, start_ms: i64, end_ms: i64) -> PieceRow {
    PieceRow {
        camera: camera.to_owned(),
        seq,
        start_ms,
        end_ms,
        duration_s: 1.0,
        path: format!("/{camera}/{seq}.fmp4"),
        size_bytes: 4096,
        codec: "h264".to_owned(),
    }
}

fn front() -> Vec<(String, u32)> {
    vec![("front".to_owned(), 4u32)]
}

#[test]
fn open_creates_the_schema_and_reopen_keeps_inserted_rows() {
    let dir = temp_dir("open-reopen");
    let path = dir.join("index.sqlite");

    let mut first = Index::open(&path, &front()).expect("first open succeeds");
    assert!(path.exists());
    let seq = first.next_seq("front").expect("first seq handed");
    assert_eq!(seq, 1);
    first
        .insert(&piece("front", seq, 0, 1000))
        .expect("insert succeeds");
    drop(first);

    let second = Index::open(&path, &front()).expect("reopen succeeds");
    let rows = second
        .overlapping("front", 0, 1000)
        .expect("overlap query succeeds");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].seq, 1);
    assert_eq!(rows[0].camera, "front");
    assert_eq!(rows[0].start_ms, 0);
    assert_eq!(rows[0].end_ms, 1000);

    cleanup(&dir);
}

#[test]
fn next_seq_strictly_increases_across_reopen_after_inserts() {
    let dir = temp_dir("seq-across-reopen");
    let path = dir.join("index.sqlite");

    let mut before = Index::open(&path, &front()).expect("first open succeeds");
    let first = before.next_seq("front").expect("seq 1 handed");
    before
        .insert(&piece("front", first, 0, 1000))
        .expect("insert succeeds");
    let second = before.next_seq("front").expect("seq 2 handed");
    before
        .insert(&piece("front", second, 1000, 2000))
        .expect("insert succeeds");
    drop(before);

    let mut after = Index::open(&path, &front()).expect("reopen succeeds");
    let third = after.next_seq("front").expect("seq 3 handed");
    let fourth = after.next_seq("front").expect("seq 4 handed");
    assert!(third > second);
    assert!(fourth > third);

    cleanup(&dir);
}

#[test]
fn reopened_index_never_hands_a_seq_already_inserted() {
    let dir = temp_dir("seq-reopen-collision");
    let path = dir.join("index.sqlite");

    let mut before = Index::open(&path, &front()).expect("first open succeeds");
    for _ in 0..3 {
        let seq = before.next_seq("front").expect("seq handed");
        let start_ms = i64::try_from(seq).expect("seq fits i64") * 1000;
        before
            .insert(&piece("front", seq, start_ms, start_ms + 1000))
            .expect("insert succeeds");
    }
    drop(before);

    let mut after = Index::open(&path, &front()).expect("reopen succeeds");
    let handed = after.next_seq("front").expect("seq handed after reopen");
    assert_eq!(handed, 4);

    cleanup(&dir);
}

#[test]
fn overlapping_excludes_touching_ends_and_orders_by_start() {
    let dir = temp_dir("overlap-half-open");
    let path = dir.join("index.sqlite");

    let mut index = Index::open(&path, &front()).expect("open succeeds");
    index
        .insert(&piece("front", 1, 0, 10))
        .expect("piece ending at the window start");
    index
        .insert(&piece("front", 2, 5, 12))
        .expect("piece inside the window");
    index
        .insert(&piece("front", 3, 25, 35))
        .expect("piece inside the window");
    index
        .insert(&piece("front", 4, 35, 45))
        .expect("piece starting at the window end");

    let rows = index
        .overlapping("front", 10, 35)
        .expect("overlap query succeeds");
    assert_eq!(
        rows.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert_eq!(
        rows.iter().map(|row| row.start_ms).collect::<Vec<_>>(),
        vec![5, 25]
    );

    cleanup(&dir);
}

#[test]
fn coverage_merges_touching_pieces_and_splits_at_a_gap() {
    let dir = temp_dir("coverage-merge");
    let path = dir.join("index.sqlite");

    let mut index = Index::open(&path, &front()).expect("open succeeds");
    index
        .insert(&piece("front", 1, 0, 10))
        .expect("first piece");
    index
        .insert(&piece("front", 2, 30, 40))
        .expect("piece after the gap");
    index
        .insert(&piece("front", 3, 10, 20))
        .expect("piece touching the first");

    let ranges = index
        .coverage("front", 0, 1000)
        .expect("coverage query succeeds");
    assert_eq!(ranges, vec![(0, 20), (30, 40)]);

    cleanup(&dir);
}

#[test]
fn duplicate_camera_seq_insert_is_refused() {
    let dir = temp_dir("duplicate-seq");
    let path = dir.join("index.sqlite");

    let mut index = Index::open(&path, &front()).expect("open succeeds");
    index
        .insert(&piece("front", 1, 0, 1000))
        .expect("first insert succeeds");
    let error = index
        .insert(&piece("front", 1, 5000, 6000))
        .expect_err("duplicate camera and seq refused");
    assert!(matches!(error, IndexError::Query { .. }), "{error}");

    let rows = index
        .overlapping("front", 0, 10000)
        .expect("overlap query succeeds");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].start_ms, 0);

    cleanup(&dir);
}

#[test]
fn file_with_other_schema_version_is_refused() {
    let dir = temp_dir("schema-version");
    let path = dir.join("index.sqlite");

    let index = Index::open(&path, &front()).expect("first open succeeds");
    drop(index);

    let raw = rusqlite::Connection::open(&path).expect("raw open succeeds");
    raw.execute(
        "UPDATE meta SET value = '2' WHERE key = 'schema_version'",
        [],
    )
    .expect("schema version rewritten");
    drop(raw);

    let error = Index::open(&path, &front()).expect_err("foreign schema version refused");
    match error {
        IndexError::Version { found } => assert_eq!(found, "2"),
        other => panic!("expected a version refusal, got {other}"),
    }

    cleanup(&dir);
}

#[test]
fn writer_thread_inserts_rows_and_returns_the_index() {
    let dir = temp_dir("writer-thread");
    let path = dir.join("index.sqlite");

    let mut index = Index::open(&path, &front()).expect("open succeeds");
    let seq1 = index.next_seq("front").expect("seq 1 handed");
    let seq2 = index.next_seq("front").expect("seq 2 handed");

    let (sender, handle) = index::spawn_writer(index, index::PIECE_CHANNEL_CAPACITY);
    sender
        .send(piece("front", seq1, 0, 1000))
        .expect("first row sent");
    sender
        .send(piece("front", seq2, 1000, 2000))
        .expect("second row sent");
    drop(sender);

    let mut index = handle.join().expect("writer thread returned the index");
    let rows = index
        .overlapping("front", 0, 2000)
        .expect("overlap query succeeds");
    assert_eq!(
        rows.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![seq1, seq2]
    );
    assert_eq!(index.next_seq("front").expect("seq 3 handed"), 3);

    cleanup(&dir);
}

#[test]
fn opened_database_reports_wal_to_a_fresh_connection() {
    let dir = temp_dir("wal-readback");
    let path = dir.join("index.sqlite");

    let index = Index::open(&path, &front()).expect("open succeeds");
    drop(index);

    let raw = rusqlite::Connection::open(&path).expect("raw open succeeds");
    let mode: String = raw
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal mode read");
    assert!(mode.eq_ignore_ascii_case("wal"), "journal mode {mode}");

    cleanup(&dir);
}

#[test]
fn memory_database_is_refused_because_it_cannot_be_wal() {
    let error = Index::open(Path::new(":memory:"), &[]).expect_err("in-memory database refused");
    assert!(
        matches!(
            error,
            IndexError::Pragma {
                operation: "journal mode"
            }
        ),
        "{error}"
    );
}
