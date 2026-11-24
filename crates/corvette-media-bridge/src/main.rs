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

    let record = match corvette_media_bridge::record_config::load_from_env() {
        Ok(record) => record,
        Err(err) => {
            eprintln!("corvette-media-bridge: {err}");
            return std::process::ExitCode::FAILURE;
        }
    };
    if let Some(record) = record {
        for warning in &record.warnings {
            eprintln!("corvette-media-bridge: {warning}");
        }
        let recording = record
            .cameras
            .iter()
            .filter(|camera| camera.records())
            .count();
        let retention = if record.storage.retention_enforce {
            "ENFORCE"
        } else {
            "DRY-RUN"
        };
        eprintln!(
            "corvette-media-bridge: record config loaded cameras={} recording={} retention={retention}",
            record.cameras.len(),
            recording
        );
    }

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
