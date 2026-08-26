//! A from-scratch, embeddable RTSP restream server (issue #12 D-6/D-7):
//! `OPTIONS`, `DESCRIBE`, `SETUP`, `PLAY`, `TEARDOWN` over TCP-interleaved
//! transport only, against a caller-supplied frame source.
//!
//! `provider` is this crate's Corvette-agnostic boundary: the
//! [`StreamProvider`] trait and the [`Frame`]/[`StreamInfo`] shapes an
//! embedder implements to hand this crate named streams and their frames.
//! Nothing here knows about any particular camera-dialing crate, container
//! format, or MoQ/Corvette-specific type.
//!
//! `session` is the protocol/session state machine: it parses and builds
//! [`rtsp_types`] messages and tracks one client connection's state, but
//! performs no socket I/O of its own.
//!
//! `sdp` builds a `DESCRIBE` response's SDP body from a [`StreamInfo`].
//! `session` is its only caller in this crate, but it is plain, standalone
//! logic over public types, so it is exposed rather than hidden behind an
//! artificial boundary.
//!
//! `packetize` turns a [`Frame`] back into the RTP payloads a real camera
//! would have sent for the same access unit (issue #12 item X2) -- the
//! mirror image of issue #18's own `depacketize` module. This crate
//! deliberately does not implement the actual `TcpListener`/`accept()` loop
//! that drives an [`RtspSession`] over a real socket and a packetizer's
//! output onto it -- that is a separate concern layered on top of this
//! crate.

pub mod packetize;
pub mod provider;
pub mod sdp;
pub mod session;

pub use packetize::{AacPacketizer, H264Packetizer, H265Packetizer, PacketizeError};
pub use provider::{Frame, FrameReceiver, StreamInfo, StreamProvider, TrackInfo};
pub use session::RtspSession;
