use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_corvette-detect-service");
const STATUS: &str = "/corvette/api/v1/detection/status";
const OBJECTS: &str = "/corvette/api/v1/cameras/front/objects";
const WAIT: Duration = Duration::from_secs(15);

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "corvette-detect-service-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_labels(dir: &Path) {
    fs::write(dir.join("labels.txt"), "0 person\n").unwrap();
}

fn write_config(dir: &Path) {
    let text = format!(
        "worker_socket = \"{w}\"\nlabels = \"{l}\"\ndeadline_ms = 5000\n\n[[cameras]]\nname = \"front\"\nwidth = 640\nheight = 480\nfps = 5\n",
        w = dir.join("worker.sock").display(),
        l = dir.join("labels.txt").display(),
    );
    fs::write(dir.join("config.toml"), text).unwrap();
}

fn executable(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn service_command(dir: &Path, worker: &Path, restream: &str) -> Command {
    let fake_ffmpeg = executable(dir, "fake-ffmpeg", "#!/bin/sh\nexec sleep 5\n");
    service_command_with_ffmpeg(dir, worker, restream, &fake_ffmpeg)
}

fn service_command_with_ffmpeg(
    dir: &Path,
    worker: &Path,
    restream: &str,
    ffmpeg: &Path,
) -> Command {
    let mut command = Command::new(BIN);
    command
        .env_remove("CORVETTE_DETECTION_SOCKET")
        .arg("--config")
        .arg(dir.join("config.toml"))
        .arg("--socket")
        .arg(dir.join("service.sock"))
        .arg("--ffmpeg")
        .arg(ffmpeg)
        .arg("--restream-address")
        .arg(restream)
        .arg("--worker")
        .arg(worker)
        .arg("--worker-param")
        .arg(dir.join("param"))
        .arg("--worker-bin")
        .arg(dir.join("bin"))
        .arg("--worker-max-deadline-ms")
        .arg("5000")
        .arg("--worker-health-socket")
        .arg(dir.join("health.sock"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn try_get(socket: &Path, target: &str) -> Option<(u16, String)> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let request = format!("GET {target} HTTP/1.1\r\nHost: service\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).ok()?;
    let text = String::from_utf8_lossy(&response).into_owned();
    let (head, body) = text.split_once("\r\n\r\n")?;
    let code: u16 = head
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some((code, body.to_owned()))
}

fn wait_until(what: &str, mut probe: impl FnMut() -> bool) {
    let end = Instant::now() + WAIT;
    while !probe() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct Running {
    child: Child,
    socket: PathBuf,
}

// Dropping kills and waits the service even when an assertion failed on the way out.
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(tag: &str) -> Running {
    let dir = temp_dir(tag);
    write_labels(&dir);
    write_config(&dir);
    // A worker that exits at once, the same status answer /bin/true gives on hosts that have it.
    let worker = executable(&dir, "fake-worker", "#!/bin/sh\nexit 0\n");
    let child = service_command(&dir, &worker, "127.0.0.1:8554")
        .spawn()
        .unwrap();
    let socket = dir.join("service.sock");
    wait_until("the service socket", || socket.exists());
    Running { child, socket }
}

#[test]
fn usage_error_exits_two_and_prints_usage() {
    let output = Command::new(BIN)
        .env_remove("CORVETTE_DETECTION_SOCKET")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: corvette-detect-service"));
}

#[test]
fn help_exits_zero_and_prints_usage_on_stdout() {
    let output = Command::new(BIN).arg("--help").output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("usage: corvette-detect-service"));
}

#[test]
fn refused_restream_address_exits_two() {
    let dir = temp_dir("bad-address");
    let output = service_command(&dir, Path::new("/bin/true"), "noport")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn invalid_config_exits_three_and_creates_no_socket() {
    let dir = temp_dir("bad-config");
    write_labels(&dir);
    fs::write(dir.join("config.toml"), "this is not [[valid toml").unwrap();
    let output = service_command(&dir, Path::new("/bin/true"), "127.0.0.1:8554")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(!dir.join("service.sock").exists());
}

#[test]
fn missing_labels_exits_three_before_worker_or_socket() {
    let dir = temp_dir("missing-labels");
    write_config(&dir);
    let marker = dir.join("worker-ran");
    let worker = executable(
        &dir,
        "fake-worker",
        &format!("#!/bin/sh\ntouch \"{}\"\n", marker.display()),
    );
    let output = service_command(&dir, &worker, "127.0.0.1:8554")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(!dir.join("service.sock").exists());
    assert!(
        !marker.exists(),
        "the worker must not start before every plan passed"
    );
}

#[test]
fn valid_configuration_answers_status_over_the_socket() {
    let service = start("valid");
    let mut answer: Option<(u16, String)> = None;
    wait_until("the status with an exited worker", || {
        if let Some(found) = try_get(&service.socket, STATUS)
            && found.0 == 200
            && found.1.contains("\"worker_state\":\"exited\"")
        {
            answer = Some(found);
            return true;
        }
        false
    });
    let (code, body) = answer.expect("a status answer");
    assert_eq!(code, 200);
    assert!(body.contains("\"camera\":\"front\""));
    assert!(body.contains("\"state\":\"waiting_for_frames\""));
    assert!(body.contains("\"worker_state\":\"exited\""));
}

#[test]
fn stale_socket_file_is_replaced_and_the_service_answers() {
    let dir = temp_dir("stale");
    write_labels(&dir);
    write_config(&dir);
    let socket = dir.join("service.sock");
    // A dropped std listener leaves its socket file behind: the stale case to replace.
    drop(UnixListener::bind(&socket).unwrap());
    let child = service_command(&dir, Path::new("/bin/true"), "127.0.0.1:8554")
        .spawn()
        .unwrap();
    let service = Running { child, socket };
    let mut answer: Option<(u16, String)> = None;
    wait_until("the replaced socket to answer", || {
        answer = try_get(&service.socket, STATUS).filter(|found| found.0 == 200);
        answer.is_some()
    });
}

#[test]
fn ordinary_file_at_the_socket_path_is_refused_untouched() {
    let dir = temp_dir("regular-file");
    write_labels(&dir);
    write_config(&dir);
    let socket = dir.join("service.sock");
    fs::write(&socket, "keep me\n").unwrap();
    let output = service_command(&dir, Path::new("/bin/true"), "127.0.0.1:8554")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(fs::read_to_string(&socket).unwrap(), "keep me\n");
}

#[test]
fn socket_file_mode_is_0666_under_a_restrictive_umask() {
    let dir = temp_dir("socket-mode");
    write_labels(&dir);
    write_config(&dir);
    let worker = executable(&dir, "fake-worker", "#!/bin/sh\nexit 0\n");
    let ffmpeg = executable(&dir, "fake-ffmpeg", "#!/bin/sh\nexec sleep 5\n");
    let socket = dir.join("service.sock");
    // The file's helpers cannot set a umask without unsafe, so sh sets it before exec.
    let quote = |p: &Path| format!("'{}'", p.display());
    let script = format!(
        "umask 077 && exec {BIN} --config {config} --socket {socket} --ffmpeg {ffmpeg} \
         --restream-address 127.0.0.1:8554 --worker {worker} --worker-param {param} \
         --worker-bin {bindir} --worker-max-deadline-ms 5000 --worker-health-socket {health}",
        config = quote(&dir.join("config.toml")),
        socket = quote(&socket),
        ffmpeg = quote(&ffmpeg),
        worker = quote(&worker),
        param = quote(&dir.join("param")),
        bindir = quote(&dir.join("bin")),
        health = quote(&dir.join("health.sock")),
    );
    let child = Command::new("sh")
        .arg("-c")
        .arg(script)
        .env_remove("CORVETTE_DETECTION_SOCKET")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let service = Running { child, socket };
    wait_until("a status of 200", || {
        try_get(&service.socket, STATUS).is_some_and(|found| found.0 == 200)
    });
    let mode = fs::metadata(&service.socket).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o666,
        "the socket is not world-connectable under umask 077"
    );
}

#[test]
fn unpublished_camera_answers_objects_unknown() {
    let service = start("objects");
    let mut answer: Option<(u16, String)> = None;
    wait_until("the objects answer", || {
        answer = try_get(&service.socket, OBJECTS);
        answer.is_some()
    });
    let (code, body) = answer.expect("an objects answer");
    assert_eq!(code, 503);
    assert_eq!(body, "{\"reason\":\"objects_unknown\"}");
}

fn probe_command(socket: &Path) -> Command {
    let mut command = Command::new(BIN);
    command
        .env_remove("CORVETTE_DETECTION_SOCKET")
        .arg("--probe")
        .arg("--socket")
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[test]
fn probe_exits_zero_against_the_running_service() {
    let service = start("probe-valid");
    wait_until("a status of 200", || {
        try_get(&service.socket, STATUS).is_some_and(|found| found.0 == 200)
    });
    let output = probe_command(&service.socket).output().unwrap();
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn probe_exit_code_follows_the_status_first_line() {
    let dir = temp_dir("probe-answers");
    for (tag, answer, wanted) in [
        (
            "503",
            b"HTTP/1.1 503 Service Unavailable\r\n\r\n".as_slice(),
            1,
        ),
        ("200", b"HTTP/1.1 200 OK\r\n\r\n".as_slice(), 0),
        ("2000", b"HTTP/1.1 2000 odd\r\n\r\n".as_slice(), 1),
        ("200nospace", b"HTTP/1.1 200OK\r\n\r\n".as_slice(), 1),
        ("closed", b"".as_slice(), 1),
    ] {
        let socket = dir.join(format!("probe-{tag}.sock"));
        let listener = UnixListener::bind(&socket).unwrap();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // Read the request first, as the real service does: closing early races the probe's write.
                let mut request = [0u8; 128];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(answer);
            }
        });
        let output = probe_command(&socket).output().unwrap();
        assert_eq!(output.status.code(), Some(wanted), "for answer {tag}");
    }
}

#[test]
fn probe_on_a_missing_socket_exits_one_and_creates_nothing() {
    let dir = temp_dir("probe-absent");
    let socket = dir.join("absent.sock");
    let output = probe_command(&socket).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(!socket.exists());
}

#[test]
fn probe_with_no_socket_exits_two() {
    let dir = temp_dir("probe-usage");
    let output = Command::new(BIN)
        .env_remove("CORVETTE_DETECTION_SOCKET")
        .arg("--probe")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let socket = dir.join("rejected.sock");
    let output = Command::new(BIN)
        .env_remove("CORVETTE_DETECTION_SOCKET")
        .arg("--probe")
        .arg("--config")
        .arg(dir.join("config.toml"))
        .arg("--socket")
        .arg(&socket)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!socket.exists());
}

// Dropping kills and waits the probe process even when a wait-time assertion failed on the way out.
struct ProbeChild {
    child: Child,
}

impl Drop for ProbeChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn probe_gives_up_on_a_server_that_never_answers() {
    let dir = temp_dir("probe-silent");
    let socket = dir.join("silent.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut sink = [0u8; 128];
        let _ = stream.read(&mut sink);
        let _ = stream.read(&mut sink);
    });
    let started = Instant::now();
    let mut probe = ProbeChild {
        child: probe_command(&socket).spawn().unwrap(),
    };
    let ended = loop {
        if let Some(status) = probe.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(6),
            "the probe outlived its read timeout"
        );
        thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(ended.code(), Some(1));
}

struct Stopping {
    child: Child,
    dir: PathBuf,
    socket: PathBuf,
    stderr: std::sync::mpsc::Receiver<Vec<u8>>,
}

// Dropping kills and waits the service and the processes its scripts exec, even when an assertion failed.
impl Drop for Stopping {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        for name in ["worker.pid", "ffmpeg.pid"] {
            let shown = fs::read_to_string(self.dir.join(name)).unwrap_or_default();
            let pid = shown.trim();
            if !pid.is_empty() {
                let _ = Command::new("kill").arg("-KILL").arg(pid).status();
            }
        }
    }
}

fn start_stopping(tag: &str, extra: &[&str]) -> Stopping {
    let dir = temp_dir(tag);
    write_labels(&dir);
    write_config(&dir);
    // The pid file names the exec'd sleep, so it is the process a clean shutdown must end.
    let worker = executable(
        &dir,
        "pid-worker",
        &format!(
            "#!/bin/sh\necho $$ > \"{}\"\nexec sleep 30\n",
            dir.join("worker.pid").display()
        ),
    );
    let ffmpeg = executable(
        &dir,
        "pid-ffmpeg",
        &format!(
            "#!/bin/sh\necho $$ > \"{}\"\nexec sleep 5\n",
            dir.join("ffmpeg.pid").display()
        ),
    );
    let mut command = service_command_with_ffmpeg(&dir, &worker, "127.0.0.1:8554", &ffmpeg);
    command.args(extra).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let socket = dir.join("service.sock");
    wait_until("the service socket", || socket.exists());
    let (sender, stderr) = std::sync::mpsc::channel();
    let mut pipe = child.stderr.take().unwrap();
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = pipe.read_to_end(&mut buffer);
        let _ = sender.send(buffer);
    });
    Stopping {
        child,
        dir,
        socket,
        stderr,
    }
}

fn send_signal(child: &Child, name: &str) {
    let delivered = Command::new("kill")
        .arg(format!("-{name}"))
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(delivered.success(), "kill -{name} was not accepted");
}

fn wait_exit(child: &mut Child) -> std::process::ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(found) = child.try_wait().unwrap() {
            assert!(
                started.elapsed() < WAIT,
                "the service did not exit within {WAIT:?}"
            );
            return found;
        }
        assert!(
            started.elapsed() < WAIT,
            "the service did not exit within {WAIT:?}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn recorded_pid(dir: &Path, name: &str) -> String {
    let shown = fs::read_to_string(dir.join(name)).unwrap();
    let pid = shown.trim().to_owned();
    assert!(!pid.is_empty(), "{name} recorded no pid");
    pid
}

fn wait_pid_gone(who: &str, pid: &str) {
    let proc_dir = Path::new("/proc").join(pid);
    let end = Instant::now() + Duration::from_secs(5);
    while proc_dir.exists() {
        assert!(
            Instant::now() < end,
            "{who} pid {pid} still exists after 5 s"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn orderly_shutdown(tag: &str, signal: &str) {
    let mut service = start_stopping(tag, &["--stop-timeout-ms", "1500"]);
    wait_until("a status of 200", || {
        try_get(&service.socket, STATUS).is_some_and(|found| found.0 == 200)
    });
    send_signal(&service.child, signal);
    let status = wait_exit(&mut service.child);
    assert_eq!(status.code(), Some(0), "the service exited {status}");
    assert!(
        !service.socket.exists(),
        "the socket file survived the shutdown"
    );
    let stderr = String::from_utf8_lossy(
        &service
            .stderr
            .recv_timeout(Duration::from_secs(10))
            .expect("the stderr pipe never ended"),
    )
    .into_owned();
    let signal_at = stderr
        .find("shutdown-signal")
        .expect("no shutdown-signal line");
    let cameras_at = stderr
        .find("cameras-stopped")
        .expect("no cameras-stopped line");
    let complete_at = stderr
        .find("shutdown-complete")
        .expect("no shutdown-complete line");
    assert!(
        signal_at < cameras_at && cameras_at < complete_at,
        "shutdown events out of order: {stderr}"
    );
    assert!(
        !stderr.contains("stop-timeout"),
        "the stop budget was hit: {stderr}"
    );
    wait_pid_gone("worker", &recorded_pid(&service.dir, "worker.pid"));
    wait_pid_gone("camera child", &recorded_pid(&service.dir, "ffmpeg.pid"));
}

#[test]
fn sigterm_shutdown_exits_zero_and_leaves_no_socket_or_child() {
    orderly_shutdown("sigterm", "TERM");
}

#[test]
fn sigint_shutdown_exits_zero_and_leaves_no_socket_or_child() {
    orderly_shutdown("sigint", "INT");
}

#[test]
fn probe_after_a_shutdown_exits_one_on_the_unlinked_socket() {
    let mut service = start_stopping("probe-after-shutdown", &[]);
    wait_until("a status of 200", || {
        try_get(&service.socket, STATUS).is_some_and(|found| found.0 == 200)
    });
    send_signal(&service.child, "TERM");
    let status = wait_exit(&mut service.child);
    assert_eq!(status.code(), Some(0));
    let probe = probe_command(&service.socket).output().unwrap();
    assert_eq!(probe.status.code(), Some(1));
}
