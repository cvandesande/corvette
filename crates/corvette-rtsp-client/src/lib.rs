//! RTSP client for dialing physical IP cameras.
//!
//! `session` is the protocol layer: it performs the Digest-authenticated
//! `DESCRIBE`/`SETUP`/`PLAY` handshake, schedules keep-alives at a real
//! margin against each camera's own declared timeout, and demuxes the
//! resulting interleaved RTP frames. It hands raw per-track RTP payloads to
//! a later depacketization layer; it does not decode them itself.
//!
//! `mock_camera` is a test double this crate's own test suites dial against
//! in place of a real camera; it is never a production dependency.

pub mod session;

#[cfg(feature = "mock-camera")]
pub mod mock_camera;
