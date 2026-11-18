use std::time::Duration;

use corvette_detect::{SegmentError, SegmentName};

#[test]
fn parses_back_sub_corpus_segment() {
    let outcome = SegmentName::parse("back_sub-20261108T133620Z.ts");
    assert!(outcome.is_ok(), "{outcome:?}");
    if let Ok(segment) = outcome {
        assert_eq!(segment.camera(), "back_sub");
        assert_eq!(segment.start(), Duration::from_secs(1_794_144_980));
    }
}

#[test]
fn parses_the_epoch_start_as_zero() {
    let outcome = SegmentName::parse("cam-19700101T000000Z.ts");
    assert!(outcome.is_ok(), "{outcome:?}");
    if let Ok(segment) = outcome {
        assert_eq!(segment.start(), Duration::ZERO);
    }
}

#[test]
fn parses_leap_day_of_century_divisible_by_four_hundred() {
    let outcome = SegmentName::parse("cam-20000229T235959Z.ts");
    assert!(outcome.is_ok(), "{outcome:?}");
    if let Ok(segment) = outcome {
        assert_eq!(segment.start(), Duration::from_secs(951_868_799));
    }
}

#[test]
fn parses_leap_day_of_the_year_2024() {
    let outcome = SegmentName::parse("cam-20240229T000001Z.ts");
    assert!(outcome.is_ok(), "{outcome:?}");
    if let Ok(segment) = outcome {
        assert_eq!(segment.start(), Duration::from_secs(1_709_164_801));
    }
}

#[test]
fn parses_first_day_after_non_leap_century_2100() {
    let outcome = SegmentName::parse("cam-21000301T000001Z.ts");
    assert!(outcome.is_ok(), "{outcome:?}");
    if let Ok(segment) = outcome {
        assert_eq!(segment.start(), Duration::from_secs(4_107_542_401));
    }
}

#[test]
fn parses_last_second_of_the_year_1999() {
    let outcome = SegmentName::parse("cam-19991231T235959Z.ts");
    assert!(outcome.is_ok(), "{outcome:?}");
    if let Ok(segment) = outcome {
        assert_eq!(segment.start(), Duration::from_secs(946_684_799));
    }
}

#[test]
fn parses_the_second_after_the_2038_uint_boundary() {
    let outcome = SegmentName::parse("cam-20380119T031408Z.ts");
    assert!(outcome.is_ok(), "{outcome:?}");
    if let Ok(segment) = outcome {
        assert_eq!(segment.start(), Duration::from_secs(2_147_483_648));
    }
}

#[test]
fn parses_camera_containing_a_dash_and_digits() {
    let outcome = SegmentName::parse("front-door_2-20261108T133620Z.ts");
    assert!(outcome.is_ok(), "{outcome:?}");
    if let Ok(segment) = outcome {
        assert_eq!(segment.camera(), "front-door_2");
        assert_eq!(segment.start(), Duration::from_secs(1_794_144_980));
    }
}

#[test]
fn parses_camera_ending_in_an_underscore() {
    let outcome = SegmentName::parse("back_-20261108T133620Z.ts");
    assert!(outcome.is_ok(), "{outcome:?}");
    if let Ok(segment) = outcome {
        assert_eq!(segment.camera(), "back_");
        assert_eq!(segment.start(), Duration::from_secs(1_794_144_980));
    }
}

#[test]
fn parses_the_last_day_of_april() {
    let outcome = SegmentName::parse("cam-20260430T120001Z.ts");
    assert!(outcome.is_ok(), "{outcome:?}");
    if let Ok(segment) = outcome {
        assert_eq!(segment.start(), Duration::from_secs(1_777_550_401));
    }
}

#[test]
fn rejects_each_invalid_form_with_its_error() {
    for (name, expected) in [
        (
            "front/back_sub-20261108T133620Z.ts",
            SegmentError::PathSeparator,
        ),
        ("back_sub.ts", SegmentError::MissingSeparator),
        ("-20261108T133620Z.ts", SegmentError::EmptyCamera),
        (
            "back sub-20261108T133620Z.ts",
            SegmentError::InvalidCameraCharacter,
        ),
        ("back_-.ts", SegmentError::InvalidStampLength),
        (
            "back_sub-20261108T133620Z.TS",
            SegmentError::InvalidExtension,
        ),
        ("back_sub-20261108T133620Z", SegmentError::InvalidExtension),
        (
            "back_sub-20261108T1362Z.ts",
            SegmentError::InvalidStampLength,
        ),
        (
            "back_sub-20261108T133620Z.tsx.ts",
            SegmentError::InvalidStampLength,
        ),
        (
            "back_sub-2O261108T133620Z.ts",
            SegmentError::InvalidStampCharacter,
        ),
        (
            "back_sub-20261108X133620Z.ts",
            SegmentError::InvalidStampCharacter,
        ),
        (
            "back_sub-20261108T133620Q.ts",
            SegmentError::InvalidStampCharacter,
        ),
        (
            "back_sub-20261308T133620Z.ts",
            SegmentError::MonthOutOfRange,
        ),
        (
            "back_sub-20260008T133620Z.ts",
            SegmentError::MonthOutOfRange,
        ),
        ("back_sub-20260230T133620Z.ts", SegmentError::DayOutOfRange),
        ("back_sub-20261132T133620Z.ts", SegmentError::DayOutOfRange),
        ("back_sub-20260431T120000Z.ts", SegmentError::DayOutOfRange),
        ("back_sub-21000229T000000Z.ts", SegmentError::DayOutOfRange),
        ("back_sub-20261108T240000Z.ts", SegmentError::HourOutOfRange),
        (
            "back_sub-20261108T126000Z.ts",
            SegmentError::MinuteOutOfRange,
        ),
        (
            "back_sub-20261108T120060Z.ts",
            SegmentError::SecondOutOfRange,
        ),
        ("back_sub-19691231T235959Z.ts", SegmentError::YearOutOfRange),
        ("back_sub-30000101T000000Z.ts", SegmentError::YearOutOfRange),
    ] {
        assert_eq!(SegmentName::parse(name).err(), Some(expected), "{name}");
    }
}

#[test]
fn two_parses_of_the_same_name_are_equal() {
    assert_eq!(
        SegmentName::parse("back_sub-20261108T133620Z.ts"),
        SegmentName::parse("back_sub-20261108T133620Z.ts")
    );
}
