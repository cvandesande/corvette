//! Plans and enforces pressure deletion, oldest footage first: the pure [`plan_pressure`], the executing [`run_pressure_pass`], and their log vocabulary.

use crate::expiry::{self, ExpiryError, PassReport, StageWords, StepOutcome, delete_piece_staged};
use crate::index::{Index, PieceRow};
use crate::supervise::log_event;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::BuildHasher;
use std::path::Path;

const HOUR_S: u64 = 3600;
const HOUR_MS: i64 = 3_600_000;
const FALLBACK_BITS_PER_SECOND: u64 = 8_000_000;
const PRESSURE_WORDS: StageWords = StageWords {
    reason: "pressure",
    step_event: "pressure-step-failed",
};
const WOULD_DELETE_EVENT: &str = "pressure-would-delete";
const ALERT_EVENT: &str = "pressure-alert";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureReason {
    Expired,
    WithinRetention,
}

#[derive(Debug, Clone)]
pub struct PressureDeletion {
    pub row: PieceRow,
    pub reason: PressureReason,
}

#[derive(Debug, Clone)]
pub struct CameraAlert {
    pub camera: String,
    pub days_held: i64,
    pub configured_days: u32,
    pub deleted: usize,
}

#[derive(Debug, Clone)]
pub struct PressurePlan {
    pub floor_bytes: u64,
    pub hard_floor_bytes: u64,
    pub deletions: Vec<PressureDeletion>,
    pub alerts: Vec<CameraAlert>,
}

fn oldest_first(rows: &mut [&PieceRow]) {
    rows.sort_by(|a, b| {
        (a.end_ms, a.start_ms, a.camera.as_str(), a.seq).cmp(&(
            b.end_ms,
            b.start_ms,
            b.camera.as_str(),
            b.seq,
        ))
    });
}

fn fallback_floor(recording_cameras: usize) -> u64 {
    let cameras = u64::try_from(recording_cameras).unwrap_or(u64::MAX);
    (FALLBACK_BITS_PER_SECOND / 8)
        .saturating_mul(HOUR_S)
        .saturating_mul(cameras)
}

fn floor_bytes(rows: &[PieceRow], now_ms: i64, recording_cameras: usize) -> u64 {
    let hour_start = now_ms.saturating_sub(HOUR_MS);
    let oldest_start = rows.iter().map(|row| row.start_ms).min();
    match oldest_start {
        Some(oldest) if oldest <= hour_start => rows
            .iter()
            .filter(|row| row.end_ms > hour_start)
            .fold(0u64, |sum, row| sum.saturating_add(row.size_bytes)),
        _ => fallback_floor(recording_cameras),
    }
}

fn build_alerts<S: BuildHasher>(
    rows: &[PieceRow],
    deletions: &[PressureDeletion],
    now_ms: i64,
    retention_days: &HashMap<String, u32, S>,
) -> Vec<CameraAlert> {
    let mut taken: BTreeMap<&str, usize> = BTreeMap::new();
    for deletion in deletions {
        if deletion.reason == PressureReason::WithinRetention {
            *taken.entry(deletion.row.camera.as_str()).or_default() += 1;
        }
    }
    let planned: BTreeSet<(&str, u64)> = deletions
        .iter()
        .map(|deletion| (deletion.row.camera.as_str(), deletion.row.seq))
        .collect();
    let mut alerts = Vec::new();
    for (camera, deleted) in taken {
        let oldest_remaining = rows
            .iter()
            .filter(|row| row.camera == camera && !planned.contains(&(camera, row.seq)))
            .map(|row| row.start_ms)
            .min();
        let Some(oldest) = oldest_remaining else {
            continue;
        };
        alerts.push(CameraAlert {
            camera: camera.to_owned(),
            days_held: expiry::age_days(now_ms, oldest),
            configured_days: retention_days.get(camera).copied().unwrap_or(0),
            deleted,
        });
    }
    alerts
}

/// Plans the pieces one pressure pass takes from `rows` at `now_ms`.
///
/// `retention_days` is per camera (absent or zero means the camera never expires), `recording_cameras` feeds the fallback floor, and `free_bytes` is the estimated free space, which grows by each planned piece's `size_bytes`. Pieces keyed in `held` are review-held: the expired stage never takes them, and the within-retention stage takes them at the hard floor like any other within-retention piece, because freeing space wins there. The floor is one hour of the measured last-hour recording rate, falling back to the fallback rate times `recording_cameras` until an hour of history exists, and the hard floor is a quarter of it; below the floor expired pieces go oldest-first until the estimate reaches it or none remain, and still below the hard floor after that within-retention pieces go oldest-first across cameras, never one inside a camera's newest hour. Saturating throughout, so no input panics. Each alert carries `days_held`, whole days from its camera's oldest remaining piece to `now`, and `configured_days`, that camera's retention with zero meaning never expires.
#[must_use]
pub fn plan_pressure<S: BuildHasher, T: BuildHasher>(
    rows: &[PieceRow],
    now_ms: i64,
    retention_days: &HashMap<String, u32, S>,
    held: &HashSet<(String, u64), T>,
    recording_cameras: usize,
    free_bytes: u64,
) -> PressurePlan {
    let floor_bytes = floor_bytes(rows, now_ms, recording_cameras);
    let hard_floor_bytes = floor_bytes / 4;
    let expired = |row: &PieceRow| {
        expiry::cutoff_ms(now_ms, retention_days.get(&row.camera).copied())
            .is_some_and(|cutoff| row.end_ms < cutoff)
    };
    let is_held = |row: &PieceRow| held.contains(&(row.camera.clone(), row.seq));
    let mut estimate = free_bytes;
    let mut deletions: Vec<PressureDeletion> = Vec::new();
    if estimate < floor_bytes {
        let mut oldest: Vec<&PieceRow> = rows
            .iter()
            .filter(|row| expired(row) && !is_held(row))
            .collect();
        oldest_first(&mut oldest);
        for row in oldest {
            if estimate >= floor_bytes {
                break;
            }
            estimate = estimate.saturating_add(row.size_bytes);
            deletions.push(PressureDeletion {
                row: (*row).clone(),
                reason: PressureReason::Expired,
            });
        }
    }
    if estimate < hard_floor_bytes {
        let mut newest_end: BTreeMap<&str, i64> = BTreeMap::new();
        for row in rows {
            let newest = newest_end.entry(row.camera.as_str()).or_insert(i64::MIN);
            *newest = (*newest).max(row.end_ms);
        }
        let mut oldest: Vec<&PieceRow> = rows
            .iter()
            .filter(|row| !expired(row) || is_held(row))
            .filter(|row| {
                newest_end
                    .get(row.camera.as_str())
                    .is_some_and(|newest| row.end_ms <= newest.saturating_sub(HOUR_MS))
            })
            .collect();
        oldest_first(&mut oldest);
        for row in oldest {
            if estimate >= hard_floor_bytes {
                break;
            }
            estimate = estimate.saturating_add(row.size_bytes);
            deletions.push(PressureDeletion {
                row: (*row).clone(),
                reason: PressureReason::WithinRetention,
            });
        }
    }
    let alerts = build_alerts(rows, &deletions, now_ms, retention_days);
    PressurePlan {
        floor_bytes,
        hard_floor_bytes,
        deletions,
        alerts,
    }
}

/// The free-space estimate and enabled-camera count that size one pressure pass's floor.
#[derive(Debug, Clone, Copy)]
pub struct PressureBudget {
    pub recording_cameras: usize,
    pub free_bytes: u64,
}

/// Runs one pressure pass at `now_ms` over `index`'s rows rooted at `root`, deciding with [`plan_pressure`].
///
/// Dry run logs `pressure-would-delete` and touches nothing; enforce deletes through `crate::expiry::delete_piece_staged` with deletion-log reason `pressure` and logs one `pressure-alert` per camera that actually lost within-retention pieces. The delete-everything guard does not apply here; the newest-hour rule bounds the pass.
///
/// # Errors
/// [`ExpiryError::ReadRows`] when the rows cannot be read; per-piece failures are counted in the report.
pub fn run_pressure_pass<S: BuildHasher, T: BuildHasher>(
    root: &Path,
    index: &mut Index,
    now_ms: i64,
    retention_days: &HashMap<String, u32, S>,
    event_retention_days: &HashMap<String, u32, T>,
    budget: PressureBudget,
    enforce: bool,
) -> Result<PassReport, ExpiryError> {
    let rows = index
        .all_pieces()
        .map_err(|source| ExpiryError::ReadRows { source })?;
    let floors = expiry::camera_floors(now_ms, retention_days, event_retention_days);
    let held = expiry::held_from_floors(index, &floors)
        .map_err(|source| ExpiryError::ReadRows { source })?;
    let plan = plan_pressure(
        &rows,
        now_ms,
        retention_days,
        &held,
        budget.recording_cameras,
        budget.free_bytes,
    );
    let mut report = PassReport {
        enforce,
        would_delete: plan.deletions.len(),
        ..PassReport::default()
    };
    if !enforce {
        for deletion in &plan.deletions {
            log_event(
                &deletion.row.camera,
                WOULD_DELETE_EVENT,
                format!(
                    "seq={} size_bytes={}",
                    deletion.row.seq, deletion.row.size_bytes
                ),
            );
        }
        return Ok(report);
    }
    let mut within_deleted: HashMap<&str, usize> = HashMap::new();
    for deletion in &plan.deletions {
        let outcome = delete_piece_staged(
            root,
            index,
            &deletion.row,
            now_ms,
            PRESSURE_WORDS,
            &mut report,
            0,
        );
        if deletion.reason == PressureReason::WithinRetention
            && matches!(outcome, StepOutcome::Done)
        {
            *within_deleted
                .entry(deletion.row.camera.as_str())
                .or_default() += 1;
        }
    }
    for alert in &plan.alerts {
        let Some(deleted) = within_deleted.get(alert.camera.as_str()) else {
            continue;
        };
        log_event(
            &alert.camera,
            ALERT_EVENT,
            format!(
                "days_held={} configured_days={} deleted={}",
                alert.days_held, alert.configured_days, deleted
            ),
        );
    }
    Ok(report)
}
