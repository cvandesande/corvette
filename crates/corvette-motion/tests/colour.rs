// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and `clippy.toml` does not forbid them.
use corvette_motion::{Plane, RegionError, Shape, i420_to_rgb};

const FIXTURE: &str = include_str!("fixtures/i420_rgb_cases.txt");

// The generator rule of tools/goldens/motion_crop.py, reimplemented for the tests.
fn seeded_bytes(count: usize, seed: u64) -> Vec<u8> {
    let mut bytes = vec![0u8; count];
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

fn hex_of(bytes: &[u8]) -> String {
    const DIGITS: [u8; 16] = *b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(char::from(DIGITS[usize::from(byte >> 4)]));
        hex.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    hex
}

fn flat_canvas(side: usize, y: u8, u: u8, v: u8) -> Vec<u8> {
    let chroma_plane = side * side / 4;
    let mut canvas = vec![y; side * side];
    canvas.extend(std::iter::repeat_n(u, chroma_plane));
    canvas.extend(std::iter::repeat_n(v, chroma_plane));
    canvas
}

const fn canvas_plane(canvas: &[u8], side: usize) -> Plane<'_> {
    Plane::new(
        canvas,
        Shape::new(side, side + side / 2).expect("a canvas shape"),
    )
    .expect("a canvas plane")
}

#[test]
fn i420_to_rgb_matches_the_reference_on_every_case() {
    let lines: Vec<&str> = FIXTURE.lines().collect();
    assert!(lines.len() >= 40, "case count {}", lines.len());
    for line in &lines {
        let f: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(f.len(), 7, "case line: {line}");
        let side: usize = f[0].parse().expect("a side");
        let a: u32 = f[2].parse().expect("an a");
        let b: u32 = f[3].parse().expect("a b");
        let c: u32 = f[4].parse().expect("a c");
        let canvas = match f[1] {
            "seed" => seeded_bytes(side * (side + side / 2), u64::from(a)),
            "flat" => flat_canvas(
                side,
                u8::try_from(a).expect("a byte"),
                u8::try_from(b).expect("a byte"),
                u8::try_from(c).expect("a byte"),
            ),
            other => panic!("unknown kind {other}"),
        };
        let plane = canvas_plane(&canvas, side);
        let rgb =
            i420_to_rgb(plane).unwrap_or_else(|error| panic!("case {line} returned {error:?}"));
        assert_eq!(rgb.len(), side * side * 3, "length for case {line}");
        assert_eq!(
            fnv6a_hex(&rgb),
            f[5],
            "fnv mismatch for case {line}, rust rgb {}",
            hex_of(&rgb)
        );
        if f[6] != "-" {
            assert_eq!(hex_of(&rgb), f[6], "hex mismatch for case {line}");
        }
    }
}

#[test]
fn i420_to_rgb_returns_canvas_shape_error() {
    let six = vec![0u8; 6 * 9];
    assert_eq!(
        i420_to_rgb(canvas_plane(&six, 6)),
        Err(RegionError::CanvasShape)
    );
    let ragged = [0u8; 28];
    let ragged_plane = Plane::new(
        &ragged,
        Shape::new(4, 7).expect("rows not side times three halves"),
    )
    .expect("a ragged plane");
    assert_eq!(i420_to_rgb(ragged_plane), Err(RegionError::CanvasShape));
}

#[test]
fn i420_to_rgb_black_is_black() {
    let canvas = flat_canvas(4, 16, 128, 128);
    let rgb = i420_to_rgb(canvas_plane(&canvas, 4)).expect("black converts");
    assert_eq!(rgb, vec![0u8; 48]);
}

#[test]
fn i420_to_rgb_keeps_the_channel_order_and_chroma_planes() {
    let expected: Vec<u8> = (0..16).flat_map(|_| [233u8, 103, 1]).collect();
    let canvas = flat_canvas(4, 128, 64, 192);
    assert_eq!(
        i420_to_rgb(canvas_plane(&canvas, 4)).expect("the probe converts"),
        expected
    );
    let swapped = flat_canvas(4, 128, 192, 64);
    let swapped_rgb = i420_to_rgb(canvas_plane(&swapped, 4)).expect("the swap converts");
    assert_ne!(swapped_rgb, expected);
}
