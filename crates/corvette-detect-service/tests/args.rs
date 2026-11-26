use corvette_detect_service::{DEFAULT_IPC_SOCKET, Invocation, Options, parse};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::time::Duration;

fn owned(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn options(args: &[&str], env: Option<&str>) -> Options {
    match parse(&owned(args), env.map(OsStr::new), None) {
        Ok(Invocation::Serve(options)) => *options,
        Ok(other) => panic!("non-serve invocation: {other:?}"),
        Err(message) => panic!("unexpected usage error: {message}"),
    }
}

fn refused(args: &[&str]) -> String {
    match parse(&owned(args), None, None) {
        Ok(other) => panic!("unexpected success: {other:?}"),
        Err(message) => message,
    }
}

fn base() -> Vec<&'static str> {
    vec![
        "--config",
        "/c.toml",
        "--ffmpeg",
        "/bin/ffmpeg",
        "--restream-address",
        "cam:8554",
        "--worker",
        "/bin/worker",
        "--worker-param",
        "/w.param",
        "--worker-bin",
        "/w.bin",
        "--worker-max-deadline-ms",
        "5000",
    ]
}

fn with<'a>(base: &[&'a str], extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = base.to_vec();
    all.extend_from_slice(extra);
    all
}

#[test]
fn full_command_line_parses_with_defaults() {
    let opts = options(&with(&base(), &["--socket", "/s.sock"]), None);
    assert_eq!(opts.config.to_str(), Some("/c.toml"));
    assert_eq!(opts.socket.to_str(), Some("/s.sock"));
    assert_eq!(opts.ffmpeg.to_str(), Some("/bin/ffmpeg"));
    assert_eq!(opts.restream_address.as_deref(), Some("cam:8554"));
    assert_eq!(opts.worker.program.to_str(), Some("/bin/worker"));
    assert_eq!(opts.worker.param.to_str(), Some("/w.param"));
    assert_eq!(opts.worker.bin.to_str(), Some("/w.bin"));
    assert_eq!(opts.worker.max_deadline_ms, 5000);
    assert_eq!(opts.worker.device, None);
    assert_eq!(opts.frame_timeout, Duration::from_secs(10));
}

#[test]
fn optional_flags_are_kept() {
    let opts = options(
        &with(
            &base(),
            &[
                "--socket",
                "/s.sock",
                "--worker-device",
                "gpu:0",
                "--frame-timeout-ms",
                "250",
            ],
        ),
        None,
    );
    assert_eq!(opts.worker.device.as_deref(), Some("gpu:0"));
    assert_eq!(opts.frame_timeout, Duration::from_millis(250));
}

#[test]
fn socket_comes_from_environment_when_flag_absent() {
    let opts = options(&base(), Some("/env.sock"));
    assert_eq!(opts.socket.to_str(), Some("/env.sock"));
}

#[test]
fn socket_flag_overrides_environment() {
    let opts = options(&with(&base(), &["--socket", "/s.sock"]), Some("/env.sock"));
    assert_eq!(opts.socket.to_str(), Some("/s.sock"));
}

#[test]
fn missing_socket_without_environment_is_usage_error() {
    assert_eq!(refused(&base()), "missing required flag --socket");
}

#[test]
fn each_required_flag_is_demanded_when_absent() {
    let all = with(&base(), &["--socket", "/s.sock"]);
    for missing in [
        "--config",
        "--socket",
        "--ffmpeg",
        "--worker",
        "--worker-param",
        "--worker-bin",
        "--worker-max-deadline-ms",
    ] {
        let index = all.iter().position(|arg| *arg == missing).unwrap();
        let stripped: Vec<&str> = all
            .iter()
            .enumerate()
            .filter(|(at, _)| *at != index && *at != index + 1)
            .map(|(_, arg)| *arg)
            .collect();
        assert_eq!(
            refused(&stripped),
            format!("missing required flag {missing}"),
            "for {missing}"
        );
    }
}

#[test]
fn help_wins_over_everything_else() {
    for args in [&["--help"][..], &["--help", "--bogus"][..]] {
        assert!(matches!(
            parse(&owned(args), None, None),
            Ok(Invocation::Help)
        ));
    }
}

#[test]
fn unknown_flag_names_itself() {
    assert_eq!(
        refused(&with(&base(), &["--status", "/x"])),
        "unknown argument --status"
    );
    assert_eq!(refused(&["--bogus"]), "unknown argument --bogus");
    assert_eq!(
        refused(&["--worker-health-socket"]),
        "unknown argument --worker-health-socket"
    );
}

fn probe_socket(args: &[&str], env: Option<&str>) -> PathBuf {
    match parse(&owned(args), env.map(OsStr::new), None) {
        Ok(Invocation::Probe { socket }) => socket,
        Ok(other) => panic!("unexpected success: {other:?}"),
        Err(message) => panic!("unexpected usage error: {message}"),
    }
}

#[test]
fn probe_takes_its_socket_from_the_flag() {
    let socket = probe_socket(&["--probe", "--socket", "/p.sock"], None);
    assert_eq!(socket.to_str(), Some("/p.sock"));
}

#[test]
fn probe_socket_flag_overrides_environment() {
    let socket = probe_socket(&["--probe", "--socket", "/p.sock"], Some("/env.sock"));
    assert_eq!(socket.to_str(), Some("/p.sock"));
}

#[test]
fn probe_takes_its_socket_from_the_environment_argument() {
    let socket = probe_socket(&["--probe"], Some("/env.sock"));
    assert_eq!(socket.to_str(), Some("/env.sock"));
}

#[test]
fn probe_without_any_socket_is_a_usage_error() {
    assert_eq!(refused(&["--probe"]), "missing required flag --socket");
}

#[test]
fn probe_refuses_every_serve_flag() {
    for (flag, value, name) in [
        ("--config", "x", "config"),
        ("--ffmpeg", "x", "ffmpeg"),
        ("--restream-address", "h:1", "restream-address"),
        ("--worker", "x", "worker"),
        ("--worker-param", "x", "worker-param"),
        ("--worker-bin", "x", "worker-bin"),
        ("--worker-max-deadline-ms", "5", "worker-max-deadline-ms"),
        ("--worker-device", "x", "worker-device"),
        ("--frame-timeout-ms", "5", "frame-timeout-ms"),
        ("--stop-timeout-ms", "5", "stop-timeout-ms"),
    ] {
        assert_eq!(
            refused(&["--probe", flag, value]),
            format!("flag --{name} is not allowed with --probe"),
            "for {flag}"
        );
    }
}

#[test]
fn probe_given_twice_is_a_usage_error() {
    assert_eq!(refused(&["--probe", "--probe"]), "flag --probe given twice");
}

#[test]
fn repeated_flag_is_refused() {
    assert_eq!(
        refused(&with(&base(), &["--socket", "/a", "--socket", "/b"])),
        "flag --socket given twice"
    );
    assert_eq!(
        refused(&with(
            &base(),
            &[
                "--socket",
                "/s",
                "--frame-timeout-ms",
                "5",
                "--frame-timeout-ms",
                "6"
            ]
        )),
        "flag --frame-timeout-ms given twice"
    );
}

#[test]
fn flag_at_the_end_without_value_is_refused() {
    assert_eq!(
        refused(&with(&base(), &["--socket"])),
        "flag --socket needs a value"
    );
}

#[test]
fn frame_timeout_accepts_only_its_bounds() {
    for shown in ["1", "600000"] {
        let opts = options(
            &with(
                &base(),
                &["--socket", "/s.sock", "--frame-timeout-ms", shown],
            ),
            None,
        );
        assert_eq!(
            opts.frame_timeout,
            Duration::from_millis(shown.parse::<u64>().unwrap())
        );
    }
    for shown in ["0", "600001", "abc", "-5", ""] {
        assert!(
            refused(&with(
                &base(),
                &["--socket", "/s.sock", "--frame-timeout-ms", shown]
            ))
            .starts_with("flag --frame-timeout-ms has value"),
            "for {shown}"
        );
    }
}

#[test]
fn stop_timeout_defaults_to_its_proposal() {
    let opts = options(&with(&base(), &["--socket", "/s.sock"]), None);
    assert_eq!(opts.stop_timeout, Duration::from_secs(15));
}

#[test]
fn stop_timeout_accepts_only_its_bounds() {
    for shown in ["1", "600000"] {
        let opts = options(
            &with(
                &base(),
                &["--socket", "/s.sock", "--stop-timeout-ms", shown],
            ),
            None,
        );
        assert_eq!(
            opts.stop_timeout,
            Duration::from_millis(shown.parse::<u64>().unwrap())
        );
    }
    for shown in ["0", "600001", "abc", "-5", ""] {
        assert!(
            refused(&with(
                &base(),
                &["--socket", "/s.sock", "--stop-timeout-ms", shown]
            ))
            .starts_with("flag --stop-timeout-ms has value"),
            "for {shown}"
        );
    }
}

#[test]
fn max_deadline_accepts_only_positive_integers() {
    for shown in ["0", "abc"] {
        assert!(
            refused(&with(
                &base(),
                &["--socket", "/s.sock", "--worker-max-deadline-ms", shown]
            ))
            .starts_with("flag --worker-max-deadline-ms has value"),
            "for {shown}"
        );
    }
}

fn with_replaced(
    base: &[&'static str],
    flag: &'static str,
    value: &'static str,
) -> Vec<&'static str> {
    let index = base.iter().position(|arg| *arg == flag).unwrap() + 1;
    let mut out = with(base, &["--socket", "/s.sock"]);
    out[index] = value;
    out
}

#[test]
fn restream_address_may_be_absent_at_parse() {
    let all = with(&base(), &["--socket", "/s.sock"]);
    let index = all
        .iter()
        .position(|arg| *arg == "--restream-address")
        .unwrap();
    let stripped: Vec<&str> = all
        .iter()
        .enumerate()
        .filter(|(at, _)| *at != index && *at != index + 1)
        .map(|(_, arg)| *arg)
        .collect();
    assert_eq!(options(&stripped, None).restream_address, None);
}

#[test]
fn restream_address_accepts_one_host_and_a_live_port() {
    for shown in ["h:1", "h:65535", "127.0.0.1:8554"] {
        let opts = options(&with_replaced(&base(), "--restream-address", shown), None);
        assert_eq!(opts.restream_address.as_deref(), Some(shown));
    }
    for shown in ["h:0", "h:65536", "h:", ":80", "noport", "a:b:1", ":0"] {
        assert!(
            refused(&with_replaced(&base(), "--restream-address", shown))
                .starts_with("flag --restream-address has value"),
            "for {shown}"
        );
    }
}

#[test]
fn track_events_socket_flag_is_optional_and_kept() {
    let without = options(&with(&base(), &["--socket", "/s.sock"]), None);
    assert_eq!(without.track_events_socket, None);
    let with = options(
        &with(
            &base(),
            &["--socket", "/s.sock", "--track-events-socket", "/t.sock"],
        ),
        None,
    );
    assert_eq!(with.track_events_socket, Some(PathBuf::from("/t.sock")));
}

#[test]
fn track_events_socket_given_twice_is_a_usage_error() {
    assert_eq!(
        refused(&with(
            &base(),
            &[
                "--socket",
                "/s.sock",
                "--track-events-socket",
                "/t.sock",
                "--track-events-socket",
                "/t2.sock",
            ],
        )),
        "flag --track-events-socket given twice"
    );
}

#[test]
fn track_events_socket_is_refused_with_probe() {
    assert_eq!(
        refused(&[
            "--probe",
            "--socket",
            "/p.sock",
            "--track-events-socket",
            "/t.sock",
        ]),
        "flag --track-events-socket is not allowed with --probe"
    );
}

#[test]
fn ipc_socket_defaults_when_no_flag_or_environment_is_given() {
    let opts = options(&with(&base(), &["--socket", "/s.sock"]), None);
    assert_eq!(opts.ipc_socket, PathBuf::from(DEFAULT_IPC_SOCKET));
}

#[test]
fn ipc_socket_flag_is_kept() {
    let with = options(
        &with(&base(), &["--socket", "/s.sock", "--ipc-socket", "/i.sock"]),
        None,
    );
    assert_eq!(with.ipc_socket, PathBuf::from("/i.sock"));
}

#[test]
fn ipc_socket_comes_from_environment_when_flag_absent() {
    let invocation = parse(
        &owned(&with(&base(), &["--socket", "/s.sock"])),
        None,
        Some(OsStr::new("/env.sock")),
    );
    let Ok(Invocation::Serve(options)) = invocation else {
        panic!("unexpected invocation: {invocation:?}");
    };
    assert_eq!(options.ipc_socket, PathBuf::from("/env.sock"));
}

#[test]
fn ipc_socket_flag_overrides_environment() {
    let invocation = parse(
        &owned(&with(
            &base(),
            &["--socket", "/s.sock", "--ipc-socket", "/i.sock"],
        )),
        None,
        Some(OsStr::new("/env.sock")),
    );
    let Ok(Invocation::Serve(options)) = invocation else {
        panic!("unexpected invocation: {invocation:?}");
    };
    assert_eq!(options.ipc_socket, PathBuf::from("/i.sock"));
}

#[test]
fn ipc_socket_given_twice_is_a_usage_error() {
    assert_eq!(
        refused(&with(
            &base(),
            &[
                "--socket",
                "/s.sock",
                "--ipc-socket",
                "/i.sock",
                "--ipc-socket",
                "/i-dup.sock",
            ],
        )),
        "flag --ipc-socket given twice"
    );
}

#[test]
fn ipc_socket_is_refused_with_probe() {
    assert_eq!(
        refused(&["--probe", "--socket", "/p.sock", "--ipc-socket", "/i.sock",]),
        "flag --ipc-socket is not allowed with --probe"
    );
}

fn refused_env(args: &[&str], env_socket: Option<&str>, env_ipc: Option<&str>) -> String {
    match parse(
        &owned(args),
        env_socket.map(OsStr::new),
        env_ipc.map(OsStr::new),
    ) {
        Ok(other) => panic!("unexpected success: {other:?}"),
        Err(message) => message,
    }
}

#[test]
fn relative_socket_flag_names_the_flag_as_usage_error() {
    assert_eq!(
        refused(&with(&base(), &["--socket", "s.sock"])),
        "flag --socket has value s.sock, expected an absolute path"
    );
}

#[test]
fn relative_ipc_socket_flag_names_the_flag_as_usage_error() {
    assert_eq!(
        refused(&with(
            &base(),
            &["--socket", "/s.sock", "--ipc-socket", "i.sock"]
        )),
        "flag --ipc-socket has value i.sock, expected an absolute path"
    );
}

#[test]
fn relative_probe_socket_names_the_flag_as_usage_error() {
    assert_eq!(
        refused(&["--probe", "--socket", "p.sock"]),
        "flag --socket has value p.sock, expected an absolute path"
    );
}

#[test]
fn relative_socket_environment_names_it_as_usage_error() {
    assert_eq!(
        refused_env(&base(), Some("env.sock"), None),
        "environment variable CORVETTE_DETECTION_SOCKET has value env.sock, expected an absolute path"
    );
}

#[test]
fn relative_ipc_socket_environment_names_it_as_usage_error() {
    assert_eq!(
        refused_env(
            &with(&base(), &["--socket", "/s.sock"]),
            None,
            Some("i.sock")
        ),
        "environment variable CORVETTE_IPC_SOCKET has value i.sock, expected an absolute path"
    );
}

#[test]
fn absolute_socket_environment_is_accepted() {
    let opts = options(&base(), Some("/env.sock"));
    assert_eq!(opts.socket, PathBuf::from("/env.sock"));
}
