use std::collections::BTreeMap;

use corvette_track::{Matrix, distance, distance_matrix};
use corvette_yolo::round_f64_to_f32;

const FIXTURE: &str = include_str!("fixtures/distance_cases.txt");

fn hex64(text: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(text, 16).expect("a float64 bit pattern"))
}

fn hex32(text: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(text, 16).expect("a float32 bit pattern"))
}

fn wanted64(text: &str) -> f64 {
    if text == "nan" { f64::NAN } else { hex64(text) }
}

fn wanted32(text: &str) -> f32 {
    if text == "nan" { f32::NAN } else { hex32(text) }
}

fn box4(text: &str) -> [i32; 4] {
    let parts: Vec<i32> = text
        .split(',')
        .map(|part| part.parse().expect("a box edge"))
        .collect();
    let [x1, y1, x2, y2]: [i32; 4] = parts.try_into().expect("four box edges");
    [x1, y1, x2, y2]
}

fn est4(text: &str) -> [f64; 4] {
    let parts: Vec<f64> = text.split(',').map(hex64).collect();
    let [a, b, c, d]: [f64; 4] = parts.try_into().expect("four estimate values");
    [a, b, c, d]
}

const fn f64_matches(actual: f64, expected: f64) -> bool {
    if expected.is_nan() {
        actual.is_nan()
    } else {
        actual.to_bits() == expected.to_bits()
    }
}

const fn f32_matches(actual: f32, expected: f32) -> bool {
    if expected.is_nan() {
        actual.is_nan()
    } else {
        actual.to_bits() == expected.to_bits()
    }
}

struct DistCase {
    det: [i32; 4],
    est: [f64; 4],
    want64: f64,
    want32: f32,
    source: String,
}

struct MatrixCase {
    cands: Vec<(String, [i32; 4])>,
    objs: Vec<(String, [f64; 4])>,
    rows: usize,
    cols: usize,
    cells: Vec<f32>,
    source: String,
}

struct CastCase {
    bits: u64,
    want: f32,
    source: String,
}

struct Parsed {
    dist: Vec<DistCase>,
    matrix: Vec<MatrixCase>,
    cast: Vec<CastCase>,
    counts: BTreeMap<String, usize>,
    floors: BTreeMap<String, usize>,
}

fn pair4(text: &str) -> (String, [i32; 4]) {
    let (label, box_text) = text.split_once(':').expect("a labelled candidate");
    (label.to_owned(), box4(box_text))
}

fn pair8(text: &str) -> (String, [f64; 4]) {
    let (label, est_text) = text.split_once(':').expect("a labelled object");
    (label.to_owned(), est4(est_text))
}

fn shape(text: &str) -> (usize, usize) {
    let (rows, cols) = text.split_once('x').expect("a rows x cols shape");
    (rows.parse().expect("rows"), cols.parse().expect("cols"))
}

fn parse_fixture() -> Parsed {
    let mut parsed = Parsed {
        dist: Vec::new(),
        matrix: Vec::new(),
        cast: Vec::new(),
        counts: BTreeMap::new(),
        floors: BTreeMap::new(),
    };
    for (index, line) in FIXTURE.lines().enumerate() {
        let number = index + 1;
        if line.starts_with('#') {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == "counts" {
                for pair in parts[2..].chunks(2) {
                    let _inserted = parsed
                        .counts
                        .insert(pair[0].to_owned(), pair[1].parse().expect("a count"));
                }
            }
            if parts.len() >= 2 && parts[1] == "floors" {
                for pair in parts[2..].chunks(2) {
                    let _inserted = parsed
                        .floors
                        .insert(pair[0].to_owned(), pair[1].parse().expect("a floor"));
                }
            }
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.first().copied() {
            Some("dist") => {
                assert_eq!(parts.len(), 9, "line {number}: {line}");
                assert_eq!(parts[2], "det", "line {number}");
                assert_eq!(parts[4], "est", "line {number}");
                assert_eq!(parts[6], "=>", "line {number}");
                parsed.dist.push(DistCase {
                    det: box4(parts[3]),
                    est: est4(parts[5]),
                    want64: wanted64(parts[7]),
                    want32: wanted32(parts[8]),
                    source: line.to_owned(),
                });
            }
            Some("matrix") => {
                assert!(parts.len() >= 6, "line {number}: {line}");
                assert_eq!(parts[2], "cands", "line {number}");
                let objs_at = parts.iter().position(|part| *part == "objs").expect("objs");
                let arrow_at = parts.iter().position(|part| *part == "=>").expect("arrow");
                let cands: Vec<(String, [i32; 4])> = if parts[3] == "-" {
                    Vec::new()
                } else {
                    parts[3..objs_at].iter().map(|text| pair4(text)).collect()
                };
                let objs: Vec<(String, [f64; 4])> = if parts[objs_at + 1] == "-" {
                    Vec::new()
                } else {
                    parts[objs_at + 1..arrow_at]
                        .iter()
                        .map(|text| pair8(text))
                        .collect()
                };
                let (rows, cols) = shape(parts[arrow_at + 1]);
                assert_eq!(rows, cands.len(), "line {number}: {line}");
                assert_eq!(cols, objs.len(), "line {number}: {line}");
                let cells: Vec<f32> = parts[arrow_at + 2..]
                    .iter()
                    .map(|text| wanted32(text))
                    .collect();
                assert_eq!(cells.len(), rows * cols, "line {number}: {line}");
                parsed.matrix.push(MatrixCase {
                    cands,
                    objs,
                    rows,
                    cols,
                    cells,
                    source: line.to_owned(),
                });
            }
            Some("cast") => {
                assert_eq!(parts.len(), 5, "line {number}: {line}");
                assert_eq!(parts[3], "=>", "line {number}");
                parsed.cast.push(CastCase {
                    bits: u64::from_str_radix(parts[2], 16).expect("an input bit pattern"),
                    want: wanted32(parts[4]),
                    source: line.to_owned(),
                });
            }
            other => panic!("line {number}: unknown case {other:?}: {line}"),
        }
    }
    parsed
}

fn dist_stat(parsed: &Parsed, name: &str) -> Option<usize> {
    match name {
        "dist_nan64" => Some(parsed.dist.iter().filter(|c| c.want64.is_nan()).count()),
        "dist_inf64" => Some(
            parsed
                .dist
                .iter()
                .filter(|c| c.want64.is_infinite())
                .count(),
        ),
        "dist_zero64" => Some(
            parsed
                .dist
                .iter()
                .filter(|c| c.want64.to_bits() << 1 == 0)
                .count(),
        ),
        "dist_finite64_inf32" => Some(
            parsed
                .dist
                .iter()
                .filter(|c| c.want64.is_finite() && c.want32.is_infinite())
                .count(),
        ),
        "dist_nonfinite_est" => Some(
            parsed
                .dist
                .iter()
                .filter(|c| c.est.iter().any(|value| !value.is_finite()))
                .count(),
        ),
        "dist_bad_dim" => Some(
            parsed
                .dist
                .iter()
                .filter(|c| {
                    let estimate_width = c.est[2] - c.est[0];
                    let estimate_height = c.est[3] - c.est[1];
                    let detection_width = f64::from(c.det[2]) - f64::from(c.det[0]);
                    let detection_height = f64::from(c.det[3]) - f64::from(c.det[1]);
                    estimate_width <= 0.0
                        || estimate_height <= 0.0
                        || detection_width <= 0.0
                        || detection_height <= 0.0
                })
                .count(),
        ),
        _ => None,
    }
}

fn matrix_stat(parsed: &Parsed, name: &str) -> Option<usize> {
    match name {
        "matrix_gated_cells" => Some(
            parsed
                .matrix
                .iter()
                .map(|case| {
                    case.cands
                        .iter()
                        .map(|(label, _)| case.objs.iter().filter(|(own, _)| own != label).count())
                        .sum::<usize>()
                })
                .sum(),
        ),
        "matrix_empty_side_cases" => Some(
            parsed
                .matrix
                .iter()
                .filter(|case| case.cands.is_empty() || case.objs.is_empty())
                .count(),
        ),
        "matrix_nan_cells" => Some(
            parsed
                .matrix
                .iter()
                .map(|case| case.cells.iter().filter(|cell| cell.is_nan()).count())
                .sum(),
        ),
        _ => None,
    }
}

fn cast_stat(parsed: &Parsed, name: &str) -> Option<usize> {
    match name {
        "cast_ties" => Some(
            parsed
                .cast
                .iter()
                .filter(|case| {
                    let exponent = (case.bits >> 52) & 0x7ff;
                    (897..=1150).contains(&exponent) && (case.bits & 0x1fff_ffff) == 0x1000_0000
                })
                .count(),
        ),
        "cast_overflow" => Some(
            parsed
                .cast
                .iter()
                .filter(|case| case.bits >> 52 & 0x7ff < 2047 && case.want.is_infinite())
                .count(),
        ),
        "cast_subnormal" => Some(
            parsed
                .cast
                .iter()
                .filter(|case| {
                    let bits = case.want.to_bits();
                    let exponent = bits >> 23 & 0xff;
                    bits & 0x007f_ffff != 0 && exponent == 0
                })
                .count(),
        ),
        "cast_underflow" => Some(
            parsed
                .cast
                .iter()
                .filter(|case| {
                    let input = f64::from_bits(case.bits);
                    input.is_finite() && case.bits << 1 != 0 && case.want.to_bits() << 1 == 0
                })
                .count(),
        ),
        _ => None,
    }
}

fn stat(parsed: &Parsed, name: &str) -> Option<usize> {
    match name {
        "dist" => Some(parsed.dist.len()),
        "matrix" => Some(parsed.matrix.len()),
        "cast" => Some(parsed.cast.len()),
        _ => dist_stat(parsed, name)
            .or_else(|| matrix_stat(parsed, name))
            .or_else(|| cast_stat(parsed, name)),
    }
}

type HandRow = (&'static str, [i32; 4], [f64; 4], &'static str, &'static str);

const HAND_ROWS: [HandRow; 13] = [
    (
        "near-estimate",
        [10, 20, 60, 90],
        [12.5, 21.0, 58.25, 91.5],
        "3fb88e32748dd329",
        "3dc47194",
    ),
    (
        "far-estimate",
        [10, 20, 60, 90],
        [100.0, 100.0, 140.0, 180.0],
        "40035f7113ae7317",
        "401afb89",
    ),
    (
        "identical-boxes",
        [100, 100, 140, 180],
        [100.0, 100.0, 140.0, 180.0],
        "0000000000000000",
        "00000000",
    ),
    (
        "far-estimate-large-detection",
        [100, 100, 140, 180],
        [12.5, 21.0, 58.25, 91.5],
        "4001f3fc86ef5773",
        "400f9fe4",
    ),
    (
        "zero-estimate-width",
        [5, 5, 8, 9],
        [5.0, 5.0, 5.0, 9.0],
        "7ff0000000000000",
        "7f800000",
    ),
    (
        "zero-over-zero",
        [5, 5, 5, 9],
        [5.0, 5.0, 5.0, 9.0],
        "nan",
        "nan",
    ),
    (
        "inverted-estimate",
        [10, 20, 60, 90],
        [60.0, 20.0, 10.0, 90.0],
        "4000000000000000",
        "40000000",
    ),
    (
        "small-detection-large-estimate",
        [0, 0, 10, 10],
        [0.0, 0.0, 30.0, 30.0],
        "400766602d1e6fd9",
        "403b3301",
    ),
    (
        "large-detection-small-estimate",
        [0, 0, 30, 30],
        [0.0, 0.0, 10.0, 10.0],
        "400cd82b446159f3",
        "4066c15a",
    ),
    (
        "extreme-integer-corners",
        [-16_777_216, -16_777_216, 16_777_216, 16_777_216],
        [0.5, 0.25, 1e-3, 2.0],
        "4190d3b9657691a5",
        "4c869dcb",
    ),
    (
        "nan-estimate",
        [10, 20, 60, 90],
        [f64::NAN, 20.0, 60.0, 90.0],
        "nan",
        "nan",
    ),
    (
        "tiny-estimate",
        [10, 20, 60, 90],
        [1e-300, 1e-300, 2e-300, 3e-300],
        "7ff0000000000000",
        "7f800000",
    ),
    (
        "distant-small-estimate",
        [10, 20, 60, 90],
        [350.0, 420.0, 360.25, 431.75],
        "404591ccb7b56cde",
        "422c8e66",
    ),
];

#[test]
fn hand_rows_match_the_real_function() {
    for (name, det, est, want64, want32) in HAND_ROWS {
        let actual = distance(det, est);
        assert!(
            f64_matches(actual, wanted64(want64)),
            "{name}: got {:016x}, want {want64}",
            actual.to_bits()
        );
        let cell = round_f64_to_f32(actual);
        assert!(
            f32_matches(cell, wanted32(want32)),
            "{name}: cell got {:08x}, want {want32}",
            cell.to_bits()
        );
    }
    let candidates = [
        ("car", [10, 20, 60, 90]),
        ("person", [100, 100, 140, 180]),
        ("car", [100, 100, 140, 180]),
    ];
    let objects = [
        ("car", [12.5, 21.0, 58.25, 91.5]),
        ("person", [100.0, 100.0, 140.0, 180.0]),
        ("car", [100.0, 100.0, 140.0, 180.0]),
    ];
    let wanted_cells = [
        "3dc47194", "7f800000", "401afb89", "7f800000", "00000000", "7f800000", "400f9fe4",
        "7f800000", "00000000",
    ];
    let matrix = distance_matrix(&candidates, &objects);
    assert_eq!(matrix.rows(), 3);
    assert_eq!(matrix.cols(), 3);
    for (index, cell) in matrix.values().iter().enumerate() {
        assert_eq!(
            cell.to_bits(),
            hex32(wanted_cells[index]).to_bits(),
            "cell {index}"
        );
    }
    let no_objects = distance_matrix(&candidates[..2], &[]);
    assert_eq!(no_objects.rows(), 2);
    assert_eq!(no_objects.cols(), 0);
    assert!(no_objects.values().is_empty());
    let no_candidates = distance_matrix(&[], &objects[..2]);
    assert_eq!(no_candidates.rows(), 0);
    assert_eq!(no_candidates.cols(), 2);
    assert!(no_candidates.values().is_empty());
}

#[test]
fn rounding_matches_numpy_on_hand_values() {
    let rows: [(&str, &str); 17] = [
        ("3ff0000000000000", "3f800000"),
        ("3fb999999999999a", "3dcccccd"),
        ("4170000010000000", "4b800000"),
        ("4170000030000000", "4b800002"),
        ("47effffff0000000", "7f800000"),
        ("47efffffefffffff", "7f7fffff"),
        ("47efffffe0000000", "7f7fffff"),
        ("366244ce242c5561", "00000000"),
        ("3690000000000000", "00000000"),
        ("3690000000000001", "00000001"),
        ("3696d601ad376ab9", "00000001"),
        ("0000000000000001", "00000000"),
        ("3810000000000000", "00800000"),
        ("380fffffbb1dd6a1", "007fffff"),
        ("4004000000000000", "40200000"),
        ("48078287f49c4a1d", "7f800000"),
        ("7ff0000000000000", "7f800000"),
    ];
    for (input, output) in rows {
        let value = hex64(input);
        let rounded = round_f64_to_f32(value);
        assert_eq!(
            rounded.to_bits(),
            hex32(output).to_bits(),
            "input {input}: got {:08x}, want {output}",
            rounded.to_bits()
        );
    }
}

#[test]
fn matrix_shape_and_accessors() {
    let matrix = Matrix::new(2, 3, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).expect("a 2 by 3 matrix");
    assert_eq!(matrix.rows(), 2);
    assert_eq!(matrix.cols(), 3);
    assert_eq!(matrix.values().len(), 6);
    for row in 0..3 {
        for col in 0..4 {
            let inside = row < 2 && col < 3;
            assert_eq!(matrix.get(row, col).is_some(), inside, "cell {row} {col}");
            if inside {
                assert_eq!(matrix.get(row, col), Some(matrix.values()[row * 3 + col]));
            }
        }
    }
    let empty_columns = Matrix::new(2, 0, Vec::new()).expect("two rows, no columns");
    assert_eq!(empty_columns.rows(), 2);
    assert_eq!(empty_columns.cols(), 0);
    assert_eq!(empty_columns.get(0, 0), None);
    let empty_rows = Matrix::new(0, 5, Vec::new()).expect("no rows, five columns");
    assert_eq!(empty_rows.rows(), 0);
    assert_eq!(empty_rows.cols(), 5);
    assert_eq!(empty_rows.get(0, 0), None);
    assert!(Matrix::new(2, 3, vec![0.0; 5]).is_none());
    assert!(Matrix::new(2, 3, vec![0.0; 7]).is_none());
    assert!(Matrix::new(usize::MAX, 2, Vec::new()).is_none());
}

#[test]
fn extreme_input_never_panics() {
    let corners = [i32::MIN, i32::MIN, i32::MAX, i32::MAX];
    let equal = [7, 7, 7, 7];
    let estimates: [[f64; 4]; 8] = [
        [f64::NAN, 0.0, 0.0, 0.0],
        [0.0, f64::NAN, 0.0, 0.0],
        [0.0, 0.0, f64::INFINITY, f64::NEG_INFINITY],
        [f64::MAX, f64::MAX, f64::MIN_POSITIVE, 0.0],
        [0.0, 0.0, 0.0, 0.0],
        [-0.0, -0.0, -0.0, -0.0],
        [5.0, 5.0, 5.0, 5.0],
        [f64::MIN_POSITIVE, -f64::MIN_POSITIVE, f64::MAX, -f64::MAX],
    ];
    for estimate in estimates {
        for detection in [corners, equal, [i32::MIN, i32::MAX, i32::MIN, i32::MAX]] {
            let value = distance(detection, estimate);
            assert!(value.is_nan() || value.is_infinite() || value.is_finite());
        }
        let matrix = distance_matrix(
            &[("car", corners), ("dog", equal)],
            &[("car", estimate), ("a", estimate)],
        );
        assert_eq!(matrix.rows(), 2);
        assert_eq!(matrix.cols(), 2);
        assert!(matrix.get(1, 1).is_some());
        assert_eq!(matrix.get(2, 0), None);
    }
    let no_objects = distance_matrix(&[("car", corners)], &[]);
    assert_eq!((no_objects.rows(), no_objects.cols()), (1, 0));
    let no_candidates = distance_matrix(&[], &[("car", [0.0, 0.0, 1.0, 1.0])]);
    assert_eq!((no_candidates.rows(), no_candidates.cols()), (0, 1));
}

#[test]
fn fixture_matches_numpy_bit_for_bit() {
    let parsed = parse_fixture();
    let mut names: Vec<&str> = vec!["dist", "matrix", "cast"];
    names.extend(parsed.counts.keys().map(String::as_str));
    names.extend(parsed.floors.keys().map(String::as_str));
    for name in names {
        let recomputed =
            stat(&parsed, name).unwrap_or_else(|| panic!("unknown header name {name}"));
        if let Some(stated) = parsed.counts.get(name) {
            assert_eq!(recomputed, *stated, "header count {name}");
        }
        if let Some(floor) = parsed.floors.get(name) {
            assert!(recomputed >= *floor, "floor {name}: {recomputed} < {floor}");
        }
    }
    assert_eq!(parsed.counts.get("dist").copied(), Some(3000));
    assert_eq!(parsed.counts.get("matrix").copied(), Some(700));
    assert_eq!(parsed.counts.get("cast").copied(), Some(900));
    for case in &parsed.dist {
        let actual = distance(case.det, case.est);
        assert!(
            f64_matches(actual, case.want64),
            "{}: f64 got {:016x}, want {:016x}",
            case.source,
            actual.to_bits(),
            case.want64.to_bits()
        );
        let cell = round_f64_to_f32(actual);
        assert!(
            f32_matches(cell, case.want32),
            "{}: f32 got {:08x}, want {:08x}",
            case.source,
            cell.to_bits(),
            case.want32.to_bits()
        );
    }
    for case in &parsed.matrix {
        let candidates: Vec<(&str, [i32; 4])> = case
            .cands
            .iter()
            .map(|(label, points)| (label.as_str(), *points))
            .collect();
        let objects: Vec<(&str, [f64; 4])> = case
            .objs
            .iter()
            .map(|(label, estimate)| (label.as_str(), *estimate))
            .collect();
        let matrix = distance_matrix(&candidates, &objects);
        assert_eq!(matrix.rows(), case.rows, "{}", case.source);
        assert_eq!(matrix.cols(), case.cols, "{}", case.source);
        assert_eq!(matrix.values().len(), case.cells.len(), "{}", case.source);
        for (index, (got, want)) in matrix.values().iter().zip(&case.cells).enumerate() {
            assert!(
                f32_matches(*got, *want),
                "{}: cell {index} got {:08x}, want {:08x}",
                case.source,
                got.to_bits(),
                want.to_bits()
            );
        }
    }
    for case in &parsed.cast {
        let rounded = round_f64_to_f32(f64::from_bits(case.bits));
        assert!(
            f32_matches(rounded, case.want),
            "{}: got {:08x}, want {:08x}",
            case.source,
            rounded.to_bits(),
            case.want.to_bits()
        );
    }
}
