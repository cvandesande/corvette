use std::process::Stdio;
use std::time::Duration;

use corvette_detect::{
    Decode, Frame, FrameClock, FrameSource, FrameSpec, Policy, ReadError, SourceError,
};
use tokio::process::Command;

const SMALL: Policy = Policy {
    initial: Duration::from_millis(10),
    max: Duration::from_millis(40),
    healthy_after: Duration::from_mins(1),
};

fn spec() -> FrameSpec {
    FrameSpec::new(4, 2, 5, Decode::Cpu).expect("valid frame spec")
}

fn shell(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(script);
    command.stdout(Stdio::piped());
    command
}

fn replay_source(script: &'static str) -> FrameSource {
    FrameSource::spawn(
        "test".to_string(),
        SMALL,
        spec(),
        FrameClock::replay(Duration::from_secs(1_000), spec()),
        move || shell(script),
    )
}

async fn next(source: &mut FrameSource) -> Result<Frame, SourceError> {
    tokio::time::timeout(Duration::from_secs(10), source.next_frame())
        .await
        .expect("no result within 10s")
}

#[tokio::test]
async fn frames_from_a_child_are_returned_whole_and_in_order() {
    let mut source = replay_source("printf abcdefghijklmnopqrstuvwx");
    let frame = next(&mut source).await.expect("first frame");
    assert_eq!(b"abcdefghijkl".as_slice(), frame.data.as_slice());
    assert_eq!(0, frame.index);
    assert_eq!(Duration::from_secs(1_000), frame.time);
    let frame = next(&mut source).await.expect("second frame");
    assert_eq!(b"mnopqrstuvwx".as_slice(), frame.data.as_slice());
    assert_eq!(1, frame.index);
    assert_eq!(Duration::from_millis(1_000_200), frame.time);
    let error = next(&mut source).await.expect_err("stream end");
    assert!(matches!(error, SourceError::Ended));
}

#[tokio::test]
async fn a_child_that_exits_is_started_again_and_its_next_stream_is_read() {
    let mut source = replay_source("printf abcdefghijklmnopqrstuvwx");
    let _ = next(&mut source).await.expect("first frame");
    let _ = next(&mut source).await.expect("second frame");
    let error = next(&mut source).await.expect_err("stream end");
    assert!(matches!(error, SourceError::Ended));
    let frame = next(&mut source).await.expect("restarted child frame");
    assert_eq!(b"abcdefghijkl".as_slice(), frame.data.as_slice());
    assert_eq!(0, frame.index);
    assert!(source.status().starts >= 2);
}

#[tokio::test]
async fn a_stream_that_ends_inside_a_frame_is_a_read_error() {
    let mut source = replay_source("printf abcdefghijklmnop");
    let frame = next(&mut source).await.expect("first frame");
    assert_eq!(b"abcdefghijkl".as_slice(), frame.data.as_slice());
    let error = next(&mut source).await.expect_err("truncated stream");
    match &error {
        SourceError::Read(ReadError::Truncated { got, want }) => {
            assert_eq!(&4, got);
            assert_eq!(&12, want);
        }
        other => panic!("not a truncated read error: {other:?}"),
    }
    assert_eq!(
        "stream ended inside a frame: got 4 of 12 bytes",
        error.to_string()
    );
}

#[tokio::test]
async fn a_command_that_cannot_start_is_a_spawn_failure() {
    let policy = Policy {
        initial: Duration::from_millis(200),
        max: Duration::from_millis(400),
        healthy_after: Duration::from_mins(1),
    };
    let mut source = FrameSource::spawn(
        "missing".to_string(),
        policy,
        spec(),
        FrameClock::replay(Duration::from_secs(1_000), spec()),
        || {
            let mut command = Command::new("/nonexistent/corvette-frame-command");
            command.stdout(Stdio::piped());
            command
        },
    );
    let error = next(&mut source).await.expect_err("spawn failure");
    let first = match error {
        SourceError::SpawnFailed { failures } => {
            assert!(failures >= 1);
            failures
        }
        other => panic!("not a spawn failure: {other:?}"),
    };
    assert_eq!(0, source.status().starts);
    let error = next(&mut source).await.expect_err("second spawn failure");
    match error {
        SourceError::SpawnFailed { failures } => assert!(failures > first),
        other => panic!("not a spawn failure: {other:?}"),
    }
    assert!(
        error
            .to_string()
            .starts_with("the frame command failed to start (")
    );
}

#[tokio::test]
async fn a_stream_nobody_reads_in_time_is_dropped_and_counted() {
    let mut source = replay_source("printf abcdefghijkl");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while source.status().starts < 3 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no three starts within 10s"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(source.dropped_streams() >= 1);
    let frame = next(&mut source).await.expect("oldest stream frame");
    assert_eq!(b"abcdefghijkl".as_slice(), frame.data.as_slice());
    assert_eq!(0, frame.index);
    let error = next(&mut source).await.expect_err("oldest stream end");
    assert!(matches!(error, SourceError::Ended));
}

#[tokio::test]
async fn a_command_without_a_piped_stream_is_counted_as_dropped() {
    let mut source = FrameSource::spawn(
        "unpiped".to_string(),
        SMALL,
        spec(),
        FrameClock::replay(Duration::from_secs(1_000), spec()),
        || {
            let mut command = Command::new("sh");
            command.args(["-c", "exit 0"]);
            command
        },
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while source.dropped_streams() < 1 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no dropped stream within 10s"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let outcome = tokio::time::timeout(Duration::from_millis(100), source.next_frame()).await;
    assert!(outcome.is_err());
}

#[tokio::test]
async fn a_cancelled_call_keeps_the_frame_in_progress() {
    let mut source = replay_source("printf abcde; sleep 1; printf fghijkl");
    let outcome = tokio::time::timeout(Duration::from_millis(300), source.next_frame()).await;
    assert!(outcome.is_err());
    let frame = next(&mut source).await.expect("completed frame");
    assert_eq!(b"abcdefghijkl".as_slice(), frame.data.as_slice());
    assert_eq!(0, frame.index);
}

#[tokio::test]
async fn a_stopped_supervisor_is_reported_as_stopped() {
    let mut source = FrameSource::spawn(
        "panicking".to_string(),
        SMALL,
        spec(),
        FrameClock::replay(Duration::from_secs(1_000), spec()),
        || panic!("the command closure panicked"),
    );
    let error = next(&mut source).await.expect_err("stopped supervisor");
    assert!(matches!(error, SourceError::Stopped));
}

#[test]
fn the_error_messages_name_the_cause() {
    assert_eq!("the frame stream ended", SourceError::Ended.to_string());
    assert_eq!(
        "the frame command failed to start (3 failures so far)",
        SourceError::SpawnFailed { failures: 3 }.to_string()
    );
    assert_eq!("the frame source stopped", SourceError::Stopped.to_string());
    assert_eq!(
        "stream ended inside a frame: got 4 of 12 bytes",
        SourceError::Read(ReadError::Truncated { got: 4, want: 12 }).to_string()
    );
    let error: &dyn std::error::Error = &SourceError::Stopped;
    assert_eq!("the frame source stopped", error.to_string());
}
