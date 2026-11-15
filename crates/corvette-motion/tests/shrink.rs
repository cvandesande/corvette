use corvette_motion::{
    Image, MotionDetector, MotionSettings, Shape, fill_polygons, motion_shape, shrink_keep_mask,
};

const FIXTURE: &str = include_str!("fixtures/shrink_cases.txt");

struct Case {
    kind: String,
    index: usize,
    from: Shape,
    to: Shape,
    input: String,
    expected: Vec<bool>,
}

fn whole(text: &str) -> usize {
    text.parse().unwrap()
}

fn bit_at(hex: &str, i: usize) -> bool {
    let nibble = u8::from_str_radix(&hex[(i / 4)..=(i / 4)], 16).unwrap();
    (nibble >> (3 - i % 4)) & 1 == 1
}

fn read_cases() -> (Vec<(String, usize)>, Vec<Case>) {
    let mut lines = FIXTURE.lines();
    let header: Vec<&str> = lines.next().unwrap().split(' ').collect();
    let mut counts = Vec::new();
    for pair in header[1..].chunks_exact(2) {
        counts.push((pair[0].to_string(), whole(pair[1])));
    }
    let mut cases = Vec::new();
    for line in lines {
        let field: Vec<&str> = line.split(' ').collect();
        let to = Shape::new(whole(field[5]), whole(field[6])).unwrap();
        cases.push(Case {
            kind: field[1].to_string(),
            index: whole(field[2]),
            from: Shape::new(whole(field[3]), whole(field[4])).unwrap(),
            to,
            input: field[7].to_string(),
            expected: (0..to.pixels()).map(|i| bit_at(field[9], i)).collect(),
        });
    }
    (counts, cases)
}

fn input_picture(case: &Case) -> Image {
    if case.kind == "camera" {
        let polygons: Vec<Vec<(f64, f64)>> = case
            .input
            .split(';')
            .map(|string| {
                let values: Vec<f64> = string.split(',').map(|v| v.parse().unwrap()).collect();
                values
                    .chunks_exact(2)
                    .map(|pair| (pair[0], pair[1]))
                    .collect()
            })
            .collect();
        fill_polygons(case.from, &polygons).unwrap()
    } else {
        let bytes: Vec<u8> = (0..case.from.pixels())
            .map(|i| if bit_at(&case.input, i) { 0 } else { 255 })
            .collect();
        Image::new(bytes, case.from).unwrap()
    }
}

fn shrink(
    width: usize,
    height: usize,
    kept: &[(usize, usize)],
    to_width: usize,
    to_height: usize,
) -> Vec<u8> {
    let mut data = vec![0u8; width * height];
    for &(x, y) in kept {
        data[y * width + x] = 255;
    }
    shrink_bytes(data, width, height, to_width, to_height)
}

fn shrink_bytes(
    data: Vec<u8>,
    width: usize,
    height: usize,
    to_width: usize,
    to_height: usize,
) -> Vec<u8> {
    let picture = Image::new(data, Shape::new(width, height).unwrap()).unwrap();
    shrink_keep_mask(picture.plane(), Shape::new(to_width, to_height).unwrap())
        .unwrap()
        .into_data()
}

#[test]
fn the_shrink_matches_the_deployed_zero_set_for_every_case() {
    let (counts, cases) = read_cases();
    for (kind, count) in &counts {
        let read = cases.iter().filter(|case| &case.kind == kind).count();
        assert_eq!(read, *count, "header count for {kind}");
    }
    assert_eq!(
        counts.iter().map(|(_, count)| count).sum::<usize>(),
        cases.len()
    );
    for case in &cases {
        let result = shrink_keep_mask(input_picture(case).plane(), case.to)
            .unwrap_or_else(|| panic!("{} {}: refused", case.kind, case.index));
        assert_eq!(
            result.shape(),
            case.to,
            "{} {}: shape",
            case.kind,
            case.index
        );
        let got = result.data();
        let want: Vec<u8> = case
            .expected
            .iter()
            .map(|&zero| if zero { 0 } else { 255 })
            .collect();
        let first = (0..want.len()).find(|&i| got[i] != want[i]);
        if let Some(i) = first {
            panic!(
                "{} {}: cell ({}, {}) is {} where the record says {}",
                case.kind,
                case.index,
                i / case.to.width(),
                i % case.to.width(),
                got[i],
                want[i]
            );
        }
    }
}

#[test]
fn the_three_deployed_cameras_have_the_measured_motion_masks() {
    let (_counts, cases) = read_cases();
    let cameras: Vec<&Case> = cases.iter().filter(|case| case.kind == "camera").collect();
    assert_eq!(cameras.len(), 3);
    let sizes = [(122usize, 100usize), (122, 100), (133, 100)];
    let zeros = [117usize, 176, 228];
    let settings = MotionSettings {
        threshold: 30,
        lightning_threshold: 0.8,
        improve_contrast: true,
        contour_area: 2,
        frame_alpha: 0.01,
        frame_height: 100,
    };
    for ((case, size), zero) in cameras.iter().zip(sizes).zip(zeros) {
        let motion = motion_shape(case.from, 100).unwrap();
        assert_eq!(
            (motion.width(), motion.height()),
            size,
            "{}: motion shape",
            case.index
        );
        let result = shrink_keep_mask(input_picture(case).plane(), motion).unwrap();
        assert_eq!(result.shape(), motion);
        let zeros_seen = result.data().iter().fold(
            0usize,
            |seen, &byte| if byte == 0 { seen + 1 } else { seen },
        );
        assert_eq!(zeros_seen, zero, "{}: zero bytes", case.index);
        assert!(
            MotionDetector::new(settings, case.from, Some(result.plane())).is_some(),
            "{}: the detector refused the mask it is built for",
            case.index
        );
    }
}

#[test]
fn the_worked_pictures_are_reproduced() {
    let top_left_only = shrink(5, 5, &[(0, 0)], 2, 2);
    assert_eq!(
        top_left_only,
        [255, 0, 0, 0],
        "each cell covers 2.5 by 2.5 source pixels: the kept cell holds 255 / 6.25 = 40.8 above 0.5, the other three see no kept pixel"
    );
    let centre_only = shrink(5, 5, &[(2, 2)], 2, 2);
    assert_eq!(
        centre_only,
        [255, 255, 255, 255],
        "each cell sees half of the centre pixel on each axis: 255 * 0.2 * 0.2 = 10.2 above 0.5 in every cell"
    );
    let three_pixels = shrink(41, 83, &[(3, 3), (4, 3), (3, 4)], 1, 2);
    assert_eq!(
        three_pixels,
        [0, 0],
        "the top cell of the 41 by 41.5 pair holds three pixels: 3 * 255 / 1701.5 = 0.4497 below 0.5, the bottom cell sees none"
    );
    let four_pixels = shrink(41, 83, &[(3, 3), (4, 3), (3, 4), (4, 4)], 1, 2);
    assert_eq!(
        four_pixels,
        [255, 0],
        "the fourth pixel lifts the top cell to 4 * 255 / 1701.5 = 0.5995 above 0.5 and the bottom cell stays at zero"
    );
}

#[test]
fn a_target_that_matches_one_source_side_is_shrunk_along_the_other() {
    let same_width = shrink(5, 5, &[(0, 0)], 5, 2);
    assert_eq!(
        same_width,
        [255, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        "the kept pixel holds 255 / 2.5 = 102 in the top-left cell, the rest see no kept pixel"
    );
    let same_height = shrink(5, 5, &[(0, 0)], 2, 5);
    assert_eq!(
        same_height,
        [255, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        "the kept pixel holds 255 / 2.5 = 102 in the top-left cell, the rest see no kept pixel"
    );
    let tall_one_pixel = shrink(1, 1021, &[(0, 0)], 1, 2);
    assert_eq!(
        tall_one_pixel,
        [0, 0],
        "one pixel in a 510.5-pixel cell accumulates 255 / 510.5 = 0.4995, below 0.5"
    );
    let tall_two_pixels = shrink(1, 1021, &[(0, 0), (0, 1)], 1, 2);
    assert_eq!(
        tall_two_pixels,
        [255, 0],
        "two pixels accumulate 0.999, above 0.5"
    );
    let wide_one_pixel = shrink(1021, 1, &[(0, 0)], 2, 1);
    assert_eq!(
        wide_one_pixel,
        [0, 0],
        "one pixel in a 510.5-pixel cell accumulates 255 / 510.5 = 0.4995, below 0.5"
    );
    let wide_two_pixels = shrink(1021, 1, &[(0, 0), (1, 0)], 2, 1);
    assert_eq!(
        wide_two_pixels,
        [255, 0],
        "two pixels accumulate 0.999, above 0.5"
    );
}

#[test]
fn a_pair_of_shapes_the_deployed_shrink_treats_differently_is_refused() {
    let frame = Image::new(vec![0u8; 704 * 576], Shape::new(704, 576).unwrap()).unwrap();
    let plane = frame.plane();
    assert!(shrink_keep_mask(plane, Shape::new(705, 100).unwrap()).is_none());
    assert!(shrink_keep_mask(plane, Shape::new(100, 577).unwrap()).is_none());
    assert!(shrink_keep_mask(plane, Shape::new(704, 576).unwrap()).is_none());
    assert!(shrink_keep_mask(plane, Shape::new(352, 288).unwrap()).is_none());
    assert!(shrink_keep_mask(plane, Shape::new(352, 100).unwrap()).is_some());
    assert!(shrink_keep_mask(plane, Shape::new(122, 288).unwrap()).is_some());
    assert!(shrink_keep_mask(plane, Shape::new(704, 100).unwrap()).is_some());
    assert!(shrink_keep_mask(plane, Shape::new(500, 576).unwrap()).is_some());
}

#[test]
fn any_nonzero_byte_counts_as_kept() {
    let expected_top_left = shrink(5, 5, &[(0, 0)], 2, 2);
    let mut data = vec![0u8; 25];
    data[0] = 1;
    let only_one = shrink_bytes(data, 5, 5, 2, 2);
    assert_eq!(
        only_one, expected_top_left,
        "the kept byte 1 shrinks like the byte 255 in the first worked picture"
    );
    let expected_four = shrink(41, 83, &[(3, 3), (4, 3), (3, 4), (4, 4)], 1, 2);
    let mut data = vec![0u8; 41 * 83];
    data[3 * 41 + 3] = 128;
    data[3 * 41 + 4] = 1;
    data[4 * 41 + 3] = 128;
    data[4 * 41 + 4] = 1;
    let mixed = shrink_bytes(data, 41, 83, 1, 2);
    assert_eq!(
        mixed, expected_four,
        "128 and 1 on the four worked pixels give the same result as 255"
    );
}
