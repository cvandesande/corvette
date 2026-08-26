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
//! `supervise` is this item's own restart-on-panic convention (INV-5(b),
//! DP-3), matching `corvette-rtsp-client`'s own established shape.
//!
//! `rtp_clock` converts each track's raw RTP timestamps into elapsed
//! `Duration`s, shared by both roles.
//!
//! `run` wires every configured camera's `corvette_rtsp_client::Client` to
//! both roles and hosts the one whole-process `rtsp_restream::RtspServer`
//! (Do steps 3-5).
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
pub mod moq_publish;
pub mod restream_provider;
pub mod rtp_clock;
pub mod supervise;

use config::Config;
use corvette_rtsp_client::client::Client;
use restream_provider::MultiCameraProvider;
use std::sync::Arc;
use supervise::{Supervised, log_event};

/// One configured camera's running state: the `Client` dialing it, and the
/// two independent supervised tasks (Do step 5, INV-5, DP-3) reading its two
/// independent subscriptions.
#[derive(Debug)]
pub struct RunningCamera {
    pub client: Arc<Client>,
    pub restream_feed: Supervised,
    pub moq_publish: Supervised,
}

/// Wires every configured camera to both roles and hosts the one
/// whole-process `rtsp_restream::RtspServer` (Do steps 3-5). Never returns on
/// success -- `RtspServer::serve` itself never returns.
///
/// Returns the started cameras' own handles (needed only so a caller, e.g.
/// this item's own integration tests, can hold or drop them) alongside the
/// listener future to drive.
///
/// # Errors
///
/// Returns an error if the RTSP-restream listener fails to bind.
pub async fn start(
    config: Config,
) -> std::io::Result<(Vec<RunningCamera>, impl std::future::Future<Output = ()>)> {
    // Every camera is registered before any task that reads the provider is
    // spawned, so the map itself is never mutated concurrently with a
    // lookup -- see `MultiCameraProvider::camera`'s own doc.
    let mut provider = MultiCameraProvider::default();
    for camera in &config.cameras {
        provider.register(&camera.name);
    }
    let provider = Arc::new(provider);

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

        log_event(
            &camera.name,
            "camera-started",
            format_args!("host={}", camera.host),
        );
        running.push(RunningCamera {
            client,
            restream_feed,
            moq_publish,
        });
    }

    let server = rtsp_restream::RtspServer::bind(config.rtsp_bind_addr).await?;
    let listener = async move {
        server
            .serve(provider as Arc<dyn rtsp_restream::StreamProvider>)
            .await;
    };

    Ok((running, listener))
}
