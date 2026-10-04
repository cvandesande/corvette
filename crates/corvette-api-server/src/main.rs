//! Entrypoint: binds the Unix socket named by `CORVETTE_API_SOCKET` and serves
//! until the process is asked to terminate (issue #4 A2, A3).

use corvette_api_server::config::{
    CONFIG_ADDR_ENV, ConfigSource, DEFAULT_CACHE_TTL, DEFAULT_CONFIG_ADDR,
};
use corvette_api_server::database::{DB_PATH_ENV, DEFAULT_DB_PATH};
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT, SOCKET_PATH_ENV, shutdown_signal};
use std::path::Path;
use std::sync::Arc;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let Ok(socket_path) = std::env::var(SOCKET_PATH_ENV) else {
        eprintln!(
            "corvette-api-server: {SOCKET_PATH_ENV} is not set; it must name the Unix socket to listen on"
        );
        return std::process::ExitCode::FAILURE;
    };
    let config_addr =
        std::env::var(CONFIG_ADDR_ENV).unwrap_or_else(|_| DEFAULT_CONFIG_ADDR.to_owned());
    let db_path = std::env::var(DB_PATH_ENV).unwrap_or_else(|_| DEFAULT_DB_PATH.to_owned());
    let config = Arc::new(ConfigSource::new(config_addr, DEFAULT_CACHE_TTL));
    let server = match ApiServer::bind(
        Path::new(&socket_path),
        CONNECTION_TIMEOUT,
        config,
        Path::new(&db_path),
    ) {
        Ok(server) => server,
        Err(err) => {
            eprintln!("corvette-api-server: failed to bind {socket_path}: {err}");
            return std::process::ExitCode::FAILURE;
        }
    };

    server.serve_until(shutdown_signal()).await;
    std::process::ExitCode::SUCCESS
}
