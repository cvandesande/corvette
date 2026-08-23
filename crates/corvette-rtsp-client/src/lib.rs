//! RTSP client for dialing physical IP cameras.
//!
//! The client itself has not landed yet. This crate currently ships only its
//! own test double, `mock_camera`, which the session/protocol layer's future
//! test suites dial against in place of a real camera.

#[cfg(feature = "mock-camera")]
pub mod mock_camera;
