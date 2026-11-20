//! The detection service binary: collects the arguments and the socket default, then runs startup.
#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let env_socket = std::env::var_os("CORVETTE_DETECTION_SOCKET");
    corvette_detect_service::run(&args, env_socket.as_deref())
}
