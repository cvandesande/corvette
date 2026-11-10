use std::path::PathBuf;
use std::time::Duration;

use corvette_detect::{
    Decode, Frame, FrameClock, FrameSource, FrameSpec, Input, Policy, SourceError, decode_command,
};

const SMALL: Policy = Policy {
    initial: Duration::from_millis(10),
    max: Duration::from_millis(40),
    healthy_after: Duration::from_mins(1),
};

const START: Duration = Duration::from_secs(1_000);

const GAMMA_LUMA_OF_FRAME_49: f64 = 134.32;

const LUMA_TOLERANCE: f64 = 0.05;

fn spec() -> FrameSpec {
    FrameSpec::new(704, 576, 5, Decode::Cpu).expect("valid frame spec")
}

fn corpus_segment() -> PathBuf {
    let Some(value) = std::env::var_os("CORVETTE_CORPUS_SEGMENT") else {
        panic!("CORVETTE_CORPUS_SEGMENT is not set");
    };
    let path = PathBuf::from(value);
    assert!(
        path.is_file(),
        "CORVETTE_CORPUS_SEGMENT names no file: {}",
        path.display()
    );
    path
}

fn source_for(program: &'static str, path: PathBuf) -> FrameSource {
    FrameSource::spawn(
        "decode".to_string(),
        SMALL,
        spec(),
        FrameClock::replay(START, spec()),
        move || decode_command(program, &Input::Segment { path: path.clone() }, spec()),
    )
}

async fn next(source: &mut FrameSource) -> Result<Frame, SourceError> {
    tokio::time::timeout(Duration::from_secs(30), source.next_frame())
        .await
        .expect("no result within 30s")
}

async fn first_fifty(source: &mut FrameSource) -> Vec<Frame> {
    let mut frames = Vec::new();
    for call in 1..=50 {
        let message = format!("a frame at call {call}");
        frames.push(next(source).await.expect(&message));
    }
    frames
}

#[tokio::test]
async fn a_missing_ffmpeg_is_a_spawn_failure() {
    let mut source = source_for(
        "/nonexistent/corvette-ffmpeg",
        PathBuf::from("/nonexistent/segment.ts"),
    );
    let error = next(&mut source).await.expect_err("spawn failure");
    match &error {
        SourceError::SpawnFailed { failures } => assert!(*failures >= 1),
        other => panic!("not a spawn failure: {other:?}"),
    }
    assert_eq!(0, source.status().starts);
    assert!(
        error
            .to_string()
            .starts_with("the frame command failed to start (")
    );
}

#[tokio::test]
#[ignore = "needs ffmpeg and CORVETTE_CORPUS_SEGMENT naming a recorded 704x576 segment; run with --ignored"]
async fn fifty_frames_of_a_segment_arrive_whole_and_200_ms_apart() {
    let mut source = source_for("ffmpeg", corpus_segment());
    let frames = first_fifty(&mut source).await;
    assert_eq!(50, frames.len());
    for (i, frame) in (0..50_u64).zip(&frames) {
        assert_eq!(608_256, frame.data.len());
        assert_eq!(i, frame.index);
        assert_eq!(START + Duration::from_millis(200 * i), frame.time);
    }
}

#[tokio::test]
#[ignore = "needs ffmpeg and CORVETTE_CORPUS_SEGMENT naming a recorded 704x576 segment; run with --ignored"]
async fn the_decoded_luma_carries_the_gamma_filter() {
    let mut source = source_for("ffmpeg", corpus_segment());
    let frames = first_fifty(&mut source).await;
    let sum: u32 = frames[49].data[..704 * 576]
        .iter()
        .map(|byte| u32::from(*byte))
        .sum();
    let mean = f64::from(sum) / 405_504.0;
    assert!(
        (mean - GAMMA_LUMA_OF_FRAME_49).abs() < LUMA_TOLERANCE,
        "mean luma of frame 49 was {mean}"
    );
}
