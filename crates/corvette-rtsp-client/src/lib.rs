//! RTSP client for dialing physical IP cameras.
//!
//! `session` is the protocol layer: it performs the Digest-authenticated
//! `DESCRIBE`/`SETUP`/`PLAY` handshake, schedules keep-alives at a real
//! margin against each camera's own declared timeout, and demuxes the
//! resulting interleaved RTP frames. It hands raw per-track RTP payloads to
//! `depacketize`; it does not decode them itself.
//!
//! `depacketize` turns those raw RTP payloads into Annex-B-framed
//! H.264/H.265 access units and raw, non-ADTS AAC access units -- no
//! container format anywhere in this crate.
//!
//! `client` ties the two together: one supervised background task per
//! camera runs the session and depacketizer, publishing frames onto a
//! broadcast channel every subscriber gets its own receiver of.
//!
//! `mock_camera` is a test double this crate's own test suites dial against
//! in place of a real camera; it is never a production dependency.

pub mod client;
pub mod depacketize;
pub mod session;

#[cfg(feature = "mock-camera")]
pub mod mock_camera;
