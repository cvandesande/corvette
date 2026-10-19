//! Adapts camera frames to the restream shapes and dispatches by stream name.

use crate::parameter_sets::RawParameterSets;
use crate::rtp_clock::RtpClock;
use crate::supervise::log_event;
use bytes::Bytes;
use corvette_rtsp_client::depacketize::{Codec as ClientCodec, Frame as ClientFrame};
use rtsp_restream::{Frame as RestreamFrame, FrameReceiver, StreamInfo, StreamProvider, TrackInfo};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

const INTERNAL_CHANNEL_CAPACITY: usize = 64;

struct CameraStream {
    info: Mutex<Option<StreamInfo>>,
    frames: broadcast::Sender<RestreamFrame>,
}

/// A [`StreamProvider`] dispatching frames by camera name.
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
    /// Registers `name` with a fresh internal frame channel.
    ///
    /// # Panics
    ///
    /// Panics if `name` is already registered: cameras are fixed at startup.
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

struct BroadcastFrameReceiver {
    receiver: broadcast::Receiver<RestreamFrame>,
    handle: tokio::runtime::Handle,
}

impl FrameReceiver for BroadcastFrameReceiver {
    fn recv(&mut self) -> Option<RestreamFrame> {
        loop {
            match self.handle.block_on(self.receiver.recv()) {
                Ok(frame) => return Some(frame),
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

/// Feeds one camera's frames to the restream server until the source closes.
///
/// # Panics
///
/// Panics if `name` was never [`MultiCameraProvider::register`]ed.
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
                let _ = camera.frames.send(adapted);
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => {
                log_event(&name, "restream-feed-disconnect", "camera client dropped");
                return;
            }
        }
    }
}

#[derive(Default)]
struct ParameterSetCache {
    raw: RawParameterSets,
    h264_resolved: bool,
    h265_resolved: bool,
}

impl ParameterSetCache {
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
