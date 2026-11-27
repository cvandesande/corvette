//! The camera on and off state: persistent per-camera threads behind the status entries, one async toggle lock per camera, and the `POST /corvette/api/v1/cameras/<name>/enabled` answers.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tokio::task::spawn_blocking;

use crate::camera_plan::{CameraPlan, PlanError, StreamAddress};
use crate::camera_thread::{
    CameraChannels, CameraThreadSettings, CameraThreads, JoinReport, ProducerPlan, StartError,
    ThreadEnd,
};
use crate::config::{CameraConfig, Config};
use crate::config_routes::{header, remote_user_is_auditable, unauthorized};
use crate::log_event;
use crate::object_stage::CameraObjects;
use crate::record::json_string;
use crate::service_socket::{CameraEntry, ServiceState, http_response, not_found};
use crate::shared_link::SharedLink;

// The join budget when the reader gave none: the same proposal the service's shutdown uses.
pub const DEFAULT_TOGGLE_JOIN_TIMEOUT: Duration = Duration::from_secs(15);

const LATE_STOP_POLL: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub enum RestartFailure {
    Plan(PlanError),
    Thread(StartError),
}

impl std::fmt::Display for RestartFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(error) => write!(f, "{error}"),
            Self::Thread(error) => write!(f, "{error}"),
        }
    }
}

/// The inputs that restart one camera's threads: the plan, paths, stream address and worker link as the process started with, so a restart never picks up a configuration applied mid-run.
pub struct CameraRecipe {
    config: Config,
    camera: CameraConfig,
    ffmpeg: PathBuf,
    address: StreamAddress,
    link: SharedLink,
    settings: CameraThreadSettings,
}

impl std::fmt::Debug for CameraRecipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CameraRecipe")
            .field("camera", &self.camera.name)
            .finish_non_exhaustive()
    }
}

impl CameraRecipe {
    #[must_use]
    pub const fn new(
        config: Config,
        camera: CameraConfig,
        ffmpeg: PathBuf,
        address: StreamAddress,
        link: SharedLink,
        settings: CameraThreadSettings,
    ) -> Self {
        Self {
            config,
            camera,
            ffmpeg,
            address,
            link,
            settings,
        }
    }

    fn start(&self, channels: &CameraChannels) -> Result<CameraThreads, RestartFailure> {
        let plan = CameraPlan::new(&self.config, &self.camera).map_err(RestartFailure::Plan)?;
        let name = plan.name().to_owned();
        let make_command = plan.command_factory(self.ffmpeg.clone(), &self.address);
        CameraThreads::start(
            &name,
            self.settings,
            channels,
            ProducerPlan {
                spec: plan.spec(),
                clock: plan.clock(),
            },
            make_command,
            &self.link,
            move |ids, infer| plan.pipeline(ids, infer),
        )
        .map_err(RestartFailure::Thread)
    }
}

/// One camera as the routes see it: its channels and recipe, and the lock that serializes this camera's toggles; the entry and threads live in `state` behind a short std lock, taken and released with no await inside.
pub struct CameraSlot {
    pub(super) name: String,
    pub(super) channels: Option<CameraChannels>,
    pub(super) recipe: Option<Arc<CameraRecipe>>,
    pub(super) toggle: tokio::sync::Mutex<()>,
    pub(super) stopping: AtomicBool,
    pub(super) state: Mutex<SlotState>,
}

#[derive(Debug)]
pub struct SlotState {
    pub(super) entry: CameraEntry,
    pub(super) threads: Option<CameraThreads>,
}

impl std::fmt::Debug for CameraSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CameraSlot")
            .field("name", &self.name)
            .field(
                "entry",
                &self.state.lock().map_or("poisoned", |s| match &s.entry {
                    CameraEntry::Off => "off",
                    CameraEntry::Live(_) => "live",
                }),
            )
            .finish_non_exhaustive()
    }
}

pub fn slot_state(slot: &Arc<CameraSlot>) -> MutexGuard<'_, SlotState> {
    // A panic here could only interrupt these plain slot moves, so the state stays usable and reclaiming it beats failing every later route.
    slot.state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug)]
pub struct CameraStartup {
    pub name: String,
    pub channels: CameraChannels,
    pub recipe: CameraRecipe,
    pub enabled: bool,
}

/// A cloneable handle to the camera toggle state, held by the ipc listener and by shutdown.
#[derive(Clone)]
pub struct ToggleService {
    cameras: Arc<Mutex<Vec<Arc<CameraSlot>>>>,
    closing: Arc<AtomicBool>,
}

impl std::fmt::Debug for ToggleService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToggleService").finish_non_exhaustive()
    }
}

impl ToggleService {
    /// Ends every camera and refuses new toggles: the closing flag lands first, then each camera's toggle lock is held while its threads stop and join, so a toggle in progress is joined rather than missed and no start lands after the stop.
    pub async fn close(&self, budget: Duration) {
        self.closing.store(true, Ordering::SeqCst);
        let slots: Vec<Arc<CameraSlot>> = cameras_locked(&self.cameras).clone();
        let mut held = Vec::with_capacity(slots.len());
        for slot in &slots {
            held.push(slot.toggle.lock().await);
        }
        let deadline = Instant::now() + budget;
        let count = slots.len();
        let mut taken = Vec::new();
        for slot in &slots {
            if let Some(threads) = take_threads(slot) {
                threads.stop();
                taken.push((slot.name.clone(), threads));
            }
        }
        for (name, threads) in taken {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match spawn_blocking(move || threads.wait_ends(remaining)).await {
                Ok(report) => report_camera(&name, report),
                Err(error) => log_event("service", "join-task-failed", error),
            }
        }
        drop(held);
        log_event("service", "cameras-stopped", format!("count={count}"));
    }
}

impl ServiceState {
    /// Replaces the plain camera entries with the full toggle state: one slot per startup, each Off until `start_camera` starts the ones the configuration file enables.
    #[must_use]
    pub fn with_cameras(self, startups: Vec<CameraStartup>) -> Self {
        let slots: Vec<Arc<CameraSlot>> = startups
            .into_iter()
            .map(|startup| {
                Arc::new(CameraSlot {
                    name: startup.name,
                    channels: Some(startup.channels),
                    recipe: Some(Arc::new(startup.recipe)),
                    toggle: tokio::sync::Mutex::new(()),
                    stopping: AtomicBool::new(false),
                    state: Mutex::new(SlotState {
                        entry: CameraEntry::Off,
                        threads: None,
                    }),
                })
            })
            .collect();
        *self.cameras_locked() = slots;
        self
    }

    #[must_use]
    pub fn toggles(&self) -> ToggleService {
        ToggleService {
            cameras: Arc::clone(&self.cameras),
            closing: Arc::clone(&self.closing),
        }
    }

    #[must_use]
    pub fn camera_object_receivers(&self) -> Vec<(String, watch::Receiver<CameraObjects>)> {
        self.camera_slots()
            .into_iter()
            .filter_map(|slot| {
                let channels = slot.channels.as_ref()?;
                Some((slot.name.clone(), channels.objects_receiver()))
            })
            .collect()
    }

    pub(crate) fn camera_slots(&self) -> Vec<Arc<CameraSlot>> {
        cameras_locked(&self.cameras).clone()
    }

    /// Starts one camera from its recipe on the calling thread; startup only, before any listener can toggle.
    ///
    /// # Errors
    /// The restart failure's text when the plan or a thread refuses.
    pub fn start_camera(&self, name: &str) -> Result<(), String> {
        let slot = self
            .slot_for(name)
            .ok_or_else(|| format!("camera={name} is not a configured camera"))?;
        let (Some(recipe), Some(channels)) = (slot.recipe.clone(), slot.channels.clone()) else {
            return Err(format!("camera={name} has no restart recipe"));
        };
        let threads = recipe
            .start(&channels)
            .map_err(|failure| format!("camera={name} {failure}"))?;
        let mut state = slot_state(&slot);
        state.threads = Some(threads);
        state.entry = CameraEntry::Live(channels.handle());
        drop(state);
        Ok(())
    }

    /// Stops and joins every camera that has threads, joining each with the time left of one shared `budget`; startup failure cleanup, before any listener runs.
    pub fn stop_started(&self, budget: Duration) {
        let deadline = Instant::now() + budget;
        let slots = self.camera_slots();
        let count = slots.len();
        let mut taken = Vec::new();
        for slot in &slots {
            if let Some(threads) = take_threads(slot) {
                threads.stop();
                taken.push((slot.name.clone(), threads));
            }
        }
        for (name, threads) in taken {
            let remaining = deadline.saturating_duration_since(Instant::now());
            report_camera(&name, threads.join(remaining));
        }
        log_event("service", "cameras-stopped", format!("count={count}"));
    }

    /// Answers `POST /corvette/api/v1/cameras/<name>/enabled`; `route` has checked the path shape, method, body length and listener already.
    pub(crate) async fn enabled_answer(
        &self,
        target: &str,
        camera: &str,
        head: &[u8],
        body: &[u8],
    ) -> String {
        let Some(enabled) = enabled_request(body) else {
            return bad_enabled_body();
        };
        let Some(user) = header(head, "Remote-User").filter(|user| remote_user_is_auditable(user))
        else {
            return unauthorized(target);
        };
        let Some(slot) = self.slot_for(camera) else {
            return not_found();
        };
        let _toggle = slot.toggle.lock().await;
        if self.closing.load(Ordering::SeqCst) {
            return reason_answer(503, "stopping");
        }
        let live = matches!(slot_state(&slot).entry, CameraEntry::Live(_));
        if live == enabled {
            return toggle_answer(camera, enabled);
        }
        if enabled {
            self.toggle_on(&slot, camera, user).await
        } else {
            self.toggle_off(&slot, camera, user).await
        }
    }

    pub(crate) fn slot_for(&self, name: &str) -> Option<Arc<CameraSlot>> {
        cameras_locked(&self.cameras)
            .iter()
            .find(|slot| slot.name == name)
            .cloned()
    }

    pub(crate) fn cameras_locked(&self) -> MutexGuard<'_, Vec<Arc<CameraSlot>>> {
        cameras_locked(&self.cameras)
    }

    async fn toggle_off(&self, slot: &Arc<CameraSlot>, camera: &str, user: &str) -> String {
        let Some(threads) = take_threads(slot) else {
            unreachable!("camera={camera} is live without threads")
        };
        let budget = self.stop_timeout;
        let timed_out = match spawn_blocking(move || {
            threads.stop();
            (threads.wait_ends(budget), threads)
        })
        .await
        {
            Ok((report, threads)) => {
                let timed_out =
                    report.runner == ThreadEnd::TimedOut || report.producer == ThreadEnd::TimedOut;
                if timed_out {
                    slot.stopping.store(true, Ordering::SeqCst);
                    let finished = Arc::clone(slot);
                    let channels = slot.channels.clone();
                    let name = camera.to_owned();
                    let closing = Arc::clone(&self.closing);
                    // A runtime drop waits for blocking tasks, so this wait stops when shutdown starts and the camera stays recorded as stopping.
                    tokio::task::spawn_blocking(move || {
                        while !closing.load(Ordering::SeqCst) {
                            let report = threads.wait_ends(LATE_STOP_POLL);
                            if report.runner == ThreadEnd::TimedOut
                                || report.producer == ThreadEnd::TimedOut
                            {
                                continue;
                            }
                            report_camera(&name, report);
                            if let Some(channels) = channels {
                                channels.mark_stopped();
                            }
                            finished.stopping.store(false, Ordering::SeqCst);
                            return;
                        }
                    });
                }
                timed_out
            }
            Err(error) => {
                log_event("service", "toggle-stop-task-failed", error);
                false
            }
        };
        if !timed_out && let Some(channels) = &slot.channels {
            channels.mark_stopped();
        }
        audit_toggle(user, camera, false);
        toggle_answer(camera, false)
    }

    async fn toggle_on(&self, slot: &Arc<CameraSlot>, camera: &str, user: &str) -> String {
        if slot.stopping.load(Ordering::SeqCst) {
            return reason_answer(503, "stopping");
        }
        let Some(recipe) = slot.recipe.clone() else {
            log_event(
                "service",
                "toggle-state-broken",
                format!("camera={camera} has no restart recipe"),
            );
            return reason_answer(503, "start_failed");
        };
        let Some(channels) = slot.channels.clone() else {
            return reason_answer(503, "start_failed");
        };
        let start_channels = channels.clone();
        match spawn_blocking(move || recipe.start(&start_channels)).await {
            Err(error) => {
                log_event("service", "toggle-start-task-failed", error);
                reason_answer(503, "start_failed")
            }
            Ok(Err(failure)) => {
                log_event(
                    "service",
                    "camera-start-failed",
                    format!("camera={camera} {failure}"),
                );
                reason_answer(503, "start_failed")
            }
            Ok(Ok(threads)) => {
                let mut state = slot_state(slot);
                state.threads = Some(threads);
                state.entry = CameraEntry::Live(channels.handle());
                drop(state);
                audit_toggle(user, camera, true);
                toggle_answer(camera, true)
            }
        }
    }
}

fn cameras_locked(
    cameras: &Arc<Mutex<Vec<Arc<CameraSlot>>>>,
) -> MutexGuard<'_, Vec<Arc<CameraSlot>>> {
    // A panic here could only interrupt these plain slot moves, so the table stays usable and reclaiming it beats failing every later route.
    cameras
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn take_threads(slot: &Arc<CameraSlot>) -> Option<CameraThreads> {
    let mut state = slot_state(slot);
    state.entry = CameraEntry::Off;
    state.threads.take()
}

fn report_camera(name: &str, report: JoinReport) {
    let timed_out = report.runner == ThreadEnd::TimedOut || report.producer == ThreadEnd::TimedOut;
    let event = if timed_out {
        "stop-timeout"
    } else if report.runner == ThreadEnd::Panicked || report.producer == ThreadEnd::Panicked {
        "camera-thread-panicked"
    } else {
        return;
    };
    let word = |end: ThreadEnd| match end {
        ThreadEnd::Finished => "finished",
        ThreadEnd::Panicked => "panicked",
        ThreadEnd::TimedOut => "timed_out",
    };
    log_event(
        "service",
        event,
        format!(
            "camera={name} runner={} producer={}",
            word(report.runner),
            word(report.producer)
        ),
    );
}

fn enabled_request(body: &[u8]) -> Option<bool> {
    let value = serde_json::from_slice::<serde_json::Value>(body).ok()?;
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    object.get("enabled")?.as_bool()
}

fn bad_enabled_body() -> String {
    http_response(400, "Bad Request", "{\"reason\":\"bad_request\"}", None)
}

fn reason_answer(status: u16, reason: &str) -> String {
    let phrase = match status {
        503 => "Service Unavailable",
        _ => "Error",
    };
    http_response(
        status,
        phrase,
        &format!("{{\"reason\":\"{reason}\"}}"),
        None,
    )
}

fn toggle_answer(camera: &str, enabled: bool) -> String {
    http_response(
        200,
        "OK",
        &format!(
            "{{\"camera\":{},\"enabled\":{}}}",
            json_string(camera),
            enabled
        ),
        None,
    )
}

fn audit_toggle(user: &str, camera: &str, enabled: bool) {
    log_event(
        "service",
        "camera-toggled",
        format!("user=\"{user}\" camera={camera} enabled={enabled}"),
    );
}
