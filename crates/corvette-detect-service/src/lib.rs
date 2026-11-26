//! The detection service process: argument parsing, startup order and the serve loops over the status socket and the ipc socket.
#![forbid(unsafe_code)]

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{
    DirBuilderExt as _, FileTypeExt as _, MetadataExt as _, PermissionsExt as _,
};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Component, Path, PathBuf, absolute};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use corvette_detect::{
    CameraEntry, CameraPlan, CameraThreadSettings, CameraThreads, Config, DEFAULT_POLICY,
    FrameObjects, JoinReport, ServiceState, SharedLink, StreamAddress, Supervised, ThreadEnd,
    WorkerLink, revision, serve, serve_track_events, stream_address,
};
use tokio::process::Command;
use tokio::runtime::Runtime;
use tokio::signal::unix::{Signal, SignalKind};
use tokio::sync::watch;

const USAGE: &str = "\
usage: corvette-detect-service --config PATH --ffmpeg PROGRAM
           --worker PROGRAM --worker-param PATH --worker-bin PATH
           --worker-max-deadline-ms N
           [--socket PATH] [--ipc-socket PATH] [--track-events-socket PATH]
           [--restream-address HOST:PORT] [--worker-device VALUE]
           [--frame-timeout-ms N] [--stop-timeout-ms N]
       corvette-detect-service --probe [--socket PATH]
       --ipc-socket defaults to /run/corvette-detect/ipc/ipc.sock
       corvette-detect-service --help
       --restream-address is required when any camera has no source table
exit codes:
     0  help, or serve ended by SIGTERM or SIGINT
     1  probe failed or serve loop ended with an error
     2  usage error
     3  startup failure (configuration, camera plan, socket claim)
";

const EXIT_SERVE: u8 = 1;
const EXIT_PROBE_FAILED: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_STARTUP: u8 = 3;
const DEFAULT_FRAME_TIMEOUT_MS: u64 = 10_000;
const MAX_FRAME_TIMEOUT_MS: u64 = 600_000;
// Proposal until a measured camera stop time replaces this total shutdown budget.
const DEFAULT_STOP_TIMEOUT_MS: u64 = 15_000;
const MAX_STOP_TIMEOUT_MS: u64 = 600_000;
pub const DEFAULT_IPC_SOCKET: &str = "/run/corvette-detect/ipc/ipc.sock";
const PROBE_IO_TIMEOUT: Duration = Duration::from_secs(3);
const PROBE_REQUEST: &str = "GET /corvette/api/v1/detection/status HTTP/1.1\r\nHost: corvette-detect\r\nConnection: close\r\n\r\n";
// A peer that streams without a newline cannot grow the probe past this read cap.
const MAX_PROBE_ANSWER: usize = 8192;

#[derive(Debug)]
pub enum Invocation {
    Help,
    Probe { socket: PathBuf },
    Serve(Box<Options>),
}

#[derive(Clone, Debug)]
pub struct Options {
    pub config: PathBuf,
    pub socket: PathBuf,
    pub ipc_socket: PathBuf,
    pub track_events_socket: Option<PathBuf>,
    pub ffmpeg: PathBuf,
    pub restream_address: Option<String>,
    pub worker: WorkerLaunch,
    pub frame_timeout: Duration,
    pub stop_timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct WorkerLaunch {
    pub program: PathBuf,
    pub param: PathBuf,
    pub bin: PathBuf,
    pub max_deadline_ms: u32,
    pub device: Option<String>,
}

#[derive(Default)]
struct FlagSlots {
    config: Option<PathBuf>,
    socket: Option<PathBuf>,
    ipc_socket: Option<PathBuf>,
    track_events_socket: Option<PathBuf>,
    ffmpeg: Option<PathBuf>,
    restream_address: Option<String>,
    worker: Option<PathBuf>,
    worker_param: Option<PathBuf>,
    worker_bin: Option<PathBuf>,
    worker_max_deadline_ms: Option<u32>,
    worker_device: Option<String>,
    frame_timeout_ms: Option<u64>,
    stop_timeout_ms: Option<u64>,
}

impl FlagSlots {
    // The first serve-only flag set, used to refuse any of them beside --probe.
    fn extra_with_probe(&self) -> Option<&'static str> {
        [
            self.config.as_ref().map(|_| "config"),
            self.ipc_socket.as_ref().map(|_| "ipc-socket"),
            self.track_events_socket
                .as_ref()
                .map(|_| "track-events-socket"),
            self.ffmpeg.as_ref().map(|_| "ffmpeg"),
            self.restream_address.as_ref().map(|_| "restream-address"),
            self.worker.as_ref().map(|_| "worker"),
            self.worker_param.as_ref().map(|_| "worker-param"),
            self.worker_bin.as_ref().map(|_| "worker-bin"),
            self.worker_max_deadline_ms
                .as_ref()
                .map(|_| "worker-max-deadline-ms"),
            self.worker_device.as_ref().map(|_| "worker-device"),
            self.frame_timeout_ms.as_ref().map(|_| "frame-timeout-ms"),
            self.stop_timeout_ms.as_ref().map(|_| "stop-timeout-ms"),
        ]
        .into_iter()
        .flatten()
        .next()
    }
}

/// Parses one command line; `env_socket` and `env_ipc_socket` are the environment defaults used when the matching socket flag is absent.
///
/// # Errors
/// Usage text naming the unknown flag, the repeated flag, the missing value, the missing required flag or the refused value.
pub fn parse(
    args: &[OsString],
    env_socket: Option<&OsStr>,
    env_ipc_socket: Option<&OsStr>,
) -> Result<Invocation, String> {
    if args.iter().any(|arg| arg.as_os_str() == "--help") {
        return Ok(Invocation::Help);
    }
    let mut slots = FlagSlots::default();
    let mut saw_probe = false;
    let mut index = 0;
    while index < args.len() {
        let flag = text(&args[index]);
        match flag.as_str() {
            "--probe" => {
                if saw_probe {
                    return Err("flag --probe given twice".to_string());
                }
                saw_probe = true;
                index += 1;
                continue;
            }
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
                set_millisecond_flag(
                    &mut slots.frame_timeout_ms,
                    "frame-timeout-ms",
                    argument(args, index, &flag)?,
                    MAX_FRAME_TIMEOUT_MS,
                )?;
            }
            "--stop-timeout-ms" => {
                set_millisecond_flag(
                    &mut slots.stop_timeout_ms,
                    "stop-timeout-ms",
                    argument(args, index, &flag)?,
                    MAX_STOP_TIMEOUT_MS,
                )?;
            }
            name => {
                let slot = match name {
                    "--config" => &mut slots.config,
                    "--socket" => &mut slots.socket,
                    "--ipc-socket" => &mut slots.ipc_socket,
                    "--track-events-socket" => &mut slots.track_events_socket,
                    "--ffmpeg" => &mut slots.ffmpeg,
                    "--worker" => &mut slots.worker,
                    "--worker-param" => &mut slots.worker_param,
                    "--worker-bin" => &mut slots.worker_bin,
                    other => return Err(format!("unknown argument {other}")),
                };
                set_path(slot, &name[2..], argument(args, index, &flag)?)?;
            }
        }
        index += 2;
    }
    if saw_probe {
        if let Some(name) = slots.extra_with_probe() {
            return Err(format!("flag --{name} is not allowed with --probe"));
        }
        return Ok(Invocation::Probe {
            socket: take_socket(slots.socket, env_socket)?,
        });
    }
    Ok(Invocation::Serve(Box::new(serve_options(
        slots,
        env_socket,
        env_ipc_socket,
    )?)))
}

fn serve_options(
    slots: FlagSlots,
    env_socket: Option<&OsStr>,
    env_ipc_socket: Option<&OsStr>,
) -> Result<Options, String> {
    Ok(Options {
        config: take_path(slots.config, "config")?,
        socket: take_socket(slots.socket, env_socket)?,
        ipc_socket: take_ipc_socket(slots.ipc_socket, env_ipc_socket)?,
        track_events_socket: slots.track_events_socket,
        ffmpeg: take_path(slots.ffmpeg, "ffmpeg")?,
        restream_address: slots.restream_address,
        worker: WorkerLaunch {
            program: take_path(slots.worker, "worker")?,
            param: take_path(slots.worker_param, "worker-param")?,
            bin: take_path(slots.worker_bin, "worker-bin")?,
            max_deadline_ms: slots
                .worker_max_deadline_ms
                .ok_or("missing required flag --worker-max-deadline-ms")?,
            device: slots.worker_device,
        },
        frame_timeout: Duration::from_millis(
            slots.frame_timeout_ms.unwrap_or(DEFAULT_FRAME_TIMEOUT_MS),
        ),
        stop_timeout: Duration::from_millis(
            slots.stop_timeout_ms.unwrap_or(DEFAULT_STOP_TIMEOUT_MS),
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

fn set_millisecond_flag(
    slot: &mut Option<u64>,
    name: &str,
    value: &OsStr,
    max: u64,
) -> Result<(), String> {
    let shown = text(value);
    let found = shown.parse::<u64>().unwrap_or(0);
    if !(1..=max).contains(&found) {
        return Err(format!(
            "flag --{name} has value {shown}, expected an integer between 1 and {max}"
        ));
    }
    if slot.is_some() {
        return Err(format!("flag --{name} given twice"));
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
        (Some(path), _) => require_absolute(path, "flag --socket"),
        (None, Some(value)) => require_absolute(
            PathBuf::from(value),
            "environment variable CORVETTE_DETECTION_SOCKET",
        ),
        (None, None) => Err("missing required flag --socket".to_string()),
    }
}

fn take_ipc_socket(slot: Option<PathBuf>, env_ipc: Option<&OsStr>) -> Result<PathBuf, String> {
    match (slot, env_ipc) {
        (Some(path), _) => require_absolute(path, "flag --ipc-socket"),
        (None, Some(value)) => require_absolute(
            PathBuf::from(value),
            "environment variable CORVETTE_IPC_SOCKET",
        ),
        (None, None) => Ok(PathBuf::from(DEFAULT_IPC_SOCKET)),
    }
}

// The identity check folds `..` lexically when the parent does not exist yet; a relative path would dodge that fold.
fn require_absolute(path: PathBuf, source: &str) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!(
            "{source} has value {}, expected an absolute path",
            path.display()
        ))
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

/// Runs one startup sequence in order: parse, config read, camera plans, signal handlers, worker child, camera threads, socket claim, then serve.
///
/// A first `SIGTERM` or `SIGINT`, and a camera-start failure after earlier cameras started, stops and joins the started cameras within `--stop-timeout-ms`; the worker guard and the runtime then drop so every child process is killed.
///
/// Camera secrets reach the plan build only through `secret_lookup`, read once per camera that names a `secret_env`.
#[must_use]
pub fn run(
    args: &[OsString],
    env_socket: Option<&OsStr>,
    env_ipc_socket: Option<&OsStr>,
    secret_lookup: impl Fn(&str) -> Option<String>,
) -> ExitCode {
    let options = match parse(args, env_socket, env_ipc_socket) {
        Err(message) => return usage_error(&message),
        Ok(Invocation::Help) => {
            let _ = std::io::stdout().write_all(USAGE.as_bytes());
            return ExitCode::SUCCESS;
        }
        Ok(Invocation::Probe { socket }) => return run_probe(&socket),
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
    let plans = match plan_all(&config, options.restream_address.as_deref(), &secret_lookup) {
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
    let entered = runtime.enter();
    let (mut terminate, mut interrupt) = match install_signals() {
        Ok(streams) => streams,
        Err(reason) => {
            log_event("service", "signal-failed", reason);
            return ExitCode::from(EXIT_STARTUP);
        }
    };
    let worker = Supervised::spawn(
        "worker".to_owned(),
        DEFAULT_POLICY,
        worker_command(&options.worker, &config.worker_socket),
        |_child| {},
    );
    let mut started = Vec::new();
    if let Err(reason) = start_cameras(&mut started, &options, &config, plans) {
        log_event("service", "camera-start-failed", reason);
        stop_and_join(started, options.stop_timeout);
        return ExitCode::from(EXIT_STARTUP);
    }
    let entries: Vec<(String, CameraEntry)> = started
        .iter()
        .map(|(name, threads)| (name.clone(), CameraEntry::Live(threads.handle())))
        .collect();
    let state = Arc::new(
        ServiceState::new(entries)
            .with_worker(worker.subscribe())
            .with_config_path(options.config.clone()),
    );
    let bound = start_ipc(&runtime, &options, &state)
        .and_then(|()| bind_socket(&options.socket))
        .and_then(|listener| start_track_events(&runtime, &options, &started).map(|()| listener));
    let listener = match bound {
        Ok(listener) => listener,
        Err((event, reason)) => {
            log_event("service", event, reason);
            return ExitCode::from(EXIT_STARTUP);
        }
    };
    match runtime.block_on(serve_or_signal(
        listener,
        state,
        &mut terminate,
        &mut interrupt,
    )) {
        Ended::Served(Ok(())) => ExitCode::SUCCESS,
        Ended::Served(Err(error)) => {
            log_event("service", "serve-ended", error);
            ExitCode::from(EXIT_SERVE)
        }
        Ended::Signaled(name) => {
            log_event("service", "shutdown-signal", format!("signal={name}"));
            remove_all_sockets(&options);
            stop_and_join(started, options.stop_timeout);
            // The worker guard's drop kills its child, so the runtime must outlive the guard.
            drop(entered);
            drop(worker);
            drop(runtime);
            log_event("service", "shutdown-complete", "exit=0");
            ExitCode::SUCCESS
        }
    }
}

enum Ended {
    Served(std::io::Result<()>),
    Signaled(&'static str),
}

/// Ends when `serve` returns or a first `SIGTERM` or `SIGINT` arrives; a signal branch dropping the serve future also drops the listener.
async fn serve_or_signal(
    listener: tokio::net::UnixListener,
    state: Arc<ServiceState>,
    terminate: &mut Signal,
    interrupt: &mut Signal,
) -> Ended {
    tokio::select! {
        served = serve(listener, state) => Ended::Served(served),
        Some(()) = terminate.recv() => Ended::Signaled("SIGTERM"),
        Some(()) = interrupt.recv() => Ended::Signaled("SIGINT"),
    }
}

fn install_signals() -> Result<(Signal, Signal), String> {
    let terminate = install_signal(SignalKind::terminate(), "SIGTERM")?;
    let interrupt = install_signal(SignalKind::interrupt(), "SIGINT")?;
    Ok((terminate, interrupt))
}

fn install_signal(kind: SignalKind, name: &str) -> Result<Signal, String> {
    tokio::signal::unix::signal(kind)
        .map_err(|error| format!("{name} handler could not be installed: {error}"))
}

fn remove_socket(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log_event(
            "service",
            "socket-unlink-failed",
            format!("{}: {error}", path.display()),
        );
    }
}

fn remove_all_sockets(options: &Options) {
    remove_socket(&options.socket);
    remove_socket(&options.ipc_socket);
    if let Some(path) = &options.track_events_socket {
        remove_socket(path);
    }
}

/// Stops every started camera at once, then joins each with the time left of the one shared `budget`.
fn stop_and_join(started: Vec<(String, CameraThreads)>, budget: Duration) {
    let count = started.len();
    let deadline = Instant::now() + budget;
    for (_, threads) in &started {
        threads.stop();
    }
    for (name, threads) in started {
        let remaining = deadline.saturating_duration_since(Instant::now());
        report_camera(&name, threads.join(remaining));
    }
    log_event("service", "cameras-stopped", format!("count={count}"));
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
    log_event(
        "service",
        event,
        format!(
            "camera={name} runner={} producer={}",
            end_word(report.runner),
            end_word(report.producer)
        ),
    );
}

const fn end_word(end: ThreadEnd) -> &'static str {
    match end {
        ThreadEnd::Finished => "finished",
        ThreadEnd::Panicked => "panicked",
        ThreadEnd::TimedOut => "timed_out",
    }
}

/// Runs one readiness probe: a status request on the socket, exit `0` only on a first line of `HTTP/1.1 200`.
fn run_probe(socket: &Path) -> ExitCode {
    match probe_status(socket) {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => {
            log_event("service", "probe-failed", reason);
            ExitCode::from(EXIT_PROBE_FAILED)
        }
    }
}

fn probe_status(path: &Path) -> Result<(), String> {
    let mut stream = UnixStream::connect(path)
        .map_err(|error| format!("{} connect failed: {error}", path.display()))?;
    stream
        .set_read_timeout(Some(PROBE_IO_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(PROBE_IO_TIMEOUT)))
        .map_err(|error| format!("{} timeouts could not be set: {error}", path.display()))?;
    stream
        .write_all(PROBE_REQUEST.as_bytes())
        .map_err(|error| format!("status request could not be written: {error}"))?;
    let first_line = read_status_line(&mut stream)?;
    let status_ok = first_line == "HTTP/1.1 200" || first_line.starts_with("HTTP/1.1 200 ");
    if status_ok {
        Ok(())
    } else {
        Err(format!(
            "first line was {first_line:?}, expected \"HTTP/1.1 200\" followed by a space or end of line"
        ))
    }
}

fn read_status_line(stream: &mut UnixStream) -> Result<String, String> {
    let mut answer = vec![0u8; MAX_PROBE_ANSWER];
    let mut filled = 0;
    while filled < answer.len() {
        match stream.read(&mut answer[filled..]) {
            Ok(0) => break,
            Ok(read) => {
                filled += read;
                if answer[..filled].contains(&b'\n') {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(format!("no status line within {PROBE_IO_TIMEOUT:?}"));
            }
            Err(error) => return Err(format!("status answer could not be read: {error}")),
        }
    }
    if filled == 0 {
        return Err("the answer was empty".to_string());
    }
    let head = String::from_utf8_lossy(&answer[..filled]);
    let first_line = head.lines().next().map_or_else(String::new, str::to_owned);
    Ok(first_line)
}

/// Builds one plan per camera together with the address that camera dials, refusing at the first refused camera.
fn plan_all(
    config: &Config,
    restream_address: Option<&str>,
    secret_lookup: &impl Fn(&str) -> Option<String>,
) -> Result<Vec<(CameraPlan, StreamAddress)>, String> {
    let mut plans = Vec::with_capacity(config.cameras.len());
    for camera in &config.cameras {
        if camera.source.is_none() && restream_address.is_none() {
            return Err(format!(
                "camera={}: --restream-address is required for a camera without a source table",
                camera.name
            ));
        }
        let address =
            match stream_address(camera, restream_address.unwrap_or_default(), secret_lookup) {
                Err(error) => return Err(error.to_string()),
                Ok(address) => address,
            };
        match CameraPlan::new(config, camera) {
            Err(error) => return Err(format!("camera={} {error}", camera.name)),
            Ok(plan) => plans.push((plan, address)),
        }
    }
    Ok(plans)
}

fn start_cameras(
    started: &mut Vec<(String, CameraThreads)>,
    options: &Options,
    config: &Config,
    plans: Vec<(CameraPlan, StreamAddress)>,
) -> Result<(), String> {
    let link = SharedLink::new(WorkerLink::new(
        config.worker_socket.clone(),
        config.deadline,
        DEFAULT_POLICY,
    ));
    let settings = CameraThreadSettings {
        frame_timeout: options.frame_timeout,
        backoff: DEFAULT_POLICY,
    };
    started.reserve(plans.len());
    for (plan, address) in plans {
        let name = plan.name().to_owned();
        let make_command = plan.command_factory(options.ffmpeg.clone(), address);
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
    Ok(())
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
            .arg("--max-deadline-ms")
            .arg(launch.max_deadline_ms.to_string());
        if let Some(device) = &launch.device {
            command.arg("--device").arg(device);
        }
        command
    }
}

// One path can reach one socket under several spellings; the parent's own resolution folds symlinks and `..` that text comparison would miss.
fn socket_identity(path: &Path) -> PathBuf {
    let resolved = match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => fs::canonicalize(parent)
            .unwrap_or_else(|_| absolute(parent).unwrap_or_else(|_| parent.to_path_buf()))
            .join(name),
        _ => path.to_path_buf(),
    };
    let mut folded = PathBuf::new();
    for part in resolved.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                folded.pop();
            }
            other => folded.push(other),
        }
    }
    folded
}

// Binds and serves the ipc socket at the flag, environment or default path; the error names the event the caller logs.
fn start_ipc(
    runtime: &Runtime,
    options: &Options,
    state: &Arc<ServiceState>,
) -> Result<(), (&'static str, String)> {
    let path = &options.ipc_socket;
    // One path twice would let the legacy claim steal the ipc socket and rebind it at 0666.
    if socket_identity(path) == socket_identity(&options.socket) {
        return Err((
            "ipc-socket-refused",
            format!("{} names both sockets", path.display()),
        ));
    }
    let listener = bind_ipc_socket(path).map_err(|reason| ("ipc-socket-refused", reason))?;
    runtime.spawn(serve(listener, Arc::clone(state)));
    Ok(())
}

// Binds and serves the track-event socket when the flag named a path; the error names the event the caller logs.
fn start_track_events(
    runtime: &Runtime,
    options: &Options,
    started: &[(String, CameraThreads)],
) -> Result<(), (&'static str, String)> {
    let Some(path) = &options.track_events_socket else {
        return Ok(());
    };
    let listener = bind_socket(path)?;
    let streams: Vec<(String, watch::Receiver<Option<FrameObjects>>)> = started
        .iter()
        .map(|(name, threads)| (name.clone(), threads.handle().objects))
        .collect();
    runtime.spawn(serve_track_events(listener, streams));
    Ok(())
}

// The error names the event the caller logs, so one failure emits one line.
fn bind_socket(path: &Path) -> Result<tokio::net::UnixListener, (&'static str, String)> {
    let std_listener = claim_socket(path).map_err(|reason| ("socket-refused", reason))?;
    let listener = std_listener
        .set_nonblocking(true)
        .and_then(|()| tokio::net::UnixListener::from_std(std_listener))
        .map_err(|error| {
            (
                "socket-refused",
                format!("{} could not enter the runtime: {error}", path.display()),
            )
        })?;
    // The API server's uid needs write on the socket file; same mode as corvette-api-server's bind.
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o666)) {
        remove_socket(path);
        return Err(("socket-mode-failed", format!("{}: {error}", path.display())));
    }
    log_event(
        "service",
        "socket-bound",
        format!("path={}", path.display()),
    );
    Ok(listener)
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

// Binds the ipc socket at 0600 inside its checked directory; the returned reason is the one line the caller logs.
fn bind_ipc_socket(path: &Path) -> Result<tokio::net::UnixListener, String> {
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    prepare_ipc_dir(dir)?;
    let std_listener = claim_socket(path)?;
    let listener = std_listener
        .set_nonblocking(true)
        .and_then(|()| tokio::net::UnixListener::from_std(std_listener))
        .map_err(|error| format!("{} could not enter the runtime: {error}", path.display()))?;
    // Chmod after bind as the status socket does: the 0700 directory already excludes every other holder.
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
        remove_socket(path);
        return Err(format!("{}: {error}", path.display()));
    }
    log_event(
        "service",
        "socket-bound",
        format!("ipc path={}", path.display()),
    );
    Ok(listener)
}

// The ipc directory holds the pod's internal socket: a real directory owned by this process at 0700, created here when absent.
fn prepare_ipc_dir(dir: &Path) -> Result<(), String> {
    let own = own_uid()?;
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => Err(format!(
            "{} is a symlink, expected a directory",
            dir.display()
        )),
        Ok(meta) if !meta.is_dir() => Err(format!("{} is not a directory", dir.display())),
        Ok(meta) if meta.uid() != own => Err(format!(
            "{} is owned by uid {}, expected this process's uid {own}",
            dir.display(),
            meta.uid()
        )),
        Ok(meta) if meta.permissions().mode() & 0o777 != 0o700 => Err(format!(
            "{} has mode {:04o}, expected 0700",
            dir.display(),
            meta.permissions().mode() & 0o777
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::DirBuilder::new()
            .mode(0o700)
            .create(dir)
            .and_then(|()| fs::set_permissions(dir, fs::Permissions::from_mode(0o700)))
            .map_err(|error| format!("{} could not be created: {error}", dir.display())),
        Err(error) => Err(format!("{} could not be inspected: {error}", dir.display())),
    }
}

// std exposes no getuid; on Linux the kernel owns /proc/self with this process's own uid.
fn own_uid() -> Result<u32, String> {
    fs::metadata("/proc/self")
        .map(|meta| meta.uid())
        .map_err(|error| format!("this process's uid could not be read: {error}"))
}

fn log_event(unit: &str, event: &str, detail: impl std::fmt::Display) {
    eprintln!("corvette-detect unit=\"{unit}\" event={event} detail=\"{detail}\"");
}

fn usage_error(message: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "corvette-detect-service: {message}");
    let _ = std::io::stderr().write_all(USAGE.as_bytes());
    ExitCode::from(EXIT_USAGE)
}
