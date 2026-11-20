//! The detection service process: argument parsing, startup order and the serve loop over the status socket.
#![forbid(unsafe_code)]

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::FileTypeExt as _;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use corvette_detect::{
    CameraEntry, CameraPlan, CameraThreadSettings, CameraThreads, Config, DEFAULT_POLICY,
    ServiceState, SharedLink, Supervised, WorkerLink, revision, serve,
};
use tokio::process::Command;

const USAGE: &str = "\
usage: corvette-detect-service --config PATH --ffmpeg PROGRAM --restream-address HOST:PORT
           --worker PROGRAM --worker-param PATH --worker-bin PATH
           --worker-max-deadline-ms N --worker-health-socket PATH
           [--socket PATH] [--worker-device VALUE] [--frame-timeout-ms N]
       corvette-detect-service --help
exit codes:
     0  help
     1  the serve loop ended with an error
     2  usage error
     3  startup failure (configuration, camera plan, socket claim)
";

const EXIT_SERVE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_STARTUP: u8 = 3;
const DEFAULT_FRAME_TIMEOUT_MS: u64 = 10_000;
const MAX_FRAME_TIMEOUT_MS: u64 = 600_000;

#[derive(Debug)]
pub enum Invocation {
    Help,
    Serve(Box<Options>),
}

#[derive(Clone, Debug)]
pub struct Options {
    pub config: PathBuf,
    pub socket: PathBuf,
    pub ffmpeg: PathBuf,
    pub restream_address: String,
    pub worker: WorkerLaunch,
    pub frame_timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct WorkerLaunch {
    pub program: PathBuf,
    pub param: PathBuf,
    pub bin: PathBuf,
    pub max_deadline_ms: u32,
    pub health_socket: PathBuf,
    pub device: Option<String>,
}

#[derive(Default)]
struct FlagSlots {
    config: Option<PathBuf>,
    socket: Option<PathBuf>,
    ffmpeg: Option<PathBuf>,
    restream_address: Option<String>,
    worker: Option<PathBuf>,
    worker_param: Option<PathBuf>,
    worker_bin: Option<PathBuf>,
    worker_max_deadline_ms: Option<u32>,
    worker_health_socket: Option<PathBuf>,
    worker_device: Option<String>,
    frame_timeout_ms: Option<u64>,
}

/// Parses one command line; `env_socket` is the environment socket default used when no `--socket` flag is given.
///
/// # Errors
/// Usage text naming the unknown flag, the repeated flag, the missing value, the missing required flag or the refused value.
pub fn parse(args: &[OsString], env_socket: Option<&OsStr>) -> Result<Invocation, String> {
    if args.iter().any(|arg| arg.as_os_str() == "--help") {
        return Ok(Invocation::Help);
    }
    let mut slots = FlagSlots::default();
    let mut index = 0;
    while index < args.len() {
        let flag = text(&args[index]);
        match flag.as_str() {
            "--restream-address" => {
                set_restream_address(&mut slots.restream_address, argument(args, index, &flag)?)?;
            }
            "--worker-max-deadline-ms" => {
                set_max_deadline(
                    &mut slots.worker_max_deadline_ms,
                    argument(args, index, &flag)?,
                )?;
            }
            "--worker-device" => {
                set_text(
                    &mut slots.worker_device,
                    "worker-device",
                    argument(args, index, &flag)?,
                )?;
            }
            "--frame-timeout-ms" => {
                set_frame_timeout(&mut slots.frame_timeout_ms, argument(args, index, &flag)?)?;
            }
            name => {
                let slot = match name {
                    "--config" => &mut slots.config,
                    "--socket" => &mut slots.socket,
                    "--ffmpeg" => &mut slots.ffmpeg,
                    "--worker" => &mut slots.worker,
                    "--worker-param" => &mut slots.worker_param,
                    "--worker-bin" => &mut slots.worker_bin,
                    "--worker-health-socket" => &mut slots.worker_health_socket,
                    other => return Err(format!("unknown argument {other}")),
                };
                set_path(slot, &name[2..], argument(args, index, &flag)?)?;
            }
        }
        index += 2;
    }
    Ok(Invocation::Serve(Box::new(serve_options(
        slots, env_socket,
    )?)))
}

fn serve_options(slots: FlagSlots, env_socket: Option<&OsStr>) -> Result<Options, String> {
    Ok(Options {
        config: take_path(slots.config, "config")?,
        socket: take_socket(slots.socket, env_socket)?,
        ffmpeg: take_path(slots.ffmpeg, "ffmpeg")?,
        restream_address: slots
            .restream_address
            .ok_or("missing required flag --restream-address")?,
        worker: WorkerLaunch {
            program: take_path(slots.worker, "worker")?,
            param: take_path(slots.worker_param, "worker-param")?,
            bin: take_path(slots.worker_bin, "worker-bin")?,
            max_deadline_ms: slots
                .worker_max_deadline_ms
                .ok_or("missing required flag --worker-max-deadline-ms")?,
            health_socket: take_path(slots.worker_health_socket, "worker-health-socket")?,
            device: slots.worker_device,
        },
        frame_timeout: Duration::from_millis(
            slots.frame_timeout_ms.unwrap_or(DEFAULT_FRAME_TIMEOUT_MS),
        ),
    })
}

fn set_path(slot: &mut Option<PathBuf>, name: &str, value: &OsStr) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("flag --{name} given twice"));
    }
    *slot = Some(PathBuf::from(value));
    Ok(())
}

fn set_text(slot: &mut Option<String>, name: &str, value: &OsStr) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("flag --{name} given twice"));
    }
    *slot = Some(text(value));
    Ok(())
}

fn set_restream_address(slot: &mut Option<String>, value: &OsStr) -> Result<(), String> {
    let shown = text(value);
    if restream_address_ok(&shown).is_none() {
        return Err(format!(
            "flag --restream-address has value {shown}, expected HOST:PORT with a port between 1 and 65535"
        ));
    }
    if slot.is_some() {
        return Err("flag --restream-address given twice".to_string());
    }
    *slot = Some(shown);
    Ok(())
}

fn set_max_deadline(slot: &mut Option<u32>, value: &OsStr) -> Result<(), String> {
    let shown = text(value);
    match shown.parse::<u32>() {
        Ok(0) | Err(_) => Err(format!(
            "flag --worker-max-deadline-ms has value {shown}, expected an integer of at least 1"
        )),
        Ok(number) => {
            if slot.is_some() {
                return Err("flag --worker-max-deadline-ms given twice".to_string());
            }
            *slot = Some(number);
            Ok(())
        }
    }
}

fn set_frame_timeout(slot: &mut Option<u64>, value: &OsStr) -> Result<(), String> {
    let shown = text(value);
    let found = shown.parse::<u64>().unwrap_or(0);
    if !(1..=MAX_FRAME_TIMEOUT_MS).contains(&found) {
        return Err(format!(
            "flag --frame-timeout-ms has value {shown}, expected an integer between 1 and {MAX_FRAME_TIMEOUT_MS}"
        ));
    }
    if slot.is_some() {
        return Err("flag --frame-timeout-ms given twice".to_string());
    }
    *slot = Some(found);
    Ok(())
}

fn restream_address_ok(shown: &str) -> Option<&str> {
    let (host, port) = shown.split_once(':')?;
    if host.is_empty() || port.contains(':') {
        return None;
    }
    let port = port.parse::<u16>().ok()?;
    (port > 0).then_some(shown)
}

fn take_path(slot: Option<PathBuf>, name: &str) -> Result<PathBuf, String> {
    slot.ok_or_else(|| format!("missing required flag --{name}"))
}

fn take_socket(slot: Option<PathBuf>, env_socket: Option<&OsStr>) -> Result<PathBuf, String> {
    match (slot, env_socket) {
        (Some(path), _) => Ok(path),
        (None, Some(value)) => Ok(PathBuf::from(value)),
        (None, None) => Err("missing required flag --socket".to_string()),
    }
}

fn argument<'a>(args: &'a [OsString], at: usize, flag: &str) -> Result<&'a OsStr, String> {
    args.get(at + 1)
        .map(OsString::as_os_str)
        .ok_or_else(|| format!("flag {flag} needs a value"))
}

fn text(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

/// Runs one startup sequence in order: parse, config read, camera plans, worker child, camera threads, socket claim, then serve until the loop returns.
///
/// The worker child and every started camera thread run until the process ends; the serve loop returns only when `accept` fails.
#[must_use]
pub fn run(args: &[OsString], env_socket: Option<&OsStr>) -> ExitCode {
    let options = match parse(args, env_socket) {
        Err(message) => return usage_error(&message),
        Ok(Invocation::Help) => {
            let _ = std::io::stdout().write_all(USAGE.as_bytes());
            return ExitCode::SUCCESS;
        }
        Ok(Invocation::Serve(options)) => options,
    };
    let (config, bytes) = match read_config(&options.config) {
        Ok(loaded) => loaded,
        Err(reason) => {
            log_event("service", "config-failed", &reason);
            return ExitCode::from(EXIT_STARTUP);
        }
    };
    let revision = revision(&bytes);
    log_event(
        "service",
        "config-loaded",
        format!("path={} revision={revision}", options.config.display()),
    );
    let plans = match plan_all(&config) {
        Ok(plans) => plans,
        Err(reason) => {
            log_event("service", "camera-plan-failed", reason);
            return ExitCode::from(EXIT_STARTUP);
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log_event("service", "runtime-failed", error);
            return ExitCode::from(EXIT_STARTUP);
        }
    };
    let _entered = runtime.enter();
    let worker = Supervised::spawn(
        "worker".to_owned(),
        DEFAULT_POLICY,
        worker_command(&options.worker, &config.worker_socket),
        |_child| {},
    );
    let started = match start_cameras(&options, &config, plans) {
        Ok(started) => started,
        Err(reason) => {
            log_event("service", "camera-start-failed", reason);
            return ExitCode::from(EXIT_STARTUP);
        }
    };
    let entries: Vec<(String, CameraEntry)> = started
        .iter()
        .map(|(name, threads)| (name.clone(), CameraEntry::Live(threads.handle())))
        .collect();
    let listener = match claim_socket(&options.socket) {
        Ok(listener) => listener,
        Err(reason) => {
            log_event("service", "socket-refused", reason);
            return ExitCode::from(EXIT_STARTUP);
        }
    };
    let listener = match listener
        .set_nonblocking(true)
        .and_then(|()| tokio::net::UnixListener::from_std(listener))
    {
        Ok(listener) => listener,
        Err(error) => {
            log_event(
                "service",
                "socket-refused",
                format!(
                    "{} could not enter the runtime: {error}",
                    options.socket.display()
                ),
            );
            return ExitCode::from(EXIT_STARTUP);
        }
    };
    log_event(
        "service",
        "socket-bound",
        format!("path={}", options.socket.display()),
    );
    let state = Arc::new(ServiceState::new(entries).with_worker(worker.subscribe()));
    match runtime.block_on(serve(listener, Arc::clone(&state))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log_event("service", "serve-ended", error);
            ExitCode::from(EXIT_SERVE)
        }
    }
}

fn plan_all(config: &Config) -> Result<Vec<CameraPlan>, String> {
    let mut plans = Vec::with_capacity(config.cameras.len());
    for camera in &config.cameras {
        match CameraPlan::new(config, camera) {
            Err(error) => return Err(format!("camera={} {error}", camera.name)),
            Ok(plan) => plans.push(plan),
        }
    }
    Ok(plans)
}

fn start_cameras(
    options: &Options,
    config: &Config,
    plans: Vec<CameraPlan>,
) -> Result<Vec<(String, CameraThreads)>, String> {
    let link = SharedLink::new(WorkerLink::new(
        config.worker_socket.clone(),
        config.deadline,
        DEFAULT_POLICY,
    ));
    let settings = CameraThreadSettings {
        frame_timeout: options.frame_timeout,
        backoff: DEFAULT_POLICY,
    };
    let mut started = Vec::with_capacity(plans.len());
    for plan in plans {
        let name = plan.name().to_owned();
        let make_command =
            plan.command_factory(options.ffmpeg.clone(), options.restream_address.clone());
        let (spec, clock) = (plan.spec(), plan.clock());
        match CameraThreads::start(
            &name,
            settings,
            spec,
            clock,
            make_command,
            &link,
            move |ids, infer| plan.pipeline(ids, infer),
        ) {
            Ok(camera_threads) => started.push((name, camera_threads)),
            Err(error) => return Err(format!("camera={name} {error}")),
        }
    }
    Ok(started)
}

fn read_config(path: &Path) -> Result<(Config, Vec<u8>), String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let shown = std::str::from_utf8(&bytes)
        .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?;
    let config =
        Config::from_toml(shown).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok((config, bytes))
}

fn worker_command(
    launch: &WorkerLaunch,
    worker_socket: &Path,
) -> impl FnMut() -> Command + Send + 'static {
    let launch = launch.clone();
    let worker_socket = worker_socket.to_path_buf();
    move || {
        let mut command = Command::new(&launch.program);
        command
            .arg("--param")
            .arg(&launch.param)
            .arg("--bin")
            .arg(&launch.bin)
            .arg("--socket")
            .arg(&worker_socket)
            .arg("--health-socket")
            .arg(&launch.health_socket)
            .arg("--max-deadline-ms")
            .arg(launch.max_deadline_ms.to_string());
        if let Some(device) = &launch.device {
            command.arg("--device").arg(device);
        }
        command
    }
}

// Same claim rule as the worker binary: a socket file at the path is removed then rebound; any other existing path is refused untouched.
fn claim_socket(path: &Path) -> Result<UnixListener, String> {
    let stale = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => true,
        Ok(meta) => {
            return Err(format!(
                "{} exists as {:?}",
                path.display(),
                meta.file_type()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(format!(
                "{} could not be inspected: {error}",
                path.display()
            ));
        }
    };
    if stale && let Err(error) = fs::remove_file(path) {
        return Err(format!("{} could not be removed: {error}", path.display()));
    }
    UnixListener::bind(path).map_err(|error| format!("{} bind failed: {error}", path.display()))
}

fn log_event(unit: &str, event: &str, detail: impl std::fmt::Display) {
    eprintln!("corvette-detect unit=\"{unit}\" event={event} detail=\"{detail}\"");
}

fn usage_error(message: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "corvette-detect-service: {message}");
    let _ = std::io::stderr().write_all(USAGE.as_bytes());
    ExitCode::from(EXIT_USAGE)
}
