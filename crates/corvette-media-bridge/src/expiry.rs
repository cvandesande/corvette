//! Removes stored footage past each camera's retention, and nothing else.
//!
//! Dry run logs would-deletes and touches nothing; enforce stages rename-row-unlink-log so every crash point between steps is repaired by `crate::recovery::scan`.

use crate::index::{Index, IndexError, PieceRow};
use crate::supervise::log_event;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::hash::BuildHasher;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone)]
pub struct CameraPlan {
    pub camera: String,
    pub to_delete: Vec<PieceRow>,
    pub total_pieces: usize,
    pub would_delete: usize,
    pub guard_tripped: bool,
}

#[derive(Debug, Clone)]
pub struct ExpiryPlan {
    pub cameras: Vec<CameraPlan>,
}

/// Decides per camera which pieces expire at `now_ms`; more than 90 percent or all expired trips the guard, refusing with an empty `to_delete`.
#[must_use]
pub fn plan<S: BuildHasher>(
    rows: &[PieceRow],
    now_ms: i64,
    retention_days: &HashMap<String, u32, S>,
) -> ExpiryPlan {
    let mut by_camera: BTreeMap<&str, Vec<&PieceRow>> = BTreeMap::new();
    for row in rows {
        by_camera.entry(&row.camera).or_default().push(row);
    }
    let mut cameras = Vec::new();
    for (camera, cam_rows) in by_camera {
        let total = cam_rows.len();
        let expired: Vec<PieceRow> = cutoff_ms(now_ms, retention_days.get(camera).copied())
            .map_or_else(Vec::new, |cutoff| {
                cam_rows
                    .iter()
                    .filter(|row| row.end_ms < cutoff)
                    .map(|row| (*row).clone())
                    .collect()
            });
        let would_delete = expired.len();
        let guard_tripped = would_delete.saturating_mul(100) > total.saturating_mul(90);
        cameras.push(CameraPlan {
            camera: camera.to_owned(),
            to_delete: if guard_tripped { Vec::new() } else { expired },
            total_pieces: total,
            would_delete,
            guard_tripped,
        });
    }
    ExpiryPlan { cameras }
}

/// None means this camera never expires: absent from the map or an explicit zero.
fn cutoff_ms(now_ms: i64, days: Option<u32>) -> Option<i64> {
    let days = days.filter(|days| *days != 0)?;
    let window_ms = i64::from(days).checked_mul(DAY_MS)?;
    Some(now_ms.saturating_sub(window_ms))
}

/// One pass's counts; `step_failures[k]` tallies staged step k+1 failures.
///
/// Each failure leaves the piece in a state `crate::recovery::scan` repairs: trash copy, dangling row, orphan, or missing log line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PassReport {
    pub enforce: bool,
    pub would_delete: usize,
    pub deleted: usize,
    pub refused_cameras: usize,
    pub step_failures: [usize; 4],
}

#[derive(Debug)]
pub enum ExpiryError {
    ReadRows { source: IndexError },
}

impl std::fmt::Display for ExpiryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadRows { .. } => write!(f, "expiry could not read the recording index"),
        }
    }
}

impl std::error::Error for ExpiryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ReadRows { source } => Some(source),
        }
    }
}

/// Runs one expiry pass at `now_ms` over `index`'s rows rooted at `root`.
///
/// Dry run logs `expiry-would-delete` and touches nothing; enforce stages rename-row-unlink-log; a refused camera logs `expiry-refused` plus `expiry-alert` and deletes nothing.
///
/// # Errors
/// [`ExpiryError::ReadRows`] when the rows cannot be read; per-piece failures are counted in the report.
pub fn run_pass<S: BuildHasher>(
    root: &Path,
    index: &mut Index,
    now_ms: i64,
    retention_days: &HashMap<String, u32, S>,
    enforce: bool,
) -> Result<PassReport, ExpiryError> {
    run_pass_at(root, index, now_ms, retention_days, enforce, 0)
}

/// [`run_pass`] but a piece's enforce sequence stops after its first `steps` staged steps, leaving that crash-window state for recovery tests; 0 runs whole pieces.
///
/// # Errors
/// See [`run_pass`].
pub fn run_pass_crash_after<S: BuildHasher>(
    root: &Path,
    index: &mut Index,
    now_ms: i64,
    retention_days: &HashMap<String, u32, S>,
    enforce: bool,
    steps: u8,
) -> Result<PassReport, ExpiryError> {
    run_pass_at(root, index, now_ms, retention_days, enforce, steps)
}

fn run_pass_at<S: BuildHasher>(
    root: &Path,
    index: &mut Index,
    now_ms: i64,
    retention_days: &HashMap<String, u32, S>,
    enforce: bool,
    crash_after: u8,
) -> Result<PassReport, ExpiryError> {
    let rows = index
        .all_pieces()
        .map_err(|source| ExpiryError::ReadRows { source })?;
    let decided = plan(&rows, now_ms, retention_days);
    let mut report = PassReport {
        enforce,
        ..PassReport::default()
    };
    for camera in decided.cameras {
        if camera.guard_tripped {
            let numbers = format!(
                "would_delete={} total={}",
                camera.would_delete, camera.total_pieces
            );
            log_event(&camera.camera, "expiry-refused", &numbers);
            log_event(&camera.camera, "expiry-alert", &numbers);
            report.refused_cameras += 1;
            continue;
        }
        report.would_delete += camera.to_delete.len();
        if !enforce {
            for row in &camera.to_delete {
                log_event(
                    &camera.camera,
                    "expiry-would-delete",
                    format!("seq={} age_days={}", row.seq, age_days(now_ms, row.end_ms)),
                );
            }
            continue;
        }
        for row in &camera.to_delete {
            match expire_piece(root, index, row, now_ms, &mut report, crash_after) {
                StepOutcome::Crashed => return Ok(report),
                StepOutcome::Done | StepOutcome::Failed => {}
            }
        }
    }
    Ok(report)
}

enum StepOutcome {
    Done,
    Failed,
    Crashed,
}

fn expire_piece(
    root: &Path,
    index: &mut Index,
    row: &PieceRow,
    now_ms: i64,
    report: &mut PassReport,
    crash_after: u8,
) -> StepOutcome {
    let staged = match stage_piece(root, row) {
        Stage::Moved(path) => Some(path),
        Stage::AlreadyGone => None,
        Stage::Failed => return note_step_failure(report, row, 1),
    };
    if crash_after == 1 {
        return StepOutcome::Crashed;
    }
    if index.remove(&row.camera, row.seq).is_err() {
        return note_step_failure(report, row, 2);
    }
    if crash_after == 2 {
        return StepOutcome::Crashed;
    }
    if let Some(trashed) = &staged
        && fs::remove_file(trashed).is_err()
    {
        return note_step_failure(report, row, 3);
    }
    if append_deletion_log(root, row, now_ms).is_err() {
        return note_step_failure(report, row, 4);
    }
    report.deleted += 1;
    StepOutcome::Done
}

fn note_step_failure(report: &mut PassReport, row: &PieceRow, step: usize) -> StepOutcome {
    report.step_failures[step - 1] += 1;
    log_event(&row.camera, "expiry-step-failed", format!("step={step}"));
    StepOutcome::Failed
}

enum Stage {
    Moved(PathBuf),
    AlreadyGone,
    Failed,
}

fn stage_piece(root: &Path, row: &PieceRow) -> Stage {
    let Some(source) = visible_piece_path(root, &row.path) else {
        return Stage::Failed;
    };
    match source.try_exists() {
        Ok(true) => {}
        Ok(false) => return Stage::AlreadyGone,
        Err(_) => return Stage::Failed,
    }
    let trash_dir = root.join(".trash");
    if fs::create_dir_all(&trash_dir).is_err() {
        return Stage::Failed;
    }
    let staged = trash_dir.join(format!("{}-{}-{}.mp4", row.camera, row.seq, row.start_ms));
    if fs::rename(&source, &staged).is_err() {
        return Stage::Failed;
    }
    Stage::Moved(staged)
}

/// Only the recorder's five-component layout `recordings/<camera>/<date>/<hour>/<name>.mp4` is accepted; every other stored path is rejected here.
fn visible_piece_path(root: &Path, stored: &str) -> Option<PathBuf> {
    let parts = Path::new(stored).components().collect::<Vec<_>>();
    let [
        Component::Normal(recordings),
        Component::Normal(_),
        Component::Normal(_),
        Component::Normal(_),
        Component::Normal(name),
    ] = parts.as_slice()
    else {
        return None;
    };
    if *recordings != "recordings" || !name.to_string_lossy().ends_with(".mp4") {
        return None;
    }
    Some(root.join(stored))
}

fn append_deletion_log(root: &Path, row: &PieceRow, now_ms: i64) -> std::io::Result<()> {
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("deletions.log.jsonl"))?;
    let line = format!(
        "{{\"camera\":\"{camera}\",\"seq\":{seq},\"path\":\"{path}\",\"start_ms\":{start},\
         \"end_ms\":{end},\"age_days\":{age},\"reason\":\"expired\",\"size_bytes\":{size},\
         \"mode\":\"enforce\"}}\n",
        camera = json_escape(&row.camera),
        seq = row.seq,
        path = json_escape(&row.path),
        start = row.start_ms,
        end = row.end_ms,
        age = age_days(now_ms, row.end_ms),
        size = row.size_bytes,
    );
    log.write_all(line.as_bytes())?;
    log.sync_all()
}

/// JSON string escaping by hand. Row writers build camera names and paths from `[A-Za-z0-9_./+-]` only (`recorder.rs`'s `close_piece`, mirrored by `recovery.rs`'s name parsing), so no value needs escaping today; this is the boundary that keeps the log well-formed if that changes.
fn json_escape(value: &str) -> String {
    const HEX: [char; 16] = [
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
    ];
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            control if (control as u32) < 0x20 => {
                escaped.push_str("\\u");
                for shift in (0..4).rev() {
                    let nibble = ((control as u32) >> (4 * shift)) & 0xF;
                    escaped.push(HEX[nibble as usize]);
                }
            }
            plain => escaped.push(plain),
        }
    }
    escaped
}

const fn age_days(now_ms: i64, end_ms: i64) -> i64 {
    now_ms.saturating_sub(end_ms).div_euclid(DAY_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000_000;

    fn piece(camera: &str, seq: u64, end_ms: i64) -> PieceRow {
        PieceRow {
            camera: camera.to_owned(),
            seq,
            start_ms: end_ms - 1_000,
            end_ms,
            duration_s: 1.0,
            path: format!(
                "recordings/{camera}/2026-11-21/00/{}-{seq}.mp4",
                end_ms - 1_000
            ),
            size_bytes: 10,
            codec: "h264".to_owned(),
        }
    }

    fn retention(entries: &[(&str, u32)]) -> HashMap<String, u32> {
        entries
            .iter()
            .map(|(camera, days)| ((*camera).to_owned(), *days))
            .collect()
    }

    fn aged(rows: &[(u64, i64)]) -> Vec<PieceRow> {
        rows.iter()
            .map(|(seq, days_ago)| piece("front", *seq, NOW - days_ago * DAY_MS))
            .collect()
    }

    #[test]
    fn a_piece_ending_exactly_at_the_cutoff_survives_and_one_millisecond_older_expires() {
        let days = retention(&[("front", 7)]);
        let cutoff = NOW - 7 * DAY_MS;
        let rows = vec![piece("front", 1, cutoff), piece("front", 2, cutoff - 1)];
        let plan = plan(&rows, NOW, &days);
        assert_eq!(plan.cameras.len(), 1);
        let camera = &plan.cameras[0];
        assert_eq!(
            camera
                .to_delete
                .iter()
                .map(|row| row.seq)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(camera.total_pieces, 2);
        assert!(!camera.guard_tripped);
    }

    #[test]
    fn zero_and_absent_retention_never_expire_and_a_camera_without_pieces_stays_out_of_the_plan() {
        let rows = aged(&[(1, 400), (2, 500)]);
        let zeroed = plan(&rows, NOW, &retention(&[("front", 0)]));
        assert_eq!(zeroed.cameras[0].to_delete.len(), 0);
        assert_eq!(zeroed.cameras[0].total_pieces, 2);
        assert!(!zeroed.cameras[0].guard_tripped);

        let absent = plan(&rows, NOW, &retention(&[("back", 7)]));
        assert_eq!(absent.cameras[0].to_delete.len(), 0);
        assert!(!absent.cameras[0].guard_tripped);

        let no_pieces = plan(&[], NOW, &retention(&[("front", 7)]));
        assert!(no_pieces.cameras.is_empty());
    }

    #[test]
    fn a_clock_jump_past_retention_with_a_full_expired_camera_trips_the_guard() {
        let rows = aged(&[(1, 1), (2, 2), (3, 3)]);
        let plan = plan(&rows, NOW + 400 * DAY_MS, &retention(&[("front", 7)]));
        let camera = &plan.cameras[0];
        assert!(camera.guard_tripped);
        assert!(camera.to_delete.is_empty());
        assert_eq!(camera.would_delete, 3);
        assert_eq!(camera.total_pieces, 3);
    }

    #[test]
    fn the_guard_allows_exactly_nine_of_ten_and_trips_at_ten_of_ten_and_nine_of_a_hundred_and_one()
    {
        let days = retention(&[("front", 7)]);
        let expired_rows = |count: usize| -> Vec<(u64, i64)> {
            (1..=u64::try_from(count).expect("test count fits"))
                .map(|seq| (seq, 400))
                .collect()
        };
        let mut rows = aged(&expired_rows(9));
        rows.push(piece("front", 10, NOW - DAY_MS));
        let nine_of_ten = plan(&rows, NOW, &days);
        assert!(
            !nine_of_ten.cameras[0].guard_tripped,
            "exactly 90 percent is allowed"
        );
        assert_eq!(nine_of_ten.cameras[0].to_delete.len(), 9);

        let ten_of_ten = plan(&aged(&expired_rows(10)), NOW, &days);
        assert!(ten_of_ten.cameras[0].guard_tripped, "every piece trips");

        let mut rows: Vec<PieceRow> = aged(&expired_rows(91));
        for seq in 92..=100 {
            rows.push(piece("front", seq, NOW - DAY_MS));
        }
        let ninety_one_of_hundred = plan(&rows, NOW, &days);
        assert!(
            ninety_one_of_hundred.cameras[0].guard_tripped,
            "91 of 100 trips"
        );
    }

    #[test]
    fn a_tripped_camera_never_trips_the_other_cameras() {
        let days = retention(&[("front", 7), ("back", 7)]);
        let mut rows = aged(&[(1, 400), (2, 400)]);
        rows.push(piece("back", 1, 0));
        rows.push(piece("back", 2, NOW - DAY_MS));
        let plan = plan(&rows, NOW, &days);
        let front = plan
            .cameras
            .iter()
            .find(|c| c.camera == "front")
            .expect("front planned");
        let back = plan
            .cameras
            .iter()
            .find(|c| c.camera == "back")
            .expect("back planned");
        assert!(front.guard_tripped);
        assert!(!back.guard_tripped);
        assert_eq!(back.to_delete.len(), 1);
    }

    #[test]
    fn huge_retention_windows_saturate_instead_of_wrapping() {
        let rows = aged(&[(1, 1)]);
        let plan = plan(&rows, i64::MIN, &retention(&[("front", 3650)]));
        assert_eq!(plan.cameras[0].to_delete.len(), 0);
    }

    #[test]
    fn a_max_clock_expires_without_wrapping_under_a_one_day_window() {
        let rows = aged(&[(1, 1)]);
        let camera = &plan(&rows, i64::MAX, &retention(&[("front", 1)])).cameras[0];
        assert_eq!(camera.would_delete, 1);
        assert!(
            camera.guard_tripped,
            "the only expired piece is the whole camera"
        );
    }

    #[test]
    fn the_guard_boundary_is_ninety_percent_plus_one_piece_across_several_totals() {
        let days = retention(&[("front", 7)]);
        for (total, expired, trips) in [
            (10usize, 9usize, false),
            (10, 10, true),
            (100, 91, true),
            (100, 90, false),
            (2, 1, false),
            (2, 2, true),
            (1, 1, true),
            (3, 0, false),
        ] {
            let mut rows: Vec<PieceRow> = Vec::new();
            let expired64 = u64::try_from(expired).expect("test count fits");
            let total64 = u64::try_from(total).expect("test count fits");
            for seq in 1..=expired64 {
                rows.push(piece("front", seq, NOW - 400 * DAY_MS));
            }
            for seq in (expired64 + 1)..=total64 {
                rows.push(piece("front", seq, NOW - DAY_MS));
            }
            let camera = &plan(&rows, NOW, &days).cameras[0];
            assert_eq!(camera.would_delete, expired, "expired {expired} of {total}");
            assert_eq!(
                camera.guard_tripped, trips,
                "expired {expired} of {total} must trip={trips}"
            );
        }
    }
}
