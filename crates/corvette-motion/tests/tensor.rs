// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and `clippy.toml` does not forbid them.
use corvette_motion::{
    Plane, Region, RegionError, Shape, TensorError, crop_pad_i420, i420_to_rgb, region_tensor,
    resize_linear_rgb,
};

const FIXTURE: &str = include_str!("fixtures/tensor_cases.txt");

// The generator rule of tools/goldens/motion_tensor.py, reimplemented for the tests.
fn seeded_frame(width: usize, height: usize, seed: u64) -> Vec<u8> {
    let mut bytes = vec![0u8; width * height * 3 / 2];
    let mut state = seed;
    for slot in &mut bytes {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *slot = u8::try_from((state >> 33) & 0xFF).expect("a masked byte");
    }
    bytes
}

const fn frame_plane(bytes: &[u8], width: usize, height: usize) -> Plane<'_> {
    Plane::new(
        bytes,
        Shape::new(width, height * 3 / 2).expect("a frame shape"),
    )
    .expect("a frame plane")
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
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("two hex digits"))
        .collect()
}

struct Case {
    frame: Vec<u8>,
    width: usize,
    height: usize,
    region: Region,
    model: Shape,
    label: String,
    fnv: String,
    hex: Vec<u8>,
}

fn cases(class: &str) -> Vec<Case> {
    FIXTURE
        .lines()
        .filter(|line| line.starts_with(class))
        .map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(f.len(), 12, "fixture line: {line}");
            let width = f[1].parse::<usize>().expect("frame width");
            let height = f[2].parse::<usize>().expect("frame height");
            Case {
                width,
                height,
                frame: seeded_frame(width, height, f[3].parse::<u64>().expect("seed")),
                region: Region {
                    x1: f[4].parse::<i32>().expect("x1"),
                    y1: f[5].parse::<i32>().expect("y1"),
                    x2: f[6].parse::<i32>().expect("x2"),
                    y2: f[7].parse::<i32>().expect("y2"),
                },
                model: Shape::new(
                    f[8].parse::<usize>().expect("model width"),
                    f[9].parse::<usize>().expect("model height"),
                )
                .expect("model shape"),
                label: f[..10].join(" "),
                fnv: f[10].to_string(),
                hex: unhex(f[11]),
            }
        })
        .collect()
}

#[test]
fn region_tensor_matches_frigate_on_the_exact_cases() {
    let exact = cases("exact");
    assert!(exact.len() >= 10, "exact case count {}", exact.len());
    for case in &exact {
        let plane = frame_plane(&case.frame, case.width, case.height);
        let got = region_tensor(plane, case.region, case.model).expect("exact case");
        assert_eq!(fnv6a_hex(&got), case.fnv, "fnv for case {}", case.label);
        assert_eq!(got, case.hex, "bytes for case {}", case.label);
    }
}

#[test]
fn region_tensor_stays_within_one_level_on_the_tolerant_cases() {
    let tolerant = cases("tolerant");
    assert!(
        tolerant.len() >= 11,
        "tolerant case count {}",
        tolerant.len()
    );
    let mut differing = 0usize;
    let mut total = 0usize;
    for case in &tolerant {
        let plane = frame_plane(&case.frame, case.width, case.height);
        let got = region_tensor(plane, case.region, case.model).expect("tolerant case");
        assert_eq!(got.len(), case.hex.len(), "length for case {}", case.label);
        for (index, (a, b)) in got.iter().zip(&case.hex).enumerate() {
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
fn region_tensor_is_the_resized_converted_canvas() {
    for case in cases("tolerant") {
        let plane = frame_plane(&case.frame, case.width, case.height);
        let canvas = crop_pad_i420(plane, case.region).expect("canvas");
        let rgb = i420_to_rgb(canvas.plane()).expect("canvas converts");
        let side = canvas.shape().width();
        let square = Shape::new(side, side).expect("square canvas");
        let want = resize_linear_rgb(&rgb, square, case.model).expect("model resize");
        let got = region_tensor(plane, case.region, case.model).expect("tolerant case");
        assert_eq!(got, want, "resized chain for case {}", case.label);
    }
    for case in cases("exact") {
        let plane = frame_plane(&case.frame, case.width, case.height);
        let canvas = crop_pad_i420(plane, case.region).expect("canvas");
        let side = canvas.shape().width();
        if side == case.model.width() && side == case.model.height() {
            let want = i420_to_rgb(canvas.plane()).expect("canvas converts");
            let got = region_tensor(plane, case.region, case.model).expect("exact case");
            assert_eq!(got, want, "unresized canvas for case {}", case.label);
        }
    }
}

#[test]
fn region_tensor_passes_the_region_error_through() {
    let frame = seeded_frame(32, 24, 1);
    let plane = frame_plane(&frame, 32, 24);
    assert_eq!(
        region_tensor(
            plane,
            Region {
                x1: -16,
                y1: -16,
                x2: -4,
                y2: -4,
            },
            Shape::new(16, 16).expect("model shape"),
        ),
        Err(TensorError::Region(RegionError::OutsideFrame))
    );
}

#[test]
fn region_tensor_refuses_a_model_side_over_the_limit() {
    let frame = seeded_frame(32, 24, 1);
    let plane = frame_plane(&frame, 32, 24);
    assert_eq!(
        region_tensor(
            plane,
            Region {
                x1: 4,
                y1: 4,
                x2: 20,
                y2: 20,
            },
            Shape::new(16_385, 1).expect("a side over the resize limit is a valid shape"),
        ),
        Err(TensorError::ModelShape)
    );
}
