use corvette_media_bridge::expiry;
use corvette_media_bridge::index::{Index, PieceRow};
use corvette_media_bridge::pressure;
use corvette_media_bridge::utc::utc_parts;
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const DAY: i64 = 86_400_000;
const HOUR: i64 = 3_600_000;
const NOW: i64 = 1_790_000_000_000;
const NOW_S: i64 = NOW / 1000;

fn maps(entries: &[(&str, u32)]) -> HashMap<String, u32> {
    entries
        .iter()
        .map(|(camera, days)| ((*camera).to_owned(), *days))
        .collect()
}

fn fresh_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("corvette-hold-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("test root created");
    dir
}

fn open_index(root: &Path) -> Index {
    Index::open(
        &root.join("index.sqlite"),
        &[("front".to_owned(), 4), ("back".to_owned(), 4)],
    )
    .expect("index opens")
}

fn piece_relative_path(camera: &str, start_ms: i64, seq: u64) -> String {
    let parts = utc_parts(start_ms);
    format!(
        "recordings/{camera}/{:04}-{:02}-{:02}/{:02}/{start_ms}-{seq}.mp4",
        parts.year, parts.month, parts.day, parts.hour
    )
}

fn seed_piece(
    root: &Path,
    index: &mut Index,
    camera: &str,
    seq: u64,
    start_ms: i64,
    end_ms: i64,
    size: usize,
) {
    let relative = piece_relative_path(camera, start_ms, seq);
    let full = root.join(&relative);
    std::fs::create_dir_all(full.parent().expect("piece path names a directory"))
        .expect("piece directory created");
    std::fs::write(&full, vec![b'p'; size]).expect("piece file written");
    index
        .insert(&PieceRow {
            camera: camera.to_owned(),
            seq,
            start_ms,
            end_ms,
            duration_s: f64::from(u32::try_from(end_ms - start_ms).expect("test spans fit")),
            path: relative,
            size_bytes: size as u64,
            codec: "h264".to_owned(),
        })
        .expect("row inserted");
}

fn db(root: &Path) -> Connection {
    Connection::open(root.join("index.sqlite")).expect("raw connection opens")
}

fn seed_review(
    root: &Path,
    camera: &str,
    start_ms: i64,
    end_ms: Option<i64>,
    last_activity_ms: i64,
) {
    db(root)
        .execute(
            "INSERT INTO reviews (camera, severity, start_ts, end_ts, last_activity_ts)
             VALUES (?1, 'alert', ?2, ?3, ?4)",
            params![camera, start_ms, end_ms, last_activity_ms],
        )
        .expect("review row inserted");
}

fn seed_event(root: &Path, track: &str, start_ms: i64, end_ms: Option<i64>, last_motion_ms: i64) {
    db(root)
        .execute(
            "INSERT INTO events (camera, track_id, label, start_ts, end_ts, last_motion_ts, first_motion, box_json, zones_json)
             VALUES ('front', ?1, 'person', ?2, ?3, ?4, NULL, '[]', '[]')",
            params![track, start_ms, end_ms, last_motion_ms],
        )
        .expect("event row inserted");
}

fn seed_motion(root: &Path, camera: &str, second: i64) {
    db(root)
        .execute(
            "INSERT INTO motion (camera, second) VALUES (?1, ?2)",
            params![camera, second],
        )
        .expect("motion row inserted");
}

fn surviving(root: &Path, column: &str, table: &str, camera: &str) -> Vec<i64> {
    let conn = db(root);
    let mut statement = conn
        .prepare(&format!(
            "SELECT {column} FROM {table} WHERE camera = ?1 ORDER BY rowid"
        ))
        .expect("select prepares");
    statement
        .query_map(params![camera], |row| row.get(0))
        .expect("query runs")
        .collect::<Result<Vec<i64>, _>>()
        .expect("ids read")
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
        .map(|row| (row.camera.clone(), row.seq))
        .collect()
}

fn tracked_snapshot(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir).expect("directory readable") {
        let path = entry.expect("entry readable").path();
        if path.is_dir() {
            tracked_snapshot(&path, root, out);
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("snapshot stays under the root")
                .display()
                .to_string();
            if relative != "index.sqlite-shm" {
                out.push((relative, std::fs::read(&path).expect("file readable")));
            }
        }
    }
}

const CHILD_ENV: &str = "CORVETTE_HOLD_CHILD_RUN";

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
fn a_piece_overlapping_a_young_review_survives_a_pass_that_would_otherwise_delete_it() {
    let root = fresh_root("young-review");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        "front",
        1,
        NOW - 8 * DAY - 1000,
        NOW - 8 * DAY,
        10,
    );
    seed_piece(&root, &mut index, "front", 2, NOW - HOUR, NOW, 10);
    seed_review(
        &root,
        "front",
        NOW - 8 * DAY - 5000,
        Some(NOW - 8 * DAY),
        NOW - 8 * DAY,
    );

    let report = expiry::run_pass(
        &root,
        &mut index,
        NOW,
        &maps(&[("front", 7)]),
        &maps(&[("front", 30)]),
        true,
    )
    .expect("pass runs");
    assert_eq!(report.held, 1, "the held piece counts");
    assert_eq!(report.would_delete, 0);
    assert_eq!(report.deleted, 0);
    assert!(file_present(&root, "front", NOW - 8 * DAY - 1000, 1));
    assert!(!root.join("deletions.log.jsonl").exists());
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_review_holds_at_exactly_the_age_limit_and_releases_one_millisecond_later() {
    let root = fresh_root("age-boundary");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        "front",
        1,
        NOW - 30 * DAY - 1000,
        NOW - 30 * DAY,
        10,
    );
    seed_piece(&root, &mut index, "front", 2, NOW - HOUR, NOW, 10);
    seed_review(
        &root,
        "front",
        NOW - 30 * DAY - 60_000,
        Some(NOW - 30 * DAY),
        NOW - 30 * DAY,
    );

    let first = expiry::run_pass(
        &root,
        &mut index,
        NOW,
        &maps(&[("front", 7)]),
        &maps(&[("front", 30)]),
        true,
    )
    .expect("pass at the limit runs");
    assert_eq!(first.held, 1, "an age of exactly 30 days is still held");
    assert_eq!(first.deleted, 0);
    assert!(file_present(&root, "front", NOW - 30 * DAY - 1000, 1));

    let second = expiry::run_pass(
        &root,
        &mut index,
        NOW + 1,
        &maps(&[("front", 7)]),
        &maps(&[("front", 30)]),
        true,
    )
    .expect("pass past the limit runs");
    assert_eq!(second.held, 0, "30 days plus a millisecond has passed");
    assert_eq!(second.deleted, 1);
    assert!(!file_present(&root, "front", NOW - 30 * DAY - 1000, 1));
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_piece_overlapping_only_an_open_review_is_held_and_one_starting_at_its_last_activity_expires() {
    let root = fresh_root("open-review");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        "front",
        1,
        NOW - 10 * DAY - 1000,
        NOW - 10 * DAY,
        10,
    );
    seed_piece(
        &root,
        &mut index,
        "front",
        2,
        NOW - 10 * DAY,
        NOW - 10 * DAY + 1000,
        10,
    );
    seed_piece(&root, &mut index, "front", 3, NOW - HOUR, NOW, 10);
    seed_review(
        &root,
        "front",
        NOW - 10 * DAY - 60_000,
        None,
        NOW - 10 * DAY,
    );

    let report = expiry::run_pass(
        &root,
        &mut index,
        NOW,
        &maps(&[("front", 7)]),
        &maps(&[("front", 30)]),
        true,
    )
    .expect("pass runs");
    assert_eq!(
        report.held, 1,
        "the open review's last activity ends the interval"
    );
    assert_eq!(report.deleted, 1);
    assert!(file_present(&root, "front", NOW - 10 * DAY - 1000, 1));
    assert!(!file_present(&root, "front", NOW - 10 * DAY, 2));
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn unheld_pieces_still_expire_and_a_piece_touching_the_review_start_is_not_held() {
    let root = fresh_root("other-pieces");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        "front",
        1,
        NOW - 8 * DAY - 1000,
        NOW - 8 * DAY,
        10,
    );
    seed_piece(
        &root,
        &mut index,
        "front",
        2,
        NOW - 20 * DAY - 1000,
        NOW - 20 * DAY,
        10,
    );
    seed_piece(
        &root,
        &mut index,
        "front",
        3,
        NOW - 8 * DAY - 6000,
        NOW - 8 * DAY - 5000,
        10,
    );
    seed_piece(&root, &mut index, "front", 4, NOW - HOUR, NOW, 10);
    seed_review(
        &root,
        "front",
        NOW - 8 * DAY - 5000,
        Some(NOW - 8 * DAY),
        NOW - 8 * DAY,
    );

    let report = expiry::run_pass(
        &root,
        &mut index,
        NOW,
        &maps(&[("front", 7)]),
        &maps(&[("front", 30)]),
        true,
    )
    .expect("pass runs");
    assert_eq!(report.held, 1);
    assert_eq!(report.deleted, 2);
    assert!(file_present(&root, "front", NOW - 8 * DAY - 1000, 1));
    assert!(!file_present(&root, "front", NOW - 20 * DAY - 1000, 2));
    assert!(!file_present(&root, "front", NOW - 8 * DAY - 6000, 3));
    assert!(file_present(&root, "front", NOW - HOUR, 4));
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn one_cameras_young_review_never_holds_the_other_cameras_expired_piece() {
    let root = fresh_root("two-cameras");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        "front",
        1,
        NOW - 8 * DAY - 1000,
        NOW - 8 * DAY,
        10,
    );
    seed_piece(&root, &mut index, "front", 2, NOW - HOUR, NOW, 10);
    seed_piece(
        &root,
        &mut index,
        "back",
        1,
        NOW - 8 * DAY - 1000,
        NOW - 8 * DAY,
        10,
    );
    seed_piece(&root, &mut index, "back", 2, NOW - HOUR, NOW, 10);
    seed_review(
        &root,
        "front",
        NOW - 8 * DAY - 5000,
        Some(NOW - 8 * DAY),
        NOW - 8 * DAY,
    );

    let report = expiry::run_pass(
        &root,
        &mut index,
        NOW,
        &maps(&[("front", 7), ("back", 7)]),
        &maps(&[("front", 30), ("back", 30)]),
        true,
    )
    .expect("pass runs");
    assert_eq!(report.held, 1);
    assert_eq!(report.deleted, 1);
    assert!(file_present(&root, "front", NOW - 8 * DAY - 1000, 1));
    assert!(!file_present(&root, "back", NOW - 8 * DAY - 1000, 1));
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_review_holding_every_expired_piece_leaves_the_guard_quiet() {
    let root = fresh_root("guard-quiet");
    let mut index = open_index(&root);
    for (seq, age) in [(1u64, 8i64), (2, 9), (3, 10)] {
        seed_piece(
            &root,
            &mut index,
            "front",
            seq,
            NOW - age * DAY - 1000,
            NOW - age * DAY,
            10,
        );
    }
    seed_review(
        &root,
        "front",
        NOW - 10 * DAY - 60_000,
        Some(NOW - 8 * DAY),
        NOW - 8 * DAY,
    );

    let report = expiry::run_pass(
        &root,
        &mut index,
        NOW,
        &maps(&[("front", 7)]),
        &maps(&[("front", 30)]),
        true,
    )
    .expect("pass runs");
    assert_eq!(
        report.refused_cameras, 0,
        "the held pieces leave no over-retention set"
    );
    assert_eq!(report.held, 3);
    assert_eq!(report.deleted, 0);
    assert_eq!(row_set(&index).len(), 3);
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

const DRY_TOKEN: &str = "dry-child";
const DRY_TEST: &str =
    "a_dry_run_reports_held_without_touching_files_rows_or_rows_past_the_age_limit";

#[test]
fn a_dry_run_reports_held_without_touching_files_rows_or_rows_past_the_age_limit() {
    if std::env::var(CHILD_ENV).unwrap_or_default() == DRY_TOKEN {
        let root = fresh_root("dry-child");
        let mut index = open_index(&root);
        seed_piece(
            &root,
            &mut index,
            "front",
            1,
            NOW - 8 * DAY - 1000,
            NOW - 8 * DAY,
            10,
        );
        seed_piece(
            &root,
            &mut index,
            "front",
            2,
            NOW - 20 * DAY - 1000,
            NOW - 20 * DAY,
            10,
        );
        seed_piece(&root, &mut index, "front", 3, NOW - HOUR, NOW, 10);
        seed_review(
            &root,
            "front",
            NOW - 8 * DAY - 5000,
            Some(NOW - 8 * DAY),
            NOW - 8 * DAY,
        );
        seed_event(
            &root,
            "track-old",
            NOW - 400 * DAY,
            Some(NOW - 400 * DAY),
            NOW - 400 * DAY,
        );

        let mut before = Vec::new();
        tracked_snapshot(&root, &root, &mut before);
        before.sort();
        let rows_before = row_set(&index);

        let report = expiry::run_pass(
            &root,
            &mut index,
            NOW,
            &maps(&[("front", 7)]),
            &maps(&[("front", 30)]),
            false,
        )
        .expect("dry pass runs");
        assert_eq!(report.held, 1, "the dry run reports the held count");
        assert_eq!(report.would_delete, 1);
        assert_eq!(report.deleted, 0);

        let mut after = Vec::new();
        tracked_snapshot(&root, &root, &mut after);
        after.sort();
        assert_eq!(before, after, "every file and byte is identical");
        assert_eq!(row_set(&index), rows_before);
        assert_eq!(
            surviving(&root, "id", "events", "front"),
            vec![1],
            "the dry run prunes nothing"
        );
        assert!(!root.join("deletions.log.jsonl").exists());
        std::fs::remove_dir_all(&root).expect("temp root removed");
        return;
    }
    let output = spawn_child(DRY_TOKEN, DRY_TEST);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "the child run failed:\n{stderr}");
    assert_eq!(
        stderr.matches("event=expiry-held").count(),
        1,
        "held is logged\n{stderr}"
    );
    assert_eq!(
        stderr.matches("event=expiry-would-delete").count(),
        1,
        "one would-delete line\n{stderr}"
    );
    assert_eq!(stderr.matches("event=expiry-pruned").count(), 0);
}

#[test]
fn the_pressure_expired_stage_spares_a_held_piece_and_the_within_retention_stage_still_takes_it() {
    let rows: Vec<PieceRow> = [
        (1u64, NOW - 30 * DAY - 1000, NOW - 30 * DAY, 5000u64),
        (2, NOW - 8 * DAY - 1000, NOW - 8 * DAY, 100),
        (3, NOW - 5 * DAY - 1000, NOW - 5 * DAY, 100),
        (4, NOW - 2000, NOW, 4000),
    ]
    .iter()
    .map(|(seq, start, end, size)| PieceRow {
        camera: "front".to_owned(),
        seq: *seq,
        start_ms: *start,
        end_ms: *end,
        duration_s: 1.0,
        path: format!("recordings/front/x/{start}-{seq}.mp4"),
        size_bytes: *size,
        codec: "h264".to_owned(),
    })
    .collect();
    let held: std::collections::HashSet<(String, u64)> =
        std::collections::HashSet::from([("front".to_owned(), 1u64)]);

    let plan = pressure::plan_pressure(&rows, NOW, &maps(&[("front", 7)]), &held, 1, 0);
    let taken: Vec<(u64, pressure::PressureReason)> = plan
        .deletions
        .iter()
        .map(|deletion| (deletion.row.seq, deletion.reason))
        .collect();
    assert_eq!(
        taken,
        vec![
            (2, pressure::PressureReason::Expired),
            (1, pressure::PressureReason::WithinRetention),
        ],
        "the expired stage skips the held piece and the hard floor reaches it"
    );
    assert_eq!(plan.alerts.len(), 1);
}

const PRESSURE_TOKEN: &str = "pressure-child";
const PRESSURE_TEST: &str =
    "a_hard_floor_pressure_pass_takes_a_held_piece_and_the_alert_line_names_the_days";

#[test]
fn a_hard_floor_pressure_pass_takes_a_held_piece_and_the_alert_line_names_the_days() {
    if std::env::var(CHILD_ENV).unwrap_or_default() == PRESSURE_TOKEN {
        let root = fresh_root("pressure-child");
        let mut index = open_index(&root);
        seed_piece(
            &root,
            &mut index,
            "front",
            1,
            NOW - 8 * DAY - 1000,
            NOW - 8 * DAY,
            5000,
        );
        seed_piece(&root, &mut index, "front", 2, NOW - 2000, NOW, 8000);
        seed_review(
            &root,
            "front",
            NOW - 8 * DAY - 5000,
            Some(NOW - 8 * DAY),
            NOW - 8 * DAY,
        );

        let report = pressure::run_pressure_pass(
            &root,
            &mut index,
            NOW,
            &maps(&[("front", 7)]),
            &maps(&[("front", 30)]),
            pressure::PressureBudget {
                recording_cameras: 1,
                free_bytes: 0,
            },
            true,
        )
        .expect("pressure pass runs");
        assert_eq!(report.deleted, 1);
        assert!(
            !file_present(&root, "front", NOW - 8 * DAY - 1000, 1),
            "the held piece goes at the hard floor"
        );
        std::fs::remove_dir_all(&root).expect("temp root removed");
        return;
    }
    let output = spawn_child(PRESSURE_TOKEN, PRESSURE_TEST);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "the child run failed:\n{stderr}");
    assert_eq!(
        stderr.matches("event=pressure-alert").count(),
        1,
        "one alert naming the camera and days\n{stderr}"
    );
}

const PRUNE_TOKEN: &str = "prune-child";
const PRUNE_TEST: &str =
    "row_pruning_removes_only_closed_rows_older_than_the_limit_and_never_an_open_one";

#[test]
fn row_pruning_removes_only_closed_rows_older_than_the_limit_and_never_an_open_one() {
    if std::env::var(CHILD_ENV).unwrap_or_default() == PRUNE_TOKEN {
        let root = fresh_root("prune-child");
        let mut index = open_index(&root);
        seed_event(
            &root,
            "track-old",
            NOW - 30 * DAY - 1,
            Some(NOW - 30 * DAY - 1),
            NOW - 30 * DAY - 1,
        );
        seed_event(
            &root,
            "track-limit",
            NOW - 30 * DAY,
            Some(NOW - 30 * DAY),
            NOW - 30 * DAY,
        );
        seed_event(&root, "track-open", NOW - 400 * DAY, None, NOW - 400 * DAY);
        seed_event(&root, "track-fresh", NOW - DAY, Some(NOW - DAY), NOW - DAY);
        seed_review(
            &root,
            "front",
            NOW - 30 * DAY - 1,
            Some(NOW - 30 * DAY - 1),
            NOW - 30 * DAY - 1,
        );
        seed_review(
            &root,
            "front",
            NOW - 30 * DAY,
            Some(NOW - 30 * DAY),
            NOW - 30 * DAY,
        );
        seed_review(&root, "front", NOW - 400 * DAY, None, NOW - 400 * DAY);
        seed_motion(&root, "front", NOW_S - 30 * 86_400 - 1);
        seed_motion(&root, "front", NOW_S - 30 * 86_400);
        seed_motion(&root, "front", NOW_S - 1);

        let report = expiry::run_pass(
            &root,
            &mut index,
            NOW,
            &maps(&[("front", 7)]),
            &maps(&[("front", 30)]),
            true,
        )
        .expect("pass runs");
        assert_eq!(report.deleted, 0);
        assert_eq!(
            surviving(&root, "id", "events", "front"),
            vec![2, 3, 4],
            "only the row past the limit leaves"
        );
        assert_eq!(
            surviving(&root, "id", "reviews", "front"),
            vec![2, 3],
            "the open review at a huge age stays"
        );
        assert_eq!(
            surviving(&root, "second", "motion", "front"),
            vec![NOW_S - 30 * 86_400, NOW_S - 1],
            "the tick older than the limit leaves"
        );
        std::fs::remove_dir_all(&root).expect("temp root removed");
        return;
    }
    let output = spawn_child(PRUNE_TOKEN, PRUNE_TEST);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "the child run failed:\n{stderr}");
    assert_eq!(
        stderr.matches("event=expiry-pruned").count(),
        1,
        "the prune is logged\n{stderr}"
    );
    assert!(stderr.contains("events=1 reviews=1 motion=1"));
}

#[test]
fn reopening_the_index_leaves_the_hold_behaviour_unchanged() {
    let root = fresh_root("restart");
    {
        let mut index = open_index(&root);
        seed_piece(
            &root,
            &mut index,
            "front",
            1,
            NOW - 8 * DAY - 1000,
            NOW - 8 * DAY,
            10,
        );
        seed_piece(&root, &mut index, "front", 2, NOW - HOUR, NOW, 10);
        seed_review(
            &root,
            "front",
            NOW - 8 * DAY - 5000,
            Some(NOW - 8 * DAY),
            NOW - 8 * DAY,
        );
        let first = expiry::run_pass(
            &root,
            &mut index,
            NOW,
            &maps(&[("front", 7)]),
            &maps(&[("front", 30)]),
            true,
        )
        .expect("first pass runs");
        assert_eq!(first.held, 1);
    }
    let mut index = open_index(&root);
    let second = expiry::run_pass(
        &root,
        &mut index,
        NOW,
        &maps(&[("front", 7)]),
        &maps(&[("front", 30)]),
        true,
    )
    .expect("pass after reopen runs");
    assert_eq!(second.held, 1);
    assert_eq!(second.deleted, 0);
    assert!(file_present(&root, "front", NOW - 8 * DAY - 1000, 1));
    assert_eq!(row_set(&index).len(), 2);
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_zero_window_holds_without_ageing_out_and_prunes_nothing() {
    let root = fresh_root("zero-window");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        "front",
        1,
        NOW - 400 * DAY - 1000,
        NOW - 400 * DAY,
        10,
    );
    seed_piece(&root, &mut index, "front", 2, NOW - HOUR, NOW, 10);
    seed_review(
        &root,
        "front",
        NOW - 400 * DAY - 60_000,
        Some(NOW - 400 * DAY),
        NOW - 400 * DAY,
    );
    seed_event(
        &root,
        "track-old",
        NOW - 400 * DAY,
        Some(NOW - 400 * DAY),
        NOW - 400 * DAY,
    );

    let report = expiry::run_pass(
        &root,
        &mut index,
        NOW,
        &maps(&[("front", 7)]),
        &maps(&[("front", 0)]),
        true,
    )
    .expect("pass runs");
    assert_eq!(report.held, 1, "zero never ages out");
    assert_eq!(report.deleted, 0);
    assert!(file_present(&root, "front", NOW - 400 * DAY - 1000, 1));
    assert_eq!(
        surviving(&root, "id", "events", "front"),
        vec![1],
        "zero prunes no rows"
    );
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_zero_window_pressure_pass_at_the_hard_floor_frees_held_footage() {
    let root = fresh_root("zero-pressure");
    let mut index = open_index(&root);
    seed_piece(
        &root,
        &mut index,
        "front",
        1,
        NOW - 8 * DAY - 1000,
        NOW - 8 * DAY,
        5000,
    );
    seed_piece(&root, &mut index, "front", 2, NOW - 2000, NOW, 8000);
    seed_review(
        &root,
        "front",
        NOW - 8 * DAY - 5000,
        Some(NOW - 8 * DAY),
        NOW - 8 * DAY,
    );
    let report = pressure::run_pressure_pass(
        &root,
        &mut index,
        NOW,
        &maps(&[("front", 7)]),
        &maps(&[("front", 0)]),
        pressure::PressureBudget {
            recording_cameras: 1,
            free_bytes: 0,
        },
        true,
    )
    .expect("pressure pass runs");
    assert_eq!(
        report.deleted, 1,
        "the held piece is freed at the hard floor"
    );
    assert!(
        !file_present(&root, "front", NOW - 8 * DAY - 1000, 1),
        "the within-retention stage takes held footage"
    );
    assert!(
        file_present(&root, "front", NOW - 2000, 2),
        "the newest hour stays"
    );
    std::fs::remove_dir_all(&root).expect("temp root removed");
}
