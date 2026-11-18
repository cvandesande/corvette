use std::collections::BTreeMap;

use corvette_detect::{CandidateError, CandidateStage, EngineError, Infer, Labels, RegionTensor};
use corvette_detect_proto::{Row, TENSOR_BYTES};
use corvette_motion::Region;
use corvette_track::{FrameDetection, ObjectFilter, ObjectFilters};

#[derive(Debug, Default)]
struct FakeEngine {
    replies: Vec<Result<Vec<Row>, EngineError>>,
    received: Vec<Vec<u8>>,
}

impl Infer for FakeEngine {
    fn infer(&mut self, tensor: &[u8; TENSOR_BYTES]) -> Result<Vec<Row>, EngineError> {
        self.received.push(tensor.to_vec());
        match self.replies.get(self.received.len() - 1) {
            Some(Ok(rows)) => Ok(rows.clone()),
            Some(Err(error)) => Err(error.clone()),
            None => Ok(Vec::new()),
        }
    }
}

fn labels() -> Labels {
    Labels::parse("0 person\n1 dog").expect("hand labels")
}

fn tracker_filters(tracked: &[&str]) -> ObjectFilters {
    let mut filters = BTreeMap::new();
    for label in tracked {
        let filter = ObjectFilter {
            min_area: 0,
            max_area: 24_000_000,
            min_score: 0.5,
            min_ratio: 0.0,
            max_ratio: 24_000_000.0,
            mask: None,
        };
        filters.insert((*label).to_owned(), filter);
    }
    let track = tracked.iter().map(|label| (*label).to_owned()).collect();
    ObjectFilters::new(track, filters)
}

fn stage(tracked: &[&str]) -> CandidateStage {
    CandidateStage::new(labels(), tracker_filters(tracked), 704, 576).expect("hand stage")
}

const fn row(class_id: u32, score: f32, y_min: f32, x_min: f32, y_max: f32, x_max: f32) -> Row {
    Row {
        class_id,
        score,
        y_min,
        x_min,
        y_max,
        x_max,
    }
}

fn tensor_for(x1: i32, y1: i32, x2: i32, y2: i32) -> RegionTensor {
    RegionTensor {
        region: Region { x1, y1, x2, y2 },
        tensor: vec![0u8; TENSOR_BYTES],
    }
}

#[test]
fn one_row_becomes_the_hand_computed_frame_detection() {
    let mut engine = FakeEngine::default();
    engine
        .replies
        .push(Ok(vec![row(0, 0.9, 0.25, 0.5, 0.75, 0.875)]));
    let got = stage(&["person"])
        .detect(&mut engine, &[tensor_for(0, 0, 320, 320)])
        .expect("a frame of detections");
    assert_eq!(
        got,
        vec![FrameDetection {
            label: "person".to_owned(),
            score: f64::from(0.9_f32),
            edges: [160, 80, 280, 240],
            area: 19200,
            ratio: 0.75,
            region: vec![[0, 0, 320, 320]],
        }]
    );
}

#[test]
fn the_raw_box_follows_the_top_left_bottom_right_detector_order() {
    // Non-square region (320 wide, 160 tall) and box: reading the raw box
    // as [left, top, right, bottom] would give edges [80, 40, 160, 200].
    let mut engine = FakeEngine::default();
    engine
        .replies
        .push(Ok(vec![row(0, 0.9, 0.25, 0.125, 0.5, 0.625)]));
    let got = stage(&["person"])
        .detect(&mut engine, &[tensor_for(0, 0, 320, 160)])
        .expect("a frame of detections");
    assert_eq!(got[0].edges, [40, 80, 200, 160]);
    assert_eq!(got[0].area, 12800);
    assert_eq!(got[0].ratio.to_bits(), 2.0_f64.to_bits());
}

#[test]
fn a_class_id_outside_the_label_map_is_skipped() {
    // The filters even track the empty name, so a mutated stage that maps an
    // unknown class id to "" would leave a second detection here.
    let mut engine = FakeEngine::default();
    engine.replies.push(Ok(vec![
        row(7, 0.9, 0.25, 0.5, 0.75, 0.875),
        row(0, 0.9, 0.25, 0.5, 0.75, 0.875),
    ]));
    let got = stage(&["person", ""])
        .detect(&mut engine, &[tensor_for(0, 0, 320, 320)])
        .expect("a frame of detections");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].label, "person");
}

#[test]
fn untracked_labels_and_low_scores_are_filtered() {
    let mut engine = FakeEngine::default();
    engine.replies.push(Ok(vec![
        row(1, 0.9, 0.25, 0.5, 0.75, 0.875),
        row(0, 0.4, 0.25, 0.5, 0.75, 0.875),
    ]));
    let got = stage(&["person"])
        .detect(&mut engine, &[tensor_for(0, 0, 320, 320)])
        .expect("a frame of detections");
    assert!(got.is_empty());
    assert_eq!(engine.received.len(), 1);
}

#[test]
fn two_identical_rows_merge_into_the_higher_scored_one() {
    // The two rows map to the same box, so NMS (limit 0.4, score floor 0.5)
    // keeps the 0.9 row and drops the 0.7 row.
    let mut engine = FakeEngine::default();
    engine.replies.push(Ok(vec![
        row(0, 0.9, 0.25, 0.5, 0.75, 0.875),
        row(0, 0.7, 0.25, 0.5, 0.75, 0.875),
    ]));
    let got = stage(&["person"])
        .detect(&mut engine, &[tensor_for(0, 0, 320, 320)])
        .expect("a frame of detections");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].score.to_bits(), f64::from(0.9_f32).to_bits());
    assert_eq!(got[0].edges, [160, 80, 280, 240]);
    assert_eq!(got[0].area, 19200);
    assert_eq!(got[0].ratio.to_bits(), 0.75_f64.to_bits());
}

#[test]
fn duplicate_boxes_from_two_regions_merge_into_one_detection() {
    // A and B overlap: both raw boxes map to frame edges [160, 80, 280, 240].
    let mut engine = FakeEngine::default();
    engine
        .replies
        .push(Ok(vec![row(0, 0.9, 0.25, 0.5, 0.75, 0.875)]));
    engine
        .replies
        .push(Ok(vec![row(0, 0.9, 0.25, 0.0, 0.75, 0.375)]));
    let tensors = [tensor_for(0, 0, 320, 320), tensor_for(160, 0, 480, 320)];
    let got = stage(&["person"])
        .detect(&mut engine, &tensors)
        .expect("a frame of detections");
    assert_eq!(engine.received.len(), 2);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].label, "person");
    assert_eq!(got[0].score.to_bits(), f64::from(0.9_f32).to_bits());
    assert_eq!(got[0].edges, [160, 80, 280, 240]);
    assert_eq!(got[0].area, 19200);
    assert_eq!(got[0].ratio.to_bits(), 0.75_f64.to_bits());
}

#[test]
fn rows_from_two_regions_come_back_in_input_order() {
    let mut engine = FakeEngine::default();
    engine
        .replies
        .push(Ok(vec![row(0, 0.9, 0.25, 0.5, 0.75, 0.875)]));
    engine
        .replies
        .push(Ok(vec![row(1, 0.9, 0.125, 0.25, 0.375, 0.625)]));
    let tensors = [tensor_for(0, 0, 320, 320), tensor_for(320, 0, 640, 320)];
    let got = stage(&["person", "dog"])
        .detect(&mut engine, &tensors)
        .expect("a frame of detections");
    assert_eq!(
        got,
        vec![
            FrameDetection {
                label: "person".to_owned(),
                score: f64::from(0.9_f32),
                edges: [160, 80, 280, 240],
                area: 19200,
                ratio: 0.75,
                region: vec![[0, 0, 320, 320]],
            },
            FrameDetection {
                label: "dog".to_owned(),
                score: f64::from(0.9_f32),
                edges: [400, 40, 520, 120],
                area: 9600,
                ratio: 1.5,
                region: vec![[320, 0, 640, 320]],
            },
        ]
    );
}

#[test]
fn a_short_tensor_is_rejected_before_the_engine() {
    let mut engine = FakeEngine::default();
    let short = RegionTensor {
        region: Region {
            x1: 0,
            y1: 0,
            x2: 320,
            y2: 320,
        },
        tensor: vec![0u8; TENSOR_BYTES - 3],
    };
    let got = stage(&["person"]).detect(&mut engine, &[short]);
    assert_eq!(
        got,
        Err(CandidateError::WrongTensorLength {
            expected: TENSOR_BYTES,
            got: TENSOR_BYTES - 3,
        })
    );
    assert!(engine.received.is_empty());
}

#[test]
fn an_engine_error_stops_the_frame_at_that_region() {
    let mut engine = FakeEngine::default();
    engine.replies.push(Err(EngineError::Io(
        "the scripted engine failed".to_owned(),
    )));
    engine
        .replies
        .push(Ok(vec![row(0, 0.9, 0.25, 0.5, 0.75, 0.875)]));
    let tensors = [tensor_for(0, 0, 320, 320), tensor_for(320, 0, 640, 320)];
    let got = stage(&["person", "dog"]).detect(&mut engine, &tensors);
    assert_eq!(
        got,
        Err(CandidateError::Engine(EngineError::Io(
            "the scripted engine failed".to_owned()
        )))
    );
    assert_eq!(engine.received.len(), 1);
}

#[test]
fn the_engine_receives_exactly_each_regions_tensor_bytes() {
    let mut engine = FakeEngine::default();
    let mut first = tensor_for(0, 0, 320, 320);
    first.tensor = vec![0xAA_u8; TENSOR_BYTES];
    let mut second = tensor_for(320, 0, 640, 320);
    second.tensor = vec![0xBB_u8; TENSOR_BYTES];
    let got = stage(&["person", "dog"])
        .detect(&mut engine, &[first, second])
        .expect("a frame of detections");
    assert!(got.is_empty());
    assert_eq!(
        engine.received,
        vec![vec![0xAA_u8; TENSOR_BYTES], vec![0xBB_u8; TENSOR_BYTES],]
    );
}

#[test]
fn no_tensors_infer_nothing_and_return_nothing() {
    let mut engine = FakeEngine::default();
    let got = stage(&["person"])
        .detect(&mut engine, &[])
        .expect("a frame of detections");
    assert!(got.is_empty());
    assert!(engine.received.is_empty());
}

#[test]
fn a_box_beyond_the_frame_edge_is_clamped_to_the_last_column() {
    // Region x_max 720 hangs past the 704-wide frame: map_box clamps the
    // right edge to width-1 = 703 (mapping.rs hand case (560, 32, 703, 160)).
    let mut engine = FakeEngine::default();
    engine
        .replies
        .push(Ok(vec![row(0, 0.9, 0.1, 0.5, 0.5, 1.0)]));
    let got = stage(&["person"])
        .detect(&mut engine, &[tensor_for(400, 0, 720, 320)])
        .expect("a frame of detections");
    assert_eq!(
        got,
        vec![FrameDetection {
            label: "person".to_owned(),
            score: f64::from(0.9_f32),
            edges: [560, 32, 703, 160],
            area: 18304,
            ratio: 1.117_187_5,
            region: vec![[400, 0, 720, 320]],
        }]
    );
}
