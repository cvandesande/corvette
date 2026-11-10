use corvette_motion::{Image, Plane, Shape, resize_nearest};

const FIXTURE: &str = include_str!("fixtures/resize_cases.txt");

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
fn resize_matches_the_reference_on_every_case() {
    let lines: Vec<&str> = FIXTURE.lines().collect();
    assert_eq!(lines.len(), 15);
    for line in lines {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let sw: usize = fields[0].parse().expect("a source width");
        let sh: usize = fields[1].parse().expect("a source height");
        let dw: usize = fields[2].parse().expect("a destination width");
        let dh: usize = fields[3].parse().expect("a destination height");
        let seed: u32 = fields[4].parse().expect("a seed");

        let mut state = seed;
        let mut source = Vec::with_capacity(sw * sh);
        for _ in 0..sw * sh {
            source.push(seeded_byte(&mut state));
        }

        let plane = Plane::new(&source, Shape::new(sw, sh).expect("a source shape"))
            .expect("a source plane");
        let to = Shape::new(dw, dh).expect("a destination shape");
        let image: Image = resize_nearest(plane, to).expect("a resized image");
        let want = unhex(fields[5]);

        assert_eq!(image.shape(), to);
        assert!(
            image.data() == want.as_slice(),
            "case line: {} {} {} {} {}",
            fields[0],
            fields[1],
            fields[2],
            fields[3],
            fields[4]
        );
    }
}

#[test]
fn a_destination_wider_than_u32_is_refused() {
    let plane =
        Plane::new(&[0u8; 6], Shape::new(3, 2).expect("a valid shape")).expect("a valid plane");
    let past = usize::try_from(1u64 << 32).expect("a 64-bit usize");
    assert!(resize_nearest(plane, Shape::new(past, 1).expect("a wide shape")).is_none());
    assert!(resize_nearest(plane, Shape::new(1, past).expect("a tall shape")).is_none());
}
