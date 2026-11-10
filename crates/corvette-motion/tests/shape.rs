use corvette_motion::{Image, Plane, Shape, motion_shape};

const FIXTURE: &str = include_str!("fixtures/shape_cases.txt");

#[test]
fn motion_shape_matches_the_reference_on_every_case() {
    let lines: Vec<&str> = FIXTURE.lines().collect();
    assert_eq!(lines.len(), 8);
    for line in lines {
        let fields: Vec<usize> = line
            .split_whitespace()
            .map(|v| v.parse().expect("a number"))
            .collect();
        let frame = Shape::new(fields[0], fields[1]).expect("a valid shape");
        let want = Shape::new(fields[3], fields[4]).expect("a valid shape");
        assert_eq!(
            motion_shape(frame, fields[2]),
            Some(want),
            "case line: {line}"
        );
    }
}

#[test]
fn a_shape_with_a_zero_side_is_refused() {
    assert!(Shape::new(0, 5).is_none());
    assert!(Shape::new(5, 0).is_none());
    let shape = Shape::new(3, 4).expect("a valid shape");
    assert_eq!(shape.width(), 3);
    assert_eq!(shape.height(), 4);
    assert_eq!(shape.pixels(), 12);
}

#[test]
fn a_shape_whose_pixel_count_overflows_is_refused() {
    assert!(Shape::new(usize::MAX, 2).is_none());
}

#[test]
fn a_motion_frame_of_zero_width_or_height_is_refused() {
    assert!(motion_shape(Shape::new(1, 1000).expect("a valid shape"), 100).is_none());
    assert!(motion_shape(Shape::new(704, 576).expect("a valid shape"), 0).is_none());
    assert!(motion_shape(Shape::new(usize::MAX, 1).expect("a valid shape"), 2).is_none());
}

#[test]
fn a_plane_needs_exactly_the_pixel_count() {
    let shape = Shape::new(3, 4).expect("a valid shape");
    assert!(Plane::new(&[0u8; 11], shape).is_none());
    assert!(Plane::new(&[0u8; 12], shape).is_some());
    assert!(Plane::new(&[0u8; 13], shape).is_none());
    assert!(Plane::new(&[0u8; 24], shape).is_none());
    assert!(Image::new(vec![0u8; 11], shape).is_none());
    assert!(Image::new(vec![0u8; 12], shape).is_some());
    assert!(Image::new(vec![0u8; 13], shape).is_none());
    assert!(Image::new(vec![0u8; 24], shape).is_none());
}

#[test]
fn an_image_hands_out_its_plane_and_its_bytes() {
    let shape = Shape::new(2, 2).expect("a valid shape");
    let image = Image::new(vec![1, 2, 3, 4], shape).expect("a 2x2 image");
    let plane = image.plane();
    assert_eq!(plane.data(), [1, 2, 3, 4]);
    assert_eq!(plane.shape(), shape);
    assert_eq!(image.into_data(), vec![1, 2, 3, 4]);
}
