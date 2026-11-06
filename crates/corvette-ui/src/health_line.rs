//! The dashboard's one camera-status poll, read by the health line, the camera tiles and the System section.

use std::cell::RefCell;
use std::rc::Rc;

use corvette_api::{CameraEntry, CameraState, CameraStatus};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

const POLL_SECONDS: u32 = 30;
const AMBER_AFTER_SECONDS: f64 = 120.0;
const CLEAR_AFTER_HEALTHY_POLLS: u32 = 2;
const RETRY_FIRST_SECONDS: u32 = 5;
const RETRY_CEILING_SECONDS: u32 = 60;

#[derive(Clone, Debug, Default, PartialEq)]
enum HealthReading {
    /// No poll has finished yet, so the line has nothing to say.
    #[default]
    Checking,
    Unavailable {
        last_ok: Option<f64>,
    },
    Amber(Vec<HealthProblem>),
    Healthy {
        updated: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct HealthProblem {
    name: String,
    since: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct FlaggedCamera {
    name: String,
    since: f64,
    healthy_polls: u32,
}

#[derive(Default)]
struct HealthTracker {
    flagged: Vec<FlaggedCamera>,
    failed_polls: u32,
    last_success: Option<f64>,
    reading: HealthReading,
}

const fn is_fault(state: CameraState) -> bool {
    matches!(state, CameraState::Stale | CameraState::NoPicture)
}

impl HealthTracker {
    fn record_success(&mut self, now: f64, statuses: &[CameraStatus]) {
        self.failed_polls = 0;
        self.last_success = Some(now);

        let mut flagged: Vec<FlaggedCamera> = Vec::new();
        for camera in statuses {
            let previous = self.flagged.iter().find(|flag| flag.name == camera.name);
            if is_fault(camera.state) {
                if now - camera.since >= AMBER_AFTER_SECONDS || previous.is_some() {
                    flagged.push(FlaggedCamera {
                        name: camera.name.clone(),
                        since: camera.since,
                        healthy_polls: 0,
                    });
                }
            } else if let Some(previous) = previous {
                let healthy_polls = previous.healthy_polls + 1;
                if healthy_polls < CLEAR_AFTER_HEALTHY_POLLS {
                    flagged.push(FlaggedCamera {
                        name: previous.name.clone(),
                        since: previous.since,
                        healthy_polls,
                    });
                }
            }
        }
        self.flagged = flagged;

        self.reading = if self.flagged.is_empty() {
            HealthReading::Healthy { updated: now }
        } else {
            HealthReading::Amber(
                self.flagged
                    .iter()
                    .map(|flag| HealthProblem {
                        name: flag.name.clone(),
                        since: flag.since,
                    })
                    .collect(),
            )
        };
    }

    fn record_failure(&mut self) {
        self.failed_polls += 1;
        self.reading = HealthReading::Unavailable {
            last_ok: self.last_success,
        };
    }

    fn next_delay_seconds(&self) -> u32 {
        if self.failed_polls == 0 {
            return POLL_SECONDS;
        }
        RETRY_FIRST_SECONDS
            .saturating_mul(2u32.saturating_pow(self.failed_polls - 1))
            .min(RETRY_CEILING_SECONDS)
    }
}

/// The one camera-status reading, shared by the health line, the camera tiles and the System section.
#[derive(Clone, Copy)]
pub(crate) struct CameraStatusFeed {
    reading: RwSignal<HealthReading>,
    statuses: RwSignal<Vec<CameraStatus>>,
}

impl CameraStatusFeed {
    /// The time the named camera's persisted fault began, while the current reading flags that camera.
    pub(crate) fn problem_since(&self, name: &str) -> Option<f64> {
        match self.reading.get() {
            HealthReading::Amber(problems) => problems
                .into_iter()
                .find(|problem| problem.name == name)
                .map(|problem| problem.since),
            HealthReading::Checking
            | HealthReading::Healthy { .. }
            | HealthReading::Unavailable { .. } => None,
        }
    }
}

/// Called once by the dashboard; the poll starts when that scope mounts and stops when it is cleaned up.
pub(crate) fn provide_camera_status_feed() {
    let reading = RwSignal::new(HealthReading::Checking);
    let statuses = RwSignal::new(Vec::new());
    let poller = StoredValue::new_local(None::<Rc<RefCell<PollState>>>);

    Effect::new(move |_| {
        let started = Rc::new(RefCell::new(PollState {
            tracker: HealthTracker::default(),
            reading,
            statuses,
            timer: None,
        }));
        poller.set_value(Some(Rc::clone(&started)));
        poll_now(&started);
    });

    on_cleanup(move || {
        if let Some(Some(finished)) = poller.try_update_value(Option::take) {
            finished.borrow_mut().stop();
        }
    });

    provide_context(CameraStatusFeed { reading, statuses });
}

#[component]
pub(crate) fn HealthLine(cameras: Vec<CameraEntry>) -> impl IntoView {
    let feed = expect_context::<CameraStatusFeed>();

    view! {
        <div class="health-line-group" aria-live="polite">
            {move || match feed.reading.get() {
                HealthReading::Checking => ().into_any(),
                HealthReading::Healthy { updated } => view! {
                    <p class="health-line health-line-ok">{healthy_text(updated)}</p>
                }.into_any(),
                HealthReading::Amber(problems) => problems
                    .into_iter()
                    .map(|problem| view! {
                        <p class="health-line health-line-amber">{amber_text(&cameras, &problem)}</p>
                    })
                    .collect_view()
                    .into_any(),
                HealthReading::Unavailable { last_ok } => view! {
                    <p class="health-line health-line-offline">{offline_text(last_ok)}</p>
                }.into_any(),
            }}
        </div>
    }
}

#[component]
pub(crate) fn SystemCameras(cameras: Signal<Vec<CameraEntry>>) -> impl IntoView {
    let feed = expect_context::<CameraStatusFeed>();

    view! {
        {move || match feed.reading.get() {
            HealthReading::Checking => view! {
                <p class="system-note">"Checking cameras."</p>
            }.into_any(),
            HealthReading::Unavailable { last_ok } => view! {
                <p class="system-note system-note-offline">{offline_text(last_ok)}</p>
            }.into_any(),
            HealthReading::Healthy { .. } | HealthReading::Amber(_) => {
                let cameras = cameras.get();
                let rows = feed.statuses
                    .get()
                    .into_iter()
                    .map(|status| view! {
                        <li class="system-row" class:system-row-fault=is_fault(status.state)>
                            <span class="system-name">{display_name(&cameras, &status.name).to_owned()}</span>
                            <span class="system-state">{system_state_text(status.state, status.since)}</span>
                        </li>
                    })
                    .collect_view();
                view! { <ul class="system-list">{rows}</ul> }.into_any()
            }
        }}
    }
}

fn clock_hhmm(unix_seconds: f64) -> String {
    crate::local_time::format_clock_time(unix_seconds)
        .chars()
        .take(5)
        .collect()
}

fn healthy_text(updated: f64) -> String {
    format!(
        "All cameras that are on are recording. Updated {}.",
        clock_hhmm(updated)
    )
}

fn display_name<'a>(cameras: &'a [CameraEntry], name: &'a str) -> &'a str {
    cameras
        .iter()
        .find(|camera| camera.name == name)
        .map_or(name, |camera| camera.display_name.as_str())
}

fn amber_text(cameras: &[CameraEntry], problem: &HealthProblem) -> String {
    let display = display_name(cameras, &problem.name);
    format!(
        "{display} has had no picture since {}. Its live view and new recordings are paused.",
        clock_hhmm(problem.since)
    )
}

fn offline_text(last_ok: Option<f64>) -> String {
    last_ok.map_or_else(
        || "Page offline — not updated yet. Retrying.".to_owned(),
        |updated| {
            format!(
                "Page offline — last updated {}. Retrying.",
                clock_hhmm(updated)
            )
        },
    )
}

pub(crate) fn tile_state_text(since: f64) -> String {
    format!("No new picture since {}.", clock_hhmm(since))
}

fn system_state_text(state: CameraState, since: f64) -> String {
    match state {
        CameraState::Live => "Live".to_owned(),
        CameraState::OffByChoice => "Off by choice".to_owned(),
        CameraState::Stale => format!("Picture not updating since {}", clock_hhmm(since)),
        CameraState::NoPicture => format!("No picture since {}", clock_hhmm(since)),
    }
}

struct PollState {
    tracker: HealthTracker,
    reading: RwSignal<HealthReading>,
    statuses: RwSignal<Vec<CameraStatus>>,
    timer: Option<i32>,
}

impl PollState {
    fn stop(&mut self) {
        if let Some(window) = web_sys::window()
            && let Some(timer) = self.timer.take()
        {
            window.clear_timeout_with_handle(timer);
        }
    }
}

fn poll_now(poller: &Rc<RefCell<PollState>>) {
    let weak = Rc::downgrade(poller);
    leptos::task::spawn_local(async move {
        let outcome = crate::api::fetch_camera_status().await;
        let Some(alive) = weak.upgrade() else {
            return;
        };
        let now = js_sys::Date::now() / 1_000.0;
        let reading = {
            let mut state = alive.borrow_mut();
            match &outcome {
                Ok(statuses) => {
                    state.statuses.set(statuses.clone());
                    state.tracker.record_success(now, statuses);
                }
                // The offline line is the handling of a failed poll.
                Err(_) => state.tracker.record_failure(),
            }
            state.tracker.reading.clone()
        };
        alive.borrow().reading.set(reading);
        schedule_next(&alive);
    });
}

fn schedule_next(poller: &Rc<RefCell<PollState>>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let delay_ms = i32::try_from(
        poller
            .borrow()
            .tracker
            .next_delay_seconds()
            .saturating_mul(1_000),
    )
    .unwrap_or(i32::MAX);
    let weak = Rc::downgrade(poller);
    let fired = Closure::once_into_js(move || {
        if let Some(alive) = weak.upgrade() {
            poll_now(&alive);
        }
    });
    if let Ok(timer) = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(fired.unchecked_ref(), delay_ms)
    {
        poller.borrow_mut().timer = Some(timer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(name: &str, state: CameraState, since: f64) -> CameraStatus {
        CameraStatus {
            name: name.to_owned(),
            state,
            since,
        }
    }

    fn live(name: &str) -> CameraStatus {
        status(name, CameraState::Live, 0.0)
    }

    #[test]
    fn a_fault_is_amber_at_two_minutes_and_silent_just_under_it() {
        let mut tracker = HealthTracker::default();
        tracker.record_success(1_000.0, &[status("front", CameraState::Stale, 881.0)]);
        assert_eq!(tracker.reading, HealthReading::Healthy { updated: 1_000.0 });

        tracker.record_success(1_000.0, &[status("front", CameraState::Stale, 880.0)]);
        assert_eq!(
            tracker.reading,
            HealthReading::Amber(vec![HealthProblem {
                name: "front".to_owned(),
                since: 880.0,
            }])
        );
    }

    #[test]
    fn no_picture_faults_like_stale() {
        let mut tracker = HealthTracker::default();
        tracker.record_success(1_000.0, &[status("front", CameraState::NoPicture, 800.0)]);
        assert!(matches!(tracker.reading, HealthReading::Amber(_)));
    }

    #[test]
    fn a_camera_off_by_choice_is_never_a_fault() {
        let mut tracker = HealthTracker::default();
        tracker.record_success(
            100_000.0,
            &[status("living", CameraState::OffByChoice, 0.0)],
        );
        assert_eq!(
            tracker.reading,
            HealthReading::Healthy { updated: 100_000.0 }
        );
    }

    #[test]
    fn an_amber_line_clears_after_two_healthy_polls_and_a_fault_resets_the_count() {
        let mut tracker = HealthTracker::default();
        tracker.record_success(1_000.0, &[status("front", CameraState::Stale, 800.0)]);
        assert!(matches!(tracker.reading, HealthReading::Amber(_)));

        tracker.record_success(1_030.0, &[live("front")]);
        assert!(matches!(tracker.reading, HealthReading::Amber(_)));
        tracker.record_success(1_060.0, &[live("front")]);
        assert_eq!(tracker.reading, HealthReading::Healthy { updated: 1_060.0 });

        tracker.record_success(2_000.0, &[status("front", CameraState::Stale, 1_700.0)]);
        tracker.record_success(2_030.0, &[live("front")]);
        tracker.record_success(2_060.0, &[status("front", CameraState::Stale, 1_700.0)]);
        tracker.record_success(2_090.0, &[live("front")]);
        assert!(matches!(tracker.reading, HealthReading::Amber(_)));
        tracker.record_success(2_120.0, &[live("front")]);
        assert_eq!(tracker.reading, HealthReading::Healthy { updated: 2_120.0 });
    }

    #[test]
    fn a_flagged_camera_missing_from_the_answer_is_dropped() {
        let mut tracker = HealthTracker::default();
        tracker.record_success(1_000.0, &[status("front", CameraState::Stale, 800.0)]);
        tracker.record_success(1_030.0, &[live("garage")]);
        assert_eq!(tracker.reading, HealthReading::Healthy { updated: 1_030.0 });
    }

    #[test]
    fn a_failed_poll_shows_offline_and_a_success_ends_it() {
        let mut tracker = HealthTracker::default();
        tracker.record_failure();
        assert_eq!(
            tracker.reading,
            HealthReading::Unavailable { last_ok: None }
        );

        tracker.record_success(500.0, &[]);
        assert_eq!(tracker.reading, HealthReading::Healthy { updated: 500.0 });
        tracker.record_failure();
        assert_eq!(
            tracker.reading,
            HealthReading::Unavailable {
                last_ok: Some(500.0)
            }
        );
    }

    #[test]
    fn a_failed_poll_leaves_flags_in_place_and_healthy_polls_still_count_after_it() {
        let mut tracker = HealthTracker::default();
        tracker.record_success(1_000.0, &[status("front", CameraState::Stale, 800.0)]);
        tracker.record_failure();
        tracker.record_success(1_030.0, &[live("front")]);
        assert!(matches!(tracker.reading, HealthReading::Amber(_)));
        tracker.record_success(1_060.0, &[live("front")]);
        assert_eq!(tracker.reading, HealthReading::Healthy { updated: 1_060.0 });
    }

    #[test]
    fn retries_double_from_five_seconds_to_the_ceiling_and_a_success_restores_the_poll() {
        let mut tracker = HealthTracker::default();
        assert_eq!(tracker.next_delay_seconds(), POLL_SECONDS);
        for delay in [5, 10, 20, 40, 60, 60] {
            tracker.record_failure();
            assert_eq!(tracker.next_delay_seconds(), delay);
        }
        tracker.record_success(1_000.0, &[]);
        assert_eq!(tracker.next_delay_seconds(), POLL_SECONDS);
    }
}
