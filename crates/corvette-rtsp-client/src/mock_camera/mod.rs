//! A minimal RTSP/1.0 server reproducing camera quirks this project has
//! confirmed by direct testing against real hardware.
//!
//! See `docs/design/api-contracts.md`'s keep-alive section for the source of
//! one of them: Digest authentication that never accepts a stale nonce, an
//! SDP body whose session-level and track-level `a=control` lines can trip
//! up a careless parser, a declared `SETUP` timeout the caller controls, and
//! a keep-alive window whose expiry silently stops the RTP stream rather
//! than closing the connection or returning an error.
//!
//! This module exists only to be dialed by this crate's own test suites
//! (`mock-camera` feature) -- it is never a real camera implementation and
//! never binds anything but a loopback address.

mod connection;
mod digest;
mod rtp;
mod rtsp_message;

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Declared `SETUP` timeout the real, directly tested Reolink cameras use;
/// see `docs/design/api-contracts.md`'s keep-alive section.
const DEFAULT_DECLARED_TIMEOUT: Duration = Duration::from_secs(65);
const DEFAULT_FRAME_INTERVAL: Duration = Duration::from_millis(40);
const DEFAULT_USERNAME: &str = "admin";
const DEFAULT_PASSWORD: &str = "test-password-not-real";

/// Configuration for a [`MockCamera`], controlling which of its quirks a
/// test exercises.
#[derive(Debug, Clone)]
pub struct MockCameraConfig {
    username: String,
    password: String,
    declared_timeout: Duration,
    frame_interval: Duration,
    delay_keepalive_reply_to_next_frame: bool,
}

impl Default for MockCameraConfig {
    fn default() -> Self {
        Self {
            username: DEFAULT_USERNAME.to_string(),
            password: DEFAULT_PASSWORD.to_string(),
            declared_timeout: DEFAULT_DECLARED_TIMEOUT,
            frame_interval: DEFAULT_FRAME_INTERVAL,
            delay_keepalive_reply_to_next_frame: false,
        }
    }
}

impl MockCameraConfig {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the `timeout=` value the mock declares on `SETUP` and the
    /// keep-alive window after which it silently stops sending frames.
    #[must_use]
    pub const fn declared_timeout(mut self, timeout: Duration) -> Self {
        self.declared_timeout = timeout;
        self
    }

    /// Sets the interval between synthetic RTP frames sent after `PLAY`.
    #[must_use]
    pub const fn frame_interval(mut self, interval: Duration) -> Self {
        self.frame_interval = interval;
        self
    }

    /// When `true`, a `GET_PARAMETER`/`OPTIONS` reply that arrives mid-stream
    /// is deliberately held back and sent right after the next scheduled RTP
    /// frame, deterministically reproducing the interleaved binary/text
    /// ordering a client's demuxer must handle, rather than relying on
    /// incidental timing.
    #[must_use]
    pub const fn delay_keepalive_reply_to_next_frame(mut self, delay: bool) -> Self {
        self.delay_keepalive_reply_to_next_frame = delay;
        self
    }

    /// Overrides the Digest credentials the mock expects. Both values must
    /// be obviously-fake test fixtures -- this mock never talks to, and must
    /// never be configured with, a real camera's own address or password.
    #[must_use]
    pub fn credentials(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.username = username.into();
        self.password = password.into();
        self
    }
}

/// A running mock camera, bound to a loopback address for the duration of
/// one test.
#[derive(Debug)]
pub struct MockCamera {
    addr: SocketAddr,
    close_signal: Arc<Notify>,
    accept_task: JoinHandle<()>,
}

impl MockCamera {
    /// Binds a loopback listener and starts accepting connections.
    ///
    /// # Errors
    ///
    /// Returns an error if binding a loopback TCP listener fails.
    pub async fn spawn(config: MockCameraConfig) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;
        let config = Arc::new(config);
        let close_signal = Arc::new(Notify::new());

        let accept_task = tokio::spawn({
            let close_signal = Arc::clone(&close_signal);
            async move {
                loop {
                    let Ok((stream, _peer)) = listener.accept().await else {
                        return;
                    };
                    tokio::spawn(connection::handle(
                        stream,
                        Arc::clone(&config),
                        Arc::clone(&close_signal),
                        addr,
                    ));
                }
            }
        });

        Ok(Self {
            addr,
            close_signal,
            accept_task,
        })
    }

    /// The loopback address this mock is listening on.
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Simulates a mid-session TCP close: any connection currently streaming
    /// frames drops its socket immediately, with no RTSP `TEARDOWN`.
    pub fn disconnect(&self) {
        self.close_signal.notify_waiters();
    }
}

impl Drop for MockCamera {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}
