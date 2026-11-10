use corvette_motion::{Plane, Shape, contrast_bounds};

const FIXTURE: &str = include_str!("fixtures/contrast_bounds_cases.txt");

fn seeded_byte(state: &mut u32) -> u8 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    u8::try_from(*state >> 24).expect("a top byte")
}

fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("two hex digits"))
        .collect()
}

fn check(line: &str, data: &[u8], sw: usize, sh: usize) {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let last = fields.len() - 1;
    let want = (
        fields[last - 1].parse::<u8>().expect("a bound"),
        fields[last].parse::<u8>().expect("a bound"),
    );
    let plane = Plane::new(data, Shape::new(sw, sh).expect("a shape")).expect("a plane");
    let got = contrast_bounds(plane).expect("bounds for a picture");
    assert!(got == want, "case line: {line}");
}

#[test]
fn the_fixture_has_the_stated_records() {
    let lines: Vec<&str> = FIXTURE.lines().collect();
    assert_eq!(lines.first().copied(), Some("counts 84 480 12 15"));
    let count_of = |kind: &str| {
        lines
            .iter()
            .filter(|line| line.starts_with(&format!("{kind} ")))
            .count()
    };
    assert_eq!(count_of("seeded"), 84);
    assert_eq!(count_of("split"), 480);
    assert_eq!(count_of("explicit"), 12);
    assert_eq!(count_of("branch"), 15);
    let foreign: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| {
            !(line.starts_with("counts ")
                || line.starts_with("seeded ")
                || line.starts_with("split ")
                || line.starts_with("explicit ")
                || line.starts_with("branch "))
        })
        .collect();
    assert!(foreign.is_empty(), "unexpected kinds: {foreign:?}");
    assert_eq!(lines.len(), 592);
}

#[test]
fn contrast_bounds_match_the_reference_on_every_picture() {
    for line in FIXTURE.lines().filter(|line| !line.starts_with("counts ")) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields[0] {
            "seeded" => {
                let sw: usize = fields[1].parse().expect("a width");
                let sh: usize = fields[2].parse().expect("a height");
                let seed: u32 = fields[3].parse().expect("a seed");
                let lo: u32 = fields[4].parse().expect("a floor");
                let span: u32 = fields[5].parse().expect("a span");
                let mut state = seed;
                let mut pixels = Vec::with_capacity(sw * sh);
                for _ in 0..sw * sh {
                    let byte = seeded_byte(&mut state);
                    let pixel = lo + u32::from(byte) % span;
                    pixels.push(u8::try_from(pixel).expect("a bounded pixel"));
                }
                check(line, &pixels, sw, sh);
            }
            "split" => {
                let sw: usize = fields[1].parse().expect("a width");
                let sh: usize = fields[2].parse().expect("a height");
                let split_at: usize = fields[3].parse().expect("a count");
                let lo: u8 = fields[4].parse().expect("a level");
                let hi: u8 = fields[5].parse().expect("a level");
                let pixels: Vec<u8> = (0..sw * sh)
                    .map(|index| if index < split_at { lo } else { hi })
                    .collect();
                check(line, &pixels, sw, sh);
            }
            "explicit" | "branch" => {
                let sw: usize = fields[1].parse().expect("a width");
                let sh: usize = fields[2].parse().expect("a height");
                let pixels = unhex(fields[3]);
                check(line, &pixels, sw, sh);
            }
            other => panic!("unexpected kind: {other}"),
        }
    }
}

#[test]
fn a_picture_of_one_pixel_has_that_pixel_as_both_bounds() {
    for order in 0..=255_u32 {
        let value = u8::try_from(order).expect("a level");
        let bytes = [value];
        let plane = Plane::new(&bytes, Shape::new(1, 1).expect("a shape")).expect("a plane");
        assert_eq!(
            contrast_bounds(plane),
            Some((value, value)),
            "level {value}"
        );
    }
}
