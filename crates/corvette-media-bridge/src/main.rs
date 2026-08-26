//! Entrypoint: reads this process's own configuration
//! (`corvette-media-bridge::config`) and runs it (issue #12, item G1).

// See corvette_media_bridge::lib's own doc comment for why: every split is
// internal to the git-pinned moq-dev/moq dependency tree, not this crate's
// own choice.
#![allow(clippy::multiple_crate_versions)]

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let config = match corvette_media_bridge::config::load_from_env() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("corvette-media-bridge: {err}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let (_cameras, listener) = match corvette_media_bridge::start(config).await {
        Ok(started) => started,
        Err(err) => {
            eprintln!("corvette-media-bridge: failed to start: {err}");
            return std::process::ExitCode::FAILURE;
        }
    };

    listener.await;
    std::process::ExitCode::SUCCESS
}
