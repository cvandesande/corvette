use corvette_detect::{FrameObjects, ObjectStage, TrackedObject};
use corvette_track::{FrameDetection, IdSource, TrackerSettings, Zone, ZoneSet};

const fn settings(
    min_initialized: u32,
    max_disappeared: u32,
    stationary_threshold: u64,
) -> TrackerSettings {
    TrackerSettings {
        width: 640,
        height: 480,
        min_initialized,
        max_disappeared,
        stationary_threshold,
        max_frames_default: None,
        max_frames_overrides: Vec::new(),
        own_labels: Vec::new(),
    }
}

fn person(score: f64, edges: [i32; 4]) -> FrameDetection {
    FrameDetection {
        label: "person".to_owned(),
        score,
        edges,
        area: 20_000,
        ratio: 0.5,
        region: Vec::new(),
    }
}

const fn same(actual: f64, expected: f64) -> bool {
    actual.to_bits() == expected.to_bits()
}

fn one_box(object: &FrameObjects) -> &TrackedObject {
    assert_eq!(object.objects.len(), 1, "one tracked object expected");
    &object.objects[0]
}

const STAY: [i32; 4] = [100, 140, 200, 340];

#[test]
fn an_empty_first_frame_reports_no_objects_and_no_endings() {
    let mut ids = IdSource::default();
    let mut stage =
        ObjectStage::new(settings(0, 5, 100), &mut ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    let frame = stage.step(1.0, Vec::new()).unwrap();
    assert!(same(frame.frame_time, 1.0));
    assert!(frame.objects.is_empty());
    assert!(frame.ended.is_empty());
}

#[test]
fn a_tracked_object_reports_exact_fields_and_confirms_when_its_score_reaches_the_threshold() {
    let mut ids = IdSource::default();
    let mut stage =
        ObjectStage::new(settings(0, 5, 100), &mut ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    let first = stage.step(1.0, vec![person(0.4, STAY)]).unwrap();
    let object = one_box(&first);
    assert_eq!(
        object.id, "1",
        "the first id of a fresh source is one, as decimal"
    );
    assert_eq!(object.label, "person");
    assert!(same(object.score, 0.4));
    assert!(!object.confirmed, "0.4 is below the 0.5 threshold");
    assert_eq!(
        object.bounds.map(f64::to_bits),
        [
            (100.0_f64 / 640.0).to_bits(),
            (140.0_f64 / 480.0).to_bits(),
            (200.0_f64 / 640.0).to_bits(),
            (340.0_f64 / 480.0).to_bits(),
        ]
    );
    assert_eq!(
        object.edges,
        [100, 140, 200, 340],
        "the detection box, passed through by the tracker"
    );
    assert!(same(object.first_seen, 1.0));
    assert!(same(object.last_seen, 1.0));
    assert!(object.zones_now.is_empty());
    assert!(object.zones_entered.is_empty());
    assert!(!object.stationary);
    let second = stage.step(2.0, vec![person(0.9, STAY)]).unwrap();
    let object = one_box(&second);
    assert!(same(object.score, 0.65), "median of [0.4, 0.9]");
    assert!(object.confirmed, "the 0.65 median reaches the threshold");
    assert!(same(object.first_seen, 1.0));
    assert!(same(object.last_seen, 2.0));
    let third = stage.step(3.0, vec![person(0.9, STAY)]).unwrap();
    let object = one_box(&third);
    assert!(same(object.score, 0.9), "median of [0.4, 0.9, 0.9]");
    assert!(object.confirmed);
    assert!(same(object.first_seen, 1.0));
    assert!(same(object.last_seen, 3.0));
    let fourth = stage.step(4.0, vec![person(0.2, STAY)]).unwrap();
    let object = one_box(&fourth);
    assert!(
        same(object.score, 0.65),
        "median of the sorted [0.2, 0.4, 0.9, 0.9]"
    );
    assert!(object.confirmed, "confirmation stays after the low score");
    assert!(same(object.last_seen, 4.0));
}

#[test]
fn a_score_exactly_at_the_threshold_confirms_at_once() {
    let mut ids = IdSource::default();
    let mut stage =
        ObjectStage::new(settings(0, 5, 100), &mut ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    let frame = stage.step(1.0, vec![person(0.5, STAY)]).unwrap();
    assert!(one_box(&frame).confirmed, "exactly 0.5 is at the threshold");
}

#[test]
fn a_low_score_after_confirmation_keeps_the_object_confirmed() {
    let mut ids = IdSource::default();
    let mut stage =
        ObjectStage::new(settings(0, 5, 100), &mut ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    stage.step(1.0, vec![person(0.9, STAY)]).unwrap();
    let frame = stage.step(2.0, vec![person(0.1, STAY)]).unwrap();
    let object = one_box(&frame);
    assert!(same(object.score, 0.5), "median of [0.1, 0.9]");
    assert!(
        object.confirmed,
        "confirmation is sticky for the object's life"
    );
}

#[test]
fn stationary_turns_true_on_the_frame_the_motionless_count_reaches_the_threshold() {
    let mut ids = IdSource::default();
    let mut stage =
        ObjectStage::new(settings(0, 5, 2), &mut ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    let first = stage.step(1.0, vec![person(0.9, STAY)]).unwrap();
    assert!(!one_box(&first).stationary, "one motionless frame");
    let second = stage.step(2.0, vec![person(0.9, STAY)]).unwrap();
    assert!(
        !one_box(&second).stationary,
        "one below the threshold of two"
    );
    let third = stage.step(3.0, vec![person(0.9, STAY)]).unwrap();
    assert!(one_box(&third).stationary, "exactly the threshold of two");
    let fourth = stage.step(4.0, vec![person(0.9, STAY)]).unwrap();
    assert!(one_box(&fourth).stationary, "above the threshold");
}

#[test]
fn the_median_decays_with_zeroes_on_frames_without_a_detection() {
    let mut ids = IdSource::default();
    let mut stage =
        ObjectStage::new(settings(0, 5, 100), &mut ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    stage.step(1.0, vec![person(0.4, STAY)]).unwrap();
    stage.step(2.0, vec![person(0.9, STAY)]).unwrap();
    let third = stage.step(3.0, Vec::new()).unwrap();
    let object = one_box(&third);
    assert!(
        same(object.score, 0.4),
        "median of the sorted [0.0, 0.4, 0.9]"
    );
    assert!(object.confirmed, "the confirmed flag stays through decay");
    let fourth = stage.step(4.0, Vec::new()).unwrap();
    let object = one_box(&fourth);
    assert!(
        same(object.score, 0.2),
        "median of the sorted [0.0, 0.0, 0.4, 0.9] is (0.0 + 0.4) / 2"
    );
    assert!(object.confirmed, "the confirmed flag stays through decay");
}

#[test]
fn objects_are_listed_in_registration_order_with_their_ids() {
    let cat = FrameDetection {
        label: "cat".to_owned(),
        score: 0.8,
        edges: [400, 100, 520, 260],
        area: 19_200,
        ratio: 0.75,
        region: Vec::new(),
    };
    let mut ids = IdSource::default();
    let mut stage =
        ObjectStage::new(settings(0, 5, 100), &mut ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    let frame = stage.step(1.0, vec![person(0.9, STAY), cat]).unwrap();
    assert_eq!(frame.objects.len(), 2, "both register");
    let first = &frame.objects[0];
    let second = &frame.objects[1];
    assert_eq!((first.id.as_str(), first.label.as_str()), ("1", "person"));
    assert_eq!(
        (second.id.as_str(), second.label.as_str()),
        ("2", "cat"),
        "registration order"
    );
}

#[test]
fn ended_lists_ids_that_retired_together_in_the_previous_frames_order() {
    let cat = FrameDetection {
        label: "cat".to_owned(),
        score: 0.8,
        edges: [400, 100, 520, 260],
        area: 19_200,
        ratio: 0.75,
        region: Vec::new(),
    };
    let mut ids = IdSource::default();
    let mut stage =
        ObjectStage::new(settings(0, 2, 100), &mut ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    for time in 1..=3 {
        stage
            .step(f64::from(time), vec![person(0.9, STAY), cat.clone()])
            .unwrap();
    }
    stage.step(4.0, Vec::new()).unwrap();
    stage.step(5.0, Vec::new()).unwrap();
    let retired = stage.step(6.0, Vec::new()).unwrap();
    assert!(retired.objects.is_empty(), "both objects retired");
    assert_eq!(
        retired.ended,
        ["1", "2"],
        "previous-frame order, person first"
    );
}

#[test]
fn ids_beyond_nine_are_decimal_strings() {
    let mut ids = IdSource::default();
    let mut stage =
        ObjectStage::new(settings(0, 5, 100), &mut ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    let mut detections = Vec::new();
    for index in 0..17 {
        let left = 5 + 37 * index;
        detections.push(person(0.9, [left, 20, left + 30, 120]));
    }
    let frame = stage.step(1.0, detections).unwrap();
    assert_eq!(frame.objects.len(), 17, "all register");
    let seen: Vec<&str> = frame.objects.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(seen[15], "16", "the sixteenth id is decimal sixteen");
    assert_eq!(seen[16], "17", "the seventeenth id is decimal seventeen");
}

#[test]
fn a_zone_at_the_box_bottom_centre_is_entered_then_left_while_entered_stays() {
    let zone = Zone {
        name: "dock".to_owned(),
        polygon: vec![[140, 320], [400, 320], [400, 420], [140, 420]],
        labels: Vec::new(),
        inertia: 1,
    };
    let mut ids = IdSource::default();
    let mut stage =
        ObjectStage::new(settings(0, 5, 100), &mut ids, ZoneSet::new(vec![zone]), 0.5).unwrap();
    let inside = stage.step(1.0, vec![person(0.9, STAY)]).unwrap();
    let object = one_box(&inside);
    assert_eq!(object.zones_now, ["dock"], "foot at 150,340 is inside");
    assert_eq!(object.zones_entered, ["dock"]);
    let left = stage
        .step(2.0, vec![person(0.9, [80, 140, 180, 340])])
        .unwrap();
    let object = one_box(&left);
    assert_eq!(object.id, "1", "the moved box stays the same object");
    assert!(object.zones_now.is_empty(), "foot at 130,340 is outside");
    assert_eq!(object.zones_entered, ["dock"], "entered only grows");
}

#[test]
fn a_retired_object_ends_exactly_once_after_the_tracker_forgets_it() {
    let mut ids = IdSource::default();
    let mut stage =
        ObjectStage::new(settings(0, 2, 100), &mut ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    for time in 1..=3 {
        stage
            .step(f64::from(time), vec![person(0.9, STAY)])
            .unwrap();
    }
    let first_gap = stage.step(4.0, Vec::new()).unwrap();
    assert_eq!(first_gap.objects.len(), 1, "kept while unseen");
    assert!(first_gap.ended.is_empty());
    let second_gap = stage.step(5.0, Vec::new()).unwrap();
    assert!(second_gap.ended.is_empty());
    let third_gap = stage.step(6.0, Vec::new()).unwrap();
    assert!(third_gap.objects.is_empty());
    assert_eq!(
        third_gap.ended,
        ["1"],
        "ends on the frame it leaves the tracker"
    );
    let fourth_gap = stage.step(7.0, Vec::new()).unwrap();
    assert!(fourth_gap.ended.is_empty(), "an id ends once");
}

#[test]
fn two_stages_over_the_same_frames_produce_the_same_output() {
    let inputs: Vec<(f64, Vec<FrameDetection>)> = vec![
        (1.0, vec![person(0.4, STAY)]),
        (2.0, vec![person(0.9, STAY)]),
        (3.0, vec![person(0.9, STAY)]),
        (4.0, Vec::new()),
        (5.0, Vec::new()),
        (6.0, Vec::new()),
    ];
    let mut runs: Vec<Vec<FrameObjects>> = Vec::new();
    for _ in 0..2 {
        let mut ids = IdSource::default();
        let zone = Zone {
            name: "dock".to_owned(),
            polygon: vec![[140, 320], [400, 320], [400, 420], [140, 420]],
            labels: Vec::new(),
            inertia: 1,
        };
        let mut stage =
            ObjectStage::new(settings(0, 2, 2), &mut ids, ZoneSet::new(vec![zone]), 0.5).unwrap();
        let mut run = Vec::new();
        for (time, detections) in &inputs {
            run.push(stage.step(*time, detections.clone()).unwrap());
        }
        runs.push(run);
    }
    assert_eq!(runs[0], runs[1], "same input, same output, all fields");
}

#[test]
fn edges_touching_the_frame_border_divide_without_clamping() {
    let mut ids = IdSource::default();
    let mut stage =
        ObjectStage::new(settings(0, 5, 100), &mut ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    let frame = stage
        .step(1.0, vec![person(0.9, [540, 380, 639, 479])])
        .unwrap();
    let object = one_box(&frame);
    assert_eq!(
        object.bounds.map(f64::to_bits),
        [
            (540.0_f64 / 640.0).to_bits(),
            (380.0_f64 / 480.0).to_bits(),
            (639.0_f64 / 640.0).to_bits(),
            (479.0_f64 / 480.0).to_bits(),
        ]
    );
    assert!(
        same(object.bounds[2], 639.0 / 640.0),
        "the edge reaches 639 uncapped"
    );
    assert!(
        same(object.bounds[3], 479.0 / 480.0),
        "the edge reaches 479 uncapped"
    );
    assert_eq!(
        object.edges,
        [540, 380, 639, 479],
        "the tracker reports the box unclamped"
    );
}
