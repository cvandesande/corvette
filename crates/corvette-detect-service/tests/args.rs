use corvette_detect_service::{Invocation, Options, parse};
use std::ffi::{OsStr, OsString};
use std::time::Duration;

fn owned(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn options(args: &[&str], env: Option<&str>) -> Options {
    match parse(&owned(args), env.map(OsStr::new)) {
        Ok(Invocation::Serve(options)) => *options,
        Ok(Invocation::Help) => panic!("help answered a serve command line"),
        Err(message) => panic!("unexpected usage error: {message}"),
    }
}

fn refused(args: &[&str]) -> String {
    match parse(&owned(args), None) {
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
        "--worker-health-socket",
        "/health.sock",
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
    assert_eq!(opts.restream_address, "cam:8554");
    assert_eq!(opts.worker.program.to_str(), Some("/bin/worker"));
    assert_eq!(opts.worker.param.to_str(), Some("/w.param"));
    assert_eq!(opts.worker.bin.to_str(), Some("/w.bin"));
    assert_eq!(opts.worker.max_deadline_ms, 5000);
    assert_eq!(opts.worker.health_socket.to_str(), Some("/health.sock"));
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
        "--restream-address",
        "--worker",
        "--worker-param",
        "--worker-bin",
        "--worker-max-deadline-ms",
        "--worker-health-socket",
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
        assert!(matches!(parse(&owned(args), None), Ok(Invocation::Help)));
    }
}

#[test]
fn unknown_flag_names_itself() {
    assert_eq!(
        refused(&with(&base(), &["--probe", "/x"])),
        "unknown argument --probe"
    );
    assert_eq!(refused(&["--bogus"]), "unknown argument --bogus");
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
fn restream_address_accepts_one_host_and_a_live_port() {
    for shown in ["h:1", "h:65535", "127.0.0.1:8554"] {
        let opts = options(&with_replaced(&base(), "--restream-address", shown), None);
        assert_eq!(opts.restream_address, shown);
    }
    for shown in ["h:0", "h:65536", "h:", ":80", "noport", "a:b:1", ":0"] {
        assert!(
            refused(&with_replaced(&base(), "--restream-address", shown))
                .starts_with("flag --restream-address has value"),
            "for {shown}"
        );
    }
}
