use std::collections::BTreeMap;

use corvette_track::{
    Frame, Geometry, ObjectFilter, ObjectFilters, ObjectMask, Region, Rejection, Skipped, map_box,
};

const FIXTURE: &str = include_str!("fixtures/mapping_cases.txt");
const SAT: i64 = 536_870_912;

fn mapping(
    raw: [f32; 4],
    region: (i64, i64, i64, i64),
    frame: (i64, i64),
) -> Result<Geometry, Skipped> {
    let region = Region::new(region.0, region.1, region.2, region.3).expect("the hand region");
    let frame = Frame::new(frame.0, frame.1).expect("the hand frame");
    map_box(raw, region, frame)
}

fn expect_box(got: Result<Geometry, Skipped>, edges: (i64, i64, i64, i64), area: i64, ratio: f64) {
    let got = got.expect("a geometry");
    assert_eq!((got.x_min, got.y_min, got.x_max, got.y_max), edges);
    assert_eq!(got.area, area);
    assert_eq!(got.ratio.to_bits(), ratio.to_bits());
}

fn expect_outside(got: Result<Geometry, Skipped>) {
    assert_eq!(got, Err(Skipped::OutsideFrame));
}

const R320: (i64, i64, i64, i64) = (0, 0, 320, 320);
const F704: (i64, i64) = (704, 576);

#[test]
fn hand_rows_match_frigates_detect() {
    expect_box(
        mapping([0.1, 0.7, 0.5, 0.9], R320, F704),
        (224, 32, 288, 160),
        8192,
        0.5,
    );
    expect_box(
        mapping([0.1, 0.7, 0.5, 0.9], (0, 0, 640, 640), (1280, 720)),
        (448, 64, 576, 320),
        32768,
        0.5,
    );
    expect_outside(mapping(
        [0.1, 0.996_875, 0.5, 1.0],
        (384, 0, 704, 320),
        F704,
    ));
    expect_outside(mapping(
        [0.996_875, 0.1, 1.0, 0.5],
        (0, 256, 320, 576),
        F704,
    ));
    expect_box(
        mapping([0.1, 0.99375, 0.5, 1.0], (384, 0, 704, 320), F704),
        (702, 32, 703, 160),
        128,
        0.007_812_5,
    );
    expect_box(
        mapping([0.2, 0.1, 0.8, 0.9], (-400, 100, -80, 420), F704),
        (0, 164, -112, 356),
        -21504,
        -0.583_333_333_333_333_4,
    );
    expect_box(
        mapping([0.2, 0.1, 0.8, 0.9], (-400, -400, -80, -80), F704),
        (0, 0, -112, -144),
        16128,
        -112.0,
    );
    expect_box(
        mapping([0.2, 0.1, 0.615_625, 0.623_125], (-100, -100, 60, 60), F704),
        (0, 0, 0, -1),
        0,
        0.0,
    );
    expect_box(
        mapping([0.1, 0.5, 0.5, 1.0], (400, 0, 720, 320), F704),
        (560, 32, 703, 160),
        18304,
        1.117_187_5,
    );
    expect_box(
        mapping([0.1, 0.1, 0.499_999_9, 0.499_999_9], R320, F704),
        (32, 32, 159, 159),
        16129,
        1.0,
    );
    expect_box(
        mapping([-2000.0, -2000.0, -1000.0, -1000.0], R320, F704),
        (0, 0, -320_000, -320_000),
        102_400_000_000,
        -320_000.0,
    );
    expect_box(
        mapping([0.5, 0.1, 0.5, 0.9], R320, F704),
        (32, 160, 288, 160),
        0,
        256.0,
    );
    expect_box(
        mapping([0.1, 0.7, 0.5, 0.9], (0, 0, 640, 640), F704),
        (448, 64, 576, 320),
        32768,
        0.5,
    );
}

fn hand_filters(bottom_row_masked: bool) -> ObjectFilters {
    let masked = vec![0u8; 704];
    let open = vec![255u8; 704 * 575];
    let person_mask = if bottom_row_masked {
        [open.clone(), masked.clone()].concat()
    } else {
        [masked.clone(), open.clone()].concat()
    };
    let car_mask = if bottom_row_masked { masked } else { open };
    let person = ObjectFilter {
        min_area: 0,
        max_area: 24_000_000,
        min_score: 0.5,
        min_ratio: 0.0,
        max_ratio: 24_000_000.0,
        mask: ObjectMask::new(704, 576, person_mask),
    };
    let car = ObjectFilter {
        min_area: -1_000_000_000,
        max_area: 24_000_000,
        min_score: 0.5,
        min_ratio: -1_000_000_000.0,
        max_ratio: 24_000_000.0,
        mask: ObjectMask::new(704, 576, car_mask),
    };
    let mut filters = BTreeMap::new();
    filters.insert("person".to_owned(), person);
    filters.insert("car".to_owned(), car);
    ObjectFilters::new(vec!["person".to_owned(), "car".to_owned()], filters)
}

#[test]
fn rows_through_the_filter() {
    let person = hand_filters(true);
    let car = hand_filters(false);
    let h6 = mapping([0.2, 0.1, 0.8, 0.9], (-400, 100, -80, 420), F704).expect("h6");
    let h7 = mapping([0.2, 0.1, 0.8, 0.9], (-400, -400, -80, -80), F704).expect("h7");
    let h8 = mapping([0.2, 0.1, 0.615_625, 0.623_125], (-100, -100, 60, 60), F704).expect("h8");
    let h11 = mapping([-2000.0, -2000.0, -1000.0, -1000.0], R320, F704).expect("h11");
    assert_eq!(
        person.first_rejection(&h6.candidate("person", 0.9)),
        Some(Rejection::MinArea)
    );
    assert_eq!(car.first_rejection(&h6.candidate("car", 0.9)), None);
    assert_eq!(
        person.first_rejection(&h7.candidate("person", 0.9)),
        Some(Rejection::MinRatio)
    );
    assert_eq!(car.first_rejection(&h7.candidate("car", 0.9)), None);
    assert_eq!(
        person.first_rejection(&h8.candidate("person", 0.9)),
        Some(Rejection::Masked)
    );
    assert_eq!(car.first_rejection(&h8.candidate("car", 0.9)), None);
    assert_eq!(
        person.first_rejection(&h11.candidate("person", 0.9)),
        Some(Rejection::MaxArea)
    );
}

#[test]
fn truncation_and_f32_arithmetic_decide() {
    expect_box(
        mapping([0.5, 0.5, 0.5, -0.125], (0, 0, 4, 4), F704),
        (2, 2, 0, 2),
        0,
        -2.0,
    );
    expect_box(
        mapping([0.5, 0.5, 0.5, -0.425], (0, 0, 4, 4), F704),
        (2, 2, -1, 2),
        0,
        -3.0,
    );
    expect_box(
        mapping([0.1, 0.7, 0.5, 0.9], R320, F704),
        (224, 32, 288, 160),
        8192,
        0.5,
    );
    expect_box(
        mapping([0.1, 0.7, 0.5, 0.9], (0, 0, 640, 640), F704),
        (448, 64, 576, 320),
        32768,
        0.5,
    );
    expect_box(
        mapping([0.1, 0.1, 0.499_999_9, 0.499_999_9], R320, F704),
        (32, 32, 159, 159),
        16129,
        1.0,
    );
    expect_box(
        mapping([0.1, 0.021_875, 0.5, 0.9], R320, F704),
        (7, 32, 288, 160),
        35968,
        2.195_312_5,
    );
    expect_box(
        mapping([0.1, 0.7, 0.5, 0.9], (0, 0, 320, 160), F704),
        (224, 32, 288, 160),
        8192,
        0.5,
    );
    expect_box(
        mapping([0.1, 0.7, 0.5, 0.9], (0, 0, 160, 320), F704),
        (112, 16, 144, 80),
        2048,
        0.5,
    );
}

#[test]
fn non_finite_values_are_named_and_dropped() {
    assert_eq!(
        mapping([f32::NAN, 0.1, 0.5, 0.5], R320, F704),
        Err(Skipped::NonFinite)
    );
    assert_eq!(
        mapping([0.1, f32::NAN, 0.5, 0.5], R320, F704),
        Err(Skipped::NonFinite)
    );
    assert_eq!(
        mapping([0.1, 0.1, f32::NAN, 0.5], R320, F704),
        Err(Skipped::NonFinite)
    );
    assert_eq!(
        mapping([0.1, 0.1, 0.5, f32::NAN], R320, F704),
        Err(Skipped::NonFinite)
    );
    assert_eq!(
        mapping([0.1, 0.1, 0.5, f32::INFINITY], R320, F704),
        Err(Skipped::NonFinite)
    );
    assert_eq!(
        mapping([0.1, f32::NEG_INFINITY, 0.5, 0.5], R320, F704),
        Err(Skipped::NonFinite)
    );
    assert_eq!(
        mapping([0.1, 0.1, 0.5, 3e38], R320, F704),
        Err(Skipped::NonFinite)
    );
    assert_eq!(
        mapping([0.1, 0.1, 0.5, -3e38], R320, F704),
        Err(Skipped::NonFinite)
    );
}

#[test]
fn huge_values_saturate_at_two_to_the_29() {
    expect_box(
        mapping([0.1, 0.1, -3e8, 0.5], R320, F704),
        (32, 32, 160, -536_870_912),
        -68_719_480_832,
        128.0,
    );
    expect_box(
        mapping([0.1, 0.1, 0.5, -3e8], R320, F704),
        (32, 32, -536_870_912, 160),
        -68_719_480_832,
        -4_194_304.25,
    );
    expect_box(
        mapping([0.1, 0.1, 0.5, -3e5], R320, F704),
        (32, 32, -96_000_000, 160),
        -12_288_004_096,
        -750_000.25,
    );
    expect_box(
        mapping([0.1, 0.1, -1e12, -1e12], R320, F704),
        (32, 32, -536_870_912, -536_870_912),
        288_230_410_511_451_136,
        -536_870_944.0,
    );
}

#[test]
fn region_and_frame_limits() {
    assert!(Region::new(8_388_609, 0, 0, 0).is_none());
    assert!(Region::new(0, 8_388_609, 0, 0).is_none());
    assert!(Region::new(0, 0, 8_388_609, 0).is_none());
    assert!(Region::new(0, 0, 0, 8_388_609).is_none());
    assert!(Region::new(8_388_608, 0, 0, 0).is_some());
    assert!(Region::new(-8_388_608, 0, 0, 0).is_some());
    assert!(Frame::new(0, 576).is_none());
    assert!(Frame::new(704, 0).is_none());
    assert!(Frame::new(8_388_609, 1).is_none());
    assert!(Frame::new(1, 1).is_some());
    assert!(Frame::new(8_388_608, 8_388_608).is_some());
    expect_outside(mapping([0.1, 0.1, 0.5, 0.5], R320, (1, 1)));
    expect_outside(mapping([0.1, 0.1, 0.5, 0.5], (5, 7, 325, 327), (33, 17)));
    expect_outside(mapping([0.1, 0.1, 0.5, 0.5], R320, (2, 2)));
}

fn strict_filters(mask: Option<ObjectMask>) -> ObjectFilters {
    let person = ObjectFilter {
        min_area: 0,
        max_area: 24_000_000,
        min_score: 0.5,
        min_ratio: 0.0,
        max_ratio: 24_000_000.0,
        mask,
    };
    let mut filters = BTreeMap::new();
    filters.insert("person".to_owned(), person);
    ObjectFilters::new(vec!["person".to_owned()], filters)
}

fn fixture_mask(rects: &str, width: usize, height: usize, line_no: usize) -> Option<ObjectMask> {
    if rects == "-" {
        return None;
    }
    let limit_x = i64::try_from(width).expect("fixture width fits i64") - 1;
    let limit_y = i64::try_from(height).expect("fixture height fits i64") - 1;
    let mut bytes = vec![255u8; width * height];
    for rect in rects.split(';') {
        let parts: Vec<i64> = rect
            .split(',')
            .map(|v| v.parse().expect("a rect number"))
            .collect();
        assert_eq!(
            parts.len(),
            4,
            "fixture line {line_no}: a rect has four numbers"
        );
        let x0 = usize::try_from(parts[0].clamp(0, limit_x)).expect("a rect edge");
        let x1 = usize::try_from(parts[2].clamp(0, limit_x)).expect("a rect edge");
        let y0 = usize::try_from(parts[1].clamp(0, limit_y)).expect("a rect edge");
        let y1 = usize::try_from(parts[3].clamp(0, limit_y)).expect("a rect edge");
        for y in y0..=y1 {
            for x in x0..=x1 {
                bytes[y * width + x] = 0;
            }
        }
    }
    ObjectMask::new(width, height, bytes)
}

type Counts<'a> = BTreeMap<&'a str, usize>;

fn bump<'a>(counts: &mut Counts<'a>, name: &'a str) {
    *counts.entry(name).or_insert(0) += 1;
}

fn check_box_case(f: &[&str], line_no: usize, line: &str, got: Geometry, counts: &mut Counts<'_>) {
    assert_eq!(f.len(), 26, "fixture line {line_no}: a box result");
    bump(counts, "box");
    let edges = [
        f[19].parse::<i64>().expect("an edge"),
        f[20].parse::<i64>().expect("an edge"),
        f[21].parse::<i64>().expect("an edge"),
        f[22].parse::<i64>().expect("an edge"),
    ];
    let area = f[23].parse::<i64>().expect("an area");
    let ratio_bits = u64::from_str_radix(f[24], 16).expect("a ratio hex");
    assert_eq!(
        [got.x_min, got.y_min, got.x_max, got.y_max],
        edges,
        "fixture line {line_no}: {line}"
    );
    assert_eq!(got.area, area, "fixture line {line_no}: {line}");
    assert_eq!(
        got.ratio.to_bits(),
        ratio_bits,
        "fixture line {line_no}: {line}"
    );
    if area < 0 {
        bump(counts, "neg_area");
    }
    if edges[2] < 0 {
        bump(counts, "neg_x_max");
    }
    if edges.iter().any(|e| *e == SAT || *e == -SAT) {
        bump(counts, "saturated");
    }
    let x_extent: i64 =
        f[5].parse::<i64>().expect("an x_max") - f[3].parse::<i64>().expect("an x_min");
    let y_extent: i64 =
        f[6].parse::<i64>().expect("a y_max") - f[4].parse::<i64>().expect("a y_min");
    if y_extent != x_extent {
        bump(counts, "nonsquare");
    }
    let width = f[8].parse().expect("a width");
    let height = f[9].parse().expect("a height");
    let mask = fixture_mask(f[16], width, height, line_no);
    let dropped = strict_filters(mask)
        .first_rejection(&got.candidate("person", 0.9_f32))
        .is_some();
    assert_eq!(
        dropped,
        f[25] == "dropped",
        "fixture line {line_no}: {line}"
    );
    bump(counts, if dropped { "dropped" } else { "kept" });
}

fn check_case(f: &[&str], line_no: usize, line: &str, counts: &mut Counts<'_>) {
    assert_eq!(f[0], "case", "fixture line {line_no}");
    assert_eq!(f[2], "region", "fixture line {line_no}");
    assert_eq!(f[7], "frame", "fixture line {line_no}");
    assert_eq!(f[10], "box", "fixture line {line_no}");
    assert_eq!(f[15], "mask", "fixture line {line_no}");
    assert_eq!(f[17], "=>", "fixture line {line_no}");
    let region = Region::new(
        f[3].parse().expect("a region number"),
        f[4].parse().expect("a region number"),
        f[5].parse().expect("a region number"),
        f[6].parse().expect("a region number"),
    )
    .expect("a fixture region");
    let frame = Frame::new(
        f[8].parse().expect("a width"),
        f[9].parse().expect("a height"),
    )
    .expect("a fixture frame");
    let raw = [
        f32::from_bits(u32::from_str_radix(f[11], 16).expect("a box hex")),
        f32::from_bits(u32::from_str_radix(f[12], 16).expect("a box hex")),
        f32::from_bits(u32::from_str_radix(f[13], 16).expect("a box hex")),
        f32::from_bits(u32::from_str_radix(f[14], 16).expect("a box hex")),
    ];
    let got = map_box(raw, region, frame);
    match f[18] {
        "skipped" => {
            bump(counts, "skipped");
            assert_eq!(
                got,
                Err(Skipped::OutsideFrame),
                "fixture line {line_no}: {line}"
            );
        }
        "nonfinite" => {
            bump(counts, "nonfinite");
            assert_eq!(
                got,
                Err(Skipped::NonFinite),
                "fixture line {line_no}: {line}"
            );
        }
        "box" => {
            let geometry = got.unwrap_or_else(|skip| {
                panic!("fixture line {line_no}: expected a box, got {skip:?} ({line})")
            });
            check_box_case(f, line_no, line, geometry, counts);
        }
        other => panic!("fixture line {line_no}: unknown result {other}"),
    }
}

#[test]
fn fixture_matches_frigate_on_every_case() {
    let mut header: Counts<'static> = BTreeMap::new();
    let mut counts: Counts<'static> = BTreeMap::new();
    let mut cases = 0_usize;
    for (index, line) in FIXTURE.lines().enumerate() {
        let line_no = index + 1;
        if line.starts_with('#') {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.get(1) == Some(&"counts") {
                for pair in fields[2..].chunks(2) {
                    header.insert(pair[0], pair[1].parse().expect("a header count"));
                }
            }
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        assert!(f.len() >= 19, "fixture line {line_no}: too short");
        check_case(&f, line_no, line, &mut counts);
        cases += 1;
    }
    assert_eq!(cases, 5000, "case lines");
    assert_eq!(header["cases"], cases, "header cases");
    for (name, floor) in [
        ("box", 1500),
        ("kept", 400),
        ("dropped", 400),
        ("skipped", 150),
        ("nonfinite", 150),
        ("neg_area", 200),
        ("neg_x_max", 200),
        ("saturated", 100),
        ("nonsquare", 100),
    ] {
        let count = counts.get(name).copied().unwrap_or(0);
        assert_eq!(count, header[name], "header {name} count");
        assert!(count >= floor, "{name}: {count} < {floor}");
    }
    assert!(
        header["divergent"] >= 300,
        "divergent: {}",
        header["divergent"]
    );
    assert!(
        header["index_error"] >= 100,
        "index_error: {}",
        header["index_error"]
    );
}
