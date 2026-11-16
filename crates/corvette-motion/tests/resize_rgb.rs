// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and `clippy.toml` does not forbid them.
use corvette_motion::{Shape, resize_linear_rgb};

const FIXTURE: &str = include_str!("fixtures/resize_rgb_cases.txt");

// The generator rule of tools/goldens/motion_resize_rgb.py, reimplemented for the tests.
fn seeded_rgb(width: usize, height: usize, seed: u64) -> Vec<u8> {
    let mut bytes = vec![0u8; width * height * 3];
    let mut state = seed;
    for slot in &mut bytes {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *slot = u8::try_from((state >> 33) & 0xFF).expect("a masked byte");
    }
    bytes
}

fn fnv6a_hex(bytes: &[u8]) -> String {
    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    format!("{hash:016x}")
}

fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

struct Case {
    src: Vec<u8>,
    from: Shape,
    to: Shape,
    label: String,
    fnv: String,
    hex: Option<Vec<u8>>,
}

fn cases(class: &str) -> Vec<Case> {
    FIXTURE
        .lines()
        .filter(|line| line.starts_with(class))
        .map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(f.len(), 9, "fixture line: {line}");
            let sw = f[1].parse::<usize>().expect("source width");
            let sh = f[2].parse::<usize>().expect("source height");
            let dw = f[3].parse::<usize>().expect("destination width");
            let dh = f[4].parse::<usize>().expect("destination height");
            let seed = f[6].parse::<u64>().expect("seed");
            let src = match f[5] {
                "flat" => vec![u8::try_from(seed).expect("flat byte"); sw * sh * 3],
                "seed" => seeded_rgb(sw, sh, seed),
                other => panic!("unknown kind {other}"),
            };
            Case {
                from: Shape::new(sw, sh).expect("source shape"),
                to: Shape::new(dw, dh).expect("destination shape"),
                label: format!(
                    "{} {}",
                    f[..7].join(" "),
                    if f[8] == "-" { "fnv-only" } else { "with hex" }
                ),
                src,
                fnv: f[7].to_string(),
                hex: (f[8] != "-").then(|| unhex(f[8])),
            }
        })
        .collect()
}

#[test]
fn resize_rgb_matches_the_exact_cases_byte_for_byte() {
    let exact = cases("exact");
    assert!(!exact.is_empty(), "no exact cases in the fixture");
    assert!(exact.len() >= 25, "exact case count {}", exact.len());
    for case in &exact {
        let got = resize_linear_rgb(&case.src, case.from, case.to).expect("valid case");
        assert_eq!(fnv6a_hex(&got), case.fnv, "fnv for case {}", case.label);
        if let Some(want) = &case.hex {
            assert_eq!(&got, want, "bytes for case {}", case.label);
        }
    }
}

#[test]
fn resize_rgb_stays_within_one_level_on_the_tolerant_cases() {
    let tolerant = cases("tolerant");
    assert!(
        tolerant.len() >= 18,
        "tolerant case count {}",
        tolerant.len()
    );
    // The share bound is pooled over all tolerant cases: per case it depended on lucky
    // seeds, while the per-byte bound of one level holds for every byte of every case.
    let mut differing = 0usize;
    let mut total = 0usize;
    for case in &tolerant {
        let want = case.hex.as_ref().expect("tolerant case carries hex");
        let got = resize_linear_rgb(&case.src, case.from, case.to).expect("valid case");
        assert_eq!(got.len(), want.len(), "length for case {}", case.label);
        for (index, (a, b)) in got.iter().zip(want).enumerate() {
            let delta = a.abs_diff(*b);
            assert!(delta <= 1, "case {} byte {index}: {a} vs {b}", case.label);
            if delta == 1 {
                differing += 1;
            }
        }
        total += got.len();
    }
    assert!(
        differing * 100 <= total,
        "{differing} differing bytes of {total} across all tolerant cases exceeds one percent"
    );
}

#[test]
fn resize_rgb_two_by_one_to_four_by_one_matches_the_hand_computed_row() {
    let src: [u8; 6] = [0, 0, 0, 255, 255, 255];
    let got = resize_linear_rgb(
        &src,
        Shape::new(2, 1).expect("2 by 1"),
        Shape::new(4, 1).expect("4 by 1"),
    )
    .expect("valid shape");
    assert_eq!(got, vec![0, 0, 0, 64, 64, 64, 191, 191, 191, 255, 255, 255]);
}

#[test]
fn resize_rgb_exact_two_times_is_a_box_average() {
    let src: [u8; 12] = [1, 1, 1, 2, 2, 2, 3, 3, 3, 5, 5, 5];
    let got = resize_linear_rgb(
        &src,
        Shape::new(2, 2).expect("2 by 2"),
        Shape::new(1, 1).expect("1 by 1"),
    )
    .expect("valid shape");
    assert_eq!(got, vec![3, 3, 3]);
}

#[test]
fn resize_rgb_returns_none_for_bad_sizes() {
    let small = vec![0u8; 12];
    assert_eq!(
        resize_linear_rgb(
            &[0u8; 5],
            Shape::new(2, 2).expect("2 by 2"),
            Shape::new(4, 4).expect("4 by 4")
        ),
        None,
        "a source of the wrong length is refused"
    );
    let wide = Shape::new(16_385, 1).expect("side over the limit is a valid shape");
    assert_eq!(
        resize_linear_rgb(&small, wide, Shape::new(1, 1).expect("1 by 1")),
        None
    );
    assert_eq!(
        resize_linear_rgb(&[0u8; 3], Shape::new(1, 1).expect("1 by 1"), wide),
        None
    );
}

#[test]
fn resize_rgb_is_deterministic() {
    let exact = cases("exact");
    let case = exact.first().expect("at least one exact case");
    let first = resize_linear_rgb(&case.src, case.from, case.to).expect("valid case");
    let second = resize_linear_rgb(&case.src, case.from, case.to).expect("valid case");
    assert_eq!(first, second);
}
