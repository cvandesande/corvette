use corvette_motion::{Contour, MotionBox, MotionDecider, MotionSettings, Shape};

const FIXTURE: &str = include_str!("fixtures/decision_cases.txt");

const fn contour(x: usize, y: usize, width: usize, height: usize, twice_area: u64) -> Contour {
    Contour {
        x,
        y,
        width,
        height,
        twice_area,
    }
}

fn fresh(frame_width: usize, frame_height: usize) -> MotionDecider {
    let frame = Shape::new(frame_width, frame_height).unwrap();
    MotionDecider::new(MotionSettings::default(), frame).unwrap()
}

fn bits_of(field: &str) -> Option<u64> {
    (field != "-").then(|| u64::from_str_radix(field, 16).unwrap())
}

fn parse_frame(field: &[&str]) -> (Vec<Contour>, Vec<MotionBox>, u64, bool, Option<u64>) {
    let count: usize = field[1].parse().unwrap();
    let mut contours = Vec::with_capacity(count);
    let mut at = 2;
    for _ in 0..count {
        contours.push(contour(
            field[at].parse().unwrap(),
            field[at + 1].parse().unwrap(),
            field[at + 2].parse().unwrap(),
            field[at + 3].parse().unwrap(),
            field[at + 4].parse().unwrap(),
        ));
        at += 5;
    }
    assert_eq!(field[at], "=");
    at += 1;
    let box_count: usize = field[at].parse().unwrap();
    at += 1;
    let mut want_boxes = Vec::with_capacity(box_count);
    for _ in 0..box_count {
        want_boxes.push(MotionBox {
            x1: field[at].parse().unwrap(),
            y1: field[at + 1].parse().unwrap(),
            x2: field[at + 2].parse().unwrap(),
            y2: field[at + 3].parse().unwrap(),
        });
        at += 4;
    }
    let fraction_bits = u64::from_str_radix(field[at], 16).unwrap();
    at += 1;
    let calibrating = field[at] == "1";
    at += 1;
    assert_eq!(at + 1, field.len());
    (
        contours,
        want_boxes,
        fraction_bits,
        calibrating,
        bits_of(field[at]),
    )
}

#[test]
fn the_decisions_match_the_reference_for_every_frame() {
    const KINDS: [&str; 5] = ["scale", "filter", "fraction", "states", "random"];
    let mut lines = FIXTURE.lines();
    let header: Vec<&str> = lines.next().unwrap().split(' ').collect();
    let mut expected = [0_usize; 5];
    for (number, kind) in KINDS.iter().enumerate() {
        assert_eq!(header[1 + 2 * number], *kind);
        expected[number] = header[2 + 2 * number].parse().unwrap();
    }
    let mut seen = [0_usize; 5];
    let mut kind_index: Option<usize> = None;
    let mut index_in_kind = 0_usize;
    let mut frame_index = 0_usize;
    let mut declared_frames = 0_usize;
    let mut decider: Option<MotionDecider> = None;
    for line in lines {
        let field: Vec<&str> = line.split(' ').collect();
        if field[0] == "sequence" {
            if kind_index.is_some() {
                assert_eq!(
                    frame_index, declared_frames,
                    "frames of the previous sequence"
                );
            }
            let k = KINDS.iter().position(|name| *name == field[1]).unwrap();
            match kind_index {
                None => assert_eq!(k, 0, "kinds must start at scale: {line}"),
                Some(previous) => assert!(
                    k == previous || k == previous + 1,
                    "kinds out of order: {line}"
                ),
            }
            kind_index = Some(k);
            seen[k] += 1;
            index_in_kind = seen[k] - 1;
            let settings = MotionSettings {
                lightning_threshold: field[4].parse().unwrap(),
                contour_area: field[5].parse().unwrap(),
                frame_alpha: field[6].parse().unwrap(),
                frame_height: field[7].parse().unwrap(),
                ..Default::default()
            };
            let frame = Shape::new(field[2].parse().unwrap(), field[3].parse().unwrap()).unwrap();
            decider = Some(MotionDecider::new(settings, frame).unwrap());
            declared_frames = field[8].parse().unwrap();
            frame_index = 0;
            continue;
        }
        assert_eq!(field[0], "frame");
        let kind_name = KINDS[kind_index.unwrap()];
        let location = format!("kind {kind_name} sequence {index_in_kind} frame {frame_index}");
        let (contours, want_boxes, fraction_bits, calibrating, want_weight) = parse_frame(&field);
        let decision = decider.as_mut().unwrap().decide(&contours);
        assert_eq!(decision.boxes, want_boxes, "{location}");
        assert_eq!(
            decision.motion_fraction.to_bits(),
            fraction_bits,
            "{location}"
        );
        assert_eq!(decision.calibrating, calibrating, "{location}");
        assert_eq!(
            decision.average_weight.map(f64::to_bits),
            want_weight,
            "{location}"
        );
        frame_index += 1;
    }
    assert_eq!(frame_index, declared_frames, "frames of the last sequence");
    assert_eq!(seen, expected);
}

#[test]
fn the_default_settings_are_the_deployed_configuration_defaults() {
    let settings = MotionSettings::default();
    assert_eq!(settings.threshold, 30);
    assert_eq!(settings.lightning_threshold.to_bits(), 0.8_f64.to_bits());
    assert!(settings.improve_contrast);
    assert_eq!(settings.contour_area, 10);
    assert_eq!(settings.frame_alpha.to_bits(), 0.01_f64.to_bits());
    assert_eq!(settings.frame_height, 100);
}

#[test]
fn a_new_decider_is_calibrating_and_an_empty_frame_ends_calibration() {
    let mut decider = fresh(1280, 720);
    assert!(decider.calibrating());
    let decision = decider.decide(&[]);
    assert!(decision.boxes.is_empty());
    assert_eq!(decision.motion_fraction.to_bits(), 0);
    assert!(!decision.calibrating);
    assert!(!decider.calibrating());
    assert_eq!(decision.average_weight, Some(0.01));
}

#[test]
fn a_contour_is_a_box_only_when_its_area_exceeds_the_minimum() {
    let mut decider = fresh(1280, 720);
    let decision = decider.decide(&[contour(10, 20, 30, 40, 22)]);
    assert_eq!(
        decision.boxes,
        vec![MotionBox {
            x1: 72,
            y1: 144,
            x2: 288,
            y2: 432
        }]
    );
    assert_eq!(
        decision.motion_fraction.to_bits(),
        (11.0_f64 / 17700.0).to_bits()
    );
    assert!(!decision.calibrating);
    assert_eq!(decision.average_weight, None);

    let mut decider = fresh(1280, 720);
    let decision = decider.decide(&[contour(10, 20, 30, 40, 20)]);
    assert!(decision.boxes.is_empty());
    assert_eq!(
        decision.motion_fraction.to_bits(),
        (10.0_f64 / 17700.0).to_bits()
    );
    assert!(!decision.calibrating);
    assert_eq!(decision.average_weight, Some(0.01));

    let mut decider = fresh(1280, 720);
    let decision = decider.decide(&[contour(0, 0, 1, 1, 21)]);
    assert_eq!(
        decision.boxes,
        vec![MotionBox {
            x1: 0,
            y1: 0,
            x2: 7,
            y2: 7
        }]
    );

    let mut decider = fresh(1280, 720);
    assert_eq!(
        decider.decide(&[contour(0, 0, 1, 1, 22)]).boxes,
        vec![MotionBox {
            x1: 0,
            y1: 0,
            x2: 7,
            y2: 7
        }]
    );
}

#[test]
fn box_corners_are_scaled_by_the_height_ratio_and_truncated() {
    let mut decider = fresh(1280, 720);
    assert_eq!(
        decider.decide(&[contour(10, 20, 30, 40, 22)]).boxes,
        vec![MotionBox {
            x1: 72,
            y1: 144,
            x2: 288,
            y2: 432
        }]
    );
    let mut decider = fresh(704, 576);
    assert_eq!(
        decider.decide(&[contour(10, 20, 30, 40, 22)]).boxes,
        vec![MotionBox {
            x1: 57,
            y1: 115,
            x2: 230,
            y2: 345
        }]
    );
    let mut decider = fresh(1920, 1080);
    assert_eq!(
        decider.decide(&[contour(45, 20, 40, 30, 22)]).boxes,
        vec![MotionBox {
            x1: 486,
            y1: 216,
            x2: 918,
            y2: 540
        }]
    );
}

#[test]
fn the_motion_fraction_sums_every_contour_not_only_the_boxes() {
    let mut decider = fresh(1280, 720);
    let decision = decider.decide(&[contour(0, 0, 2, 2, 22), contour(0, 0, 1, 1, 6)]);
    assert_eq!(decision.boxes.len(), 1);
    assert_eq!(
        decision.motion_fraction.to_bits(),
        (14.0_f64 / 17700.0).to_bits()
    );
}

#[test]
fn calibration_ends_below_five_percent_with_at_most_four_boxes() {
    let mut decider = fresh(1280, 720);
    assert!(!decider.decide(&[contour(0, 0, 1, 1, 22); 4]).calibrating);
    let mut decider = fresh(1280, 720);
    assert!(decider.decide(&[contour(0, 0, 1, 1, 22); 5]).calibrating);
    let mut decider = fresh(1280, 720);
    assert!(decider.decide(&[contour(0, 0, 1, 1, 1770)]).calibrating);
    let mut decider = fresh(1280, 720);
    assert!(!decider.decide(&[contour(0, 0, 1, 1, 1769)]).calibrating);
}

#[test]
fn a_burst_above_the_lightning_threshold_restarts_calibration() {
    let mut decider = fresh(1280, 720);
    assert!(!decider.decide(&[]).calibrating);
    assert!(decider.decide(&[contour(0, 0, 1, 1, 31860)]).calibrating);
    assert!(!decider.decide(&[]).calibrating);
    assert!(!decider.decide(&[contour(0, 0, 1, 1, 28320)]).calibrating);
    assert!(decider.decide(&[contour(0, 0, 1, 1, 28321)]).calibrating);
}

#[test]
fn the_average_learns_only_when_there_is_no_box_or_boxes_have_lasted_ten_frames() {
    let mut decider = fresh(1280, 720);
    for _ in 0..9 {
        assert!(
            decider
                .decide(&[contour(0, 0, 1, 1, 40)])
                .average_weight
                .is_none()
        );
    }
    assert_eq!(
        decider.decide(&[contour(0, 0, 1, 1, 40)]).average_weight,
        Some(0.01)
    );
    assert_eq!(decider.decide(&[]).average_weight, Some(0.01));
    for _ in 0..9 {
        assert!(
            decider
                .decide(&[contour(0, 0, 1, 1, 40)])
                .average_weight
                .is_none()
        );
    }
}

#[test]
fn the_weight_is_point_two_while_calibrating_and_the_frame_weight_otherwise() {
    let mut decider = fresh(1280, 720);
    assert_eq!(decider.decide(&[]).average_weight, Some(0.01));
    for _ in 0..9 {
        let decision = decider.decide(&[contour(0, 0, 1, 1, 31860)]);
        assert!(decision.calibrating);
        assert!(decision.average_weight.is_none());
    }
    let decision = decider.decide(&[contour(0, 0, 1, 1, 31860)]);
    assert!(decision.calibrating);
    assert_eq!(decision.average_weight, Some(0.2));
}

#[test]
fn settings_that_cannot_work_are_refused() {
    let frame = Shape::new(1280, 720).unwrap();
    let huge = Shape::new(214_748_365, 100).unwrap();
    let refusals = [
        MotionSettings {
            frame_height: 0,
            ..Default::default()
        },
        MotionSettings {
            frame_alpha: f64::NAN,
            ..Default::default()
        },
        MotionSettings {
            frame_alpha: 1.5,
            ..Default::default()
        },
        MotionSettings {
            frame_alpha: -0.1,
            ..Default::default()
        },
        MotionSettings {
            lightning_threshold: f64::NAN,
            ..Default::default()
        },
        MotionSettings {
            lightning_threshold: f64::INFINITY,
            ..Default::default()
        },
    ];
    for settings in refusals {
        assert!(MotionDecider::new(settings, frame).is_none());
    }
    assert!(MotionDecider::new(MotionSettings::default(), huge).is_none());
    assert!(MotionDecider::new(MotionSettings::default(), frame).is_some());
}
