//! Entrypoint: binds the Unix socket named by `CORVETTE_API_SOCKET` and serves
//! until the process is asked to terminate (issue #4 A2).

use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT, SOCKET_PATH_ENV, shutdown_signal};
use std::path::Path;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let Ok(socket_path) = std::env::var(SOCKET_PATH_ENV) else {
        eprintln!(
            "corvette-api-server: {SOCKET_PATH_ENV} is not set; it must name the Unix socket to listen on"
        );
        return std::process::ExitCode::FAILURE;
    };
    let server = match ApiServer::bind(Path::new(&socket_path), CONNECTION_TIMEOUT) {
        Ok(server) => server,
        Err(err) => {
            eprintln!("corvette-api-server: failed to bind {socket_path}: {err}");
            return std::process::ExitCode::FAILURE;
        }
    };

    server.serve_until(shutdown_signal()).await;
    std::process::ExitCode::SUCCESS
}
