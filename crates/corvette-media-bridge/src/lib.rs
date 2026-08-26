//! Wires issue #18's shipped `corvette-rtsp-client` to `rtsp-restream` and to
//! a direct `MoQ`-publish loop, one process, one task per camera per role
//! (issue #12, item G1, D-8/D-9).
//!
//! # A note on this crate's own `clippy::multiple_crate_versions` bypass
//!
//! Every version split `cargo clippy`'s `clippy::cargo` group would
//! otherwise flag (`base64`, `bitflags`, `getrandom`, `hashbrown`,
//! `indexmap`, `schemars`, `syn`, `thiserror`, `windows-sys`) resolves
//! entirely *within* the git-pinned `moq-dev/moq` dependency tree this
//! item's own R1 already established
//! (`.agents/issue-12/evidence/R1-dependency-pin.md`) -- confirmed by
//! `cargo tree -i <pkg>@<version>` for every flagged package before adding
//! this bypass: each duplicate's two resolution paths both terminate inside
//! `moq-mux`/`moq-net`/`hang`/`web-transport-quinn`'s own transitive
//! dependencies (e.g. `webm-iterable`'s `syn 1` proc-macro alongside
//! `web-transport-proto`'s `sfv` pulling `syn 3` via `ref-cast-impl`), never
//! between this workspace's own pre-existing crates and the new ones. There
//! is no code change on this crate's side that resolves it: the split lives
//! inside a third-party monorepo this item does not own and this item's own
//! Scope guard forbids editing (`crates/corvette-rtsp-client`,
//! `crates/rtsp-restream`) or that would require vendoring/patching several
//! of moq-dev/moq's own transitive dependencies locally, a maintenance
//! burden out of proportion to this item. `crates/corvette-ui/src/lib.rs`
//! already carries the identical bypass for the same reason (genuine
//! third-party major-version duplicates it does not control either).
//!
//! `config` reads this process's own configuration (a named, owned stub --
//! Corvette does not yet own configuration generally, per
//! `docs/design/architecture.md`'s "Risk boundary" section).
//!
//! `restream_provider` is role (a): adapts `corvette_rtsp_client::Frame` into
//! `rtsp_restream`'s own `Frame`/`FrameReceiver` shape and implements
//! `StreamProvider` dispatching by camera name.
//!
//! `moq_publish` is role (b): drives the MoQ-publish loop directly via
//! `moq_mux::codec::{h264,h265}::{Split,Import}` (D-9), dialing the relay via
//! `moq_net`/`web_transport_quinn` directly (D-5).
//!
//! `fmp4`/`ws_repackager` are role (c) (issue #12 item G2, DP-4): a third
//! independent per-camera subscription that repackages the same frame stream
//! into fragmented MP4 and serves it over one WebSocket connection per
//! viewer, for the UI's grid tile.
//!
//! `hls` is role (d) (issue #12 item G3, DP-4): a fourth independent
//! per-camera subscription that groups the same fMP4 fragmentation core
//! `fmp4` already builds (reused directly, not duplicated -- see `hls`'s own
//! module doc) into CMAF media segments plus a rolling `m3u8` playlist,
//! served over a plain HTTP listener, for the UI's expanded-view fallback.
//!
//! `parameter_sets` is the Annex-B parameter-set NAL scanning `restream_provider`
//! and `fmp4` both need, shared between them rather than duplicated.
//!
//! `supervise` is this item's own restart-on-panic convention (INV-5(b),
//! DP-3/DP-4), matching `corvette-rtsp-client`'s own established shape.
//!
//! `rtp_clock` converts each track's raw RTP timestamps into elapsed
//! `Duration`s, shared by every role.
//!
//! `start` wires every configured camera's `corvette_rtsp_client::Client` to
//! all four roles and hosts the three whole-process listeners
//! (`rtsp_restream::RtspServer`, [`ws_repackager::Fmp4WsServer`], and
//! [`hls::HlsServer`]).
//!
//! **A real, currently-shipped gap this crate inherits but does not fix**
//! (see `moq_publish`'s and `restream_provider`'s own module docs, and G1's
//! own Premise in `.agents/issue-12/PLAN-live-view.md`): `corvette-rtsp-
//! client`'s own SDP resolution only ever resolves an `m=video` section, so
//! no AAC frame is ever actually produced by that crate today, even though
//! the AAC codec-level API exists (unit-tested, unused) on both sides. This
//! crate's own dispatch is written generically over
//! `corvette_rtsp_client::depacketize::Codec`, but only video is exercised
//! end to end today.

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

/// One configured camera's running state: the `Client` dialing it, and the
/// four independent supervised tasks (Do step 5, INV-5, DP-3/DP-4) reading
/// its four independent subscriptions.
#[derive(Debug)]
pub struct RunningCamera {
    pub client: Arc<Client>,
    pub restream_feed: Supervised,
    pub moq_publish: Supervised,
    pub fmp4_repackage: Supervised,
    pub hls_segment: Supervised,
}

/// Wires every configured camera to all four roles and hosts the three
/// whole-process listeners (`rtsp_restream::RtspServer`,
/// [`ws_repackager::Fmp4WsServer`], and [`hls::HlsServer`], Do steps 3-5).
///
/// Never returns on success -- both listeners' own `serve` methods never
/// return.
///
/// Returns the started cameras' own handles (needed only so a caller, e.g.
/// this item's own integration tests, can hold or drop them) alongside the
/// listener future to drive.
///
/// # Errors
///
/// Returns an error if either listener fails to bind.
pub async fn start(
    config: Config,
) -> std::io::Result<(Vec<RunningCamera>, impl std::future::Future<Output = ()>)> {
    // Every camera is registered before any task that reads either store is
    // spawned, so neither map is ever mutated concurrently with a lookup --
    // see `MultiCameraProvider::camera`'s and `MultiCameraFmp4Store`'s own
    // doc.
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
        // Neither `Fmp4WsServer::serve` nor `HlsServer::serve` ever returns
        // on success either (both are `-> !`, matching `RtspServer::serve`),
        // so a task join completing here always means it panicked --
        // propagated into this future's own caller rather than silently
        // leaving the other two listeners running, matching what awaiting
        // `RtspServer::serve` directly would already do on its own panic.
        let fmp4_ws_task = tokio::spawn(fmp4_ws_server.serve(fmp4_store));
        let hls_task = tokio::spawn(hls_server.serve(hls_store));
        tokio::select! {
            () = rtsp_server.serve(provider as Arc<dyn rtsp_restream::StreamProvider>) => {}
            joined = fmp4_ws_task => {
                // `serve`'s own `-> !` return type means this join can never
                // observe `Ok`.
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
