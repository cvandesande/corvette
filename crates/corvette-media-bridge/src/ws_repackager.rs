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

// 64: matches restream_provider's capacity and CameraConfig's documented default.
const FRAGMENT_CHANNEL_CAPACITY: usize = 64;

#[derive(Debug)]
struct CameraFmp4 {
    init: watch::Sender<Option<Bytes>>,
    fragments: broadcast::Sender<Bytes>,
}

fn new_camera_state() -> Arc<CameraFmp4> {
    let (init, _first_init_receiver) = watch::channel(None);
    let (fragments, _first_fragment_receiver) = broadcast::channel(FRAGMENT_CHANNEL_CAPACITY);
    Arc::new(CameraFmp4 { init, fragments })
}

#[derive(Clone, Debug)]
pub struct Fmp4Camera {
    name: String,
    state: Arc<CameraFmp4>,
}

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
    pub fn new(names: impl IntoIterator<Item = impl Into<String>>) -> (Self, Vec<Fmp4Camera>) {
        let mut cameras: HashMap<String, Arc<CameraFmp4>> = HashMap::new();
        let mut handles = Vec::new();
        for name in names {
            let name = name.into();
            let state = cameras
                .entry(name.clone())
                .or_insert_with(new_camera_state)
                .clone();
            handles.push(Fmp4Camera { name, state });
        }
        (Self { cameras }, handles)
    }

    fn camera(&self, name: &str) -> Option<Arc<CameraFmp4>> {
        self.cameras.get(name).cloned()
    }
}

pub async fn run_fmp4_repackage(
    camera: Fmp4Camera,
    mut client_frames: broadcast::Receiver<ClientFrame>,
) {
    let mut init_tracker = InitSegmentTracker::default();
    let mut fragmenter = Fragmenter::default();

    loop {
        match client_frames.recv().await {
            Ok(frame) => {
                if let Some(segment) = init_tracker.observe(&frame) {
                    // send_replace: watch::send stores nothing while no viewer is connected, losing the one init segment to late joiners.
                    camera.state.init.send_replace(Some(segment));
                }
                if let Some(fragment) = fragmenter.next(&frame) {
                    let _ = camera.state.fragments.send(fragment.bytes);
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => {
                log_event(
                    &camera.name,
                    "fmp4-repackage-disconnect",
                    "camera client dropped",
                );
                return;
            }
        }
    }
}

#[derive(Debug)]
pub struct Fmp4WsServer {
    listener: TcpListener,
    addr: SocketAddr,
}

impl Fmp4WsServer {
    /// # Errors
    ///
    /// Returns an error if binding fails.
    pub async fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self {
            addr: listener.local_addr()?,
            listener,
        })
    }

    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn serve(self, store: Arc<MultiCameraFmp4Store>) -> ! {
        loop {
            match self.listener.accept().await {
                Ok((stream, _)) => {
                    let store = Arc::clone(&store);
                    tokio::spawn(async move {
                        handle_connection(stream, &store).await;
                    });
                }
                Err(err) => {
                    log_event("fmp4-ws-listener", "accept-error", err);
                }
            }
        }
    }
}

async fn handle_connection(stream: TcpStream, store: &MultiCameraFmp4Store) {
    let mut camera_name = String::new();
    #[expect(
        clippy::result_large_err,
        reason = "tungstenite's Callback trait fixes the Err type (issue #29)"
    )]
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
