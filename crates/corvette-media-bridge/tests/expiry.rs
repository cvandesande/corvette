use corvette_media_bridge::expiry::{self, PassReport};
use corvette_media_bridge::index::{Index, PieceRow};
use corvette_media_bridge::recovery::{self, ScanReport};
use corvette_media_bridge::utc::utc_parts;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

const DAY: i64 = 86_400_000;
const NOW: i64 = 1_790_000_000_000;

fn fresh_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("corvette-expiry-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("test root created");
    dir
}

fn open_index(root: &Path) -> Index {
    Index::open(&root.join("index.sqlite"), &[("front".to_owned(), 4)]).expect("index opens")
}

fn retention(entries: &[(&str, u32)]) -> HashMap<String, u32> {
    entries
        .iter()
        .map(|(camera, days)| ((*camera).to_owned(), *days))
        .collect()
}

fn piece_dir_name(start_ms: i64) -> (String, String) {
    let parts = utc_parts(start_ms);
    (
        format!("{:04}-{:02}-{:02}", parts.year, parts.month, parts.day),
        format!("{:02}", parts.hour),
    )
}

fn piece_relative_path(start_ms: i64, seq: u64) -> String {
    let (date, hour) = piece_dir_name(start_ms);
    format!("recordings/front/{date}/{hour}/{start_ms}-{seq}.mp4")
}

/// Writes the piece file at the recorder's tree location and inserts its row.
fn seed_piece(root: &Path, index: &mut Index, seq: u64, start_ms: i64, end_ms: i64) {
    let relative = piece_relative_path(start_ms, seq);
    let contents = format!("piece-front-{seq}").into_bytes();
    let full = root.join(&relative);
    std::fs::create_dir_all(full.parent().expect("the path names a directory"))
        .expect("piece directory created");
    std::fs::write(&full, &contents).expect("piece file written");
    index
        .insert(&PieceRow {
            camera: "front".to_owned(),
            seq,
            start_ms,
            end_ms,
            duration_s: std::time::Duration::from_millis(
                u64::try_from(end_ms - start_ms).expect("test piece durations are positive"),
            )
            .as_secs_f64(),
            path: relative,
            size_bytes: contents.len() as u64,
            codec: "h264".to_owned(),
        })
        .expect("row inserted");
}

fn row_set(index: &Index) -> Vec<(String, u64)> {
    index
        .all_pieces()
        .expect("rows read")
        .iter()
        .map(|row| (row.camera.clone(), row.seq))
        .collect()
}

fn file_present(root: &Path, start_ms: i64, seq: u64) -> bool {
    root.join(piece_relative_path(start_ms, seq)).exists()
}

fn deletion_lines(root: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(root.join("deletions.log.jsonl")).expect("log readable");
    text.lines().map(str::to_owned).collect()
}

fn trash_files(root: &Path) -> Vec<String> {
    match std::fs::read_dir(root.join(".trash")) {
        Ok(entries) => entries
            .map(|entry| {
                entry
                    .expect("trash entry readable")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("trash unreadable: {error}"),
    }
}

const CHILD_ENV: &str = "CORVETTE_EXPIRY_CHILD_RUN";
const GUARD_TOKEN: &str = "guard-child";
const DRY_RUN_TOKEN: &str = "dry-run-child";
const GUARD_TEST: &str = "a_four_hundred_day_clock_jump_refuses_the_pass_and_deletes_nothing";
const DRY_RUN_TEST: &str =
    "a_dry_run_changes_no_file_and_no_row_and_logs_one_would_delete_line_per_expiring_piece";

fn spawn_child(token: &str, test_name: &str) -> std::process::Output {
    std::process::Command::new(std::env::current_exe().expect("test binary path"))
        .arg("--exact")
        .arg(test_name)
        .arg("--nocapture")
        .env_remove(CHILD_ENV)
        .env(CHILD_ENV, token)
        .output()
        .expect("the child test runs")
}

#[test]
fn enforce_deletes_exactly_the_expired_pieces_and_writes_one_log_line_each() {
    let root = fresh_root("enforce-normal");
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 1, NOW - 8 * DAY, NOW - 8 * DAY + 1000);
    seed_piece(
        &root,
        &mut index,
        2,
        NOW - 7 * DAY - 2000,
        NOW - 7 * DAY - 1000,
    );
    seed_piece(&root, &mut index, 3, NOW - 7 * DAY - 2000, NOW - 7 * DAY);
    seed_piece(&root, &mut index, 4, NOW - 3_600_000, NOW - 1000);

    let report = expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 7)]), true)
        .expect("pass runs");
    assert_eq!(
        report,
        PassReport {
            enforce: true,
            would_delete: 2,
            deleted: 2,
            refused_cameras: 0,
            step_failures: [0; 4],
        }
    );
    assert!(
        !file_present(&root, NOW - 8 * DAY, 1) && !file_present(&root, NOW - 7 * DAY - 2000, 2),
        "expired files gone"
    );
    assert!(
        file_present(&root, NOW - 7 * DAY - 2000, 3),
        "a piece ending exactly on the cutoff survives"
    );
    assert!(file_present(&root, NOW - 3_600_000, 4), "fresh piece kept");
    assert_eq!(
        row_set(&index),
        vec![("front".to_owned(), 3), ("front".to_owned(), 4)]
    );
    let lines = deletion_lines(&root);
    assert_eq!(lines.len(), 2, "one log line per deleted piece");
    assert!(lines[0].contains("\"seq\":1"));
    assert!(lines[1].contains("\"seq\":2"));
    assert!(
        trash_files(&root).is_empty(),
        "the trash file is unlinked in the unlink step"
    );
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn zero_retention_deletes_nothing_in_enforce_mode() {
    let root = fresh_root("zero-retention");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        1,
        NOW - 400 * DAY,
        NOW - 400 * DAY + 1000,
    );
    seed_piece(
        &root,
        &mut index,
        2,
        NOW - 500 * DAY,
        NOW - 500 * DAY + 1000,
    );

    let report = expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 0)]), true)
        .expect("pass runs");
    assert_eq!(report.deleted, 0);
    assert_eq!(report.would_delete, 0);
    assert!(file_present(&root, NOW - 400 * DAY, 1));
    assert!(file_present(&root, NOW - 500 * DAY, 2));
    assert_eq!(row_set(&index).len(), 2);
    assert!(!root.join("deletions.log.jsonl").exists());
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_four_hundred_day_clock_jump_refuses_the_pass_and_deletes_nothing() {
    if std::env::var(CHILD_ENV).unwrap_or_default() == GUARD_TOKEN {
        let root = fresh_root("guard-child");
        let mut index = open_index(&root);
        for seq in 1..=10 {
            let age_ms = i64::try_from(seq).expect("test sequence fits i64") * 1000;
            seed_piece(&root, &mut index, seq, NOW - age_ms, NOW);
        }
        let report = expiry::run_pass(
            &root,
            &mut index,
            NOW + 400 * DAY,
            &retention(&[("front", 7)]),
            true,
        )
        .expect("pass runs");
        assert_eq!(report.deleted, 0, "a tripped guard deletes nothing");
        assert_eq!(report.refused_cameras, 1);
        assert_eq!(report.would_delete, 0);
        assert_eq!(row_set(&index).len(), 10, "every row survives");
        assert!(!root.join("deletions.log.jsonl").exists());
        std::fs::remove_dir_all(&root).expect("temp root removed");
        return;
    }
    let output = spawn_child(GUARD_TOKEN, GUARD_TEST);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "the child run failed:\n{stderr}");
    assert_eq!(
        stderr.matches("event=expiry-refused").count(),
        1,
        "one refusal line\n{stderr}"
    );
    assert_eq!(
        stderr.matches("event=expiry-alert").count(),
        1,
        "one alert line\n{stderr}"
    );
    assert_eq!(stderr.matches("event=expiry-step-failed").count(), 0);
}

#[test]
fn a_few_hour_clock_jump_expires_inside_the_window_without_a_refusal() {
    let root = fresh_root("hours-jump");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        1,
        NOW - 7 * DAY - 4 * 3_600_000,
        NOW - 7 * DAY - 3 * 3_600_000,
    );
    seed_piece(
        &root,
        &mut index,
        2,
        NOW - 7 * DAY,
        NOW - 7 * DAY + 5 * 3_600_000,
    );
    for seq in 3..=10 {
        seed_piece(&root, &mut index, seq, NOW - 2 * DAY, NOW - DAY);
    }

    let report = expiry::run_pass(
        &root,
        &mut index,
        NOW + 4 * 3_600_000,
        &retention(&[("front", 7)]),
        true,
    )
    .expect("pass runs");
    assert_eq!(report.refused_cameras, 0, "a partial set never trips");
    assert_eq!(report.deleted, 1);
    assert!(!file_present(&root, NOW - 7 * DAY - 4 * 3_600_000, 1));
    assert!(file_present(&root, NOW - 7 * DAY, 2));
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

fn tree_snapshot(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir).expect("directory readable") {
        let path = entry.expect("entry readable").path();
        if path.is_dir() {
            tree_snapshot(&path, root, out);
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("snapshot stays under the root")
                .display()
                .to_string();
            // The WAL shared-memory mirror is rewritten by SQLite on any read
            // transaction, so it cannot be byte-equal across a reading pass;
            // index.sqlite and its -wal are still compared in full.
            if relative != "index.sqlite-shm" {
                out.push((relative, std::fs::read(&path).expect("file readable")));
            }
        }
    }
}

#[test]
fn a_dry_run_changes_no_file_and_no_row_and_logs_one_would_delete_line_per_expiring_piece() {
    if std::env::var(CHILD_ENV).unwrap_or_default() == DRY_RUN_TOKEN {
        let root = fresh_root("dry-run-child");
        let mut index = open_index(&root);
        for seq in 1..=3 {
            seed_piece(&root, &mut index, seq, NOW - 8 * DAY, NOW - 8 * DAY + 1000);
        }
        seed_piece(&root, &mut index, 4, NOW - DAY, NOW);

        let mut files_before = Vec::new();
        tree_snapshot(&root, &root, &mut files_before);
        files_before.sort();
        let rows_before = row_set(&index);

        let report = expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 7)]), false)
            .expect("dry pass runs");
        assert_eq!(report.would_delete, 3);
        assert_eq!(report.deleted, 0);

        let mut files_after = Vec::new();
        tree_snapshot(&root, &root, &mut files_after);
        files_after.sort();
        assert_eq!(
            files_before, files_after,
            "every file and byte is identical"
        );
        assert_eq!(row_set(&index), rows_before, "every row survives");
        assert!(
            !root.join("deletions.log.jsonl").exists(),
            "dry run writes no deletion log"
        );
        std::fs::remove_dir_all(&root).expect("temp root removed");
        return;
    }
    let output = spawn_child(DRY_RUN_TOKEN, DRY_RUN_TEST);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "the child run failed:\n{stderr}");
    assert_eq!(
        stderr.matches("event=expiry-would-delete").count(),
        3,
        "one would-delete line per expiring piece\n{stderr}"
    );
}

#[test]
fn a_crash_after_the_stage_rename_is_repaired_by_startup_scan() {
    let root = fresh_root("crash-rename");
    let start = NOW - 40 * DAY;
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 7, start, start + 1000);
    seed_piece(&root, &mut index, 20, NOW - DAY, NOW);

    expiry::run_pass_crash_after(&root, &mut index, NOW, &retention(&[("front", 7)]), true, 1)
        .expect("the staged steps run up to the simulated stop");
    assert!(
        !file_present(&root, start, 7),
        "the file moved to the trash"
    );
    assert!(
        root.join(".trash")
            .join(format!("front-7-{start}.mp4"))
            .exists(),
        "the staged file waits in the trash"
    );
    assert_eq!(
        row_set(&index),
        vec![("front".to_owned(), 7), ("front".to_owned(), 20)]
    );

    let report = recovery::scan(&root, &mut index).expect("startup scan runs");
    assert_eq!(
        report,
        ScanReport {
            trash_removed: 1,
            rows_dropped: 1,
            ..ScanReport::default()
        }
    );
    assert_eq!(
        row_set(&index),
        vec![("front".to_owned(), 20)],
        "no row points at the moved file"
    );
    assert!(
        trash_files(&root).is_empty(),
        "the swept trash holds nothing"
    );
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_crash_after_the_row_delete_is_repaired_by_startup_scan() {
    let root = fresh_root("crash-rowdelete");
    let start = NOW - 40 * DAY;
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 7, start, start + 1000);
    seed_piece(&root, &mut index, 20, NOW - DAY, NOW);

    expiry::run_pass_crash_after(&root, &mut index, NOW, &retention(&[("front", 7)]), true, 2)
        .expect("the staged steps run up to the simulated stop");
    assert_eq!(
        row_set(&index),
        vec![("front".to_owned(), 20)],
        "the row delete committed"
    );
    assert!(
        root.join(".trash")
            .join(format!("front-7-{start}.mp4"))
            .exists(),
        "the footage is intact in the trash until its commit"
    );

    let report = recovery::scan(&root, &mut index).expect("startup scan runs");
    assert_eq!(
        report,
        ScanReport {
            trash_removed: 1,
            ..ScanReport::default()
        }
    );
    assert_eq!(row_set(&index), vec![("front".to_owned(), 20)]);
    assert!(trash_files(&root).is_empty());
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn nine_of_ten_expire_and_delete_while_ten_of_ten_refuses() {
    let root = fresh_root("nine-of-ten");
    let mut index = open_index(&root);
    for seq in 1..=9 {
        let age_days = 8 + i64::try_from(seq).expect("test sequence fits i64");
        let start = NOW - age_days * DAY;
        seed_piece(&root, &mut index, seq, start, start + 1000);
    }
    seed_piece(&root, &mut index, 10, NOW - DAY, NOW);

    let report = expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 7)]), true)
        .expect("pass runs");
    assert_eq!(report.refused_cameras, 0, "exactly 90 percent is allowed");
    assert_eq!(report.deleted, 9);
    assert_eq!(row_set(&index), vec![("front".to_owned(), 10)]);
    assert!(trash_files(&root).is_empty());
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_piece_with_an_already_missing_file_still_leaves_the_index() {
    let root = fresh_root("missing-file");
    let start = NOW - 40 * DAY;
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 1, start, start + 1000);
    seed_piece(&root, &mut index, 2, NOW - DAY, NOW);
    std::fs::remove_file(root.join(piece_relative_path(start, 1)))
        .expect("the seeded file is gone before the pass");

    let report = expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 7)]), true)
        .expect("pass runs");
    assert_eq!(report.deleted, 1);
    assert_eq!(report.step_failures, [0; 4]);
    assert_eq!(
        row_set(&index),
        vec![("front".to_owned(), 2)],
        "the row leaves even without a file"
    );
    assert!(!root.join(".trash").exists(), "no trash was needed");
    assert_eq!(deletion_lines(&root).len(), 1);
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn the_enforce_deletion_log_line_parses_with_the_documented_fields() {
    let root = fresh_root("log-json");
    let start = NOW - 8 * DAY;
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 1, start, start + 1000);
    seed_piece(&root, &mut index, 2, NOW - DAY, NOW);

    expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 7)]), true).expect("pass runs");
    let lines = deletion_lines(&root);
    assert_eq!(lines.len(), 1);
    let value: serde_json::Value = serde_json::from_str(&lines[0]).expect("the line parses");
    assert_eq!(value["camera"], "front");
    assert_eq!(value["seq"], 1);
    assert_eq!(value["path"], piece_relative_path(start, 1));
    assert_eq!(value["start_ms"], start);
    assert_eq!(value["end_ms"], start + 1000);
    assert_eq!(value["age_days"], 7);
    assert_eq!(value["reason"], "expired");
    assert_eq!(value["size_bytes"], 13);
    assert_eq!(value["mode"], "enforce");
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn the_writer_thread_runs_the_expiry_pass_when_the_clock_passes_the_hour() {
    let root = fresh_root("writer-hourly");
    let clock_start = 10 * DAY;
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 1, DAY, 2 * DAY);
    seed_piece(&root, &mut index, 2, 2 * DAY, 3 * DAY - 1);
    seed_piece(&root, &mut index, 3, 9 * DAY, 9 * DAY + 1000);
    let clock = Arc::new(AtomicI64::new(clock_start));
    let clock_for_wiring = Arc::clone(&clock);
    let wiring = corvette_media_bridge::index::ExpiryWiring {
        root: root.clone(),
        retention_days: retention(&[("front", 7)]),
        enforce: true,
        clock: Box::new(move || clock_for_wiring.load(Ordering::SeqCst)),
        recording_cameras: 1,
        free_space: Box::new(|_root| Ok(u64::MAX)),
    };
    // capacity 0: the first send returns only once the writer thread has
    // started, so the clock advance below always follows the thread's own
    // first clock read.
    let (sender, writer) = corvette_media_bridge::index::spawn_writer_with_expiry(index, 0, wiring);

    sender
        .send(corvette_media_bridge::index::IndexWrite::Piece(PieceRow {
            camera: "front".to_owned(),
            seq: 4,
            start_ms: clock_start,
            end_ms: clock_start + 1000,
            duration_s: 1.0,
            path: "recordings/front/not-a-file-yet.mp4".to_owned(),
            size_bytes: 0,
            codec: "h264".to_owned(),
        }))
        .expect("the first wake row is accepted");
    clock.store(
        clock_start + 3 * corvette_media_bridge::index::EXPIRY_PASS_INTERVAL_MS,
        Ordering::SeqCst,
    );
    sender
        .send(corvette_media_bridge::index::IndexWrite::Piece(PieceRow {
            camera: "front".to_owned(),
            seq: 5,
            start_ms: clock_start + DAY,
            end_ms: clock_start + DAY + 1000,
            duration_s: 1.0,
            path: "recordings/front/not-a-file-either.mp4".to_owned(),
            size_bytes: 0,
            codec: "h264".to_owned(),
        }))
        .expect("the second wake row is accepted");

    for _ in 0..400 {
        if !file_present(&root, DAY, 1) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    drop(sender);
    let index = writer.join().expect("the writer thread joins");
    assert!(
        !file_present(&root, DAY, 1) && !file_present(&root, 2 * DAY, 2),
        "the hourly pass removed the expired pieces on the writer thread"
    );
    assert!(
        file_present(&root, 9 * DAY, 3),
        "a piece inside the window survives"
    );
    assert_eq!(
        row_set(&index),
        vec![
            ("front".to_owned(), 3),
            ("front".to_owned(), 4),
            ("front".to_owned(), 5),
        ],
        "the pass dropped the expired rows and the woken inserts survived"
    );
    assert_eq!(deletion_lines(&root).len(), 2);
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

/// Inserts an aged front-camera row whose stored path is the given string; no file is created.
fn seed_row_at(index: &mut Index, seq: u64, path: &str) {
    let end_ms = NOW - 400 * DAY;
    index
        .insert(&PieceRow {
            camera: "front".to_owned(),
            seq,
            start_ms: end_ms - 1000,
            end_ms,
            duration_s: 1.0,
            path: path.to_owned(),
            size_bytes: 0,
            codec: "h264".to_owned(),
        })
        .expect("row inserted");
}

#[test]
fn a_row_path_naming_the_database_is_refused_and_the_database_survives() {
    let root = fresh_root("refused-database");
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 2, NOW - 8 * DAY, NOW - 8 * DAY + 1000);
    seed_piece(&root, &mut index, 3, NOW - DAY, NOW);
    seed_row_at(&mut index, 1, "index.sqlite");
    let db = root.join("index.sqlite");
    let before = std::fs::read(&db).expect("database readable");
    let report = expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 7)]), true)
        .expect("pass runs");
    assert_eq!(report.deleted, 1, "the expired piece is deleted");
    assert_eq!(report.refused_cameras, 0);
    assert_eq!(report.step_failures, [1, 0, 0, 0], "step one refuses");
    assert!(!file_present(&root, NOW - 8 * DAY, 2));
    assert_eq!(std::fs::read(&db).expect("db readable again"), before);
    let lines = deletion_lines(&root);
    assert_eq!(lines.len(), 1, "one log line");
    assert!(lines[0].contains("\"seq\":2"), "never logged");
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_row_path_naming_the_deletion_log_is_refused_and_the_log_keeps_its_earlier_line() {
    let root = fresh_root("refused-log");
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 2, NOW - 8 * DAY, NOW - 8 * DAY + 1000);
    seed_piece(&root, &mut index, 3, NOW - DAY, NOW);
    seed_row_at(&mut index, 1, "deletions.log.jsonl");
    std::fs::write(root.join("deletions.log.jsonl"), "{\"earlier\":\"line\"}\n")
        .expect("log seeded");
    let report = expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 7)]), true)
        .expect("pass runs");
    assert_eq!(report.deleted, 1, "the expired piece is deleted");
    assert_eq!(report.refused_cameras, 0);
    assert_eq!(report.step_failures, [1, 0, 0, 0], "step one refuses");
    assert!(!file_present(&root, NOW - 8 * DAY, 2));
    let lines = deletion_lines(&root);
    assert_eq!(lines.len(), 2, "the earlier line plus the deleted piece");
    assert_eq!(lines[0], "{\"earlier\":\"line\"}", "earlier bytes intact");
    assert!(lines[1].contains("\"seq\":2"), "never logged");
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_row_path_escaping_through_parent_directories_is_refused_and_the_database_survives() {
    let root = fresh_root("refused-escape");
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 2, NOW - 8 * DAY, NOW - 8 * DAY + 1000);
    seed_piece(&root, &mut index, 3, NOW - DAY, NOW);
    seed_row_at(&mut index, 1, "recordings/cam/../../index.sqlite");
    let db = root.join("index.sqlite");
    let before = std::fs::read(&db).expect("database readable");
    let report = expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 7)]), true)
        .expect("pass runs");
    assert_eq!(report.deleted, 1, "the expired piece is deleted");
    assert_eq!(report.refused_cameras, 0);
    assert_eq!(report.step_failures, [1, 0, 0, 0], "step one refuses");
    assert!(!file_present(&root, NOW - 8 * DAY, 2));
    assert_eq!(std::fs::read(&db).expect("db readable again"), before);
    let lines = deletion_lines(&root);
    assert_eq!(lines.len(), 1, "one log line");
    assert!(lines[0].contains("\"seq\":2"), "never logged");
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_row_path_that_is_absolute_is_refused_and_its_target_file_survives() {
    let root = fresh_root("refused-absolute");
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 2, NOW - 8 * DAY, NOW - 8 * DAY + 1000);
    seed_piece(&root, &mut index, 3, NOW - DAY, NOW);
    let target = root.join("sibling.mp4");
    std::fs::write(&target, "sibling bytes").expect("target written");
    seed_row_at(&mut index, 1, &target.to_string_lossy());
    let before = std::fs::read(&target).expect("target readable");
    let report = expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 7)]), true)
        .expect("pass runs");
    assert_eq!(report.deleted, 1, "the expired piece is deleted");
    assert_eq!(report.refused_cameras, 0);
    assert_eq!(report.step_failures, [1, 0, 0, 0], "step one refuses");
    assert!(!file_present(&root, NOW - 8 * DAY, 2));
    assert_eq!(std::fs::read(&target).expect("target again"), before);
    let lines = deletion_lines(&root);
    assert_eq!(lines.len(), 1, "one log line");
    assert!(lines[0].contains("\"seq\":2"), "never logged");
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_row_path_that_is_just_a_dot_is_refused_and_the_recordings_tree_survives() {
    let root = fresh_root("refused-dot");
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 2, NOW - 8 * DAY, NOW - 8 * DAY + 1000);
    seed_piece(&root, &mut index, 3, NOW - DAY, NOW);
    // The guard refuses a lone dot (and the empty path) as a non-layout path, so no rename runs.
    seed_row_at(&mut index, 1, ".");
    let report = expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 7)]), true)
        .expect("pass runs");
    assert_eq!(report.deleted, 1, "the expired piece is deleted");
    assert_eq!(report.refused_cameras, 0);
    assert_eq!(report.step_failures, [1, 0, 0, 0], "step one refuses");
    assert!(!file_present(&root, NOW - 8 * DAY, 2));
    assert!(
        file_present(&root, NOW - DAY, 3),
        "the fresh piece survives"
    );
    assert!(root.join("recordings").is_dir(), "the tree was not staged");
    let lines = deletion_lines(&root);
    assert_eq!(lines.len(), 1, "one log line");
    assert!(lines[0].contains("\"seq\":2"), "never logged");
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_row_with_an_empty_stored_path_is_refused_and_the_pass_deletes_the_real_piece() {
    let root = fresh_root("refused-empty");
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 2, NOW - 8 * DAY, NOW - 8 * DAY + 1000);
    seed_piece(&root, &mut index, 3, NOW - DAY, NOW);
    seed_row_at(&mut index, 1, "");
    let report = expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 7)]), true)
        .expect("pass runs");
    assert_eq!(report.deleted, 1, "the expired piece is deleted");
    assert_eq!(report.refused_cameras, 0);
    assert_eq!(report.step_failures, [1, 0, 0, 0], "step one refuses");
    assert!(!file_present(&root, NOW - 8 * DAY, 2));
    let lines = deletion_lines(&root);
    assert_eq!(lines.len(), 1, "one log line");
    assert!(lines[0].contains("\"seq\":2"), "never logged");
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_row_path_naming_an_existing_directory_is_refused_and_every_file_below_it_survives() {
    for stored in [
        "recordings",
        "recordings/",
        "recordings/victim",
        "recordings/victim/2026-01-01",
        "recordings/victim/2026-01-01/00",
        "recordings/victim/2026-01-01/00/staydir",
        "recordings/victim/2026-01-01/00/staydir/deep.mp4",
    ] {
        let root = fresh_root(&format!("refused-dir-{}", stored.replace('/', "-")));
        let mut index = open_index(&root);
        seed_piece(&root, &mut index, 2, NOW - 8 * DAY, NOW - 8 * DAY + 1000);
        seed_piece(&root, &mut index, 3, NOW - DAY, NOW);
        for victim in [
            "recordings/victim/2026-01-01/00/stay.mp4",
            "recordings/victim/2026-01-01/00/keep.mp4",
        ] {
            let path = root.join(victim);
            std::fs::create_dir_all(path.parent().expect("victim path names a directory"))
                .expect("victim directory created");
            std::fs::write(&path, format!("victim bytes in {victim}"))
                .expect("victim file written");
        }
        let hostile = root.join(stored);
        std::fs::create_dir_all(&hostile).expect("hostile directory created");
        std::fs::write(hostile.join("below.bin"), format!("file below {stored}"))
            .expect("file below hostile path written");
        seed_row_at(&mut index, 1, stored);
        assert!(
            root.join(stored).is_dir(),
            "the stored directory exists ({stored})"
        );
        let mut before = Vec::new();
        tree_snapshot(&root.join("recordings"), &root, &mut before);
        before.sort();
        let report = expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 7)]), true)
            .expect("pass runs");
        assert_eq!(report.deleted, 1, "the expired piece is deleted ({stored})");
        assert_eq!(report.refused_cameras, 0);
        assert_eq!(
            report.step_failures,
            [1, 0, 0, 0],
            "step one refuses ({stored})"
        );
        assert!(
            !file_present(&root, NOW - 8 * DAY, 2),
            "the expired piece is gone ({stored})"
        );
        assert!(
            file_present(&root, NOW - DAY, 3),
            "the fresh piece survives ({stored})"
        );
        let gone = piece_relative_path(NOW - 8 * DAY, 2);
        before.retain(|(relative, _)| *relative != gone);
        let mut after = Vec::new();
        tree_snapshot(&root.join("recordings"), &root, &mut after);
        after.sort();
        assert_eq!(
            before, after,
            "every surviving file is byte-identical ({stored})"
        );
        let lines = deletion_lines(&root);
        assert_eq!(lines.len(), 1, "one log line ({stored})");
        assert!(lines[0].contains("\"seq\":2"), "never logged ({stored})");
        std::fs::remove_dir_all(&root).expect("temp root removed");
    }
}

#[test]
fn each_deletion_log_line_ends_with_exactly_one_newline() {
    let root = fresh_root("log-newline");
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 1, NOW - 8 * DAY, NOW - 8 * DAY + 1000);
    seed_piece(
        &root,
        &mut index,
        2,
        NOW - 8 * DAY + 2000,
        NOW - 8 * DAY + 3000,
    );
    seed_piece(&root, &mut index, 3, NOW - DAY, NOW);
    expiry::run_pass(&root, &mut index, NOW, &retention(&[("front", 7)]), true).expect("pass runs");
    let text = std::fs::read_to_string(root.join("deletions.log.jsonl")).expect("log readable");
    assert!(text.ends_with('\n'), "the file ends in a newline");
    assert!(!text.ends_with("\n\n"), "one newline ends the file");
    assert!(!text.contains("\n\n"), "no blank line between entries");
    assert_eq!(text.lines().count(), 2, "one line per deleted piece");
    std::fs::remove_dir_all(&root).expect("temp root removed");
}
