//! Hosts the grid tile's fMP4-over-WebSocket transport (issue #12 item G2,
//! DP-4).
//!
//! A third independent per-camera subscription to the same
//! `corvette_rtsp_client::Frame` broadcast G1's own restream-feed and
//! MoQ-publish roles already read from, muxed by `crate::fmp4` and served to
//! one WebSocket connection per viewer, named by path (`/<camera name>`) --
//! the same per-viewer-connection shape go2rtc's own existing MSE tile
//! transport uses.
//!
//! [`run_fmp4_repackage`] is this camera's own supervised worker (spawned via
//! `crate::supervise::spawn_supervised_with`, matching G1's restream-feed/
//! moq-publish tasks -- INV-5, DP-4): it drains this camera's frame stream
//! into a [`CameraFmp4`]'s shared state. [`Fmp4WsServer`] is the one
//! whole-process WebSocket listener (mirroring `rtsp_restream::RtspServer`'s
//! own one-listener-for-the-process shape): each inbound connection is a
//! plain `tokio::spawn` with no restart-on-panic supervision of its own,
//! matching `rtsp-restream`'s own per-session precedent (a session-ending
//! failure just ends that connection; there is no resubscribe-and-resume
//! protocol to recover into) -- only the per-camera repackaging task above
//! needs INV-5(b)'s restart-on-panic recovery, since a dead viewer
//! connection has no state worth restarting.

use crate::fmp4::{Fragmenter, InitSegmentTracker};
use crate::supervise::log_event;
use bytes::Bytes;
use corvette_rtsp_client::depacketize::Frame as ClientFrame;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

/// The only per-camera capacity this item's own internal fragment broadcast
/// channel needs; matches `restream_provider`'s own equivalent constant and
/// `corvette_rtsp_client::CameraConfig`'s own documented default. A slow
/// viewer that falls this far behind a live camera has no use for the missed
/// fragments anyway (see [`stream_to_viewer`]'s own lag handling).
const FRAGMENT_CHANNEL_CAPACITY: usize = 64;

/// One camera's fMP4 state as viewers see it: the current initialization
/// segment (`None` until this camera's own repackaging task has resolved a
/// parameter set), broadcast over a [`watch`] channel so a connecting viewer
/// can either read it immediately or wait for the first resolution, and the
/// live fragment stream every connected viewer receives from the point they
/// connected onward (no fragment history/backfill -- matching this item's own
/// "grid tile" scope, not a seekable transport).
struct CameraFmp4 {
    init: watch::Sender<Option<Bytes>>,
    fragments: broadcast::Sender<Bytes>,
}

/// Dispatches by camera name, matching
/// `restream_provider::MultiCameraProvider`'s own shape.
///
/// The camera set is fixed at construction (every configured camera is
/// [`register`](Self::register)ed before any task that reads or serves it is
/// spawned), so `&self` methods need no synchronization for the map itself,
/// only for each [`CameraFmp4`]'s own per-camera state (already provided by
/// `watch`/`broadcast`'s own interior synchronization).
#[derive(Default)]
pub struct MultiCameraFmp4Store {
    cameras: HashMap<String, Arc<CameraFmp4>>,
}

impl std::fmt::Debug for MultiCameraFmp4Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiCameraFmp4Store")
            .field("cameras", &self.cameras.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl MultiCameraFmp4Store {
    /// Registers `name`, so a later [`run_fmp4_repackage`] call and a
    /// viewer's own WebSocket connection can both find its shared state.
    ///
    /// # Panics
    ///
    /// Panics if `name` is already registered -- this item's own camera set
    /// is fixed at startup, matching `MultiCameraProvider::register`'s own
    /// contract.
    pub fn register(&mut self, name: &str) {
        let (init, _first_init_receiver) = watch::channel(None);
        let (fragments, _first_fragment_receiver) = broadcast::channel(FRAGMENT_CHANNEL_CAPACITY);
        let previous = self
            .cameras
            .insert(name.to_string(), Arc::new(CameraFmp4 { init, fragments }));
        assert!(previous.is_none(), "camera {name:?} registered twice");
    }

    fn camera(&self, name: &str) -> Option<Arc<CameraFmp4>> {
        self.cameras.get(name).cloned()
    }
}

/// Runs one camera's fMP4-repackaging role for as long as `client_frames`
/// keeps producing.
///
/// Never returns on its own (except when the upstream camera's own `Client`
/// is dropped, matching `restream_provider::run_restream_feed`'s own
/// shutdown convention) -- intended to be spawned via
/// `crate::supervise::spawn_supervised_with`, matching DP-4/INV-5.
///
/// # Panics
///
/// Panics if `name` was never [`MultiCameraFmp4Store::register`]ed -- a
/// caller bug, matching `run_restream_feed`'s own contract.
pub async fn run_fmp4_repackage(
    name: String,
    mut client_frames: broadcast::Receiver<ClientFrame>,
    store: Arc<MultiCameraFmp4Store>,
) {
    let camera = store
        .camera(&name)
        .unwrap_or_else(|| panic!("camera {name:?} was never registered"));
    let mut init_tracker = InitSegmentTracker::default();
    let mut fragmenter = Fragmenter::default();

    loop {
        match client_frames.recv().await {
            Ok(frame) => {
                if let Some(segment) = init_tracker.observe(&frame) {
                    // `watch::Sender::send` -- unlike `broadcast::Sender::send`
                    // -- does NOT update the stored value at all when there
                    // are zero current receivers (`tokio::sync::watch`'s own
                    // source, `Sender::send`: it returns early on
                    // `receiver_count() == 0`, before ever touching the
                    // value). A camera's parameter sets typically resolve
                    // before any viewer has ever connected, so relying on
                    // `send` here would silently drop the only initialization
                    // segment this tracker will ever produce for an unchanged
                    // parameter set, leaving every future viewer waiting
                    // forever. `send_replace` has the semantics this role
                    // actually needs: it always stores the value and notifies
                    // any receivers that do exist, succeeding unconditionally.
                    camera.init.send_replace(Some(segment));
                }
                if let Some(fragment) = fragmenter.next(&frame) {
                    let _ = camera.fragments.send(fragment);
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => {
                log_event(&name, "fmp4-repackage-disconnect", "camera client dropped");
                return;
            }
        }
    }
}

/// The whole-process WebSocket listener (Do step 3, D-3's "configuration
/// point" convention for its bind address).
#[derive(Debug)]
pub struct Fmp4WsServer {
    listener: TcpListener,
}

impl Fmp4WsServer {
    /// Binds `addr`. Does not yet accept any connection -- see [`Self::serve`].
    ///
    /// # Errors
    ///
    /// Returns an error if binding fails.
    pub async fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(addr).await?,
        })
    }

    /// The address this server actually bound to -- useful when constructed
    /// with an ephemeral port (`:0`), as this item's own tests do.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying socket has no local address (never
    /// expected for a bound, non-closed listener).
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Accepts connections forever, dispatching each to the camera its
    /// request path names. Never returns on success, matching
    /// `rtsp_restream::RtspServer::serve`'s own shape.
    pub async fn serve(self, store: Arc<MultiCameraFmp4Store>) -> ! {
        loop {
            match self.listener.accept().await {
                Ok((stream, peer_addr)) => {
                    let store = Arc::clone(&store);
                    tokio::spawn(async move {
                        handle_connection(stream, &store).await;
                    });
                    let _ = peer_addr; // identifies the connection in a real log line only
                }
                Err(err) => {
                    log_event("fmp4-ws-listener", "accept-error", err);
                }
            }
        }
    }
}

/// Handles one inbound WebSocket connection end to end: the handshake
/// (which also resolves the requested camera name from the request path),
/// then streaming that camera's own output until the viewer disconnects or
/// this camera's upstream state disappears.
///
/// A request naming a camera this process never registered completes the
/// WebSocket upgrade and then closes immediately -- matching
/// `rtsp-restream`'s own per-session precedent (see this module's own
/// top-level doc): a session-ending condition just ends that connection,
/// with no further protocol beyond a close.
async fn handle_connection(stream: TcpStream, store: &MultiCameraFmp4Store) {
    let mut camera_name = String::new();
    let handshake =
        tokio_tungstenite::accept_hdr_async(stream, |request: &Request, response: Response| {
            camera_name = request.uri().path().trim_start_matches('/').to_string();
            Ok(response)
        })
        .await;

    let mut ws_stream = match handshake {
        Ok(stream) => stream,
        Err(err) => {
            log_event("fmp4-ws-listener", "handshake-error", err);
            return;
        }
    };

    let Some(camera) = store.camera(&camera_name) else {
        log_event(
            &camera_name,
            "fmp4-ws-unknown-camera",
            "closing: no such camera is configured",
        );
        let _ = ws_stream.close(None).await;
        return;
    };

    if let Err(err) = stream_to_viewer(ws_stream, &camera).await {
        log_event(&camera_name, "fmp4-ws-viewer-disconnect", err);
    }
}

/// Sends `camera`'s current initialization segment first (waiting for one if
/// this camera has not resolved its parameter sets yet), then forwards every
/// fragment produced from that point on until the viewer disconnects or this
/// camera's own repackaging task ends.
///
/// Also drains the viewer's own inbound frames (never expected to carry
/// anything beyond protocol-level pings and an eventual close), both to let
/// `tokio-tungstenite` answer a ping automatically and to notice a
/// viewer-initiated close promptly rather than only via a later failed send.
async fn stream_to_viewer(
    mut ws: tokio_tungstenite::WebSocketStream<TcpStream>,
    camera: &CameraFmp4,
) -> Result<(), ViewerError> {
    let mut init_rx = camera.init.subscribe();
    let already_resolved = init_rx.borrow_and_update().clone();
    let init_segment = match already_resolved {
        Some(segment) => segment,
        None => loop {
            init_rx
                .changed()
                .await
                .map_err(|_| ViewerError::CameraGone)?;
            let resolved = init_rx.borrow_and_update().clone();
            if let Some(segment) = resolved {
                break segment;
            }
        },
    };
    ws.send(Message::Binary(init_segment))
        .await
        .map_err(ViewerError::Send)?;

    let mut fragments = camera.fragments.subscribe();
    loop {
        tokio::select! {
            biased;
            incoming = ws.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => return Ok(()),
                    Some(Ok(_)) => {}
                    Some(Err(err)) => return Err(ViewerError::Recv(err)),
                }
            }
            fragment = fragments.recv() => {
                match fragment {
                    Ok(bytes) => ws.send(Message::Binary(bytes)).await.map_err(ViewerError::Send)?,
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }
    }
}

#[derive(Debug)]
enum ViewerError {
    /// This camera's own repackaging task ended (its `watch::Sender` was
    /// dropped) before ever resolving a parameter set.
    CameraGone,
    Send(tokio_tungstenite::tungstenite::Error),
    Recv(tokio_tungstenite::tungstenite::Error),
}

impl std::fmt::Display for ViewerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CameraGone => write!(
                f,
                "this camera's repackaging task ended before an initialization segment was ever resolved"
            ),
            Self::Send(err) => write!(f, "sending to the viewer: {err}"),
            Self::Recv(err) => write!(f, "reading from the viewer: {err}"),
        }
    }
}
