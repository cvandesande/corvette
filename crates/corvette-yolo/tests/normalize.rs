use corvette_yolo::{CHANNELS, HEIGHT, RGB_BYTES, TENSOR_ELEMENTS, WIDTH, normalize};

const TABLE: &str = include_str!("fixtures/normalize_table.txt");
const SPOTS: &str = include_str!("fixtures/normalize_spots.txt");

fn spot_value(y: usize, x: usize, c: usize) -> u8 {
    let residue = (y * 7 + x * 13 + c * 31) % 256;
    u8::try_from(residue).expect("a residue modulo 256 fits in a byte")
}

fn fixture_bits(bits: &str) -> u32 {
    let digits = bits
        .strip_prefix("0x")
        .expect("the bits field starts with 0x");
    u32::from_str_radix(digits, 16).expect("the bits field is hexadecimal")
}

fn normalize_bytes(image: &[u8]) -> Vec<f32> {
    normalize(image.try_into().expect("an image exactly RGB_BYTES long"))
}

#[test]
fn every_byte_value_normalises_to_the_reference_bits() {
    let lines: Vec<&str> = TABLE.lines().collect();
    assert_eq!(
        lines.len(),
        256,
        "the table carries one line per byte value"
    );
    let mut image = vec![0u8; RGB_BYTES];
    for (value, line) in lines.iter().enumerate() {
        let mut fields = line.split_whitespace();
        let byte: u8 = fields
            .next()
            .expect("a byte column")
            .parse()
            .expect("a decimal byte value");
        let want = fixture_bits(fields.next().expect("a bits column"));
        assert!(
            fields.next().is_none(),
            "a table line has exactly two fields"
        );
        assert_eq!(usize::from(byte), value, "table lines run 0..=255 in order");
        image.fill(byte);
        let tensor = normalize_bytes(&image);
        assert_eq!(tensor[0].to_bits(), want, "channel 0 for byte {byte}");
    }
}

#[test]
fn the_tensor_is_channel_first_and_matches_the_reference_at_sampled_positions() {
    let mut image = vec![0u8; RGB_BYTES];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            for c in 0..CHANNELS {
                image[(y * WIDTH + x) * CHANNELS + c] = spot_value(y, x, c);
            }
        }
    }
    let tensor = normalize_bytes(&image);
    assert_eq!(tensor.len(), TENSOR_ELEMENTS, "one value per input byte");
    let lines: Vec<&str> = SPOTS.lines().collect();
    assert_eq!(
        lines.len(),
        612,
        "every corner of every channel plus 600 samples"
    );
    for line in lines {
        let mut fields = line.split_whitespace();
        let c: usize = fields
            .next()
            .expect("a channel column")
            .parse()
            .expect("a decimal channel");
        let y: usize = fields
            .next()
            .expect("a row column")
            .parse()
            .expect("a decimal row");
        let x: usize = fields
            .next()
            .expect("an image column")
            .parse()
            .expect("a decimal image column");
        let want = fixture_bits(fields.next().expect("a bits column"));
        assert!(
            fields.next().is_none(),
            "a spot line has exactly four fields"
        );
        let index = c * HEIGHT * WIDTH + y * WIDTH + x;
        assert_eq!(
            tensor[index].to_bits(),
            want,
            "channel {c} row {y} column {x}"
        );
    }
}

#[test]
fn zero_normalises_to_zero_and_full_scale_to_one() {
    let zeros = vec![0u8; RGB_BYTES];
    assert!(
        normalize_bytes(&zeros)
            .iter()
            .all(|value| value.to_bits() == 0.0f32.to_bits()),
        "every element of an all-zero image is positive zero"
    );
    let fulls = vec![u8::MAX; RGB_BYTES];
    assert!(
        normalize_bytes(&fulls)
            .iter()
            .all(|value| value.to_bits() == 1.0f32.to_bits()),
        "every element of an all-255 image is exactly 1.0"
    );
}
