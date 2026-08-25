//! End-to-end tests of the per-camera client against `mock_camera`: fan-out
//! to multiple subscribers, reconnect after a mid-session TCP close, and the
//! broadcast channel's bounded-capacity behavior (INV-7).
//!
//! `src/client/task.rs`'s own unit tests cover panic isolation and recovery
//! directly (INV-2), without depending on a live session.

use corvette_rtsp_client::client::{CameraConfig, Client};
use corvette_rtsp_client::mock_camera::{MockCamera, MockCameraConfig};
use corvette_rtsp_client::session::Credentials;
use std::time::Duration;
use tokio::sync::broadcast;

const USERNAME: &str = "admin";
const PASSWORD: &str = "test-password-not-real";

fn camera_config(name: &str, camera: &MockCamera) -> CameraConfig {
    CameraConfig::new(
        name,
        camera.addr().ip(),
        camera.addr().port(),
        "/stream/",
        Credentials::new(USERNAME, PASSWORD),
    )
}

#[tokio::test]
async fn multiple_subscribers_on_one_camera_see_identical_frames() {
    let camera = MockCamera::spawn(MockCameraConfig::new())
        .await
        .expect("mock camera binds loopback");
    let client = Client::new(camera_config("camera-a", &camera));

    let mut receiver_one = client.subscribe();
    let mut receiver_two = client.subscribe();

    for _ in 0..10 {
        let frame_one = tokio::time::timeout(Duration::from_secs(2), receiver_one.recv())
            .await
            .expect("a frame arrives before the timeout")
            .expect("two freshly subscribed receivers see every frame with no lag");
        let frame_two = tokio::time::timeout(Duration::from_secs(2), receiver_two.recv())
            .await
            .expect("a frame arrives before the timeout")
            .expect("two freshly subscribed receivers see every frame with no lag");

        assert_eq!(frame_one.codec, frame_two.codec);
        assert_eq!(frame_one.timestamp, frame_two.timestamp);
        assert_eq!(
            frame_one.payload, frame_two.payload,
            "both subscribers must see byte-identical frames from one camera's channel"
        );
    }
}

#[tokio::test]
async fn two_independent_cameras_produce_frames_without_interfering() {
    let camera_a = MockCamera::spawn(MockCameraConfig::new())
        .await
        .expect("mock camera binds loopback");
    let camera_b = MockCamera::spawn(MockCameraConfig::new())
        .await
        .expect("mock camera binds loopback");

    let client_a = Client::new(camera_config("camera-a", &camera_a));
    let client_b = Client::new(camera_config("camera-b", &camera_b));

    let mut receiver_a = client_a.subscribe();
    let mut receiver_b = client_b.subscribe();

    for _ in 0..5 {
        tokio::time::timeout(Duration::from_secs(2), receiver_a.recv())
            .await
            .expect("camera a produces a frame before the timeout")
            .expect("camera a's own frames are not an error");
        tokio::time::timeout(Duration::from_secs(2), receiver_b.recv())
            .await
            .expect("camera b produces a frame before the timeout")
            .expect("camera b's own frames are not an error");
    }
}

#[tokio::test]
async fn reconnect_after_a_mid_session_tcp_close_resumes_on_the_same_receiver() {
    let camera = MockCamera::spawn(MockCameraConfig::new())
        .await
        .expect("mock camera binds loopback");
    let client = Client::new(camera_config("camera-a", &camera));
    let mut receiver = client.subscribe();

    for _ in 0..3 {
        tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("a frame arrives before the timeout")
            .expect("no lag before the disconnect");
    }

    camera.disconnect();

    // The same receiver, never resubscribed, must keep yielding frames once
    // the client's background task reconnects. A `Lagged` gap along the way
    // is exactly how a broadcast receiver reports the discontinuity and is
    // not a failure; a permanently stalled or closed receiver is.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut resumed = false;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(1), receiver.recv()).await {
            Ok(Ok(_frame)) => {
                resumed = true;
                break;
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) | Err(_) => {}
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                panic!("the broadcast channel must survive a reconnect, not close");
            }
        }
    }

    assert!(
        resumed,
        "expected the same receiver to see frames again after the client reconnected"
    );
}

#[tokio::test]
async fn slow_subscriber_sees_lagged_fast_subscriber_does_not() {
    // INV-7: a fast frame_interval piles up well past the default 64-frame
    // capacity in well under a second if a subscriber never drains.
    let camera =
        MockCamera::spawn(MockCameraConfig::new().frame_interval(Duration::from_millis(1)))
            .await
            .expect("mock camera binds loopback");
    let client = Client::new(camera_config("camera-a", &camera));

    let mut drained_receiver = client.subscribe();
    let mut slow_receiver = client.subscribe();
    let drain_window = Duration::from_millis(400);

    let fast_drain = tokio::spawn(async move {
        let mut received: u32 = 0;
        let deadline = tokio::time::Instant::now() + drain_window;
        while let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now()) {
            match tokio::time::timeout(remaining, drained_receiver.recv()).await {
                Ok(Ok(_frame)) => received += 1,
                Ok(Err(broadcast::error::RecvError::Lagged(skipped))) => {
                    panic!(
                        "a subscriber that keeps draining every frame must not lag, but skipped {skipped}"
                    );
                }
                Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => break,
            }
        }
        received
    });

    // `slow_receiver` is deliberately never drained here, while frames pile
    // up well past its channel's capacity.
    tokio::time::sleep(drain_window).await;

    let fast_received = fast_drain
        .await
        .expect("the fast-draining task does not panic");
    assert!(
        fast_received > 64,
        "expected the fast subscriber to receive well over one channel's worth of frames, got {fast_received}"
    );

    match slow_receiver.recv().await {
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            assert!(
                skipped > 0,
                "a lagged receiver must report at least one skipped frame"
            );
        }
        other => panic!("expected the slow subscriber to observe RecvError::Lagged, got {other:?}"),
    }
}
