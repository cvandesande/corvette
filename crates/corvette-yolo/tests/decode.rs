use corvette_yolo::decode::{
    ANCHORS, CLASSES, DEFAULT_NMS_THRESHOLD, DEFAULT_SCORE_THRESHOLD, Detection, RAW_LEN, decode,
};

const FIXTURE: &str = include_str!("fixtures/decode_cases.txt");
const RANDOM_CASES: [&str; 4] = ["random_dense", "random_sparse", "random_none", "random_all"];
const HAND_CASES: [&str; 4] = [
    "single",
    "score_equal_gate",
    "argmax_tie",
    "nan_class_score",
];
const CASES: [(&str, usize); 10] = [
    ("random_dense", 20),
    ("random_sparse", 8),
    ("random_none", 0),
    ("random_all", 20),
    ("single", 1),
    ("score_equal_gate", 1),
    ("argmax_tie", 1),
    ("nan_class_score", 1),
    ("quirk_nms_suppression", 1),
    ("cap_25", 20),
];
const BACKGROUND: [f32; 7] = [0.0, 0.0625, 0.125, 0.1875, 0.25, 0.3125, 0.375];

const fn next_u32(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

fn unit(v: u32) -> f32 {
    let hi = f32::from(((v >> 16) & 0xff) as u8);
    let lo = f32::from((v & 0xffff) as u16);
    (hi + lo / 65_536.0) / 256.0
}

fn random_tensor(seed: u32, strong_below: u32) -> Vec<f32> {
    let mut raw = vec![0f32; RAW_LEN];
    let mut state = seed;
    for anchor in 0..ANCHORS {
        let v: [u32; 7] = std::array::from_fn(|_| next_u32(&mut state) >> 8);
        raw[anchor] = unit(v[0]) * 640.0;
        raw[ANCHORS + anchor] = unit(v[1]) * 640.0;
        let w = unit(v[2]) * 112.0;
        let h = unit(v[3]) * 112.0;
        raw[2 * ANCHORS + anchor] = 16.0 + w;
        raw[3 * ANCHORS + anchor] = 16.0 + h;
        for class in 0..CLASSES {
            raw[(4 + class) * ANCHORS + anchor] = BACKGROUND[(anchor + 5 * class) % 7];
        }
        if v[4] < strong_below {
            let c = usize::try_from((v[5] * 46) >> 24).unwrap();
            let boost = 0.5 * unit(v[6]);
            raw[(4 + c) * ANCHORS + anchor] = 0.5 + boost;
        }
    }
    raw
}

fn f32_bits(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap())
}

struct Case {
    name: String,
    random: Option<(u32, u32)>,
    cells: Vec<(usize, usize, f32)>,
    st: f32,
    nt: f32,
    want: Vec<Detection>,
    raw: Vec<f32>,
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
                cells: Vec::new(),
                st: 0.0,
                nt: 0.0,
                want: Vec::new(),
                raw: Vec::new(),
            }),
            "random" => {
                cases.last_mut().unwrap().random = Some((
                    f.next().unwrap().parse().unwrap(),
                    f.next().unwrap().parse().unwrap(),
                ));
            }
            "cell" => {
                let row = f.next().unwrap().parse().unwrap();
                let anchor = f.next().unwrap().parse().unwrap();
                let value = f32_bits(f.next().unwrap());
                cases.last_mut().unwrap().cells.push((row, anchor, value));
            }
            "st" => cases.last_mut().unwrap().st = f32_bits(f.next().unwrap()),
            "nt" => cases.last_mut().unwrap().nt = f32_bits(f.next().unwrap()),
            "row" => {
                let class_id = f.next().unwrap().parse().unwrap();
                let [score, y_min, x_min, y_max, x_max] =
                    std::array::from_fn(|_| f32_bits(f.next().unwrap()));
                cases.last_mut().unwrap().want.push(Detection {
                    class_id,
                    score,
                    y_min,
                    x_min,
                    y_max,
                    x_max,
                });
            }
            other => panic!("unknown fixture field {other}"),
        }
    }
    for case in &mut cases {
        if let Some((seed, strong_below)) = case.random {
            case.raw = random_tensor(seed, strong_below);
        } else {
            let mut raw = vec![0f32; RAW_LEN];
            for &(row, anchor, value) in &case.cells {
                raw[row * ANCHORS + anchor] = value;
            }
            case.raw = raw;
        }
    }
    cases
}

fn case_named(name: &str) -> Case {
    parse_fixture()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("fixture has no case {name}"))
}

fn assert_case(case: &Case) {
    let raw = <&[f32; RAW_LEN]>::try_from(case.raw.as_slice()).unwrap();
    let got = decode(raw, DEFAULT_SCORE_THRESHOLD, DEFAULT_NMS_THRESHOLD);
    assert_eq!(got.len(), case.want.len(), "case {} row count", case.name);
    for (index, (g, w)) in got.iter().zip(&case.want).enumerate() {
        assert_eq!(
            g.class_id, w.class_id,
            "case {} row {} class",
            case.name, index
        );
        assert_eq!(
            g.score.to_bits(),
            w.score.to_bits(),
            "case {} row {} score",
            case.name,
            index
        );
        assert_eq!(
            g.y_min.to_bits(),
            w.y_min.to_bits(),
            "case {} row {} y_min",
            case.name,
            index
        );
        assert_eq!(
            g.x_min.to_bits(),
            w.x_min.to_bits(),
            "case {} row {} x_min",
            case.name,
            index
        );
        assert_eq!(
            g.y_max.to_bits(),
            w.y_max.to_bits(),
            "case {} row {} y_max",
            case.name,
            index
        );
        assert_eq!(
            g.x_max.to_bits(),
            w.x_max.to_bits(),
            "case {} row {} x_max",
            case.name,
            index
        );
    }
}

#[test]
fn decode_matches_frigate_on_the_random_tensors() {
    for name in RANDOM_CASES {
        assert_case(&case_named(name));
    }
}

#[test]
fn decode_matches_frigate_on_the_hand_cases() {
    for name in HAND_CASES {
        assert_case(&case_named(name));
    }
}

#[test]
fn decode_feeds_the_suppression_corner_coordinates_as_frigate_does() {
    let case = case_named("quirk_nms_suppression");
    assert_case(&case);
    assert_eq!(case.want.len(), 1, "the quirk case returns exactly one row");
}

#[test]
fn decode_returns_at_most_twenty_rows() {
    let case = case_named("cap_25");
    assert_case(&case);
    assert_eq!(case.want.len(), 20, "cap_25 returns exactly twenty rows");
}

#[test]
fn the_fixture_lists_every_case_it_promises() {
    let cases = parse_fixture();
    let names: Vec<&str> = cases.iter().map(|c| c.name.as_str()).collect();
    let expected: Vec<&str> = CASES.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, expected);
    for (case, (_, rows)) in cases.iter().zip(CASES) {
        assert_eq!(case.want.len(), rows, "case {} row count", case.name);
        assert_eq!(
            case.st.to_bits(),
            DEFAULT_SCORE_THRESHOLD.to_bits(),
            "case {} score threshold",
            case.name
        );
        assert_eq!(
            case.nt.to_bits(),
            DEFAULT_NMS_THRESHOLD.to_bits(),
            "case {} nms threshold",
            case.name
        );
    }
}

#[test]
fn decode_applies_the_thresholds_it_is_given() {
    let mut raw = vec![0f32; RAW_LEN];
    raw[0] = 320.0;
    raw[ANCHORS] = 320.0;
    raw[2 * ANCHORS] = 10.0;
    raw[3 * ANCHORS] = 10.0;
    raw[4 * ANCHORS] = 0.45;
    let tensor = <&[f32; RAW_LEN]>::try_from(raw.as_slice()).unwrap();
    assert_eq!(decode(tensor, 0.4, 0.4).len(), 1);
    assert!(decode(tensor, 0.5, 0.4).is_empty());
    let mut quirk = vec![0f32; RAW_LEN];
    for (anchor, cx, score) in [(1usize, 100.0f32, 0.875), (2, 130.0, 0.75)] {
        quirk[anchor] = cx;
        quirk[ANCHORS + anchor] = 100.0;
        quirk[2 * ANCHORS + anchor] = 40.0;
        quirk[3 * ANCHORS + anchor] = 40.0;
        quirk[4 * ANCHORS + anchor] = score;
    }
    let quirk = <&[f32; RAW_LEN]>::try_from(quirk.as_slice()).unwrap();
    assert_eq!(decode(quirk, 0.4, 0.4).len(), 1);
    assert_eq!(decode(quirk, 0.4, 0.99).len(), 2);
}

#[test]
fn decode_does_not_panic_on_nan_and_infinite_input() {
    for fill in [f32::NAN, f32::INFINITY, f32::MAX] {
        let raw = vec![fill; RAW_LEN];
        let tensor = <&[f32; RAW_LEN]>::try_from(raw.as_slice()).unwrap();
        let rows = decode(tensor, 0.4, 0.4);
        assert!(rows.len() <= 20);
    }
}
