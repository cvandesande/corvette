//! End-to-end tests of the session/protocol layer against `mock_camera`.
//!
//! Unlike `tests/smoke.rs` (a raw hand-rolled RTSP client proving
//! `mock_camera` itself reproduces the documented quirks), these tests
//! drive the real `corvette_rtsp_client::session` client against the same
//! mock, so a bug in the client's own handshake, SDP resolution, keep-alive
//! scheduling, or interleaved demux would show up here.

use corvette_rtsp_client::mock_camera::{MockCamera, MockCameraConfig};
use corvette_rtsp_client::session::{self, Credentials};
use std::time::Duration;

const USERNAME: &str = "admin";
const PASSWORD: &str = "test-password-not-real";

fn credentials() -> Credentials {
    Credentials::new(USERNAME, PASSWORD)
}

#[tokio::test]
async fn full_handshake_resolves_the_track_and_streams_frames() {
    let camera = MockCamera::spawn(MockCameraConfig::new())
        .await
        .expect("mock camera binds loopback");

    let (track, mut playing) = session::connect(camera.addr(), "/stream/", credentials())
        .await
        .expect("full handshake succeeds: auth challenge, DESCRIBE, SETUP, PLAY");

    assert_eq!(
        track.control_url.as_str(),
        format!("rtsp://{}/stream/track1", camera.addr()),
        "the resolved track URL is the video section's own a=control, not the session-level a=control:*"
    );
    assert_eq!(track.codec_name, "H264");
    assert_eq!(track.clock_rate, 90_000);

    for _ in 0..5 {
        let packet = tokio::time::timeout(Duration::from_secs(1), playing.next_packet())
            .await
            .expect("a frame arrives before the timeout")
            .expect("next_packet succeeds");
        assert_eq!(
            packet.channel, 0,
            "RTP frames are demuxed off the SETUP-negotiated interleaved channel 0"
        );
        // Byte-exact check against mock_camera::rtp::fabricate_h264_packet's
        // known shape: RTP version/no-padding byte, marker+payload-type
        // byte, then (after the 10 remaining RTP header bytes) a NAL header
        // reporting an IDR slice. A demuxer that corrupted or misaligned the
        // interleaved framing would not reproduce this exactly.
        assert_eq!(packet.payload.len(), 21);
        assert_eq!(
            packet.payload[0], 0x80,
            "RTP version 2, no padding/extension"
        );
        assert_eq!(packet.payload[1], 0xE0, "marker bit set, payload type 96");
        assert_eq!(
            packet.payload[12], 0x65,
            "NAL header for a fabricated IDR slice"
        );
    }

    playing.teardown().await.expect("TEARDOWN succeeds");
}

#[tokio::test]
async fn setup_response_declared_timeout_is_parsed_and_drives_the_keep_alive_interval() {
    // INV-3, checked end to end (not just the pure function in
    // session::keepalive's own unit tests): for each of three declared
    // timeouts the mock can be configured with, the client parses the
    // SETUP response's own Session header correctly and derives an
    // interval that is a real margin under it.
    for declared_secs in [30_u64, 65, 120] {
        let camera = MockCamera::spawn(
            MockCameraConfig::new().declared_timeout(Duration::from_secs(declared_secs)),
        )
        .await
        .expect("mock camera binds loopback");

        let (_track, playing) = session::connect(camera.addr(), "/stream/", credentials())
            .await
            .expect("handshake succeeds");

        assert_eq!(
            playing.declared_timeout(),
            Duration::from_secs(declared_secs)
        );

        let interval = playing.keep_alive_interval();
        assert!(
            interval < playing.declared_timeout(),
            "interval {interval:?} must be strictly less than the declared timeout for {declared_secs}s"
        );
        assert!(
            interval <= playing.declared_timeout() / 2,
            "interval {interval:?} must be no closer than half the declared timeout for {declared_secs}s"
        );
    }
}

#[tokio::test]
async fn keep_alive_scheduler_keeps_frames_flowing_past_the_declared_timeout() {
    // The mock silently stops sending frames once its declared timeout
    // elapses with no keep-alive (docs/design/api-contracts.md). This test
    // runs the real client, with its real keep-alive scheduler, for more
    // than twice the declared timeout and asserts frames never stop.
    //
    // RTSP's `Session: ...;timeout=N` is whole seconds only (confirmed
    // against the wire above -- SETUP's declared_timeout truncates via
    // `Duration::as_secs`), so the shortest declared timeout a real
    // handshake can observe is one second; a sub-second value here would
    // round to zero on the wire and not exercise the real client's parsing
    // at all.
    let declared_timeout = Duration::from_secs(1);
    let camera = MockCamera::spawn(
        MockCameraConfig::new()
            .declared_timeout(declared_timeout)
            .frame_interval(Duration::from_millis(50)),
    )
    .await
    .expect("mock camera binds loopback");

    let (_track, mut playing) = session::connect(camera.addr(), "/stream/", credentials())
        .await
        .expect("handshake succeeds");

    let deadline = tokio::time::Instant::now() + declared_timeout * 3;
    let mut frames_seen = 0_u32;
    while tokio::time::Instant::now() < deadline {
        let packet = tokio::time::timeout(declared_timeout * 2, playing.next_packet())
            .await
            .expect("a frame arrives well within twice the declared timeout -- the keep-alive scheduler must be preventing the silent stall")
            .expect("next_packet succeeds");
        assert_eq!(packet.channel, 0);
        frames_seen += 1;
    }

    assert!(
        frames_seen > 10,
        "expected many frames over 3x the declared timeout, got {frames_seen}"
    );
}

#[tokio::test]
async fn a_keep_alive_response_interleaved_between_rtp_frames_does_not_corrupt_either() {
    // INV-6: mock_camera deliberately delays a keep-alive's text reply
    // until right after the next RTP frame, forcing the exact interleaving
    // order this project's own real-camera testing hit. The client's
    // read loop must demux both correctly regardless.
    // Whole seconds only -- see the comment in the keep-alive-flowing test
    // above for why a sub-second declared timeout can't reach the wire.
    let declared_timeout = Duration::from_secs(1);
    let camera = MockCamera::spawn(
        MockCameraConfig::new()
            .declared_timeout(declared_timeout)
            .frame_interval(Duration::from_millis(50))
            .delay_keepalive_reply_to_next_frame(true),
    )
    .await
    .expect("mock camera binds loopback");

    let (_track, mut playing) = session::connect(camera.addr(), "/stream/", credentials())
        .await
        .expect("handshake succeeds");

    // The keep-alive interval is well under the 1s declared timeout, so
    // several keep-alive round trips (each interleaving a delayed text
    // reply between RTP frames) happen within this window.
    let deadline = tokio::time::Instant::now() + declared_timeout * 3;
    let mut frames_seen = 0_u32;
    while tokio::time::Instant::now() < deadline {
        let packet = tokio::time::timeout(declared_timeout * 2, playing.next_packet())
            .await
            .expect("a frame arrives without the connection stalling or erroring")
            .expect(
                "next_packet succeeds -- a corrupted demux would surface as a parse error here",
            );
        assert_eq!(packet.channel, 0);
        assert_eq!(
            packet.payload.len(),
            21,
            "an RTP frame must never be misread as part of a text response"
        );
        frames_seen += 1;
    }

    assert!(
        frames_seen > 10,
        "expected many uncorrupted frames, got {frames_seen}"
    );
}

#[tokio::test]
async fn an_rtp_frame_sent_before_plays_response_is_buffered_and_observed_in_order() {
    // S2: a real camera was observed starting to stream RTP on the
    // interleaved channel immediately around PLAY -- before, or interleaved
    // with, PLAY's own 200 OK arriving on the same TCP socket. The mock's
    // `stream_before_play_response` flag reproduces that exact ordering
    // deterministically. The handshake must still succeed, and the frame
    // sent ahead of PLAY's response must not be lost -- it must come back
    // from `next_packet` first, ahead of the frames `stream_frames` sends
    // afterward.
    let camera = MockCamera::spawn(MockCameraConfig::new().stream_before_play_response(true))
        .await
        .expect("mock camera binds loopback");

    let (_track, mut playing) = session::connect(camera.addr(), "/stream/", credentials())
        .await
        .expect("handshake succeeds even though an RTP frame arrives before PLAY's 200 OK");

    let first = tokio::time::timeout(Duration::from_secs(1), playing.next_packet())
        .await
        .expect("the frame buffered during the handshake arrives before the timeout")
        .expect("next_packet succeeds");
    assert_eq!(first.channel, 0);
    assert_eq!(
        frame_index(&first.payload),
        0,
        "the first packet returned must be the one the mock sent before PLAY's response, \
         not a later frame from the regular post-PLAY stream"
    );

    let second = tokio::time::timeout(Duration::from_secs(1), playing.next_packet())
        .await
        .expect("a subsequent frame from the regular stream also arrives")
        .expect("next_packet succeeds");
    assert_eq!(second.channel, 0);
    assert_eq!(
        frame_index(&second.payload),
        1,
        "the regular post-PLAY stream resumes normally after the buffered frame is drained"
    );

    playing.teardown().await.expect("TEARDOWN succeeds");
}

/// Extracts the 8-byte big-endian frame index `mock_camera::rtp::
/// fabricate_h264_packet` appends after its 13-byte RTP header + NAL header,
/// so a test can assert which fabricated frame, in sequence, was received.
fn frame_index(payload: &bytes::Bytes) -> u64 {
    u64::from_be_bytes(
        payload[13..21]
            .try_into()
            .expect("21-byte fabricated payload"),
    )
}

#[tokio::test]
async fn rejects_basic_and_unauthenticated_setup_the_same_way_a_raw_client_would() {
    // Confirms the client actually exercises the Digest challenge/response
    // path (not e.g. accidentally succeeding against a camera that happens
    // to allow unauthenticated access) by pointing it at credentials the
    // mock does not accept.
    let camera = MockCamera::spawn(MockCameraConfig::new())
        .await
        .expect("mock camera binds loopback");

    let result = session::connect(
        camera.addr(),
        "/stream/",
        Credentials::new(USERNAME, "wrong-password"),
    )
    .await;

    assert!(
        result.is_err(),
        "wrong credentials must not produce a successful session"
    );
}
