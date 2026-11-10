use corvette_motion::{ContrastHistory, Plane, Shape};

const FIXTURE: &str = include_str!("fixtures/contrast_history_cases.txt");

fn seeded_byte(state: &mut u32) -> u8 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    u8::try_from(*state >> 24).expect("a top byte")
}

fn picture(seed: u32, lo: u32, span: u32, count: usize) -> Vec<u8> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            let byte = seeded_byte(&mut state);
            u8::try_from(lo + u32::from(byte) % span).expect("a bounded pixel")
        })
        .collect()
}

fn half_split(low: u8, high: u8) -> Vec<u8> {
    let mut pixels = vec![high; 48];
    pixels[..24].fill(low);
    pixels
}

fn hex_of(data: &[u8]) -> String {
    let mut hex = String::with_capacity(data.len() * 2);
    for byte in data {
        for nibble in [*byte >> 4, *byte & 0xf] {
            hex.push(char::from(b"0123456789abcdef"[usize::from(nibble)]));
        }
    }
    hex
}

fn fnv_of(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[test]
fn the_fixture_has_the_stated_records() {
    let lines: Vec<&str> = FIXTURE.lines().collect();
    assert_eq!(lines.first().copied(), Some("counts 6 830"));
    assert_eq!(lines.len(), 837);
    let sequences: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| line.starts_with("sequence "))
        .collect();
    assert_eq!(
        sequences,
        [
            "sequence hex 8 6 140",
            "sequence hex 5 5 140",
            "sequence hex 3 3 140",
            "sequence hex 2 2 140",
            "sequence fnv 122 100 135",
            "sequence fnv 64 48 135",
        ]
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("frame "))
            .count(),
        830
    );
    let foreign: Vec<&str> = lines
        .iter()
        .copied()
        .skip(1)
        .filter(|line| !line.starts_with("sequence ") && !line.starts_with("frame "))
        .collect();
    assert!(foreign.is_empty(), "unexpected kinds: {foreign:?}");

    let mut outstanding = 0_usize;
    for line in lines.iter().skip(1) {
        if let Some(rest) = line.strip_prefix("sequence ") {
            assert_eq!(
                outstanding, 0,
                "sequence started with frames outstanding: {line}"
            );
            let fields: Vec<&str> = rest.split_whitespace().collect();
            outstanding = fields[3].parse::<usize>().expect("a frame count");
        } else {
            outstanding -= 1;
        }
    }
    assert_eq!(outstanding, 0, "the last sequence is short of its frames");
}

#[test]
fn normalised_pictures_match_the_reference_on_every_frame() {
    let lines: Vec<&str> = FIXTURE.lines().skip(1).collect();
    let mut cursor = 0;
    while cursor < lines.len() {
        let head: Vec<&str> = lines[cursor].split_whitespace().collect();
        assert_eq!(head[0], "sequence");
        let mode = head[1];
        let sw: usize = head[2].parse().expect("a width");
        let sh: usize = head[3].parse().expect("a height");
        let frames: usize = head[4].parse().expect("a frame count");
        let shape = Shape::new(sw, sh).expect("a shape");
        let mut history = ContrastHistory::default();
        for position in 0..frames {
            cursor += 1;
            let record: Vec<&str> = lines[cursor].split_whitespace().collect();
            assert_eq!(record[0], "frame");
            let seed: u32 = record[1].parse().expect("a seed");
            let lo: u32 = record[2].parse().expect("a floor");
            let span: u32 = record[3].parse().expect("a span");
            let want = record[4];
            let pixels = picture(seed, lo, span, sw * sh);
            let plane = Plane::new(&pixels, shape).expect("a plane");
            let image = history.normalise(plane).expect("a stretched picture");
            let named = format!(
                "mode {mode} size {sw}x{sh} frame {position} seed {seed} lo {lo} span {span}"
            );
            assert_eq!(image.shape(), shape, "{named}");
            let written = if mode == "fnv" {
                fnv_of(image.data())
            } else {
                hex_of(image.data())
            };
            assert_eq!(written, want, "{named}");
        }
        cursor += 1;
    }
}

#[test]
fn the_first_pictures_follow_the_worked_examples() {
    let shape = Shape::new(8, 6).expect("a shape");
    let mut history = ContrastHistory::default();
    let first = half_split(10, 200);
    let image = history
        .normalise(Plane::new(&first, shape).expect("a plane"))
        .expect("a stretched picture");
    assert_eq!(image.shape(), shape);
    assert_eq!(image.data(), half_split(9, 200).as_slice());
    let second = half_split(40, 90);
    let image = history
        .normalise(Plane::new(&second, shape).expect("a plane"))
        .expect("a stretched picture");
    assert_eq!(image.data(), half_split(39, 90).as_slice());
    let flat = vec![77_u8; 48];
    let image = history
        .normalise(Plane::new(&flat, shape).expect("a plane"))
        .expect("an unchanged picture");
    assert_eq!(image.data(), flat.as_slice());
}

#[test]
fn a_picture_with_equal_bounds_passes_through_and_leaves_the_history_unchanged() {
    let shape = Shape::new(8, 6).expect("a shape");
    let flat = vec![77_u8; 48];
    let mut history = ContrastHistory::default();

    let held = history.clone();
    let image = history
        .normalise(Plane::new(&flat, shape).expect("a plane"))
        .expect("an unchanged picture");
    assert_eq!(image.data(), flat.as_slice());
    assert_eq!(history, held);

    let tiny = Shape::new(2, 2).expect("a shape");
    let near_flat = [5_u8, 5, 5, 6];
    let held = history.clone();
    let image = history
        .normalise(Plane::new(&near_flat, tiny).expect("a plane"))
        .expect("an unchanged picture");
    assert_eq!(image.data(), near_flat.as_slice());
    assert_eq!(history, held);

    let first = half_split(10, 200);
    let image = history
        .normalise(Plane::new(&first, shape).expect("a plane"))
        .expect("a stretched picture");
    assert_eq!(image.data(), half_split(9, 200).as_slice());
    let held = history.clone();
    let image = history
        .normalise(Plane::new(&flat, shape).expect("a plane"))
        .expect("an unchanged picture");
    assert_eq!(image.data(), flat.as_slice());
    assert_eq!(history, held);
}
