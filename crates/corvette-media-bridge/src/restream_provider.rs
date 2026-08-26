//! Role (a) of the plan's Do step 3.
//!
//! Adapts `corvette_rtsp_client::depacketize::Frame` into `rtsp_restream`'s
//! own `Frame`/`FrameReceiver`/`StreamProvider` shapes, and dispatches by
//! stream name across every configured camera's own adapter (Do step 4) -- a
//! thin, mechanical adapter with no protocol logic of its own beyond the one
//! genuinely unavoidable piece of plumbing this pairing requires: scanning
//! each track's own parameter-set NAL units (SPS/PPS, or VPS/SPS/PPS for
//! H.265) out of the frame stream itself, since neither
//! `corvette_rtsp_client::Client`'s public API nor `rtsp_restream`'s own
//! `StreamProvider::describe` contract gives another source for them. The
//! scan itself is `crate::parameter_sets` (shared with this crate's own fMP4
//! muxer, issue #12 item G2); [`ParameterSetCache`] here only layers this
//! role's own "resolve once, never again" policy on top of it.
//!
//! `docs/design/api-contracts.md` records this crate's own honest limit: no
//! AAC track is ever resolved here, because `corvette-rtsp-client`'s own SDP
//! resolution never produces one today (see this crate's top-level doc).

use crate::parameter_sets::RawParameterSets;
use crate::rtp_clock::RtpClock;
use crate::supervise::log_event;
use bytes::Bytes;
use corvette_rtsp_client::depacketize::{Codec as ClientCodec, Frame as ClientFrame};
use rtsp_restream::{Frame as RestreamFrame, FrameReceiver, StreamInfo, StreamProvider, TrackInfo};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// The only per-camera capacity this item's own internal (restream-feed to
/// `FrameReceiver`) broadcast channel needs; matches
/// `corvette_rtsp_client::CameraConfig`'s own documented default.
const INTERNAL_CHANNEL_CAPACITY: usize = 64;

/// One camera's state as `rtsp_restream::StreamProvider` sees it: the
/// [`StreamInfo`] once its parameter sets are known (`None` until then -- a
/// stream a client can't yet `DESCRIBE`, matching `describe`'s own "no such
/// stream" contract for an incomplete one), and the internal broadcast
/// channel the restream-feed task publishes adapted frames onto.
struct CameraStream {
    info: Mutex<Option<StreamInfo>>,
    frames: broadcast::Sender<RestreamFrame>,
}

/// [`StreamProvider`] implementation dispatching by camera name.
///
/// Dispatches across every configured camera's own [`CameraStream`] (Do step
/// 4). The camera set is fixed at construction; this item does not support
/// adding or removing a camera at runtime (out of scope -- see the plan's own
/// Scope guard on camera discovery).
#[derive(Default)]
pub struct MultiCameraProvider {
    cameras: HashMap<String, Arc<CameraStream>>,
}

impl std::fmt::Debug for MultiCameraProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiCameraProvider")
            .field("cameras", &self.cameras.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl MultiCameraProvider {
    /// Registers `name`, returning a [`broadcast::Receiver`] of this
    /// camera's own `corvette_rtsp_client::Frame`s the caller feeds into
    /// [`run_restream_feed`] (typically as a supervised task, per INV-5/DP-3).
    ///
    /// # Panics
    ///
    /// Panics if `name` is already registered -- this item's own camera set
    /// is fixed at startup (see the config loader's own duplicate-name
    /// check), so a collision here means a caller bug, not a runtime
    /// condition to recover from.
    pub fn register(&mut self, name: &str) {
        let (frames, _first_receiver) = broadcast::channel(INTERNAL_CHANNEL_CAPACITY);
        let previous = self.cameras.insert(
            name.to_string(),
            Arc::new(CameraStream {
                info: Mutex::new(None),
                frames,
            }),
        );
        assert!(previous.is_none(), "camera {name:?} registered twice");
    }

    /// The shared per-camera state [`run_restream_feed`] needs to publish
    /// adapted frames and update the resolved [`StreamInfo`], or `None` if
    /// `name` was never [`register`](Self::register)ed.
    ///
    /// The camera set is fixed once [`start`](crate::start) finishes
    /// registering every configured camera and only then spawns any task
    /// that reads it, so this and every other `&self` method here run
    /// against a `HashMap` no longer being mutated -- no interior
    /// synchronization is needed for the map itself, only for each
    /// [`CameraStream`]'s own per-camera state.
    fn camera(&self, name: &str) -> Option<Arc<CameraStream>> {
        self.cameras.get(name).cloned()
    }
}

impl StreamProvider for MultiCameraProvider {
    fn describe(&self, name: &str) -> Option<StreamInfo> {
        self.cameras
            .get(name)?
            .info
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn subscribe(&self, name: &str) -> Option<Box<dyn FrameReceiver>> {
        let camera = self.cameras.get(name)?;
        Some(Box::new(BroadcastFrameReceiver {
            receiver: camera.frames.subscribe(),
            handle: tokio::runtime::Handle::current(),
        }))
    }
}

/// Wraps a [`broadcast::Receiver`] as a [`FrameReceiver`], per that trait's
/// own doc: a small adapter whose synchronous `recv` polls an async channel
/// from within the blocking-safe context `rtsp_restream`'s own play task
/// already runs it in (`crate::server::spawn_play_task`'s `spawn_blocking`
/// call).
struct BroadcastFrameReceiver {
    receiver: broadcast::Receiver<RestreamFrame>,
    handle: tokio::runtime::Handle,
}

impl FrameReceiver for BroadcastFrameReceiver {
    fn recv(&mut self) -> Option<RestreamFrame> {
        loop {
            match self.handle.block_on(self.receiver.recv()) {
                Ok(frame) => return Some(frame),
                // A slow subscriber missed some frames: rtsp-restream's own
                // per-connection play task has no resync concept, so treat a
                // lag the same way it treats a bad packetize -- skip forward
                // and keep going, not a hard stream end.
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                // Every sender (the restream-feed task) dropped: the upstream
                // camera's own client shut down along with the whole
                // process, or -- for the mutation this item's own evidence
                // exercises -- the feed task was torn down without a
                // replacement. Either way, this matches
                // `FrameReceiver::recv`'s own "no further frames" contract.
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

/// Runs one camera's RTSP-restream-feeding role (Do step 3a) for as long as
/// the caller wants it.
///
/// Reads `client_frames`, updates `provider`'s resolved [`StreamInfo`] for
/// `name` once parameter sets are observed, and republishes every frame onto
/// this camera's own internal broadcast channel for
/// [`BroadcastFrameReceiver`] to drain. Never returns on its own -- intended
/// to be spawned via `crate::supervise::spawn_supervised_with`, matching
/// DP-3/INV-5.
///
/// # Panics
///
/// Panics if `name` was never [`MultiCameraProvider::register`]ed -- a caller
/// bug (every configured camera is registered once, at startup, before any
/// feed task is spawned).
pub async fn run_restream_feed(
    name: String,
    mut client_frames: broadcast::Receiver<ClientFrame>,
    provider: Arc<MultiCameraProvider>,
) {
    let camera = provider
        .camera(&name)
        .unwrap_or_else(|| panic!("camera {name:?} was never registered"));
    let mut parameter_sets = ParameterSetCache::default();
    let mut clock = RtpClock::video();

    loop {
        match client_frames.recv().await {
            Ok(frame) => {
                if let Some(info) = parameter_sets.observe(&frame) {
                    *camera
                        .info
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(info);
                }
                let adapted = RestreamFrame {
                    track_index: 0, // Exactly one (video) track per camera; see this crate's top-level doc.
                    timestamp: clock.elapsed(frame.timestamp),
                    payload: frame.payload.to_vec(),
                };
                // No current PLAY session is not this task's failure to
                // handle, the same "no listeners" convention
                // corvette-rtsp-client's own per-camera task already uses.
                let _ = camera.frames.send(adapted);
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => {
                // The camera's own Client was dropped (process shutdown).
                // Nothing to restart into; end this attempt so the
                // supervisor's own restart loop just spins on a channel that
                // will never produce again -- acceptable since this only
                // happens during shutdown, which tears the whole process
                // down anyway.
                log_event(&name, "restream-feed-disconnect", "camera client dropped");
                return;
            }
        }
    }
}

/// Resolves a [`StreamInfo`] the moment a complete parameter set for one
/// codec becomes available, backed by the shared [`RawParameterSets`]
/// accumulator -- this role's own policy layered on top of it: resolve once,
/// and never update again even if the underlying bytes later change
/// (unchanged behavior from before this accumulator was extracted; see
/// [`RawParameterSets`]'s own doc for why `crate::fmp4`'s tracker needs a
/// different policy).
///
/// This is the one piece of real plumbing this adapter needs: `rtsp_restream`
/// requires the parameter sets up front to answer `DESCRIBE`
/// (`rtsp_restream::provider::TrackInfo`'s own doc), but
/// `corvette_rtsp_client::Client`'s public API exposes only the frame stream
/// itself, not the camera's SDP-resolved track description that also carries
/// them (an internal type of that crate, per its own `session`/`client::task`
/// modules -- not modified or reached into here, matching this item's own
/// Scope guard).
#[derive(Default)]
struct ParameterSetCache {
    raw: RawParameterSets,
    h264_resolved: bool,
    h265_resolved: bool,
}

impl ParameterSetCache {
    /// Observes one frame, updating the cached parameter sets for its codec
    /// if it carries one, and returns a freshly resolved [`StreamInfo`] the
    /// moment a complete set becomes available for that codec (`None`
    /// otherwise -- including every call after the first resolution, since
    /// [`MultiCameraProvider`]'s own cached value does not need to be
    /// rewritten on every later frame).
    fn observe(&mut self, frame: &ClientFrame) -> Option<StreamInfo> {
        self.raw.observe(frame);
        match frame.codec {
            ClientCodec::H264 => {
                if self.h264_resolved {
                    return None;
                }
                let (sps, pps) = self.raw.h264()?;
                self.h264_resolved = true;
                Some(stream_info_h264(sps, pps))
            }
            ClientCodec::H265 => {
                if self.h265_resolved {
                    return None;
                }
                let (vps, sps, pps) = self.raw.h265()?;
                self.h265_resolved = true;
                Some(stream_info_h265(vps, sps, pps))
            }
            // A real, currently-shipped gap this item inherits rather than
            // fixes (see this crate's top-level doc and G1's own Premise):
            // `corvette-rtsp-client`'s SDP resolution never resolves an audio
            // track, so no `Codec::Aac` frame is ever actually produced
            // today. Written generically here so a future audio-SDP fix
            // needs no change in this module -- but there is no parameter
            // set for AAC to extract this way (its config is an
            // AudioSpecificConfig, not an in-band NAL), so this arm can only
            // ever be a documented no-op.
            ClientCodec::Aac => None,
        }
    }
}

fn stream_info_h264(sps: &Bytes, pps: &Bytes) -> StreamInfo {
    StreamInfo {
        name: "corvette-media-bridge camera".to_string(),
        tracks: vec![TrackInfo::H264 {
            sps: sps.to_vec(),
            pps: pps.to_vec(),
        }],
    }
}

fn stream_info_h265(vps: &Bytes, sps: &Bytes, pps: &Bytes) -> StreamInfo {
    StreamInfo {
        name: "corvette-media-bridge camera".to_string(),
        tracks: vec![TrackInfo::H265 {
            vps: vps.to_vec(),
            sps: sps.to_vec(),
            pps: pps.to_vec(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parameter_sets::START_CODE_LEN;

    fn annex_b(nal: &[u8]) -> Bytes {
        let mut payload = Vec::with_capacity(nal.len() + START_CODE_LEN);
        payload.extend_from_slice(&[0, 0, 0, 1]);
        payload.extend_from_slice(nal);
        Bytes::from(payload)
    }

    fn frame(codec: ClientCodec, timestamp: u32, nal: &[u8]) -> ClientFrame {
        ClientFrame {
            codec,
            timestamp,
            payload: annex_b(nal),
        }
    }

    #[test]
    fn resolves_h264_only_once_both_sps_and_pps_are_seen() {
        let mut cache = ParameterSetCache::default();
        let sps_nal = [0x67, 0x42, 0xc0, 0x1f];
        let pps_nal = [0x68, 0xce, 0x3c, 0x80];
        let idr_nal = [0x65, 0x88];

        assert!(
            cache
                .observe(&frame(ClientCodec::H264, 0, &sps_nal))
                .is_none(),
            "SPS alone is not a complete set"
        );
        assert!(
            cache
                .observe(&frame(ClientCodec::H264, 0, &idr_nal))
                .is_none(),
            "a non-parameter-set NAL resolves nothing"
        );
        let info = cache
            .observe(&frame(ClientCodec::H264, 0, &pps_nal))
            .expect("SPS+PPS complete the set");
        let TrackInfo::H264 { sps, pps } = &info.tracks[0] else {
            panic!("expected an H264 track");
        };
        assert_eq!(sps, &sps_nal);
        assert_eq!(pps, &pps_nal);

        // Once resolved, later frames (even more parameter sets) don't
        // re-resolve: `MultiCameraProvider` already cached the first result.
        assert!(
            cache
                .observe(&frame(ClientCodec::H264, 1, &sps_nal))
                .is_none()
        );
    }

    #[test]
    fn resolves_h265_only_once_vps_sps_and_pps_are_all_seen() {
        let mut cache = ParameterSetCache::default();
        let vps_nal = [0x40, 0x01];
        let sps_nal = [0x42, 0x01];
        let pps_nal = [0x44, 0x01];

        assert!(
            cache
                .observe(&frame(ClientCodec::H265, 0, &vps_nal))
                .is_none()
        );
        assert!(
            cache
                .observe(&frame(ClientCodec::H265, 0, &sps_nal))
                .is_none()
        );
        let info = cache
            .observe(&frame(ClientCodec::H265, 0, &pps_nal))
            .expect("VPS+SPS+PPS complete the set");
        let TrackInfo::H265 { vps, sps, pps } = &info.tracks[0] else {
            panic!("expected an H265 track");
        };
        assert_eq!(vps, &vps_nal);
        assert_eq!(sps, &sps_nal);
        assert_eq!(pps, &pps_nal);
    }

    #[test]
    fn aac_frames_never_resolve_a_parameter_set_today() {
        let mut cache = ParameterSetCache::default();
        assert!(
            cache
                .observe(&frame(ClientCodec::Aac, 0, &[0xAB; 8]))
                .is_none()
        );
    }
}
