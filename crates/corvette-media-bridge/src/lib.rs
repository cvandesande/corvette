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
pub mod supervise;
pub mod utc;
pub mod ws_repackager;

use config::Config;
use corvette_rtsp_client::client::Client;
use hls::MultiCameraHlsStore;
use restream_provider::MultiCameraProvider;
use std::sync::Arc;
use supervise::{Supervised, log_event};
use ws_repackager::MultiCameraFmp4Store;

#[derive(Debug)]
pub struct RunningCamera {
    pub client: Arc<Client>,
    pub restream_feed: Supervised,
    pub fmp4_repackage: Supervised,
    pub hls_segment: Supervised,
    pub record: Option<Supervised>,
}

/// # Errors
///
/// Returns an error if a listener fails to bind.
pub async fn start(
    config: Config,
) -> std::io::Result<(Vec<RunningCamera>, impl std::future::Future<Output = ()>)> {
    start_with_recording(config, None).await
}

/// # Errors
pub async fn start_with_recording(
    config: Config,
    recording: Option<Arc<record_role::RecordingRuntime>>,
) -> std::io::Result<(Vec<RunningCamera>, impl std::future::Future<Output = ()>)> {
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

    let mut running = Vec::with_capacity(config.cameras.len());
    for camera in &config.cameras {
        let client = Arc::new(Client::new(camera.to_camera_config()));

        let restream_feed = {
            let name = camera.name.clone();
            let client = Arc::clone(&client);
            let provider = Arc::clone(&provider);
            supervise::spawn_supervised_with(format!("{name}/restream-feed"), move || {
                restream_provider::run_restream_feed(
                    name.clone(),
                    client.subscribe(),
                    Arc::clone(&provider),
                )
            })
        };

        let fmp4_repackage = {
            let name = camera.name.clone();
            let client = Arc::clone(&client);
            let fmp4_store = Arc::clone(&fmp4_store);
            supervise::spawn_supervised_with(format!("{name}/fmp4-repackage"), move || {
                ws_repackager::run_fmp4_repackage(
                    name.clone(),
                    client.subscribe(),
                    Arc::clone(&fmp4_store),
                )
            })
        };

        let hls_segment = {
            let name = camera.name.clone();
            let client = Arc::clone(&client);
            let hls_store = Arc::clone(&hls_store);
            supervise::spawn_supervised_with(format!("{name}/hls-segment"), move || {
                hls::run_hls_segment(name.clone(), client.subscribe(), Arc::clone(&hls_store))
            })
        };

        let record = recording
            .as_ref()
            .and_then(|runtime| runtime.maybe_spawn_record(&camera.name, &client));

        log_event(
            &camera.name,
            "camera-started",
            format_args!("host={}", camera.host),
        );
        running.push(RunningCamera {
            client,
            restream_feed,
            fmp4_repackage,
            hls_segment,
            record,
        });
    }

    if let Some(socket) = std::env::var_os(event_consumer::TRACK_EVENTS_SOCKET_ENV_VAR) {
        match &recording {
            Some(runtime) => runtime.spawn_event_consumer(std::path::PathBuf::from(socket)),
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
        let fmp4_ws_task = tokio::spawn(fmp4_ws_server.serve(fmp4_store));
        let hls_task = tokio::spawn(hls_server.serve(hls_store));
        tokio::select! {
            () = rtsp_server.serve(provider as Arc<dyn rtsp_restream::StreamProvider>) => {}
            joined = fmp4_ws_task => {
                let join_error = joined.expect_err("the fMP4-WS listener task never returns Ok");
                std::panic::resume_unwind(join_error.into_panic());
            }
            joined = hls_task => {
                let join_error = joined.expect_err("the HLS listener task never returns Ok");
                std::panic::resume_unwind(join_error.into_panic());
            }
        }
    };

    Ok((running, listener))
}
