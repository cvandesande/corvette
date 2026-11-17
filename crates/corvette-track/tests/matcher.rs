use std::cmp::Ordering;
use std::collections::BTreeMap;

use corvette_track::{Matching, Matrix, NanDistance, match_greedy};
use corvette_yolo::round_f64_to_f32;

const FIXTURE: &str = include_str!("fixtures/match_cases.txt");

fn hex64(text: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(text, 16).expect("a float64 bit pattern"))
}

fn hex32(text: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(text, 16).expect("a float32 bit pattern"))
}

fn indices(parts: &[&str]) -> Vec<usize> {
    if parts.first().copied() == Some("-") {
        Vec::new()
    } else {
        parts
            .iter()
            .map(|part| part.parse().expect("an index"))
            .collect()
    }
}

struct ExpectPair {
    candidate: usize,
    object: usize,
    distance_bits: u32,
}

struct MatchCase {
    threshold: f64,
    rows: usize,
    cols: usize,
    cells: Vec<f32>,
    pairs: Vec<ExpectPair>,
    unmatched_candidates: Vec<usize>,
    unmatched_objects: Vec<usize>,
    is_error: bool,
    source: String,
}

struct Parsed {
    cases: Vec<MatchCase>,
    counts: BTreeMap<String, usize>,
    floors: BTreeMap<String, usize>,
}

fn first_nan_cell(cells: &[f32], cols: usize) -> Option<(usize, usize)> {
    cells
        .iter()
        .position(|cell| cell.is_nan())
        .map(|index| (index / cols, index % cols))
}

fn actual_pairs(matching: &Matching) -> Vec<(usize, usize, u32)> {
    matching
        .pairs
        .iter()
        .map(|pair| (pair.candidate, pair.object, pair.distance.to_bits()))
        .collect()
}

fn parse_header(parsed: &mut Parsed, parts: &[&str]) {
    match parts.get(1).copied() {
        Some("counts" | "floors") => {}
        _ => return,
    }
    for pair in parts[2..].chunks(2) {
        let value = pair[1].parse().expect("a count or floor");
        let _inserted = if parts[1] == "counts" {
            parsed.counts.insert(pair[0].to_owned(), value)
        } else {
            parsed.floors.insert(pair[0].to_owned(), value)
        };
    }
}

fn parse_expect_pair(text: &str) -> ExpectPair {
    let mut fields = text.split(',');
    ExpectPair {
        candidate: fields
            .next()
            .and_then(|value| value.parse().ok())
            .expect("a candidate index"),
        object: fields
            .next()
            .and_then(|value| value.parse().ok())
            .expect("an object index"),
        distance_bits: u32::from_str_radix(fields.next().expect("a distance"), 16)
            .expect("a distance bit pattern"),
    }
}

fn parse_result(
    parts: &[&str],
    arrow: usize,
    number: usize,
    line: &str,
) -> (Vec<ExpectPair>, Vec<usize>, Vec<usize>) {
    let semis: Vec<usize> = parts
        .iter()
        .enumerate()
        .filter(|(_, part)| **part == ";")
        .map(|(position, _)| position)
        .collect();
    assert_eq!(semis.len(), 2, "line {number}: {line}");
    let pairs = if parts[arrow + 1] == "-" {
        Vec::new()
    } else {
        parts[arrow + 1..semis[0]]
            .iter()
            .map(|text| parse_expect_pair(text))
            .collect()
    };
    (
        pairs,
        indices(&parts[semis[0] + 1..semis[1]]),
        indices(&parts[semis[1] + 1..]),
    )
}

fn parse_case(number: usize, line: &str) -> MatchCase {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let kind = parts.first().copied().unwrap_or_default();
    assert!(
        kind == "match" || kind == "matcherr",
        "line {number}: unknown case kind: {line}"
    );
    assert_eq!(parts[2], "thr", "line {number}: {line}");
    assert_eq!(parts[4], "table", "line {number}: {line}");
    let threshold = hex64(parts[3]);
    let (rows, cols) = {
        let (r, c) = parts[5].split_once('x').expect("a rows x cols shape");
        (r.parse().expect("rows"), c.parse().expect("cols"))
    };
    let arrow = parts
        .iter()
        .position(|part| *part == "=>")
        .expect("an arrow");
    let is_error = parts[arrow + 1] == "error";
    assert_eq!(is_error, kind == "matcherr", "line {number}: {line}");
    let cells: Vec<f32> = if parts[6] == "-" {
        Vec::new()
    } else {
        parts[6..arrow].iter().map(|text| hex32(text)).collect()
    };
    assert_eq!(cells.len(), rows * cols, "line {number}: {line}");
    let (pairs, unmatched_candidates, unmatched_objects) = if is_error {
        (Vec::new(), Vec::new(), Vec::new())
    } else {
        parse_result(&parts, arrow, number, line)
    };
    MatchCase {
        threshold,
        rows,
        cols,
        cells,
        pairs,
        unmatched_candidates,
        unmatched_objects,
        is_error,
        source: line.to_owned(),
    }
}

fn parse_fixture() -> Parsed {
    let mut parsed = Parsed {
        cases: Vec::new(),
        counts: BTreeMap::new(),
        floors: BTreeMap::new(),
    };
    for (index, line) in FIXTURE.lines().enumerate() {
        if line.starts_with('#') {
            let parts: Vec<&str> = line.split_whitespace().collect();
            parse_header(&mut parsed, &parts);
            continue;
        }
        parsed.cases.push(parse_case(index + 1, line));
    }
    parsed
}

fn case_cell_at_threshold(case: &MatchCase) -> bool {
    case.threshold.is_finite()
        && case
            .cells
            .iter()
            .any(|cell| cell.to_bits() == round_f64_to_f32(case.threshold).to_bits())
}

fn case_first_pick_tie(case: &MatchCase) -> bool {
    let limit = round_f64_to_f32(case.threshold);
    let mut least: Option<(f32, usize)> = None;
    for cell in &case.cells {
        if *cell < limit {
            least = Some(match least {
                None => (*cell, 1),
                Some((lowest, count)) => match cell.partial_cmp(&lowest) {
                    Some(Ordering::Less) => (*cell, 1),
                    Some(Ordering::Equal) => (lowest, count + 1),
                    _ => (lowest, count),
                },
            });
        }
    }
    least.is_some_and(|(_, count)| count >= 2)
}

fn case_threshold_edge(case: &MatchCase) -> bool {
    if !case.threshold.is_finite() {
        return false;
    }
    let limit = round_f64_to_f32(case.threshold);
    if f64::from(limit).partial_cmp(&case.threshold) == Some(Ordering::Equal) {
        return false;
    }
    let edge_bits = [
        limit.to_bits(),
        limit.next_down().to_bits(),
        limit.next_up().to_bits(),
    ];
    case.cells
        .iter()
        .any(|cell| edge_bits.contains(&cell.to_bits()))
}

fn case_signed_zero(case: &MatchCase) -> bool {
    case.cells.iter().any(|cell| cell.to_bits() == 0x0000_0000)
        && case.cells.iter().any(|cell| cell.to_bits() == 0x8000_0000)
}

fn stat(parsed: &Parsed, name: &str) -> usize {
    let cases = parsed.cases.iter().filter(|case| !case.is_error);
    match name {
        "match" => parsed.cases.iter().filter(|case| !case.is_error).count(),
        "matcherr" => parsed.cases.iter().filter(|case| case.is_error).count(),
        "empty_side" => cases
            .filter(|case| case.rows == 0 || case.cols == 0)
            .count(),
        "pairs3" => cases.filter(|case| case.pairs.len() >= 3).count(),
        "no_pair" => cases
            .filter(|case| case.rows > 0 && case.cols > 0 && case.pairs.is_empty())
            .count(),
        "both_unmatched" => cases
            .filter(|case| {
                !case.unmatched_candidates.is_empty() && !case.unmatched_objects.is_empty()
            })
            .count(),
        "cell_at_threshold" => cases.filter(|case| case_cell_at_threshold(case)).count(),
        "first_pick_tie" => cases.filter(|case| case_first_pick_tie(case)).count(),
        "threshold_edge" => cases.filter(|case| case_threshold_edge(case)).count(),
        "nan_threshold" => cases.filter(|case| case.threshold.is_nan()).count(),
        "inf_threshold" => cases.filter(|case| case.threshold == f64::INFINITY).count(),
        "ninf_threshold" => cases
            .filter(|case| case.threshold == f64::NEG_INFINITY)
            .count(),
        "zero_threshold" => cases.filter(|case| case.threshold == 0.0).count(),
        "negative_threshold" => cases
            .filter(|case| case.threshold.is_finite() && case.threshold < 0.0)
            .count(),
        "huge_threshold" => cases
            .filter(|case| {
                case.threshold.is_finite() && case.threshold.abs() >= 3.402_823_567_797_336_6e38
            })
            .count(),
        "signed_zero" => cases.filter(|case| case_signed_zero(case)).count(),
        "inf_cells" => cases
            .map(|case| {
                case.cells
                    .iter()
                    .filter(|cell| cell.to_bits() == 0x7f80_0000)
                    .count()
            })
            .sum(),
        "ninf_cases" => cases
            .filter(|case| case.cells.iter().any(|cell| cell.to_bits() == 0xff80_0000))
            .count(),
        "single_nan" => parsed
            .cases
            .iter()
            .filter(|case| {
                case.is_error && case.cells.iter().filter(|cell| cell.is_nan()).count() == 1
            })
            .count(),
        "odd_nan_bits" => parsed
            .cases
            .iter()
            .filter(|case| {
                case.is_error
                    && case
                        .cells
                        .iter()
                        .any(|cell| cell.is_nan() && cell.to_bits() != 0x7fc0_0000)
            })
            .count(),
        other => panic!("no statistic named {other}"),
    }
}

struct HandRow {
    name: &'static str,
    rows: usize,
    cols: usize,
    cells: &'static str,
    threshold: f64,
    pairs: &'static [(usize, usize, &'static str)],
    unmatched_candidates: &'static [usize],
    unmatched_objects: &'static [usize],
}

fn check_hand_rows(hand: &[HandRow]) {
    for row in hand {
        let table = Matrix::new(
            row.rows,
            row.cols,
            row.cells.split_whitespace().map(hex32).collect(),
        )
        .expect("a hand table");
        let matching =
            match_greedy(&table, row.threshold).expect("a matching for a table without NaN");
        let want: Vec<(usize, usize, u32)> = row
            .pairs
            .iter()
            .map(|(candidate, object, bits)| {
                (
                    *candidate,
                    *object,
                    u32::from_str_radix(bits, 16).expect("hand distance bits"),
                )
            })
            .collect();
        assert_eq!(
            actual_pairs(&matching),
            want,
            "hand row {}: pairs differ",
            row.name
        );
        assert_eq!(
            matching.unmatched_candidates, row.unmatched_candidates,
            "hand row {}: unmatched candidates differ",
            row.name
        );
        assert_eq!(
            matching.unmatched_objects, row.unmatched_objects,
            "hand row {}: unmatched objects differ",
            row.name
        );
    }
}

const HAND_FIRST: &[HandRow] = &[
    HandRow {
        name: "label-gated-three-by-three",
        rows: 3,
        cols: 3,
        cells: "3dc47194 7f800000 401afb89 7f800000 00000000 7f800000 400f9fe4 7f800000 00000000",
        threshold: 2.5,
        pairs: &[(1, 1, "00000000"), (2, 2, "00000000"), (0, 0, "3dc47194")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "all-equal-two-by-two",
        rows: 2,
        cols: 2,
        cells: "3f800000 3f800000 3f800000 3f800000",
        threshold: 2.5,
        pairs: &[(0, 0, "3f800000"), (1, 1, "3f800000")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "cell-equal-to-threshold",
        rows: 1,
        cols: 1,
        cells: "40200000",
        threshold: 2.5,
        pairs: &[],
        unmatched_candidates: &[0],
        unmatched_objects: &[0],
    },
    HandRow {
        name: "greedy-differs-from-optimal",
        rows: 2,
        cols: 2,
        cells: "3f800000 3fc00000 3f8ccccd 42c80000",
        threshold: 2.5,
        pairs: &[(0, 0, "3f800000")],
        unmatched_candidates: &[1],
        unmatched_objects: &[1],
    },
    HandRow {
        name: "negative-and-negative-inf",
        rows: 2,
        cols: 2,
        cells: "ff800000 bf800000 00000000 80000000",
        threshold: 2.5,
        pairs: &[(0, 0, "ff800000"), (1, 1, "80000000")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "all-infinite",
        rows: 2,
        cols: 3,
        cells: "7f800000 7f800000 7f800000 7f800000 7f800000 7f800000",
        threshold: 2.5,
        pairs: &[],
        unmatched_candidates: &[0, 1],
        unmatched_objects: &[0, 1, 2],
    },
    HandRow {
        name: "cells-above-threshold",
        rows: 2,
        cols: 2,
        cells: "40300000 40500000 40500000 40300000",
        threshold: 2.5,
        pairs: &[],
        unmatched_candidates: &[0, 1],
        unmatched_objects: &[0, 1],
    },
    HandRow {
        name: "threshold-zero",
        rows: 2,
        cols: 2,
        cells: "00000000 80000000 3f800000 3f800000",
        threshold: 0.0,
        pairs: &[],
        unmatched_candidates: &[0, 1],
        unmatched_objects: &[0, 1],
    },
    HandRow {
        name: "negative-threshold",
        rows: 2,
        cols: 2,
        cells: "bf800000 c0000000 3f800000 3f800000",
        threshold: -1.5,
        pairs: &[(0, 1, "c0000000")],
        unmatched_candidates: &[1],
        unmatched_objects: &[0],
    },
    HandRow {
        name: "infinite-threshold",
        rows: 2,
        cols: 2,
        cells: "3f800000 7f800000 40000000 40400000",
        threshold: f64::INFINITY,
        pairs: &[(0, 0, "3f800000"), (1, 1, "40400000")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "ties-go-to-the-first-cell",
        rows: 3,
        cols: 4,
        cells: "3f000000 3f000000 3f000000 3f000000 3f000000 3f000000 3f000000 3f000000 3f000000 3f000000 3f000000 3f000000",
        threshold: 2.5,
        pairs: &[(0, 0, "3f000000"), (1, 1, "3f000000"), (2, 2, "3f000000")],
        unmatched_candidates: &[],
        unmatched_objects: &[3],
    },
    HandRow {
        name: "signed-zero-ties",
        rows: 2,
        cols: 2,
        cells: "80000000 00000000 00000000 80000000",
        threshold: 1.0,
        pairs: &[(0, 0, "80000000"), (1, 1, "80000000")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "nan-threshold",
        rows: 2,
        cols: 2,
        cells: "3f800000 3f800000 3f800000 3f800000",
        threshold: f64::NAN,
        pairs: &[],
        unmatched_candidates: &[0, 1],
        unmatched_objects: &[0, 1],
    },
    HandRow {
        name: "float32-maximum-threshold",
        rows: 2,
        cols: 2,
        cells: "3f800000 40000000 40400000 40800000",
        threshold: 3.402_823_466_385_288_6e38,
        pairs: &[(0, 0, "3f800000"), (1, 1, "40800000")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "cell-just-below-threshold",
        rows: 2,
        cols: 2,
        cells: "401fffff 40600000 40600000 401fffff",
        threshold: 2.5,
        pairs: &[(0, 0, "401fffff"), (1, 1, "401fffff")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "single-negative-infinity",
        rows: 1,
        cols: 1,
        cells: "ff800000",
        threshold: 2.5,
        pairs: &[(0, 0, "ff800000")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "one-row-wide",
        rows: 1,
        cols: 5,
        cells: "40400000 40000000 3f800000 3f000000 3f000000",
        threshold: 2.5,
        pairs: &[(0, 3, "3f000000")],
        unmatched_candidates: &[],
        unmatched_objects: &[0, 1, 2, 4],
    },
    HandRow {
        name: "one-column-tall",
        rows: 5,
        cols: 1,
        cells: "40400000 40000000 3f800000 3f000000 3f000000",
        threshold: 2.5,
        pairs: &[(3, 0, "3f000000")],
        unmatched_candidates: &[0, 1, 2, 4],
        unmatched_objects: &[],
    },
    HandRow {
        name: "order-by-value-not-by-row",
        rows: 2,
        cols: 2,
        cells: "40a00000 3f800000 40000000 41000000",
        threshold: 10.0,
        pairs: &[(0, 1, "3f800000"), (1, 0, "40000000")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "negative-infinite-threshold",
        rows: 2,
        cols: 2,
        cells: "ff800000 3f800000 3f800000 ff800000",
        threshold: f64::NEG_INFINITY,
        pairs: &[],
        unmatched_candidates: &[0, 1],
        unmatched_objects: &[0, 1],
    },
    HandRow {
        name: "all-negative-infinity",
        rows: 2,
        cols: 2,
        cells: "ff800000 ff800000 ff800000 ff800000",
        threshold: f64::INFINITY,
        pairs: &[(0, 0, "ff800000"), (1, 1, "ff800000")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "huge-cells-huge-threshold",
        rows: 2,
        cols: 2,
        cells: "7149f2ca 71c9f2ca 72177617 71977617",
        threshold: 1e31,
        pairs: &[(0, 0, "7149f2ca"), (1, 1, "71977617")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "tie-order-two-by-two",
        rows: 2,
        cols: 2,
        cells: "40a00000 3f000000 3f000000 40a00000",
        threshold: 2.5,
        pairs: &[(0, 1, "3f000000"), (1, 0, "3f000000")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "tie-order-three-by-three",
        rows: 3,
        cols: 3,
        cells: "41100000 41100000 3e800000 3e800000 41100000 41100000 41100000 3e800000 41100000",
        threshold: 2.5,
        pairs: &[(0, 2, "3e800000"), (1, 0, "3e800000"), (2, 1, "3e800000")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
];

const HAND_FLOAT32: &[HandRow] = &[
    HandRow {
        name: "cell-two-point-five-above-threshold",
        rows: 1,
        cols: 1,
        cells: "40200000",
        threshold: 2.500_000_000_1,
        pairs: &[],
        unmatched_candidates: &[0],
        unmatched_objects: &[0],
    },
    HandRow {
        name: "cell-two-point-five-below-threshold",
        rows: 1,
        cols: 1,
        cells: "40200000",
        threshold: 2.499_999_999_9,
        pairs: &[],
        unmatched_candidates: &[0],
        unmatched_objects: &[0],
    },
    HandRow {
        name: "float32-tenth-above-threshold",
        rows: 1,
        cols: 1,
        cells: "3dcccccd",
        threshold: 0.100_000_002,
        pairs: &[],
        unmatched_candidates: &[0],
        unmatched_objects: &[0],
    },
    HandRow {
        name: "float32-tenth-offset-threshold",
        rows: 1,
        cols: 1,
        cells: "3dcccccd",
        threshold: 0.100_000_001_490_116_12 + 1e-12,
        pairs: &[],
        unmatched_candidates: &[0],
        unmatched_objects: &[0],
    },
    HandRow {
        name: "float32-tenth-exact-threshold",
        rows: 1,
        cols: 1,
        cells: "3dcccccd",
        threshold: 0.100_000_001_490_116_12,
        pairs: &[],
        unmatched_candidates: &[0],
        unmatched_objects: &[0],
    },
    HandRow {
        name: "float32-maximum-below-giant-threshold",
        rows: 1,
        cols: 1,
        cells: "7f7fffff",
        threshold: 1e300,
        pairs: &[(0, 0, "7f7fffff")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "infinite-cell-giant-threshold",
        rows: 1,
        cols: 1,
        cells: "7f800000",
        threshold: 1e300,
        pairs: &[],
        unmatched_candidates: &[0],
        unmatched_objects: &[0],
    },
    HandRow {
        name: "hundred-million-cells",
        rows: 2,
        cols: 2,
        cells: "4cbebc20 4d3ebc20 4d8f0d18 4dbebc20",
        threshold: 1e9,
        pairs: &[(0, 0, "4cbebc20"), (1, 1, "4dbebc20")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
    HandRow {
        name: "near-thirty-million-cells",
        rows: 2,
        cols: 2,
        cells: "3f800000 4be4e1c0 4be4e1c0 40000000",
        threshold: 3.0e7,
        pairs: &[(0, 0, "3f800000"), (1, 1, "40000000")],
        unmatched_candidates: &[],
        unmatched_objects: &[],
    },
];

#[test]
fn hand_tables_match_the_real_matcher() {
    check_hand_rows(HAND_FIRST);
}

#[test]
fn threshold_is_compared_as_float32() {
    check_hand_rows(HAND_FLOAT32);
}

#[test]
fn nan_cell_is_a_typed_error() {
    let table = Matrix::new(
        2,
        2,
        vec![
            hex32("3f800000"),
            f32::from_bits(0x7fc0_0000),
            hex32("3f800000"),
            hex32("3f800000"),
        ],
    )
    .expect("a table");
    let error = match_greedy(&table, 2.5).expect_err("a typed error for the first table");
    assert_eq!(
        error,
        NanDistance {
            candidate: 0,
            object: 1
        }
    );
    let table = Matrix::new(
        2,
        3,
        vec![
            hex32("3f800000"),
            hex32("3f800000"),
            f32::NAN,
            f32::NAN,
            hex32("3f800000"),
            hex32("3f800000"),
        ],
    )
    .expect("a table");
    assert_eq!(
        match_greedy(&table, 2.5).expect_err("a typed error for the second table"),
        NanDistance {
            candidate: 0,
            object: 2
        }
    );
    for bits in [
        0x7fc0_0000_u32,
        0xffc0_0000,
        0x7fc0_0001,
        0x7f80_0001,
        0xff80_0001,
    ] {
        let table = Matrix::new(1, 1, vec![f32::from_bits(bits)]).expect("a table");
        assert_eq!(
            match_greedy(&table, 2.5).expect_err("a typed error for a fixture NaN bit pattern"),
            NanDistance {
                candidate: 0,
                object: 0
            }
        );
    }
    let table = Matrix::new(1, 1, vec![f32::NAN]).expect("a table");
    assert_eq!(
        match_greedy(&table, f64::NAN).expect_err("a NaN threshold does not hide a NaN cell"),
        NanDistance {
            candidate: 0,
            object: 0
        }
    );
    assert_eq!(
        NanDistance {
            candidate: 0,
            object: 1
        }
        .to_string(),
        "the distance table has a NaN at candidate 0, object 1"
    );
    let error = NanDistance {
        candidate: 0,
        object: 1,
    };
    let as_error: &dyn std::error::Error = &error;
    assert_eq!(
        as_error.to_string(),
        "the distance table has a NaN at candidate 0, object 1"
    );
    let table = Matrix::new(1, 1, vec![hex32("3f800000")]).expect("a table");
    assert!(match_greedy(&table, 2.5).is_ok());
}

#[test]
fn empty_shapes_leave_everything_unmatched() {
    for (rows, cols) in [(0_usize, 3_usize), (3, 0), (0, 0)] {
        let table = Matrix::new(rows, cols, Vec::new()).expect("an empty table");
        let before = table.clone();
        let matching = match_greedy(&table, 2.5).expect("no NaN in an empty table");
        assert!(
            matching.pairs.is_empty(),
            "shape {rows} by {cols} made pairs"
        );
        assert_eq!(
            matching.unmatched_candidates,
            (0..rows).collect::<Vec<usize>>(),
            "shape {rows} by {cols} candidates"
        );
        assert_eq!(
            matching.unmatched_objects,
            (0..cols).collect::<Vec<usize>>(),
            "shape {rows} by {cols} objects"
        );
        assert_eq!(table, before, "shape {rows} by {cols} changed the table");
    }
}

#[test]
fn extreme_input_never_panics() {
    let thresholds = [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        0.0,
        -0.0,
    ];
    for (rows, cols) in [(1_usize, 1_usize), (1, 64), (64, 1), (200, 200)] {
        for threshold in thresholds {
            let table =
                Matrix::new(rows, cols, vec![hex32("3f800000"); rows * cols]).expect("a table");
            let _ = match_greedy(&table, threshold);
        }
    }
    let weird = [
        f32::MAX,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::MIN_POSITIVE,
        0.0,
        -0.0,
    ];
    let cells: Vec<f32> = (0..64).map(|index| weird[index % weird.len()]).collect();
    let table = Matrix::new(8, 8, cells).expect("a table");
    for threshold in thresholds {
        let _ = match_greedy(&table, threshold);
    }
    let table = Matrix::new(200, 200, vec![hex32("3f800000"); 200 * 200]).expect("a table");
    let matching = match_greedy(&table, 1e9).expect("no NaN");
    assert_eq!(
        actual_pairs(&matching),
        (0..200_usize)
            .map(|index| (index, index, hex32("3f800000").to_bits()))
            .collect::<Vec<(usize, usize, u32)>>()
    );
}

#[test]
fn fixture_matches_the_real_matcher() {
    let parsed = parse_fixture();
    assert_eq!(
        parsed.counts.get("match").copied(),
        Some(stat(&parsed, "match")),
        "header match count"
    );
    assert_eq!(
        parsed.counts.get("matcherr").copied(),
        Some(stat(&parsed, "matcherr")),
        "header matcherr count"
    );
    for (name, floor) in &parsed.floors {
        let value = stat(&parsed, name);
        assert!(
            value >= *floor,
            "floor {name}: recomputed {value} < {floor}"
        );
        assert_eq!(
            parsed.counts.get(name).copied(),
            Some(value),
            "header count {name}"
        );
    }
    for case in &parsed.cases {
        let table = Matrix::new(case.rows, case.cols, case.cells.clone())
            .expect("a matrix for the case shape");
        match match_greedy(&table, case.threshold) {
            Ok(matching) => {
                assert!(!case.is_error, "expected the typed error: {}", case.source);
                let want: Vec<(usize, usize, u32)> = case
                    .pairs
                    .iter()
                    .map(|pair| (pair.candidate, pair.object, pair.distance_bits))
                    .collect();
                assert_eq!(
                    actual_pairs(&matching),
                    want,
                    "pairs disagree: {}",
                    case.source
                );
                assert_eq!(
                    matching.unmatched_candidates, case.unmatched_candidates,
                    "unmatched candidates disagree: {}",
                    case.source
                );
                assert_eq!(
                    matching.unmatched_objects, case.unmatched_objects,
                    "unmatched objects disagree: {}",
                    case.source
                );
            }
            Err(error) => {
                assert!(
                    case.is_error,
                    "typed error for a match line, {} ({error})",
                    case.source
                );
                let first =
                    first_nan_cell(&case.cells, case.cols).expect("a NaN cell in an error case");
                assert_eq!(
                    (error.candidate, error.object),
                    first,
                    "error cell disagrees: {}",
                    case.source
                );
            }
        }
    }
}
