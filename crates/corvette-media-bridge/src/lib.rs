// Duplicate dependency versions all resolve inside the git-pinned third-party moq-dev/moq tree; nothing in this crate can resolve them.
#![allow(clippy::multiple_crate_versions)]

pub mod config;
pub mod fmp4;
pub mod hls;
pub mod moq_publish;
pub mod parameter_sets;
pub mod restream_provider;
pub mod rtp_clock;
pub mod supervise;
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
    pub moq_publish: Supervised,
    pub fmp4_repackage: Supervised,
    pub hls_segment: Supervised,
}

/// # Errors
///
/// Returns an error if a listener fails to bind.
pub async fn start(
    config: Config,
) -> std::io::Result<(Vec<RunningCamera>, impl std::future::Future<Output = ()>)> {
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

        let moq_publish = {
            let name = camera.name.clone();
            let client = Arc::clone(&client);
            let moq_config = config.moq.clone();
            supervise::spawn_supervised_with(format!("{name}/moq-publish"), move || {
                moq_publish::run_publish_loop(name.clone(), moq_config.clone(), client.subscribe())
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

        log_event(
            &camera.name,
            "camera-started",
            format_args!("host={}", camera.host),
        );
        running.push(RunningCamera {
            client,
            restream_feed,
            moq_publish,
            fmp4_repackage,
            hls_segment,
        });
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
