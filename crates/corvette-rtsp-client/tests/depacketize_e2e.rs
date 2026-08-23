//! End-to-end: a full session's worth of synthetic frames from M1's mock
//! camera, demuxed by S1's session layer, depacketized by D1's H.264
//! depacketizer, must produce a non-empty, correctly Annex-B-framed output
//! stream.

use base64::Engine as _;
use corvette_rtsp_client::depacketize::{Codec, H264Depacketizer};
use corvette_rtsp_client::mock_camera::{MockCamera, MockCameraConfig};
use corvette_rtsp_client::session::{self, Credentials};
use std::time::Duration;

#[tokio::test]
async fn a_full_session_of_mock_frames_round_trips_to_annex_b_frames() {
    let camera = MockCamera::spawn(MockCameraConfig::new())
        .await
        .expect("mock camera binds loopback");

    let (_track, mut playing) = session::connect(
        camera.addr(),
        "/stream/",
        Credentials::new("admin", "test-password-not-real"),
    )
    .await
    .expect("full handshake succeeds");

    // M1's fabricated packets (mock_camera::rtp::fabricate_h264_packet)
    // carry a single-NAL payload only; a fabricated one-byte SPS is enough
    // to exercise the parameter-set-insertion path alongside them.
    let fabricated_sprop = base64::engine::general_purpose::STANDARD.encode([0x67]);
    let mut depacketizer = H264Depacketizer::new(&fabricated_sprop).expect("decodes");

    let mut frames = Vec::new();
    for _ in 0..5 {
        let packet = tokio::time::timeout(Duration::from_secs(1), playing.next_packet())
            .await
            .expect("a frame arrives before the timeout")
            .expect("next_packet succeeds");
        if packet.channel != 0 {
            continue; // RTCP or another channel, not this track's RTP
        }
        frames.extend(
            depacketizer
                .depacketize(&packet.payload)
                .expect("depacketizes a single-NAL H.264 packet"),
        );
    }

    assert!(!frames.is_empty(), "produced at least one Annex-B frame");
    for frame in &frames {
        assert_eq!(frame.codec, Codec::H264);
        assert!(
            frame.payload.starts_with(&[0, 0, 0, 1]),
            "every frame is Annex-B start-code prefixed"
        );
    }

    playing.teardown().await.expect("TEARDOWN succeeds");
}
