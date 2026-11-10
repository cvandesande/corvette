use std::path::PathBuf;
use std::time::Duration;

use corvette_detect::{
    Decode, Frame, FrameClock, FrameSource, FrameSpec, Input, Policy, SourceError, decode_command,
};
use corvette_motion::{Decision, MotionDetector, MotionSettings, Plane, Shape};

const SMALL: Policy = Policy {
    initial: Duration::from_millis(10),
    max: Duration::from_millis(40),
    healthy_after: Duration::from_mins(1),
};

const START: Duration = Duration::from_secs(1_000);

const WIDTH: usize = 704;

const HEIGHT: usize = 576;

const SEGMENT_FRAMES: usize = 300;

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

/// Feeds a recorded segment to a fresh detector, with no mask, and returns one decision per picture.
async fn decisions(path: PathBuf, frames: usize) -> Vec<Decision> {
    let shape = Shape::new(WIDTH, HEIGHT).expect("valid frame shape");
    let mut detector = MotionDetector::new(MotionSettings::default(), shape, None)
        .expect("detector for the frame shape");
    let mut source = source_for("ffmpeg", path);
    let mut replayed = Vec::new();
    for call in 1..=frames {
        let message = format!("a frame at call {call}");
        let frame = next(&mut source).await.expect(&message);
        let plane = Plane::new(&frame.data[..WIDTH * HEIGHT], shape).expect("grey plane");
        replayed.push(detector.feed(plane).expect("decision for the frame shape"));
    }
    replayed
}

#[tokio::test]
#[ignore = "needs ffmpeg and CORVETTE_CORPUS_SEGMENT naming a recorded 704x576 segment; run with --ignored"]
async fn two_replays_of_a_segment_give_identical_decisions() {
    let first = decisions(corpus_segment(), SEGMENT_FRAMES).await;
    let second = decisions(corpus_segment(), SEGMENT_FRAMES).await;
    assert_eq!(first.len(), SEGMENT_FRAMES);
    assert_eq!(second.len(), SEGMENT_FRAMES);
    for index in 0..SEGMENT_FRAMES {
        assert_eq!(first[index], second[index], "decision at index {index}");
    }
}

#[tokio::test]
#[ignore = "needs ffmpeg and CORVETTE_CORPUS_SEGMENT naming a recorded 704x576 segment; run with --ignored"]
async fn calibration_ends_during_a_segment_replay() {
    let replayed = decisions(corpus_segment(), SEGMENT_FRAMES).await;
    assert!(
        replayed.iter().any(|decision| !decision.calibrating),
        "calibration did not end within {} replayed frames",
        replayed.len()
    );
}
