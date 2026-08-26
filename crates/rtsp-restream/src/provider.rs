//! The crate's Corvette-agnostic boundary: a caller supplies frames and
//! codec parameters for named streams through [`StreamProvider`].
//!
//! This crate never knows where those streams, or their frames, actually
//! come from. Nothing here depends on any particular camera-dialing crate, container
//! format, or async runtime. [`FrameReceiver`] is a plain, `dyn`-safe
//! trait rather than a concrete channel type for the same reason: an
//! embedder's own async I/O layer (this project's own `X3` item, for
//! Corvette) decides how frames are actually delivered -- a `tokio::sync::
//! broadcast::Receiver` adapter, a channel from a different runtime, or a
//! synchronous queue in a test double -- and this crate places no
//! requirement on that choice beyond `Send`.

/// One stream's tracks and the codec parameters needed to describe them,
/// as returned by [`StreamProvider::describe`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    /// The SDP session name (the `s=` line `DESCRIBE`'s response body
    /// carries). Purely descriptive; no consumer this crate targets parses
    /// it.
    pub name: String,
    /// This stream's tracks, in the order `DESCRIBE`'s SDP body assigns
    /// `trackID=0`, `trackID=1`, ... -- the same order `SETUP` parses back
    /// out of a `SETUP` request's URI (see `session::parse_track_index`).
    pub tracks: Vec<TrackInfo>,
}

/// Codec-specific parameters for one track, sufficient to build its SDP
/// `a=rtpmap`/`a=fmtp` attributes.
///
/// Parameter-set/config bytes here are the same bytes a NAL/access-unit
/// stream already carries -- extracting them is a step any RTP packetizer
/// for the same codec needs regardless, so this crate asks the caller to
/// supply them already extracted rather than re-parsing a frame stream
/// itself to find them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackInfo {
    /// H.264 (RFC 6184).
    H264 {
        /// The Sequence Parameter Set: a full Annex-B NAL unit (including
        /// its 1-byte NAL header), without the start code. Must be at
        /// least 4 bytes -- the header byte plus the 3
        /// `profile_idc`/`constraint_flags`/`level_idc` bytes `profile-
        /// level-id` is built from.
        sps: Vec<u8>,
        /// The Picture Parameter Set, same framing as `sps`.
        pps: Vec<u8>,
    },
    /// H.265 (RFC 7798).
    H265 {
        /// The Video Parameter Set: a full Annex-B NAL unit (including its
        /// 2-byte NAL header), without the start code.
        vps: Vec<u8>,
        /// The Sequence Parameter Set, same framing as `vps`.
        sps: Vec<u8>,
        /// The Picture Parameter Set, same framing as `vps`.
        pps: Vec<u8>,
    },
    /// AAC, carried as `MPEG4-GENERIC` (RFC 3640).
    Aac {
        /// The raw MPEG-4 `AudioSpecificConfig` bytes (ISO/IEC 14496-3).
        audio_specific_config: Vec<u8>,
        /// The audio sample rate, in Hz -- also this track's RTP clock
        /// rate (RFC 3640 does not resample to a fixed rate).
        sample_rate: u32,
        /// The channel count.
        channels: u16,
    },
}

/// One access unit delivered to a subscriber of a stream's frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Index into the describing [`StreamInfo::tracks`] this frame belongs
    /// to.
    pub track_index: usize,
    /// Presentation timestamp, relative to the start of the stream.
    pub timestamp: std::time::Duration,
    /// One access unit: Annex-B start-code-framed NAL unit(s) for
    /// H.264/H.265, or one raw (non-ADTS) AAC access unit.
    pub payload: Vec<u8>,
}

/// A live per-subscriber stream of [`Frame`]s for one named stream,
/// returned by [`StreamProvider::subscribe`].
///
/// `recv` is synchronous by design: this crate does not commit to an async
/// runtime. An async caller runs it inside a blocking-safe context (e.g.
/// `tokio::task::spawn_blocking`) or, more commonly, implements this trait
/// over a channel whose own receive is already cheap/non-blocking in
/// practice (a `tokio::sync::broadcast::Receiver` wrapped in a small
/// adapter that polls it from within an async task, for instance).
pub trait FrameReceiver: Send {
    /// Returns the next available frame, or `None` once this stream will
    /// produce no further frames (the upstream source ended, or the
    /// implementation treats a missed/lagged delivery as a hard stream end
    /// rather than continuing silently after a gap).
    fn recv(&mut self) -> Option<Frame>;
}

/// The boundary between this crate's RTSP-serving half and whatever owns
/// the actual named streams and their frames.
///
/// Implementations must be safe to call from multiple sessions
/// concurrently: `describe` and `subscribe` may each be called once per
/// client connection, and several connections can describe or subscribe to
/// the same stream at the same time.
pub trait StreamProvider: Send + Sync {
    /// Looks up `name` and returns its tracks and codec parameters, or
    /// `None` if no stream by that name exists.
    fn describe(&self, name: &str) -> Option<StreamInfo>;

    /// Subscribes to `name`'s frames, or returns `None` if no stream by
    /// that name exists.
    fn subscribe(&self, name: &str) -> Option<Box<dyn FrameReceiver>>;
}
