//! Recording-availability vocabulary shared by the event and recording calendars.

use leptos::prelude::*;

#[component]
pub(crate) fn ActivityLegend() -> impl IntoView {
    view! { <div class="activity-legend" aria-label="Activity colors">
        <span><i class="activity-motion"></i>"Motion"</span>
        <span><i class="activity-detection"></i>"Detection"</span>
        <span><i class="activity-alert"></i>"Alert"</span>
    </div> }
}

pub(crate) fn calendar_day_label(
    days: Option<Result<std::collections::BTreeMap<String, bool>, String>>,
    accessible_label: &str,
    date_key: &str,
) -> String {
    match days {
        None => format!("{accessible_label}, checking recording availability"),
        Some(Err(_)) => format!("{accessible_label}, recording availability unavailable"),
        Some(Ok(days)) if has_recording(&days, date_key) => {
            format!("{accessible_label}, recordings available")
        }
        Some(Ok(_)) => format!("{accessible_label}, no recordings"),
    }
}

pub(crate) fn recording_day_available(
    days: Option<Result<std::collections::BTreeMap<String, bool>, String>>,
    date_key: &str,
) -> bool {
    days.and_then(Result::ok)
        .is_some_and(|days| has_recording(&days, date_key))
}

#[component]
pub(crate) fn CalendarRecordingStatus(
    recording_days: LocalResource<Result<std::collections::BTreeMap<String, bool>, String>>,
    empty_detail: &'static str,
) -> impl IntoView {
    move || match recording_days.get() {
        None => {
            view! { <p class="calendar-status">"Checking recording availability…"</p> }.into_any()
        }
        Some(Err(error)) => {
            view! { <p class="recording-error" role="alert">{error}</p> }.into_any()
        }
        Some(Ok(days)) if days.is_empty() => {
            view! { <p class="calendar-status">{empty_detail}</p> }.into_any()
        }
        Some(Ok(_)) => {
            view! { <p class="calendar-status">"Dimmed dates have no retained recordings."</p> }
                .into_any()
        }
    }
}

fn has_recording(days: &std::collections::BTreeMap<String, bool>, date: &str) -> bool {
    days.get(date).copied() == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dates_marked_true_are_playable() {
        let days = std::collections::BTreeMap::from([
            ("2026-07-31".to_owned(), true),
            ("2026-08-01".to_owned(), false),
        ]);

        assert!(has_recording(&days, "2026-07-31"));
        assert!(!has_recording(&days, "2026-08-01"));
        assert!(!has_recording(&days, "2026-08-02"));
    }
}
