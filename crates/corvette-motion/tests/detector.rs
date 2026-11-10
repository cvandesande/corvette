use corvette_motion::{
    Decision, MotionBox, MotionDetector, MotionSettings, Plane, Shape, motion_shape,
};

const FIXTURE: &str = include_str!("fixtures/detector_cases.txt");

struct RecordedFrame {
    grey: Vec<u8>,
    boxes: Vec<MotionBox>,
    calibrating: bool,
    average_hash: String,
}

struct Sequence {
    kind: String,
    index: usize,
    width: usize,
    height: usize,
    setting: usize,
    settings: MotionSettings,
    keep: Option<Vec<u8>>,
    frames: Vec<RecordedFrame>,
}

fn parse_number(text: &str) -> usize {
    text.parse().unwrap()
}

fn decode_hex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|position| u8::from_str_radix(&hex[position..position + 2], 16).unwrap())
        .collect()
}

fn fnv1a(values: &[f32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for value in values {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

fn read_fixture() -> (Vec<Sequence>, Vec<(String, usize)>) {
    let lines: Vec<&str> = FIXTURE.lines().collect();
    let header: Vec<&str> = lines[0].split_whitespace().collect();
    assert_eq!(header.first().copied(), Some("counts"));
    let mut counts = Vec::new();
    for pair in header[1..].chunks_exact(2) {
        counts.push((pair[0].to_string(), parse_number(pair[1])));
    }
    let mut sequences = Vec::new();
    let mut position = 1;
    while position < lines.len() {
        let fields: Vec<&str> = lines[position].split_whitespace().collect();
        position += 1;
        assert_eq!(fields.first().copied(), Some("sequence"));
        let kind = fields[1].to_string();
        let width = parse_number(fields[3]);
        let height = parse_number(fields[4]);
        let setting = parse_number(fields[5]);
        let settings = MotionSettings {
            threshold: u8::try_from(parse_number(fields[6])).unwrap(),
            lightning_threshold: fields[7].parse().unwrap(),
            improve_contrast: fields[8] == "1",
            contour_area: u32::try_from(parse_number(fields[9])).unwrap(),
            frame_alpha: fields[10].parse().unwrap(),
            frame_height: setting,
        };
        let mask_fields: Vec<&str> = lines[position].split_whitespace().collect();
        position += 1;
        assert_eq!(mask_fields.first().copied(), Some("mask"));
        let keep = if mask_fields[1] == "-" {
            None
        } else {
            Some(
                mask_fields[1]
                    .bytes()
                    .map(|byte| if byte == b'1' { 255 } else { 0 })
                    .collect::<Vec<u8>>(),
            )
        };
        let frame_count = parse_number(fields[11]);
        let mut frames = Vec::new();
        for _ in 0..frame_count {
            let parts: Vec<&str> = lines[position].split_whitespace().collect();
            position += 1;
            assert_eq!(parts.first().copied(), Some("frame"));
            assert_eq!(parts.get(2).copied(), Some("="));
            let grey = decode_hex(parts[1]);
            let mut boxes = Vec::new();
            let mut cursor = 4;
            for _ in 0..parse_number(parts[3]) {
                boxes.push(MotionBox {
                    x1: parse_number(parts[cursor]),
                    y1: parse_number(parts[cursor + 1]),
                    x2: parse_number(parts[cursor + 2]),
                    y2: parse_number(parts[cursor + 3]),
                });
                cursor += 4;
            }
            frames.push(RecordedFrame {
                grey,
                boxes,
                calibrating: parts[cursor] == "1",
                average_hash: parts[cursor + 1].to_string(),
            });
        }
        sequences.push(Sequence {
            kind,
            index: parse_number(fields[2]),
            width,
            height,
            setting,
            settings,
            keep,
            frames,
        });
    }
    (sequences, counts)
}

const fn hand_settings(improve_contrast: bool) -> MotionSettings {
    MotionSettings {
        threshold: 30,
        lightning_threshold: 0.8,
        improve_contrast,
        contour_area: 10,
        frame_alpha: 0.01,
        frame_height: 12,
    }
}

const fn hand_frame() -> Shape {
    Shape::new(32, 24).unwrap()
}

fn whole_frame() -> Vec<MotionBox> {
    vec![MotionBox {
        x1: 0,
        y1: 0,
        x2: 32,
        y2: 24,
    }]
}

fn every_value_is(values: &[f32], expected: f32) -> bool {
    values
        .iter()
        .all(|value| value.to_bits() == expected.to_bits())
}

#[test]
fn the_detector_matches_the_deployed_class_for_every_frame() {
    let (sequences, counts) = read_fixture();
    let mut read_counts: Vec<(String, usize)> = Vec::new();
    for sequence in &sequences {
        match read_counts.last_mut() {
            Some((kind, number)) if *kind == sequence.kind => *number += 1,
            _ => read_counts.push((sequence.kind.clone(), 1)),
        }
    }
    assert_eq!(
        read_counts, counts,
        "header counts equal the sequences read"
    );
    for sequence in &sequences {
        let frame = Shape::new(sequence.width, sequence.height).unwrap();
        let motion = motion_shape(frame, sequence.setting).unwrap();
        let keep = sequence
            .keep
            .as_deref()
            .map(|bytes| Plane::new(bytes, motion).unwrap());
        let mut detector = MotionDetector::new(sequence.settings, frame, keep)
            .unwrap_or_else(|| panic!("{} sequence {}: refused", sequence.kind, sequence.index));
        for (number, recorded) in sequence.frames.iter().enumerate() {
            let context = format!(
                "{} sequence {} frame {number}",
                sequence.kind, sequence.index
            );
            let picture = Plane::new(&recorded.grey, frame).unwrap();
            let decision = detector
                .feed(picture)
                .unwrap_or_else(|| panic!("{context}: refused"));
            assert_eq!(decision.boxes, recorded.boxes, "{context}");
            assert_eq!(detector.calibrating(), recorded.calibrating, "{context}");
            assert_eq!(
                format!("{:016x}", fnv1a(detector.average().values())),
                recorded.average_hash,
                "{context}"
            );
        }
    }
}

#[test]
fn a_frame_of_another_shape_is_refused_and_changes_nothing() {
    let settings = hand_settings(true);
    let frame = hand_frame();
    let mut detector = MotionDetector::new(settings, frame, None).unwrap();
    let wrong: Vec<u8> = (0..33 * 24)
        .map(|position| u8::try_from(position % 251).unwrap())
        .collect();
    let wrong_picture = Plane::new(&wrong, Shape::new(33, 24).unwrap()).unwrap();
    assert_eq!(detector.feed(wrong_picture), None);
    let mut fresh = MotionDetector::new(settings, frame, None).unwrap();
    for step in 0..12 {
        let grey: Vec<u8> = (0..32 * 24)
            .map(|position| {
                let x = position % 32;
                let y = position / 32;
                u8::try_from((7 * x + 11 * y + 5 * step) % 256).unwrap()
            })
            .collect();
        let picture = Plane::new(&grey, frame).unwrap();
        let first = detector.feed(picture);
        let second = fresh.feed(picture);
        assert_eq!(first, second, "frame {step}");
    }
    assert_eq!(
        detector
            .average()
            .values()
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<u32>>(),
        fresh
            .average()
            .values()
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<u32>>()
    );
}

#[test]
fn a_mask_of_another_shape_is_refused() {
    let settings = hand_settings(true);
    let frame = hand_frame();
    let wrong: Vec<u8> = vec![255; 16 * 13];
    let wrong_mask = Plane::new(&wrong, Shape::new(16, 13).unwrap()).unwrap();
    assert!(MotionDetector::new(settings, frame, Some(wrong_mask)).is_none());
    let right: Vec<u8> = vec![255; 16 * 12];
    let right_mask = Plane::new(&right, Shape::new(16, 12).unwrap()).unwrap();
    assert!(MotionDetector::new(settings, frame, Some(right_mask)).is_some());
    assert!(MotionDetector::new(settings, frame, None).is_some());
    let mut no_height = hand_settings(true);
    no_height.frame_height = 0;
    assert!(MotionDetector::new(no_height, frame, None).is_none());
}

#[test]
fn the_average_waits_for_ten_frames_of_motion_then_learns_with_weight_point_two() {
    let mut detector = MotionDetector::new(hand_settings(false), hand_frame(), None).unwrap();
    let grey = vec![100_u8; 32 * 24];
    let picture = Plane::new(&grey, hand_frame()).unwrap();
    for _ in 0..9 {
        let decision = detector.feed(picture).unwrap();
        assert_eq!(decision.boxes, whole_frame());
        assert!(decision.calibrating);
        assert!(every_value_is(detector.average().values(), 0.0_f32));
    }
    for expected in [20.0_f32, 36.0, 48.8] {
        detector.feed(picture).unwrap();
        assert!(every_value_is(detector.average().values(), expected));
    }
    detector.feed(picture).unwrap();
    detector.feed(picture).unwrap();
}

#[test]
fn a_flat_scene_settles_and_calibration_ends() {
    let mut detector = MotionDetector::new(hand_settings(false), hand_frame(), None).unwrap();
    let grey = vec![50_u8; 32 * 24];
    let picture = Plane::new(&grey, hand_frame()).unwrap();
    for _ in 0..12 {
        let decision = detector.feed(picture).unwrap();
        assert_eq!(decision.boxes, whole_frame());
        assert!(decision.calibrating);
    }
    assert!(every_value_is(detector.average().values(), 24.4_f32));
    let decision = detector.feed(picture).unwrap();
    assert!(decision.boxes.is_empty());
    assert!(!decision.calibrating);
    assert!(every_value_is(detector.average().values(), 24.656_f32));
}

#[test]
fn a_block_inside_the_mask_is_not_motion() {
    let keep: Vec<u8> = (0..16 * 12)
        .map(|position| {
            let x = position % 16;
            let y = position / 16;
            u8::try_from(if (4..8).contains(&y) && (8..14).contains(&x) {
                0
            } else {
                255
            })
            .unwrap()
        })
        .collect();
    let keep_picture = Plane::new(&keep, Shape::new(16, 12).unwrap()).unwrap();
    let mut masked =
        MotionDetector::new(hand_settings(false), hand_frame(), Some(keep_picture)).unwrap();
    let mut plain = MotionDetector::new(hand_settings(false), hand_frame(), None).unwrap();
    let block = vec![MotionBox {
        x1: 12,
        y1: 4,
        x2: 32,
        y2: 20,
    }];
    for number in 0..26 {
        let grey: Vec<u8> = (0..32 * 24)
            .map(|position| {
                let x = position % 32;
                let y = position / 32;
                u8::try_from(
                    if number >= 20 && (8..16).contains(&y) && (16..28).contains(&x) {
                        200
                    } else {
                        50
                    },
                )
                .unwrap()
            })
            .collect();
        let picture = Plane::new(&grey, hand_frame()).unwrap();
        let inside: Option<Decision> = masked.feed(picture);
        let outside: Option<Decision> = plain.feed(picture);
        let (inside, outside) = (inside.unwrap(), outside.unwrap());
        let context = format!("frame {number}");
        if number < 12 {
            assert_eq!(inside.boxes, whole_frame(), "{context} masked");
            assert!(inside.calibrating, "{context} masked");
            assert_eq!(outside.boxes, whole_frame(), "{context} plain");
            assert!(outside.calibrating, "{context} plain");
        } else if number < 20 {
            assert!(
                inside.boxes.is_empty() && !inside.calibrating,
                "{context} masked"
            );
            assert!(
                outside.boxes.is_empty() && !outside.calibrating,
                "{context} plain"
            );
        } else {
            assert!(
                inside.boxes.is_empty() && !inside.calibrating,
                "{context} masked"
            );
            assert_eq!(outside.boxes, block, "{context} plain");
            assert!(!outside.calibrating, "{context} plain");
        }
    }
}
