//! Per-camera public client: dials one camera, publishes its depacketized
//! frames onto a broadcast channel every subscriber can independently drain,
//! and keeps running across reconnects and task panics.
//!
//! Discovering which cameras exist in a deployment and constructing one
//! [`CameraConfig`] per camera is deliberately left to the caller -- this
//! crate dials exactly the camera it is given and never enumerates a
//! configuration source of its own.

use crate::depacketize::Frame;
use crate::session::Credentials;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::broadcast;
use tokio::task::{AbortHandle, JoinHandle};

mod task;

/// Matches the already-shipped `dahua-camera-rtsp` precedent this crate
/// follows for its own default broadcast capacity; see
/// `docs/design/api-contracts.md` for the reasoning. Overridable per camera
/// via [`CameraConfig::channel_capacity`].
const DEFAULT_CHANNEL_CAPACITY: usize = 64;

/// One camera's connection details: where to dial it, what stream path to
/// request, and which Digest credentials to authenticate with.
///
/// The RTSP transport is always TCP-interleaved; there is no configurable
/// UDP alternative.
#[derive(Debug, Clone)]
pub struct CameraConfig {
    /// A display name for this camera, used only to distinguish one
    /// camera's log lines (connect/disconnect/reconnect/panic) from
    /// another's -- it has no protocol meaning and is never sent to the
    /// camera.
    pub name: String,
    pub host: IpAddr,
    pub port: u16,
    pub path: String,
    pub credentials: Credentials,
    channel_capacity: usize,
}

impl CameraConfig {
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        host: IpAddr,
        port: u16,
        path: impl Into<String>,
        credentials: Credentials,
    ) -> Self {
        Self {
            name: name.into(),
            host,
            port,
            path: path.into(),
            credentials,
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
        }
    }

    /// Overrides the default broadcast capacity (64 frames) for this
    /// camera's channel. A slower-draining subscriber than this capacity
    /// allows observes `RecvError::Lagged` on its next receive rather than
    /// unbounded memory growth in the sender.
    #[must_use]
    pub const fn channel_capacity(mut self, capacity: usize) -> Self {
        self.channel_capacity = capacity;
        self
    }
}

/// A running connection to one camera.
///
/// Constructing a [`Client`] spawns a supervised background task that
/// performs the session handshake, depacketizes frames, and publishes them
/// onto an internal broadcast channel; [`Client::subscribe`] hands out
/// independent receivers of that same channel. The task reconnects with
/// backoff on any session-ending condition and is restarted if it ever
/// panics -- neither affects the broadcast channel itself, so existing
/// subscribers see a gap (resynced via the channel's own semantics) rather
/// than being dropped or forced to resubscribe.
///
/// Dropping a `Client` stops dialing the camera and ends its background
/// task; already-cloned [`Frame`] receivers simply stop receiving new
/// frames.
pub struct Client {
    sender: broadcast::Sender<Frame>,
    supervisor: JoinHandle<()>,
    active_worker: Arc<Mutex<Option<AbortHandle>>>,
}

impl Client {
    /// Starts dialing `config`'s camera in the background.
    ///
    /// # Panics
    ///
    /// Panics if called outside a Tokio runtime context, per
    /// [`tokio::spawn`]'s own contract.
    #[must_use]
    pub fn new(config: CameraConfig) -> Self {
        let (sender, _first_receiver) = broadcast::channel(config.channel_capacity);
        let (supervisor, active_worker) = task::spawn_supervised(config, sender.clone());
        Self {
            sender,
            supervisor,
            active_worker,
        }
    }

    /// Hands out a new, independent receiver of this camera's frames.
    /// Every subscriber -- however many, constructed at any time -- sees
    /// the same frames from the point it subscribes.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Frame> {
        self.sender.subscribe()
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").finish_non_exhaustive()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let mut guard = self
            .active_worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(worker) = guard.take() {
            worker.abort();
        }
        drop(guard);
        self.supervisor.abort();
    }
}
