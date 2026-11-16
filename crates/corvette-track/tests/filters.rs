use std::collections::BTreeMap;

use corvette_track::{Candidate, ObjectFilter, ObjectFilters, ObjectMask, Rejection};

const FIXTURE: &str = include_str!("fixtures/filter_cases.txt");

fn label_list(text: &str) -> Vec<String> {
    text.split(',').map(str::to_owned).collect()
}

fn mask_from_spec(spec: &str, line: usize) -> Option<ObjectMask> {
    if spec == "-" {
        return None;
    }
    let (size, rows) = spec.split_once(':').expect("mask width x height and rows");
    let (width, height) = size.split_once('x').expect("mask size");
    let width: usize = width.parse().expect("mask width");
    let height: usize = height.parse().expect("mask height");
    let mut bytes = Vec::with_capacity(width * height);
    for row in rows.split('/') {
        assert_eq!(row.len(), width, "fixture line {line}: mask row width");
        bytes.extend(row.bytes().map(|digit| if digit == b'0' { 0 } else { 255 }));
    }
    ObjectMask::new(width, height, bytes)
}

const fn stage_name(stage: Option<Rejection>) -> &'static str {
    match stage {
        None => "None",
        Some(Rejection::NotTracked) => "NotTracked",
        Some(Rejection::MinArea) => "MinArea",
        Some(Rejection::MaxArea) => "MaxArea",
        Some(Rejection::MinScore) => "MinScore",
        Some(Rejection::MinRatio) => "MinRatio",
        Some(Rejection::MaxRatio) => "MaxRatio",
        Some(Rejection::Masked) => "Masked",
    }
}

fn parse_row(row: &str) -> Candidate<'_> {
    let p: Vec<&str> = row.split_whitespace().collect();
    assert_eq!(p.len(), 9, "row: {row}");
    Candidate {
        label: p[0],
        score: p[1].parse().expect("a score"),
        x_min: p[2].parse().expect("an x_min"),
        y_min: p[3].parse().expect("a y_min"),
        x_max: p[4].parse().expect("an x_max"),
        y_max: p[5].parse().expect("a y_max"),
        area: p[6].parse().expect("an area"),
        ratio: p[7].parse().expect("a ratio"),
    }
}

fn check(filters: &ObjectFilters, rows: &[&str]) {
    for (index, row) in rows.iter().enumerate() {
        let p: Vec<&str> = row.split_whitespace().collect();
        assert_eq!(
            stage_name(filters.first_rejection(&parse_row(row))),
            p[8],
            "row {index}: {row}"
        );
    }
}

fn hand() -> (ObjectFilter, ObjectFilter, ObjectFilter) {
    let person = ObjectFilter {
        min_area: 400,
        max_area: 100_000,
        min_score: 0.7,
        min_ratio: 0.5,
        max_ratio: 3.0,
        mask: Some(
            ObjectMask::new(
                4,
                3,
                vec![0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255],
            )
            .expect("the hand person mask"),
        ),
    };
    let car = ObjectFilter {
        min_area: 0,
        max_area: 24_000_000,
        min_score: 0.5,
        min_ratio: 0.0,
        max_ratio: 24_000_000.0,
        mask: Some(
            ObjectMask::new(
                4,
                3,
                vec![255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0],
            )
            .expect("the hand car mask"),
        ),
    };
    let bird = ObjectFilter {
        min_area: 1,
        max_area: 2,
        min_score: 0.99,
        min_ratio: 0.9,
        max_ratio: 1.1,
        mask: None,
    };
    (person, car, bird)
}

fn build(entries: &[(&str, &ObjectFilter)]) -> ObjectFilters {
    let track = label_list("person,car,dog");
    let mut filters = BTreeMap::new();
    for (label, filter) in entries {
        filters.insert((*label).to_owned(), (*filter).clone());
    }
    ObjectFilters::new(track, filters)
}

#[test]
fn an_untracked_label_is_rejected_first() {
    let (person, car, bird) = hand();
    let filters = build(&[("person", &person), ("car", &car), ("bird", &bird)]);
    check(
        &filters,
        &[
            "person 0.9 10 10 90 90 6400 1.0 None",
            "cat 0.9 10 10 90 90 6400 1.0 NotTracked",
            "dog 0.0 0 0 0 0 0 0.0 None",
            "bird 0.5 0 0 0 0 0 0.0 NotTracked",
        ],
    );
}

#[test]
fn min_area_stage() {
    let (person, _, _) = hand();
    let filters = build(&[("person", &person)]);
    check(
        &filters,
        &[
            "person 0.9 10 10 90 90 399 1.0 MinArea",
            "person 0.9 10 10 90 90 400 1.0 None",
        ],
    );
}

#[test]
fn max_area_stage() {
    let (person, _, _) = hand();
    let filters = build(&[("person", &person)]);
    check(
        &filters,
        &[
            "person 0.9 10 10 90 90 100000 1.0 None",
            "person 0.9 10 10 90 90 100001 1.0 MaxArea",
        ],
    );
}

#[test]
fn min_score_stage() {
    let (person, _, _) = hand();
    let filters = build(&[("person", &person)]);
    let row8 = "person 0.7 10 10 90 90 6400 1.0 MinScore";
    check(&filters, &[row8, "person 0.75 10 10 90 90 6400 1.0 None"]);
    assert!(filters.is_filtered(&parse_row(row8)));
}

#[test]
fn ratio_stages() {
    let (person, _, _) = hand();
    let filters = build(&[("person", &person)]);
    check(
        &filters,
        &[
            "person 0.9 10 10 90 90 6400 0.4999 MinRatio",
            "person 0.9 10 10 90 90 6400 0.5 None",
            "person 0.9 10 10 90 90 6400 3.0 None",
            "person 0.9 10 10 90 90 6400 3.0000001 MaxRatio",
        ],
    );
}

#[test]
fn mask_stage() {
    let (person, car, _) = hand();
    let filters = build(&[("person", &person), ("car", &car)]);
    check(
        &filters,
        &[
            "person 0.9 0 0 0 0 6400 1.0 Masked",
            "person 0.9 0 0 7 2 6400 1.0 None",
            "person 0.9 1 1 1 1 6400 1.0 None",
            "car 0.9 10 10 90 90 6400 1.0 Masked",
            "car 0.9 10 0 90 1 6400 1.0 None",
            "person 0.9 0 0 1 0 6400 1.0 Masked",
            "car 0.9 10 0 90 90 6400 1.0 Masked",
        ],
    );
}

#[test]
fn stages_apply_in_frigates_order() {
    let (person, car, _) = hand();
    let filters = build(&[("person", &person), ("car", &car)]);
    check(
        &filters,
        &[
            "person 0.1 0 0 0 0 10 9.0 MinArea",
            "person 0.1 0 0 0 0 200000 9.0 MaxArea",
            "person 0.1 0 0 0 0 6400 9.0 MinScore",
            "person 0.9 0 0 0 0 6400 0.1 MinRatio",
            "person 0.9 0 0 0 0 6400 9.0 MaxRatio",
            "person 0.9 0 0 0 0 6400 1.0 Masked",
            "person 0.1 0 0 0 0 6400 0.1 MinScore",
        ],
    );
}

#[test]
fn a_tracked_label_without_a_filter_is_kept() {
    let (person, car, _) = hand();
    let filters = build(&[("person", &person), ("car", &car)]);
    check(
        &filters,
        &[
            "dog 0.0 0 0 0 0 0 0.0 None",
            "dog 0.0 10 10 90 90 0 1e9 None",
        ],
    );
}

#[test]
fn a_mask_must_match_its_size() {
    assert!(ObjectMask::new(0, 3, vec![]).is_none());
    assert!(ObjectMask::new(3, 0, vec![]).is_none());
    assert!(ObjectMask::new(2, 2, vec![255; 3]).is_none());
    assert!(ObjectMask::new(2, 2, vec![255; 5]).is_none());
    assert!(ObjectMask::new(2, 2, vec![255; 4]).is_some());
    assert!(ObjectMask::new(usize::MAX, 2, vec![]).is_none());
}

#[test]
fn defaults_match_frigate() {
    let filter = ObjectFilter::default();
    assert_eq!(filter.min_area, 0);
    assert_eq!(filter.max_area, 24_000_000);
    assert_eq!(filter.min_score.to_bits(), 0.5_f64.to_bits());
    assert_eq!(filter.min_ratio.to_bits(), 0.0_f64.to_bits());
    assert_eq!(filter.max_ratio.to_bits(), 24_000_000.0_f64.to_bits());
    assert!(filter.mask.is_none());
}

#[test]
fn mask_column_sums_box_edges_without_wrapping() {
    let mut bytes = vec![255u8; 1_000_000];
    bytes[1] = 0;
    let person = ObjectFilter {
        mask: Some(ObjectMask::new(1000, 1000, bytes).expect("the wide mask")),
        ..ObjectFilter::default()
    };
    check(
        &build(&[("person", &person)]),
        &[
            "person 0.9 4294967295 0 3 0 0 1.0 None",
            "person 0.9 4294967295 0 4294967295 0 0 1.0 None",
            "person 0.9 2 0 0 0 0 1.0 Masked",
            "person 0.9 1 0 1 0 0 1.0 Masked",
        ],
    );
}

#[test]
fn labels_match_exactly() {
    let (person, car, _) = hand();
    check(
        &build(&[("person", &person), ("car", &car)]),
        &[
            "Person 0.9 10 10 90 90 6400 1.0 NotTracked",
            "PERSON 0.9 10 10 90 90 6400 1.0 NotTracked",
            "Car 0.9 10 10 90 90 6400 1.0 NotTracked",
            "person 0.9 10 10 90 90 6400 1.0 None",
        ],
    );
}

#[test]
fn inverted_area_limits_name_the_minimum_first() {
    let person = ObjectFilter {
        min_area: 1000,
        max_area: 500,
        ..ObjectFilter::default()
    };
    check(
        &build(&[("person", &person)]),
        &[
            "person 0.9 10 10 90 90 400 1.0 MinArea",
            "person 0.9 10 10 90 90 750 1.0 MinArea",
            "person 0.9 10 10 90 90 2000 1.0 MaxArea",
        ],
    );
}

#[test]
fn fixture_matches_frigate_on_every_case() {
    let mut tracks: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut sets: BTreeMap<&str, BTreeMap<String, ObjectFilter>> = BTreeMap::new();
    let mut total = 0_usize;
    let mut stages: BTreeMap<&str, usize> = BTreeMap::new();
    for (index, line) in FIXTURE.lines().enumerate() {
        let line_no = index + 1;
        let fields: Vec<&str> = line.split_whitespace().collect();
        assert!(fields.len() >= 2, "fixture line {line_no}: too short");
        match fields[0] {
            "set" => {
                assert_eq!(fields.len(), 3, "fixture line {line_no}: set fields");
                tracks.insert(fields[1], label_list(fields[2]));
            }
            "filter" => {
                assert_eq!(fields.len(), 9, "fixture line {line_no}: filter fields");
                let filter = ObjectFilter {
                    min_area: fields[3].parse().expect("min area"),
                    max_area: fields[4].parse().expect("max area"),
                    min_score: fields[5].parse().expect("min score"),
                    min_ratio: fields[6].parse().expect("min ratio"),
                    max_ratio: fields[7].parse().expect("max ratio"),
                    mask: mask_from_spec(fields[8], line_no),
                };
                assert!(
                    filter.mask.is_some() || fields[8] == "-",
                    "fixture line {line_no}: mask"
                );
                sets.entry(fields[1])
                    .or_default()
                    .insert(fields[2].to_owned(), filter);
            }
            "case" => {
                assert_eq!(fields.len(), 11, "fixture line {line_no}: case fields");
                let set = ObjectFilters::new(
                    tracks
                        .get(fields[1])
                        .expect("case line after its set line")
                        .clone(),
                    sets.get(fields[1])
                        .expect("case line after its set line")
                        .clone(),
                );
                let candidate = parse_case(&fields);
                let got = stage_name(set.first_rejection(&candidate));
                assert_eq!(got, fields[10], "fixture line {line_no}");
                assert_eq!(
                    set.is_filtered(&candidate),
                    got != "None",
                    "fixture line {line_no}"
                );
                *stages.entry(fields[10]).or_insert(0) += 1;
                total += 1;
            }
            other => panic!("fixture line {line_no}: unknown kind {other}"),
        }
    }
    assert_eq!(total, 3000);
    assert!(stages.get("None").copied().unwrap_or(0) >= 676);
    assert!(stages.get("NotTracked").copied().unwrap_or(0) >= 437);
    assert!(stages.get("MinArea").copied().unwrap_or(0) >= 307);
    assert!(stages.get("MaxArea").copied().unwrap_or(0) >= 343);
    assert!(stages.get("MinScore").copied().unwrap_or(0) >= 311);
    assert!(stages.get("MinRatio").copied().unwrap_or(0) >= 304);
    assert!(stages.get("MaxRatio").copied().unwrap_or(0) >= 307);
    assert!(stages.get("Masked").copied().unwrap_or(0) >= 315);
}

fn parse_case<'a>(fields: &[&'a str]) -> Candidate<'a> {
    Candidate {
        label: fields[2],
        score: fields[3].parse().expect("score"),
        x_min: fields[4].parse().expect("x_min"),
        y_min: fields[5].parse().expect("y_min"),
        x_max: fields[6].parse().expect("x_max"),
        y_max: fields[7].parse().expect("y_max"),
        area: fields[8].parse().expect("area"),
        ratio: fields[9].parse().expect("ratio"),
    }
}
