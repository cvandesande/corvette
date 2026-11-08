use corvette_yolo::{Candidate, Rect, nms_boxes};

const FIXTURE: &str = include_str!("fixtures/nms_cases.txt");
const RANDOM_CASES: [&str; 4] = ["random_200", "ties_200", "dense_8400", "region_merge_200"];
const HAND_CASES: [&str; 14] = [
    "empty",
    "single",
    "all_below",
    "score_below_and_at",
    "score_at_and_above",
    "identical_same_score",
    "identical_diff_score",
    "disjoint",
    "zero_width",
    "zero_area_both",
    "zero_inside",
    "chain",
    "overlap_equal_threshold",
    "overlap_just_above_threshold",
];
const OVERLOAD_CASES: [&str; 2] = ["fractional_double_keeps", "fractional_double_suppresses"];
const EXTREME_CASES: [&str; 12] = [
    "neg_width",
    "neg_height",
    "tiny_area_pair",
    "coords_1e30",
    "touching",
    "same_box_four",
    "nt_zero_overlap",
    "nt_one_overlap",
    "zero_area_nt_one",
    "score_exactly_one",
    "negative_origin_overlap",
    "negative_origin_disjoint",
];

struct Case {
    name: String,
    random: Option<(u32, usize)>,
    ties: bool,
    score_threshold: f32,
    nms_threshold: f32,
    candidates: Vec<Candidate>,
    want: Vec<usize>,
}

const fn next_u32(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

fn unit(state: &mut u32) -> f32 {
    let v = next_u32(state) >> 8;
    let hi = f32::from(((v >> 16) & 0xff) as u8);
    let lo = f32::from((v & 0xffff) as u16);
    (hi + lo / 65_536.0) / 256.0
}

fn random_candidates(seed: u32, count: usize, ties: bool) -> Vec<Candidate> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            let d: [f32; 5] = std::array::from_fn(|_| unit(&mut state));
            let w = d[2] * 120.0;
            let h = d[3] * 120.0;
            let t = d[4] * 16.0;
            let score = if ties { (t - t % 1.0) / 16.0 } else { d[4] };
            Candidate {
                rect: Rect {
                    x: f64::from(d[0] * 600.0),
                    y: f64::from(d[1] * 600.0),
                    width: f64::from(8.0 + w),
                    height: f64::from(8.0 + h),
                },
                score,
            }
        })
        .collect()
}

fn f32_bits(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap())
}

fn parse_fixture() -> Vec<Case> {
    let mut cases: Vec<Case> = Vec::new();
    for line in FIXTURE
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
    {
        let mut f = line.split_whitespace();
        match f.next().unwrap_or_default() {
            name if name.starts_with('[') => cases.push(Case {
                name: name.trim_matches(['[', ']']).to_owned(),
                random: None,
                ties: false,
                score_threshold: 0.0,
                nms_threshold: 0.0,
                candidates: Vec::new(),
                want: Vec::new(),
            }),
            "random" => {
                cases.last_mut().unwrap().random = Some((
                    f.next().unwrap().parse().unwrap(),
                    f.next().unwrap().parse().unwrap(),
                ));
            }
            "ties" => cases.last_mut().unwrap().ties = true,
            "st" => cases.last_mut().unwrap().score_threshold = f32_bits(f.next().unwrap()),
            "nt" => cases.last_mut().unwrap().nms_threshold = f32_bits(f.next().unwrap()),
            "box" => {
                let v: Vec<f32> = f.map(f32_bits).collect();
                cases.last_mut().unwrap().candidates.push(Candidate {
                    rect: Rect {
                        x: v[0].into(),
                        y: v[1].into(),
                        width: v[2].into(),
                        height: v[3].into(),
                    },
                    score: v[4],
                });
            }
            "want" => cases.last_mut().unwrap().want = f.map(|v| v.parse().unwrap()).collect(),
            other => panic!("unknown fixture field {other}"),
        }
    }
    for case in &mut cases {
        if let Some((seed, count)) = case.random {
            case.candidates = random_candidates(seed, count, case.ties);
        }
    }
    cases
}

fn check(name: &str) {
    let case = parse_fixture()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("fixture has no case {name}"));
    let got = nms_boxes(&case.candidates, case.score_threshold, case.nms_threshold);
    assert_eq!(got, case.want, "case {}", case.name);
}

#[test]
fn nms_boxes_matches_opencv_on_the_random_cases() {
    for name in ["random_200", "region_merge_200", "dense_8400"] {
        check(name);
    }
}

#[test]
fn nms_boxes_matches_opencv_when_scores_tie() {
    check("ties_200");
}

#[test]
fn nms_boxes_matches_opencv_on_the_hand_cases() {
    for name in HAND_CASES {
        check(name);
    }
}

#[test]
fn nms_boxes_matches_opencv_on_the_cases_where_the_overloads_differ() {
    for name in OVERLOAD_CASES {
        check(name);
    }
}

#[test]
fn nms_boxes_matches_opencv_on_the_degenerate_and_extreme_boxes() {
    for name in EXTREME_CASES {
        check(name);
    }
}

#[test]
fn the_fixture_lists_every_case_it_promises() {
    let mut expected = RANDOM_CASES.to_vec();
    expected.extend(HAND_CASES);
    expected.extend(OVERLOAD_CASES);
    expected.extend(EXTREME_CASES);
    let got: Vec<String> = parse_fixture().into_iter().map(|c| c.name).collect();
    assert_eq!(got, expected);
}

const fn cand(x: f64, y: f64, width: f64, height: f64, score: f32) -> Candidate {
    Candidate {
        rect: Rect {
            x,
            y,
            width,
            height,
        },
        score,
    }
}

#[test]
fn nms_boxes_does_not_panic_on_nan_and_degenerate_input() {
    let all_nan = vec![
        cand(0., 0., 1., 1., f32::NAN),
        cand(0., 0., 1., 1., f32::NAN),
    ];
    assert!(nms_boxes(&all_nan, 0.4, 0.4).is_empty());
    let one_nan = vec![cand(0., 0., 1., 1., f32::NAN), cand(5., 5., 1., 1., 0.9)];
    assert_eq!(nms_boxes(&one_nan, 0.4, 0.4), vec![1]);
    let nan_coord = vec![cand(f64::NAN, 0., 1., 1., 0.9), cand(0., 0., 1., 1., 0.8)];
    assert!(nms_boxes(&nan_coord, 0.4, 0.4).len() <= nan_coord.len());
    let inf_width = vec![
        cand(0., 0., f64::INFINITY, 1., 0.9),
        cand(0., 0., 1., 1., 0.8),
    ];
    assert!(nms_boxes(&inf_width, 0.4, 0.4).len() <= inf_width.len());
    let zero_area = vec![cand(0., 0., 0., 0., 0.9), cand(0., 0., 0., 0., 0.8)];
    assert_eq!(nms_boxes(&zero_area, 0.4, 0.4), vec![0]);
}
