use corvette_motion::{
    GAUSSIAN_CENTRE_WEIGHT, GAUSSIAN_OUTER_WEIGHT, Image, Plane, Shape, gaussian_blur,
};

const FIXTURE: &str = include_str!("fixtures/blur_cases.txt");

fn seeded_byte(state: &mut u32) -> u8 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    u8::try_from(*state >> 24).expect("a top byte")
}

fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("two hex digits"))
        .collect()
}

#[test]
fn the_kernel_weights_equal_the_reference_bits() {
    let first = FIXTURE.lines().next().expect("a first line");
    let fields: Vec<&str> = first.split_whitespace().collect();
    assert_eq!(fields[0], "weights");
    let outer = u64::from_str_radix(fields[1], 16).expect("sixteen hex digits");
    let centre = u64::from_str_radix(fields[2], 16).expect("sixteen hex digits");
    assert_eq!(GAUSSIAN_OUTER_WEIGHT.to_bits(), outer);
    assert_eq!(GAUSSIAN_CENTRE_WEIGHT.to_bits(), centre);
}

#[test]
fn blur_matches_the_reference_on_every_picture() {
    let lines: Vec<&str> = FIXTURE.lines().collect();
    let seeded: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| line.starts_with("seeded "))
        .collect();
    let explicit: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| line.starts_with("explicit "))
        .collect();
    assert_eq!(seeded.len(), 14);
    assert_eq!(explicit.len(), 4);

    for line in seeded {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let sw: usize = fields[1].parse().expect("a width");
        let sh: usize = fields[2].parse().expect("a height");
        let seed: u32 = fields[3].parse().expect("a seed");
        let mut state = seed;
        let mut source = Vec::with_capacity(sw * sh);
        for _ in 0..sw * sh {
            source.push(seeded_byte(&mut state));
        }
        let plane = Plane::new(&source, Shape::new(sw, sh).expect("a shape")).expect("a plane");
        let image: Image = gaussian_blur(plane);
        let want = unhex(fields[4]);
        assert_eq!(image.shape(), plane.shape());
        assert!(
            image.data() == want.as_slice(),
            "case line: seeded {sw} {sh} {seed}"
        );
    }

    for line in explicit {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let sw: usize = fields[1].parse().expect("a width");
        let sh: usize = fields[2].parse().expect("a height");
        let source = unhex(fields[3]);
        let plane = Plane::new(&source, Shape::new(sw, sh).expect("a shape")).expect("a plane");
        let image: Image = gaussian_blur(plane);
        let want = unhex(fields[4]);
        assert_eq!(image.shape(), plane.shape());
        assert!(
            image.data() == want.as_slice(),
            "case line: explicit {sw} {sh}"
        );
    }
}

#[test]
fn a_flat_picture_blurs_to_the_reference_level_at_every_level() {
    let lines: Vec<&str> = FIXTURE
        .lines()
        .filter(|line| line.starts_with("level "))
        .collect();
    assert_eq!(lines.len(), 256);
    for (order, line) in lines.iter().enumerate() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let level: u8 = fields[1].parse().expect("a level");
        let want: u8 = fields[2].parse().expect("an output level");
        assert_eq!(level, u8::try_from(order).expect("a level in order"));
        let bytes = [level; 12];
        let shape = Shape::new(4, 3).expect("a shape");
        let plane = Plane::new(&bytes, shape).expect("a plane");
        let image: Image = gaussian_blur(plane);
        assert!(
            image.data().iter().all(|&byte| byte == want),
            "case line: level {level}"
        );
    }
}
