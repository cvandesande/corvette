use corvette_motion::{Plane, Shape, absolute_difference, dilate_3x3, threshold_above};

const FIXTURE: &str = include_str!("fixtures/delta_cases.txt");

fn seeded_byte(state: &mut u32) -> u8 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    u8::try_from(*state >> 24).expect("a top byte")
}

fn seeded_picture(seed: u32, lo: u32, span: u32, pixels: usize) -> Vec<u8> {
    let mut state = seed;
    let mut picture = Vec::with_capacity(pixels);
    for _ in 0..pixels {
        let byte = u32::from(seeded_byte(&mut state));
        picture.push(u8::try_from(lo + byte % span).expect("a pixel inside the span"));
    }
    picture
}

fn sparse_picture(seed: u32, modulus: u32, pixels: usize) -> Vec<u8> {
    let mut state = seed;
    let mut picture = Vec::with_capacity(pixels);
    for _ in 0..pixels {
        let byte = u32::from(seeded_byte(&mut state));
        picture.push(if byte % modulus == 0 { 255 } else { 0 });
    }
    picture
}

fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("two hex digits"))
        .collect()
}

fn fnv1a_hex(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in data {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn output_matches(mode: &str, want: &str, data: &[u8]) -> bool {
    if mode == "hex" {
        unhex(want) == data
    } else {
        fnv1a_hex(data) == want
    }
}

#[test]
fn the_fixture_has_the_stated_records() {
    let lines: Vec<&str> = FIXTURE.lines().collect();
    assert_eq!(lines[0], "counts 120 90 27 36 71");
    assert_eq!(lines.len(), 345);
    let mut rest = &lines[1..];
    for (kind, count) in [
        ("absdiff", 120),
        ("threshold", 90),
        ("dilate", 27),
        ("sparse", 36),
        ("point", 71),
    ] {
        let (block, tail) = rest.split_at(count);
        for line in block {
            let first = line.split_whitespace().next().expect("a first field");
            assert_eq!(first, kind, "record of the wrong kind: {line}");
        }
        rest = tail;
    }
    assert!(rest.is_empty(), "lines after the last stated kind");
}

#[test]
fn absolute_difference_matches_the_reference() {
    for line in FIXTURE.lines().filter(|line| line.starts_with("absdiff ")) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let mode = f[1];
        let width: usize = f[2].parse().expect("a width");
        let height: usize = f[3].parse().expect("a height");
        let shape = Shape::new(width, height).expect("a shape");
        let left = seeded_picture(
            f[4].parse().expect("a seed"),
            f[5].parse().expect("a low"),
            f[6].parse().expect("a span"),
            width * height,
        );
        let right = seeded_picture(
            f[7].parse().expect("a seed"),
            f[8].parse().expect("a low"),
            f[9].parse().expect("a span"),
            width * height,
        );
        let a = Plane::new(&left, shape).expect("a plane");
        let b = Plane::new(&right, shape).expect("a plane");
        let image = absolute_difference(a, b).expect("an image of equal shapes");
        assert_eq!(image.shape(), shape, "record shape mismatch: {line}");
        assert!(
            output_matches(mode, f[10], image.data()),
            "absdiff record, shape {width} by {height}: {line}"
        );
    }
}

#[test]
fn threshold_matches_the_reference() {
    for line in FIXTURE
        .lines()
        .filter(|line| line.starts_with("threshold "))
    {
        let f: Vec<&str> = line.split_whitespace().collect();
        let mode = f[1];
        let width: usize = f[2].parse().expect("a width");
        let height: usize = f[3].parse().expect("a height");
        let shape = Shape::new(width, height).expect("a shape");
        let source = seeded_picture(
            f[4].parse().expect("a seed"),
            f[5].parse().expect("a low"),
            f[6].parse().expect("a span"),
            width * height,
        );
        let level: u8 = f[7].parse().expect("a level");
        let image = threshold_above(Plane::new(&source, shape).expect("a plane"), level);
        assert_eq!(image.shape(), shape, "record shape mismatch: {line}");
        assert!(
            output_matches(mode, f[8], image.data()),
            "threshold record, shape {width} by {height}, level {level}: {line}"
        );
    }
}

#[test]
fn dilation_matches_the_reference_on_general_and_sparse_pictures() {
    for line in FIXTURE
        .lines()
        .filter(|line| line.starts_with("dilate ") || line.starts_with("sparse "))
    {
        let f: Vec<&str> = line.split_whitespace().collect();
        let mode = f[1];
        let width: usize = f[2].parse().expect("a width");
        let height: usize = f[3].parse().expect("a height");
        let shape = Shape::new(width, height).expect("a shape");
        let seed: u32 = f[4].parse().expect("a seed");
        let (picture, output) = match f[0] {
            "dilate" => (
                seeded_picture(
                    seed,
                    f[5].parse().expect("a low"),
                    f[6].parse().expect("a span"),
                    width * height,
                ),
                f[7],
            ),
            "sparse" => (
                sparse_picture(seed, f[5].parse().expect("a modulus"), width * height),
                f[6],
            ),
            other => panic!("unexpected kind {other} in {line}"),
        };
        let image = dilate_3x3(Plane::new(&picture, shape).expect("a plane"));
        assert_eq!(image.shape(), shape, "record shape mismatch: {line}");
        assert!(
            output_matches(mode, output, image.data()),
            "{} record, shape {width} by {height}: {line}",
            f[0]
        );
    }
}

#[test]
fn dilation_matches_the_reference_for_every_single_pixel() {
    for line in FIXTURE.lines().filter(|line| line.starts_with("point ")) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let mode = f[1];
        let width: usize = f[2].parse().expect("a width");
        let height: usize = f[3].parse().expect("a height");
        let x: usize = f[4].parse().expect("an x");
        let y: usize = f[5].parse().expect("a y");
        let mut picture = vec![0u8; width * height];
        picture[y * width + x] = 255;
        let shape = Shape::new(width, height).expect("a shape");
        let image = dilate_3x3(Plane::new(&picture, shape).expect("a plane"));
        assert_eq!(image.shape(), shape, "record shape mismatch: {line}");
        assert!(
            output_matches(mode, f[6], image.data()),
            "point record, shape {width} by {height} at ({x}, {y}): {line}"
        );
    }
}

#[test]
fn absolute_difference_of_different_shapes_is_none() {
    let six = [0u8; 6];
    let nine = [0u8; 9];
    let sixteen = [0u8; 16];
    let wide = Plane::new(&six, Shape::new(3, 2).expect("a shape")).expect("a plane");
    let tall = Plane::new(&six, Shape::new(2, 3).expect("a shape")).expect("a plane");
    assert!(absolute_difference(wide, tall).is_none());
    let square4 = Plane::new(&sixteen, Shape::new(4, 4).expect("a shape")).expect("a plane");
    let square3 = Plane::new(&nine, Shape::new(3, 3).expect("a shape")).expect("a plane");
    assert!(absolute_difference(square4, square3).is_none());
    assert!(absolute_difference(square3, square3).is_some());
    assert!(absolute_difference(wide, wide).is_some());
}
