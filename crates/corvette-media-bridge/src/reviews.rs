//! One camera's open review item: activity within the cutoff extends it, an alert upgrade happens at most once and never back, and split and sweep closes land at last activity plus the cutoff, decided without `SQLite`.

use crate::index::{ALERT_EVENT_CUTOFF_MS, DETECTION_EVENT_CUTOFF_MS};

pub const SEVERITY_ALERT: &str = "alert";
pub const SEVERITY_DETECTION: &str = "detection";

#[derive(Debug)]
pub struct OpenReview {
    pub id: i64,
    pub severity: String,
    pub last_activity_ts: i64,
}

#[derive(Debug)]
pub enum AggregationAction {
    Extend { upgrade_to_alert: bool },
    CloseThenOpen { closed_end_ts: i64 },
}

#[must_use]
pub const fn severity_for(is_alert: bool) -> &'static str {
    if is_alert {
        SEVERITY_ALERT
    } else {
        SEVERITY_DETECTION
    }
}

#[must_use]
pub fn cutoff_ms(severity: &str) -> i64 {
    if severity == SEVERITY_ALERT {
        ALERT_EVENT_CUTOFF_MS
    } else {
        DETECTION_EVENT_CUTOFF_MS
    }
}

#[must_use]
pub fn on_open(open: &OpenReview, activity_ts: i64, is_alert: bool) -> AggregationAction {
    let cutoff = cutoff_ms(&open.severity);
    if activity_ts - open.last_activity_ts > cutoff {
        AggregationAction::CloseThenOpen {
            closed_end_ts: open.last_activity_ts + cutoff,
        }
    } else {
        AggregationAction::Extend {
            upgrade_to_alert: is_alert && open.severity != SEVERITY_ALERT,
        }
    }
}

#[must_use]
pub fn sweep_close_ts(open: &OpenReview, now_ms: i64) -> Option<i64> {
    let cutoff = cutoff_ms(&open.severity);
    (now_ms - open.last_activity_ts > cutoff).then(|| open.last_activity_ts + cutoff)
}
