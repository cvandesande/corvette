use std::collections::BTreeMap;

use corvette_track::{Detection, Frame, Geometry, Region, clipped, reduce_detections};

const FIXTURE: &str = include_str!("fixtures/reduce_cases.txt");
const P: &str = "person";

fn exact_f64(value: i64) -> f64 {
    let high = i32::try_from(value.div_euclid(4_294_967_296)).expect("a high half");
    let low = u32::try_from(value.rem_euclid(4_294_967_296)).expect("a low half");
    let scaled = f64::from(high) * 4_294_967_296.0;
    scaled + f64::from(low)
}

fn geometry_of(box_: [i64; 4]) -> Geometry {
    let width = box_[2] - box_[0];
    let height = box_[3] - box_[1];
    Geometry {
        x_min: box_[0],
        y_min: box_[1],
        x_max: box_[2],
        y_max: box_[3],
        area: width * height,
        ratio: exact_f64(width) / exact_f64(height.max(1)),
    }
}

fn det_region(label: &str, score: f32, box_: [i64; 4], region: [i64; 4]) -> Detection {
    let region = Region::new(region[0], region[1], region[2], region[3]).expect("the hand region");
    Detection::new(label, score, geometry_of(box_), region).expect("a hand detection")
}

fn det(label: &str, score: f32, box_: [i64; 4]) -> Detection {
    det_region(label, score, box_, [0, 0, 320, 320])
}

const fn frame() -> Frame {
    Frame::new(704, 576).expect("the hand frame")
}

fn survivor_ids(input: &[Detection], out: &[Detection]) -> Vec<usize> {
    let mut used: Vec<bool> = vec![false; input.len()];
    out.iter()
        .map(|kept| {
            input
                .iter()
                .enumerate()
                .find(|(index, candidate)| {
                    !used[*index]
                        && candidate.label() == kept.label()
                        && candidate.score().to_bits() == kept.score().to_bits()
                        && candidate.geometry() == kept.geometry()
                })
                .map(|(index, _)| {
                    used[index] = true;
                    index
                })
                .expect("a survivor from the input")
        })
        .collect()
}

fn check(input: &[Detection], expected: &[usize]) {
    let got = reduce_detections(input.to_vec(), frame());
    assert_eq!(survivor_ids(input, &got), expected, "input {input:?}");
}

fn hand_rows_score_floor_and_confidence() {
    check(&[det(P, 0.9, [10, 10, 60, 60])], &[0]);
    check(&[det(P, 0.5, [10, 10, 60, 60])], &[]);
    check(&[det(P, 0.51, [10, 10, 60, 60])], &[0]);
    check(
        &[det_region(
            P,
            0.3,
            [103, 200, 150, 250],
            [100, 100, 420, 420],
        )],
        &[0],
    );
    check(
        &[det_region(
            P,
            0.3,
            [120, 200, 150, 250],
            [100, 100, 420, 420],
        )],
        &[],
    );
    check(
        &[det(P, 0.9, [10, 10, 60, 60]), det(P, 0.8, [10, 10, 60, 60])],
        &[0],
    );
    check(
        &[det(P, 0.9, [10, 10, 60, 60]), det(P, 0.9, [10, 10, 60, 60])],
        &[0],
    );
    check(
        &[det(P, 0.8, [10, 10, 60, 60]), det(P, 0.9, [10, 10, 60, 60])],
        &[1],
    );
}

fn hand_rows_nms_limits_and_order() {
    check(
        &[det(P, 0.9, [0, 0, 7, 7]), det(P, 0.8, [3, 0, 10, 7])],
        &[0, 1],
    );
    check(
        &[det(P, 0.9, [0, 0, 7, 7]), det(P, 0.8, [2, 0, 9, 7])],
        &[0],
    );
    check(
        &[det(P, 0.9, [0, 0, 7, 7]), det(P, 0.8, [4, 0, 11, 7])],
        &[0, 1],
    );
    check(
        &[
            det("car", 0.9, [0, 0, 8, 8]),
            det("car", 0.8, [2, 0, 10, 8]),
        ],
        &[0, 1],
    );
    check(
        &[det("car", 0.9, [0, 0, 8, 8]), det("car", 0.8, [1, 0, 9, 8])],
        &[0],
    );
    check(
        &[
            det("car", 0.9, [0, 0, 8, 8]),
            det("car", 0.8, [3, 0, 11, 8]),
        ],
        &[0, 1],
    );
    check(
        &[
            det(P, 0.6, [0, 0, 10, 10]),
            det(P, 0.9, [100, 100, 110, 110]),
        ],
        &[1, 0],
    );
    check(
        &[
            det(P, 0.9, [0, 0, 20, 20]),
            det(P, 0.6, [100, 100, 110, 110]),
            det(P, 0.7, [200, 200, 260, 260]),
        ],
        &[1, 0, 2],
    );
    check(
        &[
            det("dog", 0.3, [0, 0, 10, 10]),
            det(P, 0.9, [100, 100, 150, 150]),
        ],
        &[1],
    );
    check(
        &[
            det(P, 0.4, [0, 0, 10, 10]),
            det("car", 0.9, [100, 100, 150, 150]),
            det(P, 0.9, [200, 200, 250, 250]),
        ],
        &[2, 1],
    );
    check(
        &[det(P, 0.9, [0, 0, 5, 5]), det("car", 0.9, [0, 0, 50, 50])],
        &[0, 1],
    );
    check(
        &[
            det_region(P, 0.9, [103, 200, 150, 250], [100, 100, 420, 420]),
            det_region(P, 0.3, [103, 200, 150, 250], [100, 100, 420, 420]),
        ],
        &[0],
    );
}

fn hand_rows_degenerate_and_clipped() {
    check(
        &[
            det(P, 0.9, [-16_777_216, -16_777_216, 16_777_216, 16_777_216]),
            det(P, 0.8, [-16_777_216, -16_777_216, 16_777_216, 16_777_216]),
        ],
        &[0],
    );
    check(
        &[det(P, 0.9, [30, 30, 20, 20]), det(P, 0.8, [0, 0, 50, 50])],
        &[0, 1],
    );
    check(
        &[det(P, 0.9, [5, 5, 5, 5]), det(P, 0.8, [0, 0, 50, 50])],
        &[0, 1],
    );
    check(
        &[det(P, 0.9, [5, 5, 5, 10]), det(P, 0.8, [20, 20, 19, 20])],
        &[0],
    );
    check(
        &[det(P, 0.9, [0, 0, 20, 20]), det(P, 0.8, [0, 0, 40, 40])],
        &[1],
    );
    check(
        &[det_region(
            P,
            0.3,
            [400, 110, 693, 150],
            [384, 100, 698, 420],
        )],
        &[0],
    );
    check(
        &[det_region(
            P,
            0.3,
            [400, 110, 699, 150],
            [384, 100, 704, 420],
        )],
        &[],
    );
}

fn hand_rows_prefix_labels() {
    check(
        &[
            det("cart", 0.9, [0, 0, 10, 10]),
            det("car", 0.9, [0, 0, 10, 10]),
        ],
        &[0, 1],
    );
    check(
        &[
            det("car", 0.9, [0, 0, 10, 10]),
            det("cart", 0.9, [0, 0, 10, 10]),
        ],
        &[0, 1],
    );
    check(
        &[
            det("cart", 0.9, [0, 0, 8, 8]),
            det("cart", 0.8, [2, 0, 10, 8]),
            det("car", 0.9, [0, 0, 8, 8]),
            det("car", 0.8, [2, 0, 10, 8]),
        ],
        &[0, 2, 3],
    );
}

#[test]
fn hand_rows_match_frigates_reduce() {
    hand_rows_score_floor_and_confidence();
    hand_rows_nms_limits_and_order();
    hand_rows_degenerate_and_clipped();
    hand_rows_prefix_labels();
}

#[test]
fn reduce_is_total_on_extreme_detections() {
    assert_eq!(reduce_detections(Vec::new(), frame()).len(), 0);
    let nan = det(P, f32::NAN, [0, 0, 10, 10]);
    assert!(reduce_detections(vec![nan], frame()).len() <= 1);
    let inf = det(P, f32::INFINITY, [0, 0, 10, 10]);
    assert!(reduce_detections(vec![inf], frame()).len() <= 1);
    let neg_inf = det(P, f32::NEG_INFINITY, [0, 0, 10, 10]);
    assert!(reduce_detections(vec![neg_inf], frame()).len() <= 1);
    let big = det(P, 0.9, [-16_777_216, -16_777_216, 16_777_216, 16_777_216]);
    assert!(reduce_detections(vec![big.clone(), big], frame()).len() <= 2);
    let zero = det(P, 0.9, [0, 0, 0, 0]);
    assert_eq!(reduce_detections(vec![zero], frame()).len(), 1);
    let same = det(P, 0.9, [0, 0, 50, 50]);
    assert_eq!(reduce_detections(vec![same; 200], frame()).len(), 1);
}

fn parse_edges(text: &str, line_no: usize) -> [i64; 4] {
    let parts: Vec<i64> = text
        .split(',')
        .map(|v| v.parse().expect("an edge"))
        .collect();
    let [a, b, c, d] = parts.as_slice() else {
        panic!("line {line_no}: expected four edges");
    };
    [*a, *b, *c, *d]
}

fn parse_det(text: &str, line_no: usize) -> Detection {
    let mut parts = text.split(':');
    let label = parts.next().expect("a label");
    let bits = parts.next().expect("score bits");
    let box_text = parts.next().expect("a box");
    let region_text = parts.next().expect("a region");
    let edges = parse_edges(box_text, line_no);
    let region_edges = parse_edges(region_text, line_no);
    let score = f32::from_bits(u32::from_str_radix(bits, 16).expect("a score"));
    let region = Region::new(
        region_edges[0],
        region_edges[1],
        region_edges[2],
        region_edges[3],
    )
    .expect("a region");
    Detection::new(label, score, geometry_of(edges), region).expect("a detection")
}

fn assert_header_counts(header: &BTreeMap<&str, usize>, counts: &BTreeMap<&str, usize>) {
    for (name, want) in [
        ("cases", 4000),
        ("ordinary", 1000),
        ("exact_pairs", 800),
        ("clipped_cases", 500),
        ("score_threshold", 400),
        ("order_ties", 400),
        ("degenerate", 400),
        ("large", 500),
        ("dropped", 2508),
        ("first_pass_drop_above", 1395),
        ("score_drop", 1356),
        ("clipped_low_survive", 242),
        ("limit_keep", 216),
        ("limit_drop", 265),
        ("reordered", 808),
        ("label_vanishes", 735),
        ("zerodiv_excluded", 580),
        ("large_nms_drop", 299),
    ] {
        assert_eq!(header[name], want, "header {name}");
    }
    for name in [
        "dropped",
        "score_drop",
        "clipped_low_survive",
        "reordered",
        "label_vanishes",
    ] {
        assert_eq!(counts[name], header[name], "parsed {name}");
    }
    for (name, floor) in [
        ("dropped", 1500),
        ("first_pass_drop_above", 600),
        ("score_drop", 300),
        ("clipped_low_survive", 200),
        ("limit_keep", 150),
        ("limit_drop", 150),
        ("reordered", 300),
        ("label_vanishes", 100),
        ("zerodiv_excluded", 40),
        ("large_nms_drop", 100),
    ] {
        assert!(header[name] >= floor, "{name} floor");
    }
}

#[test]
fn fixture_matches_frigate_on_every_case() {
    let mut header: BTreeMap<&str, usize> = BTreeMap::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut cases = 0;
    for (index, line) in FIXTURE.lines().enumerate() {
        let line_no = index + 1;
        if line.starts_with('#') {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.len() >= 2 && tokens[1] == "counts" {
                let mut at = 2;
                while at + 1 < tokens.len() {
                    header.insert(tokens[at], tokens[at + 1].parse().expect("a header count"));
                    at += 2;
                }
            }
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(
            tokens.first().copied(),
            Some("reduce"),
            "line {line_no}: {line}"
        );
        cases += 1;
        let arrow = tokens
            .iter()
            .position(|token| *token == "=>")
            .unwrap_or_else(|| panic!("line {line_no}: no arrow"));
        assert_eq!(tokens[2], "frame", "line {line_no}: {line}");
        let width: i64 = tokens[3].parse().expect("a width");
        let height: i64 = tokens[4].parse().expect("a height");
        assert_eq!(tokens[5], "dets", "line {line_no}: {line}");
        let frame = Frame::new(width, height).expect("a frame");
        let dets: Vec<Detection> = tokens[6..arrow]
            .iter()
            .map(|token| parse_det(token, line_no))
            .collect();
        let expected: Vec<usize> = if arrow + 1 < tokens.len() && tokens[arrow + 1] == "-" {
            Vec::new()
        } else {
            tokens[arrow + 1..]
                .iter()
                .map(|token| token.parse().expect("a survivor index"))
                .collect()
        };
        let got = reduce_detections(dets.clone(), frame);
        assert_eq!(
            survivor_ids(&dets, &got),
            expected,
            "line {line_no}: {line}"
        );
        if expected.len() < dets.len() {
            *counts.entry("dropped").or_insert(0) += 1;
        }
        let mut sorted = expected.clone();
        sorted.sort_unstable();
        if expected != sorted {
            *counts.entry("reordered").or_insert(0) += 1;
        }
        let out_labels: Vec<&str> = expected
            .iter()
            .filter_map(|&index| dets.get(index))
            .map(Detection::label)
            .collect();
        let mut vanishes = false;
        let mut score_drop = false;
        let mut clipped_low = false;
        for detection in &dets {
            let kept = expected
                .iter()
                .any(|&index| dets.get(index).is_some_and(|k| k == detection));
            if !out_labels.contains(&detection.label()) {
                vanishes = true;
            }
            let is_clipped = clipped(detection, frame);
            if !is_clipped && detection.score() <= 0.5 && !kept {
                score_drop = true;
            }
            if is_clipped && detection.score() <= 0.5 && kept {
                clipped_low = true;
            }
        }
        if vanishes {
            *counts.entry("label_vanishes").or_insert(0) += 1;
        }
        if score_drop {
            *counts.entry("score_drop").or_insert(0) += 1;
        }
        if clipped_low {
            *counts.entry("clipped_low_survive").or_insert(0) += 1;
        }
    }
    assert_eq!(cases, 4000);
    assert_header_counts(&header, &counts);
}
