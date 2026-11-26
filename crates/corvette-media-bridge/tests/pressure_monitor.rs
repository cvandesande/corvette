use corvette_media_bridge::index::{
    EXPIRY_PASS_INTERVAL_MS, ExpiryWiring, Index, IndexWrite, PieceRow,
};
use corvette_media_bridge::utc::utc_parts;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

const DAY: i64 = 86_400_000;
const MIN: i64 = 60_000;
const START: i64 = 10 * DAY;

fn fresh_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "corvette-pressure-monitor-{name}-{}",
        std::process::id()
    ));
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
    seq: u64,
    start_ms: i64,
    end_ms: i64,
    size_bytes: u64,
) {
    let relative = piece_relative_path("front", start_ms, seq);
    let full = root.join(&relative);
    std::fs::create_dir_all(full.parent().expect("the path names a directory"))
        .expect("piece directory created");
    std::fs::write(
        &full,
        vec![b'x'; usize::try_from(size_bytes).expect("test sizes fit")],
    )
    .expect("piece file written");
    index
        .insert(&PieceRow {
            camera: "front".to_owned(),
            seq,
            start_ms,
            end_ms,
            duration_s: 1.0,
            path: relative,
            size_bytes,
            codec: "h264".to_owned(),
        })
        .expect("row inserted");
}

fn wake_row(seq: u64) -> IndexWrite {
    IndexWrite::Piece(PieceRow {
        camera: "front".to_owned(),
        seq,
        start_ms: START,
        end_ms: START + 1000,
        duration_s: 1.0,
        path: format!("recordings/front/2026-10-01/00/wake-{seq}.mp4"),
        size_bytes: 0,
        codec: "h264".to_owned(),
    })
}

fn file_present(root: &Path, start_ms: i64, seq: u64) -> bool {
    root.join(piece_relative_path("front", start_ms, seq))
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

fn deletion_lines(root: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(root.join("deletions.log.jsonl")).expect("log readable");
    text.lines().map(str::to_owned).collect()
}

fn poll_until(label: &str, condition: impl Fn() -> bool) {
    for _ in 0..400 {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(condition(), "timed out waiting for {label}");
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
            // The writer thread's own inserts rewrite the SQLite files on every
            // tick; the dry-run claim is about footage and logs, so only the
            // index storage itself is excluded here.
            if !relative.starts_with("index.sqlite") {
                out.push((relative, std::fs::read(&path).expect("file readable")));
            }
        }
    }
}

fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    tracked_snapshot(root, root, &mut files);
    files.sort();
    files
}

const CHILD_ENV: &str = "CORVETTE_PRESSURE_MONITOR_CHILD_RUN";
const DRY_TOKEN: &str = "dry-child";
const FLAP_TOKEN: &str = "flap-child";
const DRY_TEST: &str = "a_dry_run_monitor_tick_deletes_nothing_and_logs_each_would_delete_line";
const FLAP_TEST: &str =
    "an_unreadable_free_figure_is_logged_once_per_failure_streak_and_recovers_once";

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
fn free_above_the_floor_keeps_everything_across_pressure_ticks() {
    let root = fresh_root("above-floor");
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 1, 2 * DAY - 1000, 2 * DAY, 100);
    seed_piece(&root, &mut index, 2, DAY - 1000, DAY, 100);
    seed_piece(&root, &mut index, 9, START - 1000, START - 1, 1000);
    let clock = Arc::new(AtomicI64::new(START));
    let clock_for_clock = Arc::clone(&clock);
    let wiring = ExpiryWiring {
        root: root.clone(),
        retention_days: retention(&[("front", 7)]),
        enforce: true,
        clock: Box::new(move || clock_for_clock.load(Ordering::SeqCst)),
        recording_cameras: 1,
        free_space: Box::new(|_root| Ok(u64::MAX)),
    };
    let (sender, writer) = corvette_media_bridge::index::spawn_writer_with_expiry(index, 0, wiring);
    sender
        .send(wake_row(100))
        .expect("the first wake row is accepted");
    clock.store(START + MIN, Ordering::SeqCst);
    sender
        .send(wake_row(101))
        .expect("the second wake row is accepted");
    clock.store(START + 2 * MIN, Ordering::SeqCst);
    sender
        .send(wake_row(102))
        .expect("the third wake row is accepted");
    drop(sender);
    let index = writer.join().expect("the writer thread joins");
    assert!(
        file_present(&root, 2 * DAY - 1000, 1),
        "an expired piece survives"
    );
    assert!(
        file_present(&root, DAY - 1000, 2),
        "the oldest expired piece survives"
    );
    assert!(
        file_present(&root, START - 1000, 9),
        "the newest piece survives"
    );
    assert!(
        !root.join("deletions.log.jsonl").exists(),
        "no delete was logged"
    );
    assert_eq!(
        row_set(&index),
        vec![
            ("front".to_owned(), 1),
            ("front".to_owned(), 2),
            ("front".to_owned(), 9),
            ("front".to_owned(), 100),
            ("front".to_owned(), 101),
            ("front".to_owned(), 102),
        ],
        "every row survives"
    );
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn below_the_floor_the_first_pressure_tick_deletes_expired_pieces_oldest_first_and_the_next_tick_adds_nothing()
 {
    let root = fresh_root("below-floor");
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 1, 2 * DAY - 1000, 2 * DAY, 100);
    seed_piece(&root, &mut index, 2, DAY - 1000, DAY, 100);
    seed_piece(&root, &mut index, 9, START - 1000, START - 1, 1000);
    let clock = Arc::new(AtomicI64::new(START));
    let clock_for_clock = Arc::clone(&clock);
    let free_calls = Arc::new(AtomicUsize::new(0));
    let calls_for_free = Arc::clone(&free_calls);
    let wiring = ExpiryWiring {
        root: root.clone(),
        retention_days: retention(&[("front", 7)]),
        enforce: true,
        clock: Box::new(move || clock_for_clock.load(Ordering::SeqCst)),
        recording_cameras: 1,
        free_space: Box::new(move |_root| {
            if calls_for_free.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(0)
            } else {
                Ok(u64::MAX)
            }
        }),
    };
    let (sender, writer) = corvette_media_bridge::index::spawn_writer_with_expiry(index, 0, wiring);
    // Polling the pass count before advancing the clock makes each tick's clock
    // read exact: the writer cannot see a clock value until the previous pass
    // call has been observed.
    sender
        .send(wake_row(100))
        .expect("the first wake row is accepted");
    poll_until("the first pressure pass ran", || {
        free_calls.load(Ordering::SeqCst) == 1
    });
    poll_until("the first tick deleted the expired pieces", || {
        !file_present(&root, 2 * DAY - 1000, 1) && !file_present(&root, DAY - 1000, 2)
    });
    clock.store(START + MIN, Ordering::SeqCst);
    sender
        .send(wake_row(101))
        .expect("the second wake row is accepted");
    poll_until("the second pressure pass ran", || {
        free_calls.load(Ordering::SeqCst) == 2
    });
    drop(sender);
    let index = writer.join().expect("the writer thread joins");
    assert!(
        !file_present(&root, DAY - 1000, 2) && !file_present(&root, 2 * DAY - 1000, 1),
        "the first tick took both expired pieces"
    );
    assert!(
        file_present(&root, START - 1000, 9),
        "the second tick deleted nothing more"
    );
    assert_eq!(
        row_set(&index),
        vec![
            ("front".to_owned(), 9),
            ("front".to_owned(), 100),
            ("front".to_owned(), 101),
        ],
        "only the expired rows left the index"
    );
    let lines = deletion_lines(&root);
    assert_eq!(lines.len(), 2, "one line per deleted piece");
    assert!(lines[0].contains("\"seq\":2"), "oldest first: {lines:?}");
    assert!(lines[1].contains("\"seq\":1"), "oldest first: {lines:?}");
    assert!(
        lines
            .iter()
            .all(|line| line.contains("\"reason\":\"pressure\"")),
        "both deletions carry the pressure reason: {lines:?}"
    );
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn a_dry_run_monitor_tick_deletes_nothing_and_logs_each_would_delete_line() {
    if std::env::var(CHILD_ENV).unwrap_or_default() == DRY_TOKEN {
        let root = fresh_root("dry-child");
        let mut index = open_index(&root);
        seed_piece(&root, &mut index, 1, 2 * DAY - 1000, 2 * DAY, 100);
        seed_piece(&root, &mut index, 2, DAY - 1000, DAY, 100);
        seed_piece(&root, &mut index, 9, START - 1000, START - 1, 1000);
        let before = snapshot(&root);
        let clock = Arc::new(AtomicI64::new(START));
        let clock_for_clock = Arc::clone(&clock);
        let free_calls = Arc::new(AtomicUsize::new(0));
        let calls_for_free = Arc::clone(&free_calls);
        let wiring = ExpiryWiring {
            root: root.clone(),
            retention_days: retention(&[("front", 7)]),
            enforce: false,
            clock: Box::new(move || clock_for_clock.load(Ordering::SeqCst)),
            recording_cameras: 1,
            free_space: Box::new(move |_root| {
                if calls_for_free.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(0)
                } else {
                    Ok(u64::MAX)
                }
            }),
        };
        let (sender, writer) =
            corvette_media_bridge::index::spawn_writer_with_expiry(index, 0, wiring);
        sender
            .send(wake_row(100))
            .expect("the first wake row is accepted");
        poll_until("the first dry-run pressure pass ran", || {
            free_calls.load(Ordering::SeqCst) == 1
        });
        clock.store(START + MIN, Ordering::SeqCst);
        sender
            .send(wake_row(101))
            .expect("the second wake row is accepted");
        poll_until("the second pressure pass ran", || {
            free_calls.load(Ordering::SeqCst) == 2
        });
        drop(sender);
        let index = writer.join().expect("the writer thread joins");
        assert_eq!(
            snapshot(&root),
            before,
            "the footage tree is byte-identical"
        );
        assert_eq!(
            row_set(&index),
            vec![
                ("front".to_owned(), 1),
                ("front".to_owned(), 2),
                ("front".to_owned(), 9),
                ("front".to_owned(), 100),
                ("front".to_owned(), 101),
            ],
            "the expired rows survive the dry run alongside the wake rows"
        );
        assert!(
            !root.join("deletions.log.jsonl").exists(),
            "dry run writes no deletion log"
        );
        std::fs::remove_dir_all(&root).expect("temp root removed");
        return;
    }
    let output = spawn_child(DRY_TOKEN, DRY_TEST);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "the child run failed:\n{stderr}");
    assert_eq!(
        stderr.matches("event=pressure-would-delete").count(),
        2,
        "one would-delete line per planned piece\n{stderr}"
    );
}

#[test]
fn an_unreadable_free_figure_is_logged_once_per_failure_streak_and_recovers_once() {
    if std::env::var(CHILD_ENV).unwrap_or_default() == FLAP_TOKEN {
        let root = fresh_root("flap-child");
        let mut index = open_index(&root);
        seed_piece(&root, &mut index, 1, 2 * DAY - 1000, 2 * DAY, 100);
        seed_piece(&root, &mut index, 9, START - 1000, START - 1, 1000);
        let clock = Arc::new(AtomicI64::new(START));
        let clock_for_clock = Arc::clone(&clock);
        let free_calls = Arc::new(AtomicUsize::new(0));
        let calls_for_free = Arc::clone(&free_calls);
        let wiring = ExpiryWiring {
            root: root.clone(),
            retention_days: retention(&[("front", 7)]),
            enforce: true,
            clock: Box::new(move || clock_for_clock.load(Ordering::SeqCst)),
            recording_cameras: 1,
            free_space: Box::new(move |_root| {
                match calls_for_free.fetch_add(1, Ordering::SeqCst) {
                    0..=2 => Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
                    _ => Ok(u64::MAX),
                }
            }),
        };
        let (sender, writer) =
            corvette_media_bridge::index::spawn_writer_with_expiry(index, 0, wiring);
        sender
            .send(wake_row(100))
            .expect("the first wake row is accepted");
        poll_until("the first unreadable tick ran", || {
            free_calls.load(Ordering::SeqCst) == 1
        });
        clock.store(START + MIN, Ordering::SeqCst);
        sender
            .send(wake_row(101))
            .expect("the second wake row is accepted");
        poll_until("the second unreadable tick ran", || {
            free_calls.load(Ordering::SeqCst) == 2
        });
        clock.store(START + 2 * MIN, Ordering::SeqCst);
        sender
            .send(wake_row(102))
            .expect("the third wake row is accepted");
        poll_until("the third unreadable tick ran", || {
            free_calls.load(Ordering::SeqCst) == 3
        });
        clock.store(START + 3 * MIN, Ordering::SeqCst);
        sender
            .send(wake_row(103))
            .expect("the recovery wake row is accepted");
        poll_until("the recovered pressure pass ran", || {
            free_calls.load(Ordering::SeqCst) == 4
        });
        drop(sender);
        let index = writer.join().expect("the writer thread joins");
        assert!(
            file_present(&root, 2 * DAY - 1000, 1),
            "the unreadable ticks deleted nothing and the recovered tick saw free space above the floor"
        );
        assert!(file_present(&root, START - 1000, 9));
        assert!(!root.join("deletions.log.jsonl").exists());
        assert_eq!(
            row_set(&index),
            vec![
                ("front".to_owned(), 1),
                ("front".to_owned(), 9),
                ("front".to_owned(), 100),
                ("front".to_owned(), 101),
                ("front".to_owned(), 102),
                ("front".to_owned(), 103),
            ]
        );
        std::fs::remove_dir_all(&root).expect("temp root removed");
        return;
    }
    let output = spawn_child(FLAP_TOKEN, FLAP_TEST);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "the child run failed:\n{stderr}");
    assert_eq!(
        stderr.matches("event=pressure-free-unreadable").count(),
        1,
        "one unreadable line for the whole failure streak\n{stderr}"
    );
    assert_eq!(
        stderr.matches("event=pressure-free-readable").count(),
        1,
        "one readable line on recovery\n{stderr}"
    );
}

#[test]
fn pressure_ticks_do_not_run_the_expiry_pass_early_and_the_hourly_expiry_still_fires() {
    let root = fresh_root("expiry-cadence");
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 1, 2 * DAY - 1000, 2 * DAY, 100);
    seed_piece(&root, &mut index, 8, 4 * DAY - 1000, 4 * DAY, 100);
    seed_piece(&root, &mut index, 9, START - 1000, START - 1, 1000);
    let clock = Arc::new(AtomicI64::new(START));
    let clock_for_clock = Arc::clone(&clock);
    let wiring = ExpiryWiring {
        root: root.clone(),
        retention_days: retention(&[("front", 7)]),
        enforce: true,
        clock: Box::new(move || clock_for_clock.load(Ordering::SeqCst)),
        recording_cameras: 1,
        free_space: Box::new(|_root| Ok(u64::MAX)),
    };
    let (sender, writer) = corvette_media_bridge::index::spawn_writer_with_expiry(index, 0, wiring);
    // A capacity-zero send returns when the writer picks the row up, so the
    // send after a tick proves that tick's passes finished before its check.
    sender
        .send(wake_row(100))
        .expect("the first wake row is accepted");
    sender
        .send(wake_row(101))
        .expect("the second wake row is accepted");
    assert!(
        file_present(&root, 2 * DAY - 1000, 1),
        "the first pressure tick must not run the expiry pass"
    );
    clock.store(START + MIN, Ordering::SeqCst);
    sender
        .send(wake_row(102))
        .expect("the third wake row is accepted");
    clock.store(START + 2 * MIN, Ordering::SeqCst);
    sender
        .send(wake_row(103))
        .expect("the fourth wake row is accepted");
    assert!(
        file_present(&root, 2 * DAY - 1000, 1),
        "later sub-hour pressure ticks must not run the expiry pass early"
    );
    clock.store(START + EXPIRY_PASS_INTERVAL_MS, Ordering::SeqCst);
    sender
        .send(wake_row(104))
        .expect("the hourly wake row is accepted");
    poll_until("the hourly expiry pass deleted the expired piece", || {
        !file_present(&root, 2 * DAY - 1000, 1)
    });
    drop(sender);
    let index = writer.join().expect("the writer thread joins");
    assert_eq!(
        row_set(&index),
        vec![
            ("front".to_owned(), 8),
            ("front".to_owned(), 9),
            ("front".to_owned(), 100),
            ("front".to_owned(), 101),
            ("front".to_owned(), 102),
            ("front".to_owned(), 103),
            ("front".to_owned(), 104),
        ],
        "only the expiry pass deleted a row, at the hour"
    );
    let lines = deletion_lines(&root);
    assert_eq!(lines.len(), 1, "only the hourly pass deleted");
    assert!(lines[0].contains("\"reason\":\"expired\""), "{lines:?}");
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

#[test]
fn dropping_the_sender_returns_the_index_after_pressure_wiring() {
    let root = fresh_root("shutdown");
    let mut index = open_index(&root);
    seed_piece(&root, &mut index, 9, START - 1000, START - 1, 1000);
    let wiring = ExpiryWiring {
        root: root.clone(),
        retention_days: retention(&[("front", 7)]),
        enforce: true,
        clock: Box::new(move || START),
        recording_cameras: 1,
        free_space: Box::new(|_root| Ok(u64::MAX)),
    };
    let (sender, writer) = corvette_media_bridge::index::spawn_writer_with_expiry(index, 4, wiring);
    sender
        .send(wake_row(100))
        .expect("the buffered wake row is accepted");
    sender
        .send(wake_row(101))
        .expect("the buffered wake row is accepted");
    drop(sender);
    let index = writer.join().expect("the writer thread joins");
    assert_eq!(
        row_set(&index),
        vec![
            ("front".to_owned(), 9),
            ("front".to_owned(), 100),
            ("front".to_owned(), 101),
        ],
        "the buffered inserts completed before the index was returned"
    );
    assert!(
        !root.join("deletions.log.jsonl").exists(),
        "nothing deleted on the way out"
    );
    std::fs::remove_dir_all(&root).expect("temp root removed");
}
