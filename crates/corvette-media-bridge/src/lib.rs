pub mod config;
pub mod disk_free;
pub mod event_consumer;
pub mod expiry;
pub mod fmp4;
pub mod hls;
pub mod index;
pub mod parameter_sets;
pub mod piece_walk;
pub mod pressure;
pub mod record_config;
pub mod record_role;
pub mod recorder;
pub mod recovery;
pub mod restream_provider;
pub mod reviews;
pub mod rtp_clock;
pub mod shutdown;
pub mod supervise;
pub mod utc;
pub mod ws_repackager;

use config::{CameraSpec, Config};
use corvette_rtsp_client::client::Client;
use hls::MultiCameraHlsStore;
use restream_provider::MultiCameraProvider;
use std::sync::Arc;
use supervise::{Supervised, log_event};
use ws_repackager::MultiCameraFmp4Store;

#[derive(Debug)]
pub struct RecordUnit {
    pub supervised: Supervised,
    pub settled: tokio::sync::watch::Receiver<bool>, // flips when the record role finalizes its open piece
}

#[derive(Debug)]
pub struct RunningCamera {
    pub client: Arc<Client>,
    pub restream_feed: Supervised,
    pub fmp4_repackage: Supervised,
    pub hls_segment: Supervised,
    pub record: Option<RecordUnit>,
}

struct CameraRoles<'a> {
    provider: &'a Arc<MultiCameraProvider>,
    fmp4_store: &'a Arc<MultiCameraFmp4Store>,
    hls_store: &'a Arc<MultiCameraHlsStore>,
    recording: Option<&'a Arc<record_role::RecordingRuntime>>,
    stop: &'a shutdown::Shutdown,
}

/// Spawns one camera's restream, fMP4, HLS and (when recording) record roles.
fn spawn_camera_roles(
    camera: &CameraSpec,
    client: &Arc<Client>,
    roles: &CameraRoles<'_>,
) -> RunningCamera {
    let name = camera.name.clone();

    let restream_feed = {
        let name = name.clone();
        let client = Arc::clone(client);
        let provider = Arc::clone(roles.provider);
        supervise::spawn_supervised_with(format!("{name}/restream-feed"), {
            move || {
                restream_provider::run_restream_feed(
                    name.clone(),
                    client.subscribe(),
                    Arc::clone(&provider),
                )
            }
        })
    };

    let fmp4_repackage = {
        let name = name.clone();
        let client = Arc::clone(client);
        let fmp4_store = Arc::clone(roles.fmp4_store);
        supervise::spawn_supervised_with(format!("{name}/fmp4-repackage"), {
            move || {
                ws_repackager::run_fmp4_repackage(
                    name.clone(),
                    client.subscribe(),
                    Arc::clone(&fmp4_store),
                )
            }
        })
    };

    let hls_segment = {
        let client = Arc::clone(client);
        let hls_store = Arc::clone(roles.hls_store);
        supervise::spawn_supervised_with(format!("{name}/hls-segment"), {
            move || hls::run_hls_segment(name.clone(), client.subscribe(), Arc::clone(&hls_store))
        })
    };

    let record = roles.recording.and_then(|runtime| {
        runtime
            .maybe_spawn_record(&camera.name, client, roles.stop)
            .map(|(supervised, settled)| RecordUnit {
                supervised,
                settled,
            })
    });

    log_event(
        &camera.name,
        "camera-started",
        format_args!("host={}", camera.host),
    );
    RunningCamera {
        client: Arc::clone(client),
        restream_feed,
        fmp4_repackage,
        hls_segment,
        record,
    }
}

/// # Errors
pub async fn start(
    config: Config,
    stop: shutdown::Shutdown,
) -> std::io::Result<impl std::future::Future<Output = shutdown::BridgeExit>> {
    start_with_recording(config, None, stop).await
}

/// Starts the camera units, the recording runtime and the three serving sockets.
///
/// The future ends with the reason to exit: a serving task, or `stop` draining.
///
/// # Errors
///
/// Returns an error if a listener fails to bind.
pub async fn start_with_recording(
    config: Config,
    recording: Option<Arc<record_role::RecordingRuntime>>,
    stop: shutdown::Shutdown,
) -> std::io::Result<impl std::future::Future<Output = shutdown::BridgeExit>> {
    if let Some(runtime) = &recording {
        runtime.warn_unknown_cameras(config.cameras.iter().map(|camera| camera.name.as_str()));
    }
    let mut provider = MultiCameraProvider::default();
    let mut fmp4_store = MultiCameraFmp4Store::default();
    let mut hls_store = MultiCameraHlsStore::default();
    for camera in &config.cameras {
        provider.register(&camera.name);
        fmp4_store.register(&camera.name);
        hls_store.register(&camera.name);
    }
    let provider = Arc::new(provider);
    let fmp4_store = Arc::new(fmp4_store);
    let hls_store = Arc::new(hls_store);

    let roles = CameraRoles {
        provider: &provider,
        fmp4_store: &fmp4_store,
        hls_store: &hls_store,
        recording: recording.as_ref(),
        stop: &stop,
    };
    let mut running = Vec::with_capacity(config.cameras.len());
    for camera in &config.cameras {
        let client = Arc::new(Client::new(camera.to_camera_config()));
        running.push(spawn_camera_roles(camera, &client, &roles));
    }

    let mut events: Option<tokio::task::JoinHandle<()>> = None;
    let events_source = std::env::var_os(event_consumer::EVENTS_SOCKET_ENV_VAR)
        .map(|socket| (socket, event_consumer::Transport::EventsHttp))
        .or_else(|| {
            std::env::var_os(event_consumer::TRACK_EVENTS_SOCKET_ENV_VAR)
                .map(|socket| (socket, event_consumer::Transport::TrackEvents))
        });
    if let Some((socket, transport)) = events_source {
        match &recording {
            Some(runtime) => {
                events = Some(runtime.spawn_event_consumer(
                    std::path::PathBuf::from(socket),
                    transport,
                    &stop,
                ));
            }
            None => log_event(
                "events",
                "track-events-socket-unbacked",
                "no recording index to write events to",
            ),
        }
    }

    let rtsp_server = rtsp_restream::RtspServer::bind(config.rtsp_bind_addr).await?;
    let fmp4_ws_server = ws_repackager::Fmp4WsServer::bind(config.fmp4_ws_bind_addr).await?;
    let hls_server = hls::HlsServer::bind(config.hls_bind_addr).await?;
    let listener = async move {
        let mut fmp4_ws_task = tokio::spawn(fmp4_ws_server.serve(fmp4_store));
        let mut hls_task = tokio::spawn(hls_server.serve(hls_store));
        tokio::select! {
            () = rtsp_server.serve(provider as Arc<dyn rtsp_restream::StreamProvider>) => {
                shutdown::BridgeExit::Served
            }
            joined = &mut fmp4_ws_task => {
                let join_error = joined.expect_err("the fMP4-WS listener task never returns Ok");
                std::panic::resume_unwind(join_error.into_panic());
            }
            joined = &mut hls_task => {
                let join_error = joined.expect_err("the HLS listener task never returns Ok");
                std::panic::resume_unwind(join_error.into_panic());
            }
            () = stop.wait() => {
                fmp4_ws_task.abort(); // serving sockets close first: no client lands on a winding-down process
                hls_task.abort();
                let drained = tokio::time::timeout(
                    shutdown::SHUTDOWN_DEADLINE,
                    shutdown::drain_for_stop(running, recording, events),
                )
                .await;
                match drained {
                    Ok(()) => shutdown::BridgeExit::Stopped,
                    Err(_elapsed) => shutdown::BridgeExit::DeadlineExceeded,
                }
            }
        }
    };

    Ok(listener)
}
