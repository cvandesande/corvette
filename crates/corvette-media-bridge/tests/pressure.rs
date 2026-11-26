use corvette_media_bridge::expiry::PassReport;
use corvette_media_bridge::index::{Index, PieceRow};
use corvette_media_bridge::pressure::{self, PressureReason};
use corvette_media_bridge::utc::utc_parts;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const DAY: i64 = 86_400_000;
const HOUR: i64 = 3_600_000;
const NOW: i64 = 1_790_000_000_000;

fn fresh_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("corvette-pressure-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("test root created");
    dir
}

fn open_index(root: &Path) -> Index {
    Index::open(
        &root.join("index.sqlite"),
        &[
            ("front".to_owned(), 4),
            ("back".to_owned(), 4),
            ("keep".to_owned(), 4),
        ],
    )
    .expect("index opens")
}

fn retention(entries: &[(&str, u32)]) -> HashMap<String, u32> {
    entries
        .iter()
        .map(|(camera, days)| ((*camera).to_owned(), *days))
        .collect()
}

fn empty_held() -> std::collections::HashSet<(String, u64)> {
    std::collections::HashSet::new()
}

fn event_days(entries: &[(&str, u32)]) -> HashMap<String, u32> {
    entries
        .iter()
        .map(|(camera, days)| ((*camera).to_owned(), *days))
        .collect()
}

fn piece_relative_path(camera: &str, start_ms: i64, seq: u64) -> String {
    let parts = utc_parts(start_ms);
    format!(
        "recordings/{camera}/{:04}-{:02}-{:02}/{:02}/{start_ms}-{seq}.mp4",
        parts.year, parts.month, parts.day, parts.hour
    )
}

fn row(
    camera: &str,
    seq: u64,
    start_ms: i64,
    end_ms: i64,
    size_bytes: u64,
    path: String,
) -> PieceRow {
    PieceRow {
        camera: camera.to_owned(),
        seq,
        start_ms,
        end_ms,
        duration_s: 1.0,
        path,
        size_bytes,
        codec: "h264".to_owned(),
    }
}

/// Writes a real file and inserts its row; `size_bytes` sizes the index figure the planner sees.
fn seed_piece(
    root: &Path,
    index: &mut Index,
    camera: &str,
    seq: u64,
    start_ms: i64,
    end_ms: i64,
    size_bytes: u64,
) {
    let relative = piece_relative_path(camera, start_ms, seq);
    let full = root.join(&relative);
    std::fs::create_dir_all(full.parent().expect("the path names a directory"))
        .expect("piece directory created");
    std::fs::write(
        &full,
        vec![b'x'; usize::try_from(size_bytes).expect("test sizes fit")],
    )
    .expect("piece file written");
    index
        .insert(&row(camera, seq, start_ms, end_ms, size_bytes, relative))
        .expect("row inserted");
}

fn seed_row(index: &mut Index, camera: &str, seq: u64, end_ms: i64, size_bytes: u64, path: &str) {
    index
        .insert(&row(
            camera,
            seq,
            end_ms - 1000,
            end_ms,
            size_bytes,
            path.to_owned(),
        ))
        .expect("row inserted");
}

fn plan_row(camera: &str, seq: u64, start_ms: i64, end_ms: i64, size_bytes: u64) -> PieceRow {
    row(
        camera,
        seq,
        start_ms,
        end_ms,
        size_bytes,
        format!("recordings/{camera}/2026-11-21/00/{start_ms}-{seq}.mp4"),
    )
}

fn file_present(root: &Path, camera: &str, start_ms: i64, seq: u64) -> bool {
    root.join(piece_relative_path(camera, start_ms, seq))
        .exists()
}

fn row_set(index: &Index) -> Vec<(String, u64)> {
    index
        .all_pieces()
        .expect("rows read")
        .iter()
        .map(|piece| (piece.camera.clone(), piece.seq))
        .collect()
}

fn deletion_lines(root: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(root.join("deletions.log.jsonl")).expect("log readable");
    text.lines().map(str::to_owned).collect()
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
            // SQLite rewrites the WAL shared-memory mirror on any read, so it is
            // excluded; index.sqlite and -wal are still compared in full.
            if relative != "index.sqlite-shm" {
                out.push((relative, std::fs::read(&path).expect("file readable")));
            }
        }
    }
}

fn seq_reasons(plan: &pressure::PressurePlan) -> Vec<(String, u64, PressureReason)> {
    plan.deletions
        .iter()
        .map(|deletion| {
            (
                deletion.row.camera.clone(),
                deletion.row.seq,
                deletion.reason,
            )
        })
        .collect()
}

const CHILD_ENV: &str = "CORVETTE_PRESSURE_CHILD_RUN";
const ALERT_TOKEN: &str = "alert-child";
const DRY_RUN_TOKEN: &str = "dry-run-child";
const ALERT_TEST: &str =
    "below_the_hard_floor_within_retention_pieces_go_oldest_first_and_each_camera_gets_one_alert";
const DRY_RUN_TEST: &str =
    "a_dry_run_changes_no_file_and_no_row_and_logs_one_would_delete_line_per_planned_piece";

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

// The below-hard-floor scenario, shared by the alert and dry-run tests: free 0, one expired
// piece and five within-retention pieces of 2000 bytes each, floors measured from the two
// last-hour pieces at 20000 bytes, so the hard floor is reached after five planned pieces.
fn seed_below_hard_floor(root: &Path, index: &mut Index) {
    seed_piece(
        root,
        index,
        "front",
        1,
        NOW - 10 * DAY - 1000,
        NOW - 10 * DAY,
        1000,
    );
    seed_piece(
        root,
        index,
        "front",
        2,
        NOW - 5 * DAY,
        NOW - 5 * DAY + 1000,
        2000,
    );
    seed_piece(
        root,
        index,
        "front",
        3,
        NOW - 4 * DAY,
        NOW - 4 * DAY + 1000,
        2000,
    );
    seed_piece(
        root,
        index,
        "front",
        4,
        NOW - 3 * DAY,
        NOW - 3 * DAY + 1000,
        2000,
    );
    seed_piece(
        root,
        index,
        "front",
        5,
        NOW - 2 * DAY - 1000,
        NOW - 2 * DAY,
        2000,
    );
    seed_piece(root, index, "front", 6, NOW - 2000, NOW - 1000, 20000);
    seed_piece(
        root,
        index,
        "back",
        1,
        NOW - 6 * DAY - 1000,
        NOW - 6 * DAY,
        2000,
    );
    seed_piece(
        root,
        index,
        "back",
        2,
        NOW - 5 * DAY - DAY / 2,
        NOW - 5 * DAY - DAY / 2 + 1000,
        2000,
    );
    seed_piece(root, index, "back", 3, NOW - 1500, NOW - 500, 20000);
}

fn alert_facts(plan: &pressure::PressurePlan) -> Vec<(String, i64, u32, usize)> {
    plan.alerts
        .iter()
        .map(|alert| {
            (
                alert.camera.clone(),
                alert.days_held,
                alert.configured_days,
                alert.deleted,
            )
        })
        .collect()
}

#[test]
fn below_the_hard_floor_within_retention_pieces_go_oldest_first_and_each_camera_gets_one_alert() {
    if std::env::var(CHILD_ENV).unwrap_or_default() == ALERT_TOKEN {
        let root = fresh_root("alert-child");
        let mut index = open_index(&root);
        seed_below_hard_floor(&root, &mut index);
        let plan = pressure::plan_pressure(
            &index.all_pieces().expect("rows read"),
            NOW,
            &retention(&[("front", 7), ("back", 7)]),
            &empty_held(),
            3,
            0,
        );
        assert_eq!(plan.floor_bytes, 40000);
        assert_eq!(plan.hard_floor_bytes, 10000);
        assert_eq!(
            seq_reasons(&plan),
            vec![
                ("front".to_owned(), 1, PressureReason::Expired),
                ("back".to_owned(), 1, PressureReason::WithinRetention),
                ("back".to_owned(), 2, PressureReason::WithinRetention),
                ("front".to_owned(), 2, PressureReason::WithinRetention),
                ("front".to_owned(), 3, PressureReason::WithinRetention),
                ("front".to_owned(), 4, PressureReason::WithinRetention),
            ]
        );
        assert_eq!(
            alert_facts(&plan),
            vec![("back".to_owned(), 0, 7, 2), ("front".to_owned(), 2, 7, 3),]
        );
        let report = pressure::run_pressure_pass(
            &root,
            &mut index,
            NOW,
            &retention(&[("front", 7), ("back", 7)]),
            &event_days(&[]),
            pressure::PressureBudget {
                recording_cameras: 3,
                free_bytes: 0,
            },
            true,
        )
        .expect("pass runs");
        assert_eq!(report.deleted, 6);
        assert_eq!(report.step_failures, [0; 4]);
        assert!(
            file_present(&root, "front", NOW - 2 * DAY - 1000, 5)
                && file_present(&root, "front", NOW - 2000, 6)
                && file_present(&root, "back", NOW - 1500, 3),
            "newest-hour pieces and the beyond-hard-floor piece survive"
        );
        std::fs::remove_dir_all(&root).expect("temp root removed");
        return;
    }
    let output = spawn_child(ALERT_TOKEN, ALERT_TEST);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "the child run failed:\n{stderr}");
    assert_eq!(
        stderr.matches("event=pressure-alert").count(),
        2,
        "one alert line per affected camera\n{stderr}"
    );
    assert!(
        stderr.contains(
            "unit=\"front\" event=pressure-alert detail=\"days_held=2 configured_days=7 deleted=3\""
        ),
        "front alert names days held, configured days and count\n{stderr}"
    );
    assert!(
        stderr.contains(
            "unit=\"back\" event=pressure-alert detail=\"days_held=0 configured_days=7 deleted=2\""
        ),
        "back alert names days held, configured days and count\n{stderr}"
    );
    assert_eq!(stderr.matches("event=pressure-step-failed").count(), 0);
}

#[test]
fn a_dry_run_changes_no_file_and_no_row_and_logs_one_would_delete_line_per_planned_piece() {
    if std::env::var(CHILD_ENV).unwrap_or_default() == DRY_RUN_TOKEN {
        let root = fresh_root("dry-run-child");
        let mut index = open_index(&root);
        seed_below_hard_floor(&root, &mut index);
        let mut files_before = Vec::new();
        tree_snapshot(&root, &root, &mut files_before);
        files_before.sort();
        let rows_before = row_set(&index);
        let report = pressure::run_pressure_pass(
            &root,
            &mut index,
            NOW,
            &retention(&[("front", 7), ("back", 7)]),
            &event_days(&[]),
            pressure::PressureBudget {
                recording_cameras: 3,
                free_bytes: 0,
            },
            false,
        )
        .expect("dry pass runs");
        assert_eq!(report.would_delete, 6);
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
        stderr.matches("event=pressure-would-delete").count(),
        6,
        "one would-delete line per planned piece\n{stderr}"
    );
}

#[test]
fn free_at_or_above_the_floor_plans_and_deletes_nothing() {
    let root = fresh_root("above-floor");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        "front",
        1,
        NOW - 10 * DAY - 1000,
        NOW - 10 * DAY,
        5000,
    );
    seed_piece(&root, &mut index, "front", 2, NOW - 2000, NOW - 1000, 5000);
    let rows = index.all_pieces().expect("rows read");
    let plan = pressure::plan_pressure(
        &rows,
        NOW,
        &retention(&[("front", 7)]),
        &empty_held(),
        3,
        5000,
    );
    assert!(
        plan.deletions.is_empty(),
        "free at the floor is not below it"
    );
    assert!(plan.alerts.is_empty());
    let report = pressure::run_pressure_pass(
        &root,
        &mut index,
        NOW,
        &retention(&[("front", 7)]),
        &event_days(&[]),
        pressure::PressureBudget {
            recording_cameras: 3,
            free_bytes: 5000,
        },
        true,
    )
    .expect("pass runs");
    assert_eq!(
        report,
        PassReport {
            enforce: true,
            would_delete: 0,
            held: 0,
            deleted: 0,
            refused_cameras: 0,
            step_failures: [0; 4],
        }
    );
    assert!(file_present(&root, "front", NOW - 10 * DAY - 1000, 1));
    assert!(!root.join("deletions.log.jsonl").exists());
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn below_the_floor_only_the_oldest_expired_pieces_the_floor_needs_are_deleted() {
    let root = fresh_root("expired-oldest-first");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        "front",
        1,
        NOW - 10 * DAY - 1000,
        NOW - 10 * DAY,
        5000,
    );
    seed_piece(
        &root,
        &mut index,
        "front",
        2,
        NOW - 9 * DAY - 1000,
        NOW - 9 * DAY,
        5000,
    );
    seed_piece(
        &root,
        &mut index,
        "front",
        3,
        NOW - DAY,
        NOW - DAY + 1000,
        5000,
    );
    seed_piece(&root, &mut index, "front", 4, NOW - 2000, NOW - 1000, 5000);
    let rows = index.all_pieces().expect("rows read");
    let plan =
        pressure::plan_pressure(&rows, NOW, &retention(&[("front", 7)]), &empty_held(), 3, 0);
    assert_eq!(
        seq_reasons(&plan),
        vec![("front".to_owned(), 1, PressureReason::Expired)],
        "the oldest expired piece alone reaches the floor"
    );
    let report = pressure::run_pressure_pass(
        &root,
        &mut index,
        NOW,
        &retention(&[("front", 7)]),
        &event_days(&[]),
        pressure::PressureBudget {
            recording_cameras: 3,
            free_bytes: 0,
        },
        true,
    )
    .expect("pass runs");
    assert_eq!(report.deleted, 1);
    assert!(
        !file_present(&root, "front", NOW - 10 * DAY - 1000, 1),
        "the oldest expired piece is gone"
    );
    assert!(
        file_present(&root, "front", NOW - 9 * DAY - 1000, 2),
        "a newer expired piece the estimate does not need survives"
    );
    assert!(
        file_present(&root, "front", NOW - DAY, 3) && file_present(&root, "front", NOW - 2000, 4),
        "nothing within retention was touched"
    );
    assert_eq!(row_set(&index).len(), 3);
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn the_newest_hour_of_every_camera_survives_when_the_estimate_cannot_reach_the_hard_floor() {
    let root = fresh_root("newest-hour");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        "front",
        1,
        NOW - 5 * DAY,
        NOW - 5 * DAY + 1000,
        1000,
    );
    seed_piece(
        &root,
        &mut index,
        "front",
        2,
        NOW - 4 * DAY,
        NOW - 4 * DAY + 1000,
        1000,
    );
    seed_piece(
        &root,
        &mut index,
        "front",
        3,
        NOW - 3 * DAY,
        NOW - 3 * DAY + 1000,
        1000,
    );
    seed_piece(&root, &mut index, "front", 4, NOW - 2000, NOW - 1000, 40000);
    seed_piece(&root, &mut index, "back", 1, NOW - 1200, NOW - 600, 5000);
    seed_piece(&root, &mut index, "back", 2, NOW - 7200, NOW - 6600, 5000);
    let rows = index.all_pieces().expect("rows read");
    let plan = pressure::plan_pressure(
        &rows,
        NOW,
        &retention(&[("front", 7), ("back", 7)]),
        &empty_held(),
        3,
        0,
    );
    assert_eq!(
        plan.deletions.len(),
        3,
        "every eligible piece goes, then the pass stops"
    );
    assert!(
        plan.deletions
            .iter()
            .all(|deletion| deletion.row.camera == "front"
                && deletion.reason == PressureReason::WithinRetention),
        "only front holds pieces outside its newest hour"
    );
    assert!(
        !plan
            .deletions
            .iter()
            .any(|deletion| deletion.row.end_ms > NOW - HOUR),
        "no newest-hour piece is ever planned"
    );
    let report = pressure::run_pressure_pass(
        &root,
        &mut index,
        NOW,
        &retention(&[("front", 7), ("back", 7)]),
        &event_days(&[]),
        pressure::PressureBudget {
            recording_cameras: 3,
            free_bytes: 0,
        },
        true,
    )
    .expect("pass runs");
    assert_eq!(report.deleted, 3);
    assert_eq!(
        row_set(&index),
        vec![
            ("back".to_owned(), 1),
            ("back".to_owned(), 2),
            ("front".to_owned(), 4)
        ],
        "the pass stopped short at the newest hours"
    );
    assert!(file_present(&root, "front", NOW - 2000, 4));
    assert!(file_present(&root, "back", NOW - 1200, 1));
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn with_no_hour_of_history_the_floor_is_the_fallback_rate_per_recording_camera() {
    let plan = pressure::plan_pressure(&[], NOW, &HashMap::new(), &empty_held(), 3, 0);
    assert_eq!(plan.floor_bytes, 3 * 1_000_000 * 3600);
    assert_eq!(plan.hard_floor_bytes, 3 * 1_000_000 * 3600 / 4);
    assert!(plan.deletions.is_empty());
    let rows = vec![
        plan_row("front", 1, NOW - HOUR + 1, NOW - HOUR + 1001, 7),
        plan_row("front", 2, NOW - 2000, NOW - 1000, 7),
    ];
    let soon =
        pressure::plan_pressure(&rows, NOW, &retention(&[("front", 7)]), &empty_held(), 1, 0);
    assert_eq!(
        soon.floor_bytes,
        1_000_000 * 3600,
        "under an hour of history"
    );
    let exact = pressure::plan_pressure(
        &[
            plan_row("front", 1, NOW - HOUR - 1000, NOW - HOUR, 7),
            plan_row("front", 2, NOW - 2000, NOW - 1000, 7),
        ],
        NOW,
        &retention(&[("front", 7)]),
        &empty_held(),
        1,
        0,
    );
    assert_eq!(
        exact.floor_bytes, 7,
        "exactly an hour of history measures, and only the last-hour piece counts"
    );
}

#[test]
fn with_an_hour_of_history_the_floor_is_the_measured_last_hour_sum() {
    let rows = vec![
        plan_row("front", 1, NOW - 2 * HOUR, NOW - 2 * HOUR + 1000, 999),
        plan_row("front", 2, NOW - DAY, NOW - DAY + 1000, 1234),
        plan_row("front", 3, NOW - 2000, NOW - 1000, 1234),
    ];
    let plan =
        pressure::plan_pressure(&rows, NOW, &retention(&[("front", 7)]), &empty_held(), 5, 0);
    assert_eq!(
        plan.floor_bytes, 1234,
        "only the piece ending inside the last hour measures"
    );
}

#[test]
fn both_pressure_kinds_write_reason_pressure_in_the_deletion_log() {
    let root = fresh_root("reason-log");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        "front",
        1,
        NOW - 10 * DAY - 1000,
        NOW - 10 * DAY,
        1000,
    );
    seed_piece(
        &root,
        &mut index,
        "front",
        2,
        NOW - 4 * DAY,
        NOW - 4 * DAY + 1000,
        2000,
    );
    seed_piece(&root, &mut index, "front", 3, NOW - 2000, NOW - 1000, 10000);
    let rows = index.all_pieces().expect("rows read");
    let plan =
        pressure::plan_pressure(&rows, NOW, &retention(&[("front", 7)]), &empty_held(), 1, 0);
    assert_eq!(
        seq_reasons(&plan),
        vec![
            ("front".to_owned(), 1, PressureReason::Expired),
            ("front".to_owned(), 2, PressureReason::WithinRetention),
        ]
    );
    pressure::run_pressure_pass(
        &root,
        &mut index,
        NOW,
        &retention(&[("front", 7)]),
        &event_days(&[]),
        pressure::PressureBudget {
            recording_cameras: 1,
            free_bytes: 0,
        },
        true,
    )
    .expect("pass runs");
    let lines = deletion_lines(&root);
    assert_eq!(lines.len(), 2);
    for line in &lines {
        let value: serde_json::Value = serde_json::from_str(line).expect("line parses");
        assert_eq!(value["reason"], "pressure", "both kinds log as pressure");
        assert_eq!(value["mode"], "enforce");
    }
    assert!(lines[0].contains("\"seq\":1") && lines[1].contains("\"seq\":2"));
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_pressure_pass_refuses_index_paths_outside_the_layout_and_the_targets_survive() {
    let root = fresh_root("hostile-pressure");
    let mut index = open_index(&root);
    let db = root.join("index.sqlite");
    seed_row(&mut index, "front", 1, NOW - 10 * DAY, 1000, "index.sqlite");
    seed_row(
        &mut index,
        "front",
        2,
        NOW - 10 * DAY,
        1000,
        "recordings/front/2026-11-21/00/staydir",
    );
    let hostile_dir = root.join("recordings/front/2026-11-21/00/staydir");
    std::fs::create_dir_all(&hostile_dir).expect("hostile directory created");
    std::fs::write(hostile_dir.join("below.bin"), "below bytes").expect("below file written");
    seed_piece(
        &root,
        &mut index,
        "front",
        3,
        NOW - 9 * DAY - 1000,
        NOW - 9 * DAY,
        1000,
    );
    seed_piece(&root, &mut index, "front", 4, NOW - 2000, NOW - 1000, 4000);
    let db_before = std::fs::read(&db).expect("database readable");
    let report = pressure::run_pressure_pass(
        &root,
        &mut index,
        NOW,
        &retention(&[("front", 7)]),
        &event_days(&[]),
        pressure::PressureBudget {
            recording_cameras: 1,
            free_bytes: 0,
        },
        true,
    )
    .expect("pass runs");
    assert_eq!(report.deleted, 1, "only the real piece leaves");
    assert_eq!(
        report.step_failures,
        [2, 0, 0, 0],
        "both hostile paths refuse"
    );
    assert_eq!(
        std::fs::read(&db).expect("database again"),
        db_before,
        "the database survives every pressure delete"
    );
    assert!(hostile_dir.is_dir(), "the non-mp4 directory survives");
    assert!(
        hostile_dir.join("below.bin").is_file(),
        "and its file below"
    );
    let lines = deletion_lines(&root);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("\"seq\":3") && lines[0].contains("\"reason\":\"pressure\""));
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn plan_pressure_holds_up_under_arithmetic_extremes() {
    let none = pressure::plan_pressure(&[], NOW, &HashMap::new(), &empty_held(), 0, 0);
    assert_eq!((none.floor_bytes, none.hard_floor_bytes), (0, 0));
    assert!(none.deletions.is_empty());

    let rows = vec![
        plan_row("front", 1, NOW - 2 * DAY, NOW - 2 * DAY + 1000, 5),
        plan_row("front", 2, NOW - 2000, NOW - 1000, 5),
    ];
    let max_free = pressure::plan_pressure(
        &rows,
        NOW,
        &retention(&[("front", 1)]),
        &empty_held(),
        3,
        u64::MAX,
    );
    assert!(
        max_free.deletions.is_empty(),
        "free space saturates above any floor"
    );

    let max_sizes = vec![
        plan_row("front", 1, NOW - 2 * DAY - 1000, NOW - 2 * DAY, u64::MAX),
        plan_row("front", 2, NOW - 2000, NOW - 1000, u64::MAX),
        plan_row("back", 1, NOW - 2000, NOW - 1000, u64::MAX),
    ];
    let saturated = pressure::plan_pressure(
        &max_sizes,
        NOW,
        &retention(&[("front", 1)]),
        &empty_held(),
        2,
        0,
    );
    assert_eq!(
        saturated.floor_bytes,
        u64::MAX,
        "the measured floor saturates"
    );
    assert_eq!(
        saturated.deletions.len(),
        1,
        "one saturating piece meets the floor"
    );

    let deep_past = vec![
        plan_row("front", 1, i64::MIN, i64::MIN + 100, 9),
        plan_row("front", 2, NOW - 2000, NOW - 1000, 9),
    ];
    let at_min = pressure::plan_pressure(
        &deep_past,
        i64::MIN,
        &retention(&[("front", 1)]),
        &empty_held(),
        0,
        18,
    );
    assert_eq!(
        at_min.floor_bytes, 18,
        "the clock floor saturates, nothing wraps"
    );
    assert!(
        at_min.deletions.is_empty(),
        "free space meets the saturating floor"
    );

    let at_max = pressure::plan_pressure(
        &rows,
        i64::MAX,
        &retention(&[("front", 1)]),
        &empty_held(),
        3,
        0,
    );
    assert_eq!(
        at_max.floor_bytes, 0,
        "no row ends within an hour of the clock ceiling"
    );
    assert!(at_max.deletions.is_empty());
}

#[test]
fn a_never_expiring_camera_is_never_an_expired_pressure_victim_and_its_newest_hour_is_untouched() {
    let root = fresh_root("never-expiring");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        "front",
        1,
        NOW - 10 * DAY - 1000,
        NOW - 10 * DAY,
        1000,
    );
    seed_piece(&root, &mut index, "front", 2, NOW - 2000, NOW - 1000, 8000);
    seed_piece(
        &root,
        &mut index,
        "keep",
        1,
        NOW - 20 * DAY - 1000,
        NOW - 20 * DAY,
        1000,
    );
    seed_piece(
        &root,
        &mut index,
        "keep",
        2,
        NOW - 4 * DAY,
        NOW - 4 * DAY + 1000,
        1000,
    );
    seed_piece(&root, &mut index, "keep", 3, NOW - 2000, NOW - 1000, 2000);
    let rows = index.all_pieces().expect("rows read");
    let plan = pressure::plan_pressure(
        &rows,
        NOW,
        &retention(&[("front", 7), ("keep", 0)]),
        &empty_held(),
        2,
        0,
    );
    assert_eq!(
        seq_reasons(&plan),
        vec![
            ("front".to_owned(), 1, PressureReason::Expired),
            ("keep".to_owned(), 1, PressureReason::WithinRetention),
            ("keep".to_owned(), 2, PressureReason::WithinRetention),
        ],
        "the never-expiring camera loses nothing to expiry and joins only after the other camera"
    );
    assert_eq!(
        alert_facts(&plan),
        vec![("keep".to_owned(), 0, 0, 2)],
        "only the never-expiring camera alerts, holding no whole day"
    );
    pressure::run_pressure_pass(
        &root,
        &mut index,
        NOW,
        &retention(&[("front", 7), ("keep", 0)]),
        &event_days(&[]),
        pressure::PressureBudget {
            recording_cameras: 2,
            free_bytes: 0,
        },
        true,
    )
    .expect("pass runs");
    assert!(
        file_present(&root, "keep", NOW - 2000, 3) && file_present(&root, "front", NOW - 2000, 2),
        "both cameras keep their newest hour"
    );
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_within_retention_piece_ending_exactly_at_the_newest_hour_cut_is_planned_and_one_millisecond_past_is_not()
 {
    let rows = vec![
        plan_row("front", 5, NOW - 10 * DAY - 1000, NOW - 10 * DAY, 100),
        plan_row("front", 8, NOW - HOUR - 1000, NOW - HOUR, 100),
        plan_row("front", 7, NOW - HOUR + 1 - 1000, NOW - HOUR + 1, 100),
        plan_row("front", 9, NOW - 1000, NOW, 4000),
        plan_row("front", 6, NOW - 500, NOW - 1, 4000),
    ];
    let plan = pressure::plan_pressure(
        &rows,
        NOW,
        &retention(&[("front", 30)]),
        &empty_held(),
        1,
        0,
    );
    assert_eq!(
        seq_reasons(&plan),
        vec![
            ("front".to_owned(), 5, PressureReason::WithinRetention),
            ("front".to_owned(), 8, PressureReason::WithinRetention),
        ],
        "the piece ending exactly at newest-end minus one hour is eligible and the one a millisecond past it is not"
    );
}
