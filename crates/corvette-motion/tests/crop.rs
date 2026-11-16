// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and `clippy.toml` does not forbid them.
use corvette_motion::{Image, Plane, Region, RegionError, Shape, crop_pad_i420};

const FIXTURE: &str = include_str!("fixtures/crop_pad_cases.txt");

// The generator rule of tools/goldens/motion_crop.py, reimplemented for the tests.
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

const fn frame_plane(bytes: &[u8], width: usize, height: usize) -> Plane<'_> {
    Plane::new(
        bytes,
        Shape::new(width, height * 3 / 2).expect("a frame shape"),
    )
    .expect("a frame plane")
}

#[test]
fn crop_pad_matches_the_reference_on_every_case() {
    let case_lines: Vec<&str> = FIXTURE
        .lines()
        .filter(|line| !line.starts_with("raises "))
        .collect();
    assert!(case_lines.len() >= 52, "case count {}", case_lines.len());
    for line in &case_lines {
        let f: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(f.len(), 10, "case line: {line}");
        let width: usize = f[0].parse().expect("a frame width");
        let height: usize = f[1].parse().expect("a frame height");
        let seed: u64 = f[2].parse().expect("a seed");
        let x1: i32 = f[3].parse().expect("an x1");
        let y1: i32 = f[4].parse().expect("a y1");
        let x2: i32 = f[5].parse().expect("an x2");
        let y2: i32 = f[6].parse().expect("a y2");
        let size: usize = f[7].parse().expect("a size");
        let frame = seeded_frame(width, height, seed);
        let plane = frame_plane(&frame, width, height);
        let region = Region { x1, y1, x2, y2 };
        let canvas: Image = crop_pad_i420(plane, region)
            .unwrap_or_else(|error| panic!("case {line} returned {error:?}"));
        assert_eq!(
            canvas.shape(),
            Shape::new(size, size + size / 2).expect("a canvas shape"),
            "shape mismatch for case {line}"
        );
        assert_eq!(
            fnv6a_hex(canvas.data()),
            f[8],
            "fnv mismatch for case {line}"
        );
        if f[9] != "-" {
            assert_eq!(
                canvas.data(),
                unhex(f[9]).as_slice(),
                "hex mismatch for case {line}"
            );
        }
    }
}

#[test]
fn crop_pad_returns_the_named_errors() {
    let frame = seeded_frame(32, 24, 1);
    let plane = frame_plane(&frame, 32, 24);
    assert_eq!(
        crop_pad_i420(
            plane,
            Region {
                x1: 0,
                y1: 0,
                x2: 20,
                y2: 16
            }
        ),
        Err(RegionError::WiderThanCanvas)
    );
    assert_eq!(
        crop_pad_i420(
            plane,
            Region {
                x1: 0,
                y1: 0,
                x2: 16,
                y2: 1
            }
        ),
        Err(RegionError::CanvasTooSmall)
    );
    assert_eq!(
        crop_pad_i420(
            plane,
            Region {
                x1: -16,
                y1: -16,
                x2: -4,
                y2: -4
            }
        ),
        Err(RegionError::OutsideFrame)
    );
    let odd_rows = [0u8; 20];
    let odd_plane = Plane::new(
        &odd_rows,
        Shape::new(4, 5).expect("rows not divisible by 3"),
    )
    .expect("an odd frame plane");
    assert_eq!(
        crop_pad_i420(
            odd_plane,
            Region {
                x1: 0,
                y1: 0,
                x2: 4,
                y2: 4
            }
        ),
        Err(RegionError::FrameShape)
    );
    assert_eq!(
        crop_pad_i420(
            plane,
            Region {
                x1: i32::MIN,
                y1: i32::MIN,
                x2: i32::MAX,
                y2: i32::MAX
            }
        ),
        Err(RegionError::CanvasTooLarge)
    );
}

#[test]
fn crop_pad_pads_with_black_outside_the_frame() {
    let mut frame = vec![200u8; 8 * 8];
    frame.extend_from_slice(&[64u8; 2 * 8]);
    frame.extend_from_slice(&[192u8; 2 * 8]);
    let plane = frame_plane(&frame, 8, 8);
    let canvas = crop_pad_i420(
        plane,
        Region {
            x1: -4,
            y1: -4,
            x2: 4,
            y2: 4,
        },
    )
    .expect("a padded canvas");
    let mut expected: Vec<u8> = Vec::new();
    for _ in 0..4 {
        expected.extend_from_slice(&[16u8; 8]);
    }
    for _ in 0..4 {
        expected.extend_from_slice(&[16, 16, 16, 16, 200, 200, 200, 200]);
    }
    expected.extend_from_slice(&[128u8; 8]);
    expected.extend_from_slice(&[128, 128, 64, 64, 128, 128, 64, 64]);
    expected.extend_from_slice(&[128u8; 8]);
    expected.extend_from_slice(&[128, 128, 192, 192, 128, 128, 192, 192]);
    assert_eq!(canvas.data(), expected.as_slice());
}

#[test]
fn crop_pad_errors_wherever_frigate_raised() {
    let raise_lines: Vec<&str> = FIXTURE
        .lines()
        .filter(|line| line.starts_with("raises "))
        .collect();
    assert!(raise_lines.len() >= 6, "raises count {}", raise_lines.len());
    for line in &raise_lines {
        let f: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(f.len(), 7, "raises line: {line}");
        let width: usize = f[1].parse().expect("a frame width");
        let height: usize = f[2].parse().expect("a frame height");
        let x1: i32 = f[3].parse().expect("an x1");
        let y1: i32 = f[4].parse().expect("a y1");
        let x2: i32 = f[5].parse().expect("an x2");
        let y2: i32 = f[6].parse().expect("a y2");
        let frame = seeded_frame(width, height, 1);
        let plane = frame_plane(&frame, width, height);
        let region = Region { x1, y1, x2, y2 };
        assert!(
            crop_pad_i420(plane, region).is_err(),
            "expected an error for {line}"
        );
    }
}

#[test]
fn crop_pad_refuses_a_canvas_far_larger_than_the_frame() {
    let frame = seeded_frame(704, 576, 5);
    let plane = frame_plane(&frame, 704, 576);
    let huge = crop_pad_i420(
        plane,
        Region {
            x1: 0,
            y1: 0,
            x2: 704,
            y2: 100_000,
        },
    );
    assert_eq!(huge, Err(RegionError::CanvasTooLarge));
    let vast = crop_pad_i420(
        plane,
        Region {
            x1: 0,
            y1: 0,
            x2: 704,
            y2: 2_000_000_000,
        },
    );
    assert_eq!(vast, Err(RegionError::CanvasTooLarge));
    let above = crop_pad_i420(
        plane,
        Region {
            x1: 0,
            y1: 0,
            x2: 704,
            y2: 2820,
        },
    );
    assert_eq!(above, Err(RegionError::CanvasTooLarge));
    let edge = crop_pad_i420(
        plane,
        Region {
            x1: 0,
            y1: 0,
            x2: 704,
            y2: 2816,
        },
    )
    .expect("a 2816 canvas");
    assert_eq!(edge.data().len(), 2816 * 4224);
    let small = seeded_frame(32, 24, 5);
    let small_plane = frame_plane(&small, 32, 24);
    let over = crop_pad_i420(
        small_plane,
        Region {
            x1: 0,
            y1: 0,
            x2: 32,
            y2: 1028,
        },
    );
    assert_eq!(over, Err(RegionError::CanvasTooLarge));
    let bound = crop_pad_i420(
        small_plane,
        Region {
            x1: 0,
            y1: 0,
            x2: 32,
            y2: 1024,
        },
    )
    .expect("a 1024 canvas");
    assert_eq!(bound.data().len(), 1024 * 1536);
}

#[test]
fn crop_pad_is_deterministic() {
    let frame = seeded_frame(16, 16, 42);
    let plane = frame_plane(&frame, 16, 16);
    let region = Region {
        x1: 2,
        y1: 2,
        x2: 14,
        y2: 14,
    };
    let first = crop_pad_i420(plane, region).expect("a first canvas");
    let second = crop_pad_i420(plane, region).expect("a second canvas");
    assert_eq!(first.data(), second.data());
}
