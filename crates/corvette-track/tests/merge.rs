use std::collections::BTreeMap;

use corvette_track::{
    Detection, Frame, Geometry, Region, clipped, consolidate, inclusive_area, intersection,
};

const FIXTURE: &str = include_str!("fixtures/merge_cases.txt");
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

const fn box_edges(geo: Geometry) -> [i64; 4] {
    [geo.x_min, geo.y_min, geo.x_max, geo.y_max]
}

fn det_region(label: &str, score: f32, box_: [i64; 4], region: [i64; 4]) -> Detection {
    let region = Region::new(region[0], region[1], region[2], region[3]).expect("the hand region");
    Detection::new(label, score, geometry_of(box_), region).expect("a hand detection")
}

fn det(label: &str, score: f32, box_: [i64; 4]) -> Detection {
    det_region(label, score, box_, [0, 0, 320, 320])
}

fn survivor_ids(input: &[Detection], out: &[Detection]) -> Vec<usize> {
    out.iter()
        .map(|kept| {
            input
                .iter()
                .position(|d| {
                    d.label() == kept.label()
                        && d.score().to_bits() == kept.score().to_bits()
                        && d.geometry() == kept.geometry()
                })
                .expect("a survivor from the input")
        })
        .collect()
}

fn check(rows: &[(&str, f32, [i64; 4])], expected: &[usize]) {
    let input: Vec<Detection> = rows.iter().map(|(l, s, b)| det(l, *s, *b)).collect();
    let got = consolidate(input.clone());
    assert_eq!(survivor_ids(&input, &got), expected);
}

#[test]
fn hand_rows_match_frigates_merge() {
    check(&[], &[]);
    check(&[(P, 0.9, [10, 10, 60, 60])], &[0]);
    check(&[(P, 0.9, [0, 0, 99, 9]), (P, 0.1, [0, 0, 99, 9])], &[1]);
    check(
        &[(P, 0.9, [0, 0, 99, 9]), (P, 0.9, [10, 0, 109, 9])],
        &[0, 1],
    );
    check(&[(P, 0.9, [0, 0, 99, 9]), (P, 0.9, [9, 0, 108, 9])], &[1]);
    check(
        &[("car", 0.9, [0, 0, 99, 9]), ("car", 0.9, [20, 0, 119, 9])],
        &[0, 1],
    );
    check(
        &[("car", 0.9, [0, 0, 99, 9]), ("car", 0.9, [19, 0, 118, 9])],
        &[1],
    );
    check(
        &[("face", 0.9, [0, 0, 99, 9]), ("face", 0.9, [50, 0, 149, 9])],
        &[0, 1],
    );
    check(
        &[("face", 0.9, [0, 0, 99, 9]), ("face", 0.9, [49, 0, 148, 9])],
        &[1],
    );
    check(&[(P, 0.9, [0, 0, 3, 4]), (P, 0.9, [0, 0, 19, 19])], &[1]);
    check(&[(P, 0.9, [0, 0, 3, 3]), (P, 0.9, [0, 0, 19, 19])], &[0, 1]);
    check(
        &[
            ("car", 0.9, [0, 0, 50, 50]),
            (P, 0.9, [0, 0, 5, 5]),
            ("car", 0.9, [100, 100, 110, 110]),
            (P, 0.9, [100, 100, 300, 300]),
        ],
        &[2, 0, 1, 3],
    );
    check(&[(P, 0.9, [0, 0, 9, 9]), (P, 0.9, [9, 9, 50, 50])], &[0, 1]);
    check(
        &[(P, 0.9, [30, 30, 20, 20]), (P, 0.9, [0, 0, 50, 50])],
        &[0, 1],
    );
    check(
        &[
            (P, 0.9, [0, 0, 99, 9]),
            (P, 0.9, [9, 0, 108, 9]),
            (P, 0.9, [18, 0, 117, 9]),
        ],
        &[2],
    );
    check(&[(P, 0.9, [0, 0, 99, 9]), (P, 0.9, [0, 0, 99, 10])], &[1]);
    check(
        &[(P, 0.9, [0, 0, 50, 50]), (P, 0.9, [20, 20, 19, 25])],
        &[1, 0],
    );
    check(&[(P, 0.9, [0, 0, 9, 9]), (P, 0.9, [0, 0, 19, 9])], &[1]);
    check(&[(P, 0.9, [0, 0, 19, 9]), (P, 0.9, [0, 0, 9, 9])], &[0]);
    check(&[(P, 0.9, [0, 0, 9, 9]), (P, 0.9, [6, 0, 25, 9])], &[0, 1]);
}

#[test]
fn zero_area_comparisons_are_skipped() {
    check(
        &[(P, 0.9, [5, 5, 5, 10]), (P, 0.9, [20, 20, 19, 20])],
        &[0, 1],
    );
    check(
        &[(P, 0.9, [0, 0, 1, 1]), (P, 0.9, [20, 20, 19, 19])],
        &[0, 1],
    );
}

#[test]
fn area_and_intersection_rows() {
    assert_eq!(inclusive_area([0, 0, 9, 9]), 100);
    assert_eq!(inclusive_area([20, 20, 19, 20]), 0);
    assert_eq!(inclusive_area([30, 30, 20, 20]), 81);
    assert_eq!(inclusive_area([0, 0, -3, 5]), -12);
    assert_eq!(inclusive_area([5, 5, 5, 5]), 1);
    assert_eq!(
        inclusive_area([i64::MIN, 0, i64::MAX, 0]),
        9_223_372_036_854_775_807
    );
    assert_eq!(
        inclusive_area([0, 0, -2_000_000_000, 2_000_000_000]),
        -3_999_999_999_999_999_999
    );
    assert_eq!(
        intersection([0, 0, 9, 9], [9, 9, 50, 50]),
        Some([9, 9, 9, 9])
    );
    assert_eq!(intersection([0, 0, 9, 9], [10, 0, 20, 9]), None);
    assert_eq!(intersection([0, 0, 9, 9], [0, 10, 9, 20]), None);
    assert_eq!(
        intersection([0, 0, 9, 9], [2, 3, 5, 20]),
        Some([2, 3, 5, 9])
    );
    assert_eq!(intersection([9, 9, 0, 0], [0, 0, 5, 5]), None);
    assert_eq!(
        intersection([0, 0, 9, 9], [-5, -5, 2, 2]),
        Some([0, 0, 2, 2])
    );
    assert_eq!(
        intersection([i64::MIN, i64::MIN, i64::MAX, i64::MAX], [0, 0, 5, 5]),
        Some([0, 0, 5, 5])
    );
}

#[test]
fn clipped_rows() {
    let frame = Frame::new(704, 576).expect("the hand frame");
    let r420 = [100, 100, 420, 420];
    let rows: &[([i64; 4], [i64; 4], bool)] = &[
        ([105, 110, 150, 150], r420, true),
        ([106, 110, 150, 150], r420, false),
        ([110, 105, 150, 150], r420, true),
        ([110, 106, 150, 150], r420, false),
        ([110, 110, 415, 150], r420, true),
        ([110, 110, 414, 150], r420, false),
        ([110, 110, 150, 415], r420, true),
        ([110, 110, 150, 414], r420, false),
        ([10, 110, 150, 150], [5, 100, 325, 420], false),
        ([11, 110, 150, 150], [6, 100, 326, 420], true),
        ([400, 110, 699, 150], [384, 100, 704, 420], false),
        ([400, 110, 693, 150], [384, 100, 698, 420], true),
        ([400, 110, 694, 150], [384, 100, 699, 420], false),
        ([110, 160, 150, 565], [100, 150, 420, 570], true),
        ([110, 160, 150, 566], [100, 150, 420, 571], false),
        ([110, 160, 150, 564], [100, 150, 420, 570], false),
    ];
    for (box_, region, want) in rows {
        let detection = det_region(P, 0.9, *box_, *region);
        assert_eq!(clipped(&detection, frame), *want);
    }
}

#[test]
fn detection_limits() {
    let region = Region::new(0, 0, 320, 320).expect("the hand region");
    let mut edges = [0_i64, 0, 10, 10];
    for value in [16_777_216_i64, -16_777_216] {
        for slot in [0usize, 1, 2, 3] {
            edges[slot] = value;
            let geometry = Geometry {
                x_min: edges[0],
                y_min: edges[1],
                x_max: edges[2],
                y_max: edges[3],
                area: 100,
                ratio: 1.0,
            };
            let detection =
                Detection::new("person", 0.5, geometry, region).expect("an allowed detection");
            assert_eq!(detection.label(), "person");
            assert_eq!(detection.score().to_bits(), 1_056_964_608);
            assert_eq!(detection.geometry(), geometry);
            assert_eq!(detection.region(), region);
            edges[slot] = 0;
        }
    }
    for value in [16_777_217_i64, -16_777_217] {
        for slot in [0usize, 1, 2, 3] {
            edges[slot] = value;
            let geometry = Geometry {
                x_min: edges[0],
                y_min: edges[1],
                x_max: edges[2],
                y_max: edges[3],
                area: 100,
                ratio: 1.0,
            };
            assert_eq!(Detection::new("person", 0.5, geometry, region), None);
            edges[slot] = 0;
        }
    }
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

fn count(counts: &mut BTreeMap<&'static str, usize>, name: &'static str) {
    *counts.entry(name).or_insert(0) += 1;
}

fn has_limit_edge(dets: &[Detection], expected: &[usize]) -> bool {
    expected.iter().enumerate().any(|(a, left)| {
        expected[a + 1..].iter().any(|right| {
            dets[*left].label() == dets[*right].label()
                && intersection(
                    box_edges(dets[*left].geometry()),
                    box_edges(dets[*right].geometry()),
                )
                .is_some_and(|inter| inclusive_area(inter) > 0)
        })
    })
}

fn check_merge_line(
    tokens: &[&str],
    arrow: usize,
    line_no: usize,
    line: &str,
    counts: &mut BTreeMap<&'static str, usize>,
) {
    count(counts, "merge");
    assert_eq!(tokens[2], "frame", "line {line_no}: {line}");
    let width: i64 = tokens[3].parse().expect("a width");
    let height: i64 = tokens[4].parse().expect("a height");
    assert!(
        Frame::new(width, height).is_some(),
        "line {line_no}: {line}"
    );
    assert_eq!(tokens[5], "dets", "line {line_no}: {line}");
    let dets: Vec<Detection> = tokens[6..arrow]
        .iter()
        .map(|token| parse_det(token, line_no))
        .collect();
    let expected: Vec<usize> = if arrow + 1 == tokens.len() && tokens[arrow + 1] == "-" {
        Vec::new()
    } else {
        tokens[arrow + 1..]
            .iter()
            .map(|token| token.parse().expect("a survivor index"))
            .collect()
    };
    let got = consolidate(dets.clone());
    assert_eq!(
        survivor_ids(&dets, &got),
        expected,
        "line {line_no}: {line}"
    );
    if expected.len() < dets.len() {
        count(counts, "dropped");
    }
    let mut sorted = expected.clone();
    sorted.sort_unstable();
    if expected != sorted {
        count(counts, "reordered");
    }
    if has_limit_edge(&dets, &expected) {
        count(counts, "limit_edge");
    }
}

fn check_clipped_line(
    tokens: &[&str],
    arrow: usize,
    line_no: usize,
    line: &str,
    counts: &mut BTreeMap<&'static str, usize>,
) {
    count(counts, "clipped");
    assert_eq!(tokens[2], "frame", "line {line_no}: {line}");
    let width: i64 = tokens[3].parse().expect("a width");
    let height: i64 = tokens[4].parse().expect("a height");
    assert_eq!(tokens[5], "det", "line {line_no}: {line}");
    let detection = parse_det(tokens[6], line_no);
    let frame = Frame::new(width, height).expect("a frame");
    let want = match tokens[arrow + 1] {
        "true" => true,
        "false" => false,
        _ => panic!("line {line_no}: {line}"),
    };
    assert_eq!(clipped(&detection, frame), want, "line {line_no}: {line}");
    count(
        counts,
        if want {
            "clipped_true"
        } else {
            "clipped_false"
        },
    );
}

fn check_area_line(
    tokens: &[&str],
    arrow: usize,
    line_no: usize,
    line: &str,
    counts: &mut BTreeMap<&'static str, usize>,
) {
    count(counts, "area");
    assert_eq!(tokens[2], "box", "line {line_no}: {line}");
    let edges = parse_edges(tokens[3], line_no);
    let want: i64 = tokens[arrow + 1].parse().expect("an area");
    assert_eq!(inclusive_area(edges), want, "line {line_no}: {line}");
    if want < 0 {
        count(counts, "area_negative");
    }
    if want == 0 {
        count(counts, "area_zero");
    }
}

fn check_intersection_line(
    tokens: &[&str],
    arrow: usize,
    line_no: usize,
    line: &str,
    counts: &mut BTreeMap<&'static str, usize>,
) {
    count(counts, "intersection");
    assert_eq!(tokens[2], "a", "line {line_no}: {line}");
    assert_eq!(tokens[4], "b", "line {line_no}: {line}");
    let a = parse_edges(tokens[3], line_no);
    let b = parse_edges(tokens[5], line_no);
    let got = intersection(a, b);
    if tokens[arrow + 1] == "none" {
        assert_eq!(got, None, "line {line_no}: {line}");
        count(counts, "intersection_none");
    } else {
        let want = parse_edges(tokens[arrow + 1], line_no);
        assert_eq!(got, Some(want), "line {line_no}: {line}");
        count(counts, "intersection_some");
        if inclusive_area(want) == 1 {
            count(counts, "intersection_touch");
        }
    }
}

fn assert_fixture_counts(header: &BTreeMap<&str, usize>, counts: &BTreeMap<&'static str, usize>) {
    for (name, want) in [
        ("cases", 4000),
        ("merge", 2800),
        ("clipped", 600),
        ("area", 300),
        ("intersection", 300),
        ("dropped", 1110),
        ("reordered", 903),
        ("limit_edge", 1189),
        ("clipped_true", 323),
        ("clipped_false", 277),
        ("intersection_none", 162),
        ("intersection_some", 138),
        ("intersection_touch", 40),
        ("area_negative", 168),
        ("area_zero", 63),
    ] {
        assert_eq!(header[name], want, "header {name}");
    }
    for name in [
        "merge",
        "clipped",
        "area",
        "intersection",
        "dropped",
        "reordered",
        "limit_edge",
        "clipped_true",
        "clipped_false",
        "intersection_none",
        "intersection_some",
        "intersection_touch",
        "area_negative",
        "area_zero",
    ] {
        assert_eq!(counts[name], header[name], "parsed {name}");
    }
    for (name, floor) in [
        ("dropped", 1000),
        ("reordered", 300),
        ("limit_edge", 200),
        ("shift_keep_person", 100),
        ("shift_drop_person", 100),
        ("shift_keep_car", 100),
        ("shift_drop_car", 100),
        ("shift_keep_face", 100),
        ("shift_drop_face", 100),
        ("ratio_at", 150),
        ("ratio_below", 150),
        ("zerodiv_excluded", 40),
        ("clipped_true", 150),
        ("clipped_false", 150),
        ("intersection_none", 100),
        ("intersection_some", 100),
        ("intersection_touch", 30),
        ("area_negative", 50),
        ("area_zero", 20),
    ] {
        assert!(header[name] >= floor, "{name} floor");
    }
}

#[test]
fn fixture_matches_frigate_on_every_case() {
    let mut header: BTreeMap<&str, usize> = BTreeMap::new();
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
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
        let arrow = tokens
            .iter()
            .position(|token| *token == "=>")
            .unwrap_or_else(|| panic!("line {line_no}: no arrow"));
        match tokens.first().copied().expect("a kind") {
            "merge" => check_merge_line(&tokens, arrow, line_no, line, &mut counts),
            "clipped" => check_clipped_line(&tokens, arrow, line_no, line, &mut counts),
            "area" => check_area_line(&tokens, arrow, line_no, line, &mut counts),
            "intersection" => check_intersection_line(&tokens, arrow, line_no, line, &mut counts),
            other => panic!("line {line_no}: unknown kind {other}"),
        }
    }
    assert_fixture_counts(&header, &counts);
}
