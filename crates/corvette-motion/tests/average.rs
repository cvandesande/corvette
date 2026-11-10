use corvette_motion::{Plane, RunningAverage, Shape};

const FIXTURE: &str = include_str!("fixtures/average_cases.txt");

const SHAPES: [(usize, usize); 9] = [
    (1, 1),
    (1, 5),
    (1, 33),
    (1, 64),
    (1, 65),
    (7, 9),
    (8, 5),
    (177, 100),
    (133, 100),
];

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

fn bits_of(values: &[f32]) -> Vec<u32> {
    values.iter().map(|value| value.to_bits()).collect()
}

fn average_matches(mode: &str, want: &str, values: &[f32]) -> bool {
    let bytes_in = if mode == "hex" {
        u32::to_be_bytes
    } else {
        u32::to_le_bytes
    };
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&bytes_in(value.to_bits()));
    }
    if mode == "hex" {
        unhex(want) == bytes
    } else {
        fnv1a_hex(&bytes) == want
    }
}

fn image_matches(mode: &str, want: &str, data: &[u8]) -> bool {
    if mode == "hex" {
        unhex(want) == data
    } else {
        fnv1a_hex(data) == want
    }
}

#[test]
fn the_fixture_has_the_stated_records() {
    let lines: Vec<&str> = FIXTURE.lines().collect();
    assert_eq!(lines[0], "counts 216");
    assert_eq!(lines.len(), 217);
    let mut index = 1;
    for (si, &(width, height)) in SHAPES.iter().enumerate() {
        for qi in 0..4 {
            for frame in 0..6 {
                let line = lines[index];
                index += 1;
                let fields: Vec<&str> = line.split_whitespace().collect();
                assert_eq!(fields[0], "average", "record of the wrong kind: {line}");
                assert_eq!(
                    fields[2].parse::<usize>().expect("a width"),
                    width,
                    "record of the wrong shape: {line}"
                );
                assert_eq!(
                    fields[3].parse::<usize>().expect("a height"),
                    height,
                    "record of the wrong shape: {line}"
                );
                assert_eq!(
                    fields[4].parse::<usize>().expect("a sequence"),
                    4 * si + qi,
                    "record out of sequence order: {line}"
                );
                assert_eq!(
                    fields[5].parse::<usize>().expect("a frame"),
                    frame,
                    "record out of frame order: {line}"
                );
            }
        }
    }
    assert_eq!(index, lines.len(), "lines after the last stated record");
}

#[test]
fn the_running_average_matches_the_reference_after_every_frame() {
    let mut held: Option<RunningAverage> = None;
    for line in FIXTURE.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let mode = fields[1];
        let width: usize = fields[2].parse().expect("a width");
        let height: usize = fields[3].parse().expect("a height");
        let sequence: usize = fields[4].parse().expect("a sequence");
        let frame: usize = fields[5].parse().expect("a frame");
        let alpha_bits: u64 = u64::from_str_radix(fields[6], 16).expect("alpha bits");
        let seed: u32 = fields[7].parse().expect("a seed");
        let lo: u32 = fields[8].parse().expect("a low");
        let span: u32 = fields[9].parse().expect("a span");
        let shape = Shape::new(width, height).expect("a shape");
        if frame == 0 {
            held = Some(RunningAverage::new(shape));
        }
        let average = held.as_mut().expect("an average opened at frame 0");
        let picture = seeded_picture(seed, lo, span, width * height);
        let plane = Plane::new(&picture, shape).expect("a plane");
        assert_eq!(
            average.accumulate(plane, f64::from_bits(alpha_bits)),
            Some(()),
            "record refused: {line}"
        );
        assert!(
            average_matches(mode, fields[10], average.values()),
            "average mismatch, shape {width} by {height}, sequence {sequence}, frame {frame}, alpha bits {alpha_bits:016x}: {line}"
        );
        assert!(
            image_matches(mode, fields[11], average.to_image().data()),
            "image mismatch, shape {width} by {height}, sequence {sequence}, frame {frame}, alpha bits {alpha_bits:016x}: {line}"
        );
    }
}

#[test]
fn accumulate_refuses_a_frame_of_another_shape_and_an_alpha_outside_zero_to_one() {
    let shape = Shape::new(3, 2).expect("a shape");
    let bytes: Vec<u8> = (30..36).collect();
    let mut average = RunningAverage::new(shape);
    average
        .accumulate(Plane::new(&bytes, shape).expect("a plane"), 0.5)
        .expect("a frame of the average's shape");
    let before = bits_of(average.values());
    let narrow = Shape::new(2, 3).expect("a shape");
    assert_eq!(
        average.accumulate(
            Plane::new(&bytes, narrow).expect("a plane of six pixels"),
            0.5
        ),
        None,
        "a frame of another shape with the same pixel count must be refused"
    );
    assert_eq!(
        bits_of(average.values()),
        before,
        "a refusal must leave the values untouched"
    );
    let frame = Plane::new(&bytes, shape).expect("a plane");
    for alpha in [-0.001, 1.001, f64::NAN, f64::INFINITY] {
        assert_eq!(
            average.accumulate(frame, alpha),
            None,
            "alpha {alpha} must be refused"
        );
        assert_eq!(
            bits_of(average.values()),
            before,
            "alpha {alpha} must leave the values untouched"
        );
    }
    assert_eq!(
        average.accumulate(frame, 0.0),
        Some(()),
        "alpha 0.0 must be accepted"
    );
    assert_eq!(
        average.accumulate(frame, 1.0),
        Some(()),
        "alpha 1.0 must be accepted"
    );
}

#[test]
fn a_new_average_is_zero() {
    let shape = Shape::new(4, 3).expect("a shape");
    let average = RunningAverage::new(shape);
    assert_eq!(average.shape(), shape);
    assert!(
        average.values().iter().all(|value| value.to_bits() == 0),
        "a new average holds positive zero at every pixel"
    );
    let picture = average.to_image();
    assert_eq!(picture.shape(), shape);
    assert!(
        picture.data().iter().all(|&byte| byte == 0),
        "the picture of a zero average is all zero"
    );
}

#[test]
fn alpha_one_takes_the_frame_and_alpha_zero_keeps_the_average() {
    let shape = Shape::new(6, 1).expect("a shape");
    let bytes: [u8; 6] = [0, 1, 127, 128, 254, 255];
    let mut average = RunningAverage::new(shape);
    average
        .accumulate(Plane::new(&bytes, shape).expect("a plane"), 1.0)
        .expect("alpha one");
    let taken: Vec<u32> = bytes
        .iter()
        .map(|&byte| f32::from(byte).to_bits())
        .collect();
    assert_eq!(
        bits_of(average.values()),
        taken,
        "alpha one must take the frame byte"
    );
    let other: [u8; 6] = [255, 254, 128, 127, 1, 0];
    average
        .accumulate(Plane::new(&other, shape).expect("a plane"), 0.0)
        .expect("alpha zero");
    assert_eq!(
        bits_of(average.values()),
        taken,
        "alpha zero must leave the average unchanged"
    );
}

#[test]
fn the_conversion_rounds_halves_to_even() {
    let shape = Shape::new(8, 1).expect("a shape");
    let bytes: [u8; 8] = [1, 3, 5, 7, 9, 255, 0, 2];
    let mut average = RunningAverage::new(shape);
    average
        .accumulate(Plane::new(&bytes, shape).expect("a plane"), 0.5)
        .expect("a half weight");
    assert_eq!(
        average.values().to_vec(),
        vec![0.5_f32, 1.5, 2.5, 3.5, 4.5, 127.5, 0.0, 1.0],
        "a half weight on a zero average leaves half of every frame byte"
    );
    assert_eq!(
        average.to_image().data().to_vec(),
        vec![0u8, 2, 2, 4, 4, 128, 0, 1],
        "hand-derived ties to even, no reference: 0.5 rounds to 0, 1.5 and 2.5 to 2, 3.5 and 4.5 to 4, 127.5 to 128"
    );
}
