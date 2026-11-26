//! The worker binary: command line and serve mode.
#![forbid(unsafe_code)]

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::os::unix::fs::FileTypeExt as _;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use corvette_detect_worker::{NcnnEngine, ProcessExit, ServeEnd, serve};
use corvette_ncnn::Device;

const USAGE: &str = "\
usage: corvette-detect-worker --param PATH --bin PATH --socket PATH
       --max-deadline-ms N [--device VALUE]
       corvette-detect-worker --help
exit codes:
     0  help
     2  usage error
     3  startup or runtime failure (device selection, model load, socket path refused or bind failed, the serve loop's I/O error)
    20  device lost
    21  request outlived its deadline
";

const EXIT_USAGE: u8 = 2;
const EXIT_STARTUP: u8 = 3;

enum Command {
    Help,
    Serve(ServeConfig),
}

struct ServeConfig {
    param: PathBuf,
    bin: PathBuf,
    socket: PathBuf,
    max_deadline_ms: u32,
    device: Option<String>,
}

#[derive(Default)]
struct FlagSlots {
    param: Option<PathBuf>,
    bin: Option<PathBuf>,
    socket: Option<PathBuf>,
    max_deadline_ms: Option<u32>,
    device: Option<String>,
}

impl FlagSlots {
    fn set_path(&mut self, name: &'static str, value: &OsStr) -> Result<(), String> {
        let previous = match name {
            "param" => self.param.replace(PathBuf::from(value)),
            "bin" => self.bin.replace(PathBuf::from(value)),
            "socket" => self.socket.replace(PathBuf::from(value)),
            other => unreachable!("set_path has no {other} slot"),
        };
        Self::reject(name, previous.as_ref())
    }

    fn set_deadline(&mut self, value: &OsStr) -> Result<(), String> {
        let text = text(value);
        match text.parse::<u32>() {
            Ok(0) | Err(_) => Err(format!(
                "flag --max-deadline-ms has value {text}, expected an integer of at least 1"
            )),
            Ok(number) => {
                let previous = self.max_deadline_ms.replace(number);
                Self::reject("max-deadline-ms", previous.as_ref())
            }
        }
    }

    fn set_device(&mut self, value: &OsStr) -> Result<(), String> {
        let text = text(value);
        if device_spec(&text).is_none() {
            return Err(format!(
                "flag --device has value {text}, expected cpu, default or a device index"
            ));
        }
        let previous = self.device.replace(text);
        Self::reject("device", previous.as_ref())
    }

    fn reject<T>(name: &str, previous: Option<&T>) -> Result<(), String> {
        if previous.is_some() {
            return Err(format!("flag --{name} given twice"));
        }
        Ok(())
    }

    fn required_path(&self, name: &'static str) -> Result<PathBuf, String> {
        let slot = match name {
            "param" => &self.param,
            "bin" => &self.bin,
            "socket" => &self.socket,
            other => unreachable!("required_path has no {other} slot"),
        };
        slot.clone()
            .ok_or_else(|| format!("missing required flag --{name}"))
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match parse(&args) {
        Err(message) => usage_error(&message),
        Ok(Command::Help) => {
            let _ = std::io::stdout().write_all(USAGE.as_bytes());
            ExitCode::SUCCESS
        }
        Ok(Command::Serve(config)) => run_serve(&config),
    }
}

fn usage_error(message: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "corvette-detect-worker: {message}");
    let _ = std::io::stderr().write_all(USAGE.as_bytes());
    ExitCode::from(EXIT_USAGE)
}

fn parse(args: &[OsString]) -> Result<Command, String> {
    if args.iter().any(|arg| arg.as_os_str() == "--help") {
        return Ok(Command::Help);
    }
    let mut slots = FlagSlots::default();
    let mut index = 0;
    while index < args.len() {
        let flag = text(&args[index]);
        match flag.as_str() {
            "--param" => {
                slots.set_path("param", argument(args, index, &flag)?)?;
                index += 2;
            }
            "--bin" => {
                slots.set_path("bin", argument(args, index, &flag)?)?;
                index += 2;
            }
            "--socket" => {
                slots.set_path("socket", argument(args, index, &flag)?)?;
                index += 2;
            }
            "--max-deadline-ms" => {
                slots.set_deadline(argument(args, index, &flag)?)?;
                index += 2;
            }
            "--device" => {
                slots.set_device(argument(args, index, &flag)?)?;
                index += 2;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(Command::Serve(ServeConfig {
        param: slots.required_path("param")?,
        bin: slots.required_path("bin")?,
        socket: slots.required_path("socket")?,
        max_deadline_ms: slots
            .max_deadline_ms
            .ok_or("missing required flag --max-deadline-ms")?,
        device: slots.device,
    }))
}

fn argument<'a>(args: &'a [OsString], at: usize, flag: &str) -> Result<&'a OsStr, String> {
    args.get(at + 1)
        .map(OsString::as_os_str)
        .ok_or_else(|| format!("flag {flag} needs a value"))
}

fn text(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

fn run_serve(config: &ServeConfig) -> ExitCode {
    let device = match select_device(config.device.as_deref().unwrap_or("default")) {
        Ok(device) => device,
        Err(reason) => {
            eprintln!("corvette-detect-worker: device selection failed: {reason}");
            return ExitCode::from(EXIT_STARTUP);
        }
    };
    let mut engine = match NcnnEngine::load(&config.param, &config.bin, device) {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("corvette-detect-worker: model load failed: {error}");
            return ExitCode::from(EXIT_STARTUP);
        }
    };
    let listener = match claim_socket(&config.socket) {
        Ok(listener) => listener,
        Err(reason) => {
            eprintln!("corvette-detect-worker: serving socket refused: {reason}");
            return ExitCode::from(EXIT_STARTUP);
        }
    };
    eprintln!(
        "corvette-detect-worker: ready {} {} ms",
        config.socket.display(),
        config.max_deadline_ms
    );
    let stop = AtomicBool::new(false);
    let _guard = StopOnDrop(&stop);
    let outcome = serve(
        &listener,
        &mut engine,
        &ProcessExit,
        &stop,
        Duration::from_millis(u64::from(config.max_deadline_ms)),
    );
    match outcome {
        Ok(ServeEnd::Fatal(reason)) => exit_from_i32(reason.exit_code()),
        Ok(ServeEnd::Stopped) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("corvette-detect-worker: serve failed: {error}");
            ExitCode::from(EXIT_STARTUP)
        }
    }
}

fn select_device(spec: &str) -> Result<Device, String> {
    match spec {
        "cpu" => Ok(Device::Cpu),
        "default" => Device::select(None, false).map_err(|error| error.to_string()),
        raw => match raw.parse::<i32>() {
            Ok(index) if index >= 0 => {
                Device::select(Some(index), false).map_err(|error| error.to_string())
            }
            _ => Err(format!("value {raw} is not cpu, default or a device index")),
        },
    }
}

fn device_spec(text: &str) -> Option<()> {
    match text {
        "cpu" | "default" => Some(()),
        _ => text
            .parse::<i32>()
            .ok()
            .filter(|index| *index >= 0)
            .map(|_| ()),
    }
}

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

struct StopOnDrop<'a>(&'a AtomicBool);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn exit_from_i32(code: i32) -> ExitCode {
    u32::try_from(code).map_or(ExitCode::FAILURE, |wide| {
        u8::try_from(wide).map_or(ExitCode::FAILURE, ExitCode::from)
    })
}
