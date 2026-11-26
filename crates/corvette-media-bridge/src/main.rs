/// Exit status when a second stop signal forces the exit.
const FORCED_EXIT_CODE: i32 = 1;

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
    let runtime = match record {
        None => None,
        Some(record) => {
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
            match corvette_media_bridge::record_role::prepare(&record) {
                Ok(runtime) => runtime,
                Err(err) => {
                    eprintln!("corvette-media-bridge: {err}");
                    return std::process::ExitCode::FAILURE;
                }
            }
        }
    };

    let (stop_trigger, stop) = corvette_media_bridge::shutdown::Shutdown::channel();
    if let Err(err) = watch_stop_signals(stop_trigger) {
        eprintln!("corvette-media-bridge: cannot watch stop signals: {err}");
        return std::process::ExitCode::FAILURE;
    }

    let outcome = match corvette_media_bridge::start_with_recording(
        config,
        runtime.map(std::sync::Arc::new),
        stop,
    )
    .await
    {
        Ok(listener) => listener.await,
        Err(err) => {
            eprintln!("corvette-media-bridge: failed to start: {err}");
            return std::process::ExitCode::FAILURE;
        }
    };

    match outcome {
        corvette_media_bridge::shutdown::BridgeExit::Served
        | corvette_media_bridge::shutdown::BridgeExit::Stopped => std::process::ExitCode::SUCCESS,
        corvette_media_bridge::shutdown::BridgeExit::DeadlineExceeded => {
            eprintln!(
                "corvette-media-bridge: shutdown did not finish within {} seconds; exiting nonzero",
                corvette_media_bridge::shutdown::SHUTDOWN_DEADLINE.as_secs()
            );
            std::process::ExitCode::FAILURE
        }
    }
}

/// Resolves `trigger` on the first SIGTERM or SIGINT, and exits nonzero on a
/// second one, mid-drain, for an operator who needs the process gone now.
fn watch_stop_signals(
    trigger: corvette_media_bridge::shutdown::ShutdownTrigger,
) -> std::io::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    tokio::spawn(async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
        trigger.trigger(); // the container's first process ignores signals it has no handler for: this task is what makes SIGTERM reachable at all
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
        eprintln!("corvette-media-bridge: second stop signal; exiting immediately");
        std::process::exit(FORCED_EXIT_CODE);
    });
    Ok(())
}
