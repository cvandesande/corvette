//! Embeddable RTSP restream server: OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN over TCP-interleaved transport only.
//! Streams and frames come from a caller-implemented [`StreamProvider`]; nothing here knows a particular camera crate.

pub mod packetize;
pub mod provider;
pub mod sdp;
pub mod server;
pub mod session;

pub use packetize::{AacPacketizer, H264Packetizer, H265Packetizer, PacketizeError};
pub use provider::{Frame, FrameReceiver, StreamInfo, StreamProvider, TrackInfo};
pub use server::RtspServer;
pub use session::{RtspSession, SetupTrack};
