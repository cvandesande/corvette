use std::io;

use corvette_detect::{FrameObjects, RecordError, RecordHeader, RecordWriter, TrackedObject};

const HEADER_LINE: &str = "{\"schema\":1,\"camera\":\"yard\",\"segment\":\"2026-11-18-00-00-00.mp4\",\"frame_width\":640,\"frame_height\":480,\"detect_fps\":5.0}\n";

fn header() -> RecordHeader {
    RecordHeader {
        camera: "yard".to_owned(),
        segment: "2026-11-18-00-00-00.mp4".to_owned(),
        frame_width: 640,
        frame_height: 480,
        detect_fps: 5.0,
    }
}

fn object(id: &str) -> TrackedObject {
    TrackedObject {
        id: id.to_owned(),
        label: "person".to_owned(),
        score: 0.9,
        confirmed: true,
        bounds: [0.1, 0.2, 0.3, 0.4],
        edges: [0, 0, 0, 0],
        first_seen: 5.0,
        last_seen: 6.0,
        zones_now: Vec::new(),
        zones_entered: Vec::new(),
        stationary: false,
    }
}

fn frame_of(frame_time: f64, objects: Vec<TrackedObject>, ended: Vec<&str>) -> FrameObjects {
    FrameObjects {
        frame_time,
        objects,
        ended: ended.into_iter().map(str::to_owned).collect(),
        motion_triggered: false,
    }
}

fn frame_line(frame: &FrameObjects) -> String {
    let mut buffer: Vec<u8> = Vec::new();
    let mut writer = RecordWriter::new(&mut buffer, &header()).unwrap();
    writer.write_frame(frame).unwrap();
    let text = String::from_utf8(buffer).unwrap();
    text.split_once('\n').unwrap().1.to_owned()
}

#[test]
fn a_new_writer_writes_the_header_object_ending_in_one_newline() {
    let mut buffer: Vec<u8> = Vec::new();
    {
        let _writer = RecordWriter::new(&mut buffer, &header()).unwrap();
    }
    assert_eq!(String::from_utf8(buffer).unwrap(), HEADER_LINE);
}

#[test]
fn a_frame_with_no_objects_and_no_endings_lists_both_as_empty_arrays() {
    let text = frame_line(&frame_of(12.5, Vec::new(), Vec::new()));
    assert_eq!(text, "{\"frame_time\":12.5,\"objects\":[],\"ended\":[]}\n");
}

#[test]
fn a_frame_with_objects_and_endings_prints_every_field_in_contract_order() {
    let moving = TrackedObject {
        zones_now: vec!["porch".to_owned()],
        zones_entered: vec!["porch".to_owned(), "driveway".to_owned()],
        ..object("1")
    };
    let still = TrackedObject {
        label: "cat".to_owned(),
        score: 0.25,
        confirmed: false,
        bounds: [0.0, 0.25, 0.5, 0.75],
        stationary: true,
        ..object("2")
    };
    let text = frame_line(&frame_of(12.5, vec![moving, still], vec!["7", "9"]));
    let mut edged = object("3");
    edged.edges = [40, 50, 60, 70];
    let plain = object("3");
    assert_eq!(
        frame_line(&frame_of(12.5, vec![edged], Vec::new())),
        frame_line(&frame_of(12.5, vec![plain], Vec::new())),
        "the writer ignores edges, so the lines are identical"
    );
    assert_eq!(
        text,
        "{\"frame_time\":12.5,\"objects\":[\
         {\"id\":\"1\",\"label\":\"person\",\"score\":0.9,\"confirmed\":true,\"box\":[0.1,0.2,0.3,0.4],\"first_seen\":5.0,\"last_seen\":6.0,\"zones_now\":[\"porch\"],\"zones_entered\":[\"porch\",\"driveway\"],\"state\":\"moving\"},\
         {\"id\":\"2\",\"label\":\"cat\",\"score\":0.25,\"confirmed\":false,\"box\":[0.0,0.25,0.5,0.75],\"first_seen\":5.0,\"last_seen\":6.0,\"zones_now\":[],\"zones_entered\":[],\"state\":\"stationary\"}],\
         \"ended\":[\"7\",\"9\"]}\n"
    );
}

#[test]
fn integral_numbers_carry_dot_zero_and_shortest_forms_round_trip() {
    let text = frame_line(&frame_of(0.0, vec![object("1")], Vec::new()));
    assert!(text.contains("\"frame_time\":0.0"), "{text}");
    assert!(text.contains("\"first_seen\":5.0"), "{text}");
    let exact = frame_line(&frame_of(0.1, Vec::new(), Vec::new()));
    assert!(exact.starts_with("{\"frame_time\":0.1,"), "{exact}");
    let long = frame_of(0.1 + 0.2, Vec::new(), Vec::new());
    assert!(
        frame_line(&long).starts_with("{\"frame_time\":0.30000000000000004,"),
        "0.1 + 0.2 prints its shortest round-trip form"
    );
    let huge = frame_line(&frame_of(1e21, Vec::new(), Vec::new()));
    assert!(
        huge.starts_with("{\"frame_time\":1000000000000000000000.0,"),
        "1e21 prints in plain decimal, never an exponent"
    );
    assert!(
        frame_line(&frame_of(1e-7, Vec::new(), Vec::new()))
            .starts_with("{\"frame_time\":0.0000001,")
    );
    assert_eq!(
        "1000000000000000000000.0".parse::<f64>().unwrap().to_bits(),
        1e21_f64.to_bits()
    );
    assert_eq!(
        "0.0000001".parse::<f64>().unwrap().to_bits(),
        1e-7_f64.to_bits()
    );
}

#[test]
fn negative_zero_keeps_its_sign() {
    const NEGATIVE_ZERO: f64 = -0.0;
    let text = frame_line(&frame_of(NEGATIVE_ZERO, Vec::new(), Vec::new()));
    assert!(text.starts_with("{\"frame_time\":-0.0,"), "{text}");
    assert_eq!(
        "-0.0".parse::<f64>().unwrap().to_bits(),
        NEGATIVE_ZERO.to_bits()
    );
}

#[test]
fn a_non_finite_number_names_its_field_and_leaves_the_output_at_the_previous_line() {
    let sink = SharedBuffer::default();
    let mut writer = RecordWriter::new(sink.clone(), &header()).unwrap();
    writer
        .write_frame(&frame_of(12.5, Vec::new(), Vec::new()))
        .unwrap();
    let complete_length = sink.len();
    let mut nan_score = object("1");
    nan_score.score = f64::NAN;
    let error = writer
        .write_frame(&frame_of(13.0, vec![nan_score], Vec::new()))
        .unwrap_err();
    assert!(
        matches!(error, RecordError::NonFinite("score")),
        "{error:?}"
    );
    assert_eq!(
        sink.len(),
        complete_length,
        "nothing of that frame is written"
    );
    let error = writer
        .write_frame(&frame_of(f64::INFINITY, Vec::new(), Vec::new()))
        .unwrap_err();
    assert!(
        matches!(error, RecordError::NonFinite("frame_time")),
        "{error:?}"
    );
    assert_eq!(sink.len(), complete_length);
    let mut nan_edge = object("2");
    nan_edge.bounds = [0.1, f64::NAN, 0.3, 0.4];
    let error = writer
        .write_frame(&frame_of(14.0, vec![nan_edge], Vec::new()))
        .unwrap_err();
    assert!(matches!(error, RecordError::NonFinite("box")), "{error:?}");
    assert_eq!(sink.len(), complete_length);
    drop(writer);
    let text = String::from_utf8(sink.bytes()).unwrap();
    assert_eq!(
        text,
        format!("{HEADER_LINE}{{\"frame_time\":12.5,\"objects\":[],\"ended\":[]}}\n")
    );
}

#[test]
fn a_non_finite_header_fps_is_rejected_before_anything_is_written() {
    let mut buffer: Vec<u8> = Vec::new();
    let mut bad = header();
    bad.detect_fps = f64::NAN;
    let error = RecordWriter::new(&mut buffer, &bad).unwrap_err();
    assert!(
        matches!(error, RecordError::NonFinite("detect_fps")),
        "{error:?}"
    );
    assert!(
        buffer.is_empty(),
        "the header is written whole or not at all"
    );
}

#[test]
fn strings_escape_quote_backslash_control_and_pass_utf8_through() {
    let mut marked = object("1");
    marked.label = "a\"b\\c\nd\te\u{1}f\u{1f}g\u{e9}h\u{1f600}".to_owned();
    let text = frame_line(&frame_of(12.5, vec![marked], Vec::new()));
    assert!(
        text.contains("\"label\":\"a\\\"b\\\\c\\nd\\te\\u0001f\\u001fg\u{e9}h\u{1f600}\""),
        "{text}"
    );
}

#[test]
fn a_file_of_header_and_three_frames_has_one_newline_per_object_and_no_trailing_blank() {
    let mut buffer: Vec<u8> = Vec::new();
    {
        let mut writer = RecordWriter::new(&mut buffer, &header()).unwrap();
        writer
            .write_frame(&frame_of(1.0, Vec::new(), Vec::new()))
            .unwrap();
        writer
            .write_frame(&frame_of(2.0, vec![object("1")], vec!["0"]))
            .unwrap();
        writer
            .write_frame(&frame_of(3.0, Vec::new(), Vec::new()))
            .unwrap();
    }
    let text = String::from_utf8(buffer.clone()).unwrap();
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(
        lines.len(),
        5,
        "four lines plus the empty piece after the final newline"
    );
    assert!(
        lines[..4]
            .iter()
            .all(|line| line.starts_with('{') && line.ends_with('}'))
    );
    assert!(lines[4].is_empty());
    assert_eq!(buffer.last(), Some(&b'\n'));
    assert_ne!(buffer[buffer.len() - 2], b'\n');
}

#[test]
fn two_writers_over_the_same_frames_produce_identical_bytes() {
    let frames = vec![
        frame_of(1.0, Vec::new(), Vec::new()),
        frame_of(2.0, vec![object("1")], vec!["0"]),
        frame_of(3.0, vec![object("2")], Vec::new()),
    ];
    let first = collect(&frames);
    let second = collect(&frames);
    assert_eq!(first, second);
}

fn collect(frames: &[FrameObjects]) -> Vec<u8> {
    let mut buffer: Vec<u8> = Vec::new();
    {
        let mut writer = RecordWriter::new(&mut buffer, &header()).unwrap();
        for frame in frames {
            writer.write_frame(frame).unwrap();
        }
    }
    buffer
}

#[test]
fn io_failures_report_the_error_text_and_surface_no_panic() {
    let error = RecordWriter::new(FailingWriter::default(), &header()).unwrap_err();
    match error {
        RecordError::Io(text) => assert!(text.contains("disk gone"), "{text}"),
        RecordError::NonFinite(field) => panic!("expected Io, got NonFinite({field})"),
    }
    let mut writer = RecordWriter::new(FailingWriter { allow_first: true }, &header()).unwrap();
    let error = writer
        .write_frame(&frame_of(1.0, Vec::new(), Vec::new()))
        .unwrap_err();
    match error {
        RecordError::Io(text) => assert!(text.contains("disk gone"), "{text}"),
        RecordError::NonFinite(field) => panic!("expected Io, got NonFinite({field})"),
    }
}

#[derive(Clone, Default)]
struct SharedBuffer {
    bytes: std::rc::Rc<std::cell::RefCell<Vec<u8>>>,
}

impl SharedBuffer {
    fn len(&self) -> usize {
        self.bytes.borrow().len()
    }

    fn bytes(&self) -> Vec<u8> {
        self.bytes.borrow().clone()
    }
}

impl io::Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct FailingWriter {
    allow_first: bool,
}

impl io::Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("disk gone"))
    }

    fn write_all(&mut self, _buf: &[u8]) -> io::Result<()> {
        if self.allow_first {
            self.allow_first = false;
            return Ok(());
        }
        Err(io::Error::other("disk gone"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn frame_and_object_keys_keep_the_contract_order() {
    let text = frame_line(&frame_of(12.5, vec![object("1")], vec!["0"]));
    let mut cursor = 0;
    for key in ["\"frame_time\":", "\"objects\":", "\"ended\":"] {
        let found = cursor + text[cursor..].find(key).unwrap();
        assert!(found > cursor, "{key} follows the previous key");
        cursor = found + key.len();
    }
    cursor = text.find("\"objects\":").unwrap() + "\"objects\":".len();
    for key in [
        "\"id\":",
        "\"label\":",
        "\"score\":",
        "\"confirmed\":",
        "\"box\":",
        "\"first_seen\":",
        "\"last_seen\":",
        "\"zones_now\":",
        "\"zones_entered\":",
        "\"state\":",
    ] {
        let found = cursor + text[cursor..].find(key).unwrap();
        assert!(found > cursor, "{key} follows the previous key");
        cursor = found + key.len();
    }
}
