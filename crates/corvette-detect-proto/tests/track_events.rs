use corvette_detect_proto::{
    LineError, TrackFrame, TrackInfo, decode_track_line, encode_track_line,
};

fn info() -> TrackInfo {
    TrackInfo {
        camera: "front".to_owned(),
        track_id: "7".to_owned(),
        label: "person".to_owned(),
        bounds: [0.1, 0.2, 0.3, 0.4],
        zones: vec!["drive".to_owned(), "back yard".to_owned()],
        first_motion: None,
        last_seen: 12.5,
    }
}

fn round_trip(frame: &TrackFrame) -> TrackFrame {
    let line = encode_track_line(frame);
    assert_eq!(*line.last().expect("line ends"), b'\n');
    let body = &line[..line.len() - 1];
    assert!(!body.contains(&b'\n'), "frame body carries a newline");
    decode_track_line(body).expect("line decodes")
}

#[test]
fn start_update_and_end_round_trip_byte_for_byte() {
    for frame in [
        TrackFrame::Start(info()),
        TrackFrame::Update(TrackInfo {
            first_motion: Some(9.25),
            ..info()
        }),
        TrackFrame::End {
            camera: "back".to_owned(),
            track_id: "12".to_owned(),
        },
    ] {
        assert_eq!(round_trip(&frame), frame);
    }
}

#[test]
fn separators_and_newlines_survive_the_escape() {
    let tricky = TrackFrame::Start(TrackInfo {
        camera: "cam\t1".to_owned(),
        track_id: "t,2".to_owned(),
        label: "a%b\n\r".to_owned(),
        bounds: [-0.5, 0.0, 1.0, 0.1],
        zones: vec!["z,1".to_owned(), "z%2".to_owned()],
        first_motion: Some(-3.75),
        last_seen: 0.0,
    });
    assert_eq!(round_trip(&tricky), tricky);
}

#[test]
fn negative_bounds_and_fractional_times_round_trip() {
    let frame = TrackFrame::Update(TrackInfo {
        bounds: [0.333_333_333_333_333_3, 0.666_666_666_666_666_6, 1.0, 0.0],
        first_motion: Some(0.1),
        last_seen: 1_699_999_999.123,
        ..info()
    });
    assert_eq!(round_trip(&frame), frame);
}

#[test]
fn unknown_version_and_kind_are_named() {
    let line = encode_track_line(&TrackFrame::Start(info()));
    let mut body = line[..line.len() - 1].to_vec();
    body[0] = b'9';
    assert_eq!(decode_track_line(&body), Err(LineError::Version(b'9')));
    body[0] = b'1';
    body[2] = b'x';
    assert_eq!(decode_track_line(&body), Err(LineError::Kind(b'x')));
}

#[test]
fn field_count_and_number_and_escape_faults_are_named() {
    let missing = b"1\tu\tfront\t7";
    assert_eq!(
        decode_track_line(missing),
        Err(LineError::Fields {
            wanted: 10,
            found: 2
        })
    );
    let short_end = b"1\te\tfront\t7\textra";
    assert_eq!(
        decode_track_line(short_end),
        Err(LineError::Fields {
            wanted: 2,
            found: 3
        })
    );
    let bad_number = b"1\ts\tfront\t7\tperson\t0.1\t0.2\t0.3\t0.4\t-\t-\tnotanumber";
    assert_eq!(
        decode_track_line(bad_number),
        Err(LineError::Number("last_seen"))
    );
    let broken_escape = b"1\te\tfr%2ont\t7";
    assert_eq!(decode_track_line(broken_escape), Err(LineError::Escape));
}

#[test]
fn a_line_without_the_kind_separator_is_rejected() {
    let line = encode_track_line(&TrackFrame::End {
        camera: "f".to_owned(),
        track_id: "1".to_owned(),
    });
    let mut body = line[..line.len() - 1].to_vec();
    body[3] = b'x';
    assert_eq!(decode_track_line(&body), Err(LineError::Shape));
}

#[test]
fn a_line_terminator_is_not_part_of_the_frame() {
    let line = encode_track_line(&TrackFrame::End {
        camera: "f".to_owned(),
        track_id: "1".to_owned(),
    });
    assert_eq!(decode_track_line(&line), Err(LineError::Control(b'\n')));
}
