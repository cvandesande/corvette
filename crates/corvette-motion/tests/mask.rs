use corvette_motion::{MotionDetector, MotionSettings, Shape, motion_keep_mask, motion_shape};

#[test]
fn the_three_deployed_camera_masks_have_the_measured_shape_and_zero_count_and_the_detector_accepts_each()
 {
    let front_frame = Shape::new(704, 576).unwrap();
    let back_frame = Shape::new(704, 576).unwrap();
    let living_frame = Shape::new(640, 480).unwrap();
    let front_polygons: [Vec<(f64, f64)>; 1] = [vec![
        (0.75, 0.033),
        (0.968, 0.036),
        (0.968, 0.089),
        (0.751, 0.087),
    ]];
    let back_polygons: [Vec<(f64, f64)>; 1] = [vec![
        (0.66, 0.035),
        (0.66, 0.087),
        (0.981, 0.089),
        (0.984, 0.031),
    ]];
    let living_polygons: [Vec<(f64, f64)>; 1] =
        [vec![(0.714, 0.068), (0.714, 0.0), (1.0, 0.0), (1.0, 0.065)]];
    let cases = [
        (front_frame, front_polygons.as_slice(), 122usize, 117usize),
        (back_frame, back_polygons.as_slice(), 122, 176),
        (living_frame, living_polygons.as_slice(), 133, 228),
    ];
    for (frame, polygons, width, zeros) in cases {
        let mask =
            motion_keep_mask(frame, 100, polygons).expect("the deployed camera mask is built");
        let motion = motion_shape(frame, 100).unwrap();
        assert_eq!(mask.shape(), motion, "{frame:?}: motion shape");
        assert_eq!(
            (motion.width(), motion.height()),
            (width, 100),
            "{frame:?}: motion size"
        );
        let zeros_seen = mask.data().iter().fold(
            0usize,
            |seen, byte| if *byte == 0 { seen + 1 } else { seen },
        );
        assert_eq!(zeros_seen, zeros, "{frame:?}: zero bytes");
        assert!(
            mask.data().iter().all(|byte| *byte == 0 || *byte == 255),
            "{frame:?}: a byte is neither 0 nor 255"
        );
        assert!(
            MotionDetector::new(MotionSettings::default(), frame, Some(mask.plane())).is_some(),
            "{frame:?}: the detector refused the mask it is built for"
        );
    }
}

#[test]
fn two_disjoint_polygons_ignore_every_pixel_either_alone_ignores_and_more_besides() {
    let frame = Shape::new(200, 160).unwrap();
    let upper_left = vec![(0.0, 0.0), (0.2, 0.0), (0.2, 0.2), (0.0, 0.2)];
    let lower_right = vec![(0.6, 0.6), (0.8, 0.6), (0.8, 0.8), (0.6, 0.8)];
    let alone_upper =
        motion_keep_mask(frame, 64, std::slice::from_ref(&upper_left)).expect("upper-left alone");
    let alone_lower =
        motion_keep_mask(frame, 64, std::slice::from_ref(&lower_right)).expect("lower-right alone");
    let both = motion_keep_mask(frame, 64, &[upper_left, lower_right]).expect("both polygons");
    assert_eq!(
        both.shape(),
        Shape::new(80, 64).unwrap(),
        "shrink to 80 by 64"
    );
    let zero_count = |data: &[u8]| {
        data.iter().fold(
            0usize,
            |seen, byte| if *byte == 0 { seen + 1 } else { seen },
        )
    };
    assert!(
        zero_count(alone_upper.data()) > 0,
        "upper-left alone ignores nothing"
    );
    assert!(
        zero_count(alone_lower.data()) > 0,
        "lower-right alone ignores nothing"
    );
    for index in 0..both.data().len() {
        assert!(
            alone_upper.data()[index] != 0 || both.data()[index] == 0,
            "byte {index} ignored by the upper-left polygon alone is kept with both"
        );
        assert!(
            alone_lower.data()[index] != 0 || both.data()[index] == 0,
            "byte {index} ignored by the lower-right polygon alone is kept with both"
        );
    }
    assert!(
        zero_count(both.data()) > zero_count(alone_upper.data()),
        "both ignores no more than the upper-left polygon alone"
    );
    assert!(
        zero_count(both.data()) > zero_count(alone_lower.data()),
        "both ignores no more than the lower-right polygon alone"
    );
}

#[test]
fn no_polygons_ignore_no_pixels() {
    let frame = Shape::new(704, 576).unwrap();
    let mask = motion_keep_mask(frame, 100, &[]).expect("an empty polygon list gives a mask");
    assert_eq!(mask.shape(), motion_shape(frame, 100).unwrap());
    let zeros_seen = mask.data().iter().fold(
        0usize,
        |seen, byte| if *byte == 0 { seen + 1 } else { seen },
    );
    assert_eq!(zeros_seen, 0, "with no polygon no pixel is ignored");
}

#[test]
fn a_short_polygon_a_far_coordinate_a_zero_height_whole_ratios_and_an_oversized_height_are_refused()
{
    let camera_frame = Shape::new(704, 576).unwrap();
    let square = vec![(0.0, 0.0), (0.2, 0.0), (0.2, 0.2), (0.0, 0.2)];
    assert!(
        motion_keep_mask(camera_frame, 100, &[vec![(0.0, 0.0), (0.5, 0.5)]]).is_none(),
        "a polygon of two points is refused"
    );
    assert!(
        motion_keep_mask(
            camera_frame,
            100,
            &[vec![(1.5, 0.0), (0.5, 0.0), (0.5, 0.5)]]
        )
        .is_none(),
        "a coordinate of 1.5 is refused"
    );
    assert!(
        motion_keep_mask(camera_frame, 0, std::slice::from_ref(&square)).is_none(),
        "a frame_height of 0 is refused"
    );
    assert!(
        motion_keep_mask(
            Shape::new(200, 100).unwrap(),
            50,
            std::slice::from_ref(&square)
        )
        .is_none(),
        "a frame and height with whole side ratios are refused"
    );
    assert!(
        motion_keep_mask(Shape::new(200, 100).unwrap(), 200, &[square]).is_none(),
        "a frame_height larger than the frame is refused"
    );
}
