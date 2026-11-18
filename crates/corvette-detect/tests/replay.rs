// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and `clippy.toml` does not forbid them.
use corvette_detect::{
    CandidateStage, EngineError, Infer, Labels, ObjectStage, RecordHeader, RecordWriter,
    RegionStage, RegionStageError, RegionStageSettings, ReplayError, ReplayPipeline,
};
use corvette_detect_proto::{Row, TENSOR_BYTES};
use corvette_motion::{MotionSettings, Region, Shape, SizeGrid};
use corvette_track::{IdSource, ObjectFilter, ObjectFilters, TrackerSettings, ZoneSet};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::rc::Rc;
const WIDTH: usize = 160;
const HEIGHT: usize = 120;
const FRAME_BYTES: usize = WIDTH * HEIGHT * 3 / 2;
const EDGE_OBJECT: Region = Region {
    x1: 158,
    y1: 79,
    x2: 159,
    y2: 119,
};
const OBJ2_BOX: &str = "\"id\":\"2\",\"label\":\"person\",\"score\":0.8999999761581421,\"confirmed\":true,\"box\":[0.8375,0.675,0.95625,0.8916666666666667]";
const EDGE_BOX: &str = "\"box\":[0.9875,0.6583333333333333,0.99375,0.9916666666666667]";
fn flat() -> Vec<u8> {
    vec![128; FRAME_BYTES]
}
fn rect_frame(rows: std::ops::Range<usize>, cols: std::ops::Range<usize>) -> Vec<u8> {
    let mut frame = flat();
    for row in rows {
        for column in cols.clone() {
            frame[row * WIDTH + column] = 250;
        }
    }
    frame
}
const fn person_row() -> Row {
    Row {
        class_id: 0,
        score: 0.9,
        y_min: 0.25,
        x_min: 0.5,
        y_max: 0.75,
        x_max: 0.875,
    }
}
fn stage_with(motion_enabled: bool) -> RegionStage {
    let settings = RegionStageSettings {
        motion: MotionSettings::default(),
        floor: 32,
        grid: SizeGrid::empty(),
        model: Shape::new(640, 640).unwrap(),
        motion_enabled,
    };
    RegionStage::new(settings, Shape::new(WIDTH, HEIGHT).unwrap(), None).unwrap()
}
fn candidate_stage() -> CandidateStage {
    let mut filters = BTreeMap::new();
    filters.insert("person".to_owned(), ObjectFilter::default());
    let track = vec!["person".to_owned()];
    CandidateStage::new(
        Labels::parse("0 person").unwrap(),
        ObjectFilters::new(track, filters),
        160,
        120,
    )
    .unwrap()
}
const fn tracker() -> TrackerSettings {
    TrackerSettings {
        width: 160,
        height: 120,
        min_initialized: 0,
        max_disappeared: 5,
        stationary_threshold: 100,
        max_frames_default: None,
        max_frames_overrides: Vec::new(),
        own_labels: Vec::new(),
    }
}
fn header() -> RecordHeader {
    RecordHeader {
        camera: "cam".to_owned(),
        segment: "seg".to_owned(),
        frame_width: 160,
        frame_height: 120,
        detect_fps: 5.0,
    }
}
const HEADER_LINE: &str = "{\"schema\":1,\"camera\":\"cam\",\"segment\":\"seg\",\"frame_width\":160,\"frame_height\":120,\"detect_fps\":5.0}";
struct ScriptEngine {
    person: Rc<Cell<bool>>,
    error_at: Option<usize>,
    round: usize,
    calls: Rc<Cell<usize>>,
}
const fn engine(
    person: Rc<Cell<bool>>,
    calls: Rc<Cell<usize>>,
    error_at: Option<usize>,
) -> ScriptEngine {
    ScriptEngine {
        person,
        error_at,
        round: 0,
        calls,
    }
}
impl Infer for ScriptEngine {
    fn infer(&mut self, _tensor: &[u8; TENSOR_BYTES]) -> Result<Vec<Row>, EngineError> {
        self.round += 1;
        self.calls.set(self.round);
        if self.error_at == Some(self.round) {
            return Err(EngineError::Io("scripted engine failure".to_owned()));
        }
        if self.person.get() {
            Ok(vec![person_row()])
        } else {
            Ok(Vec::new())
        }
    }
}
struct Harness {
    pipeline: ReplayPipeline<'static, ScriptEngine, Vec<u8>>,
    person: Rc<Cell<bool>>,
    calls: Rc<Cell<usize>>,
}
impl Harness {
    fn build(error_at: Option<usize>) -> Self {
        let person = Rc::new(Cell::new(false));
        let calls = Rc::new(Cell::new(0));
        let ids: &'static mut IdSource = Box::leak(Box::new(IdSource::default()));
        let objects = ObjectStage::new(tracker(), ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
        let writer = RecordWriter::new(Vec::new(), &header()).unwrap();
        let engine = engine(Rc::clone(&person), Rc::clone(&calls), error_at);
        let pipeline =
            ReplayPipeline::new(stage_with(true), candidate_stage(), objects, engine, writer);
        Self {
            pipeline,
            person,
            calls,
        }
    }
    fn step(&mut self, time: u32, frame: &[u8]) -> Result<usize, ReplayError> {
        let before = self.calls.get();
        self.pipeline.step(f64::from(time), frame)?;
        Ok(self.calls.get() - before)
    }
    fn lines(self) -> Vec<String> {
        String::from_utf8(self.pipeline.finish())
            .unwrap()
            .lines()
            .map(String::from)
            .collect()
    }
}
struct BudgetWriter {
    budget: usize,
    written: Vec<u8>,
}
impl Write for BudgetWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let take = self.budget.min(buf.len());
        if take == 0 {
            return Err(io::Error::other("sink budget spent"));
        }
        self.budget -= take;
        self.written.extend_from_slice(&buf[..take]);
        Ok(take)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn first_frame_with_no_rows_writes_header_plus_one_line_without_objects() {
    let mut harness = Harness::build(None);
    assert_eq!(harness.step(0, &flat()).unwrap(), 1);
    let lines = harness.lines();
    assert_eq!(lines[0], HEADER_LINE);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1], "{\"frame_time\":0.0,\"objects\":[],\"ended\":[]}");
}
#[test]
fn moving_rect_with_rows_registers_one_confirmed_person_listed_later() {
    let mut harness = Harness::build(None);
    harness.step(0, &flat()).unwrap();
    for (time, left) in [(1u32, 60), (2u32, 62), (3u32, 64), (4u32, 66), (5u32, 68)] {
        harness.person.set(true);
        let frame = rect_frame(30..90, left..left + 60);
        harness.step(time, &frame).unwrap();
    }
    harness.person.set(false);
    for time in 6u32..14 {
        harness.step(time, &flat()).unwrap();
    }
    let lines = harness.lines();
    let first_object = lines
        .iter()
        .position(|line| line.contains("\"id\":\"1\""))
        .unwrap();
    assert_eq!(first_object, 2, "the rect frame first registers the person");
    let line = &lines[first_object];
    assert!(line.contains("\"label\":\"person\"") && line.contains("\"confirmed\":true"));
    assert_eq!(
        line.matches("\"id\":").count(),
        1,
        "one object across the run"
    );
    assert!(line.contains("\"first_seen\":1.0"));
    assert!(lines[first_object + 1].contains("\"id\":\"1\""));
    assert!(lines[first_object + 2].contains("\"id\":\"1\""));
    let ended_once = lines
        .iter()
        .filter(|line| line.contains("\"ended\":[\"1\"]"))
        .count();
    assert_eq!(ended_once, 1, "the id ends exactly once");
}
#[test]
fn retiring_object_feeds_back_regions_without_detections_then_ends_once() {
    let mut harness = Harness::build(None);
    harness.person.set(true);
    assert_eq!(harness.step(0, &flat()).unwrap(), 1);
    harness.person.set(false);
    assert_eq!(
        harness.step(1, &flat()).unwrap(),
        2,
        "feedback is requested without a detection"
    );
    assert_eq!(harness.step(2, &flat()).unwrap(), 2);
    let mut retired_counts = Vec::new();
    for time in 3u32..8 {
        retired_counts.push(harness.step(time, &flat()).unwrap());
    }
    assert_eq!(
        retired_counts,
        vec![1, 1, 1, 1, 1],
        "after retirement only the no-box motion region the region stage alone reports stays"
    );
    let lines = harness.lines();
    assert!(
        lines[2].contains("\"id\":\"1\""),
        "the kept-unseen object is listed at step one"
    );
    let ends: Vec<&String> = lines
        .iter()
        .filter(|l| l.contains("\"ended\":[\"1\"]"))
        .collect();
    assert_eq!(ends.len(), 1, "the id ends exactly once");
    assert!(ends[0].contains("\"objects\":[]"));
}
#[test]
fn previous_object_adds_one_region_to_the_next_frame_requests() {
    let shadow = |boxes: &[Region]| {
        let mut stage = stage_with(true);
        stage.step(&flat(), &[]).unwrap();
        stage.step(&flat(), boxes).unwrap().len()
    };
    assert_eq!((shadow(&[]), shadow(&[EDGE_OBJECT])), (1, 2));
    let mut control = Harness::build(None);
    assert_eq!(control.step(0, &flat()).unwrap(), 1);
    assert_eq!(control.step(1, &flat()).unwrap(), 1);
    let mut object_run = Harness::build(None);
    object_run.person.set(true);
    assert_eq!(object_run.step(0, &flat()).unwrap(), 1);
    object_run.person.set(false);
    assert_eq!(object_run.step(1, &flat()).unwrap(), 2);
}
#[test]
fn edge_touching_object_feeds_back_and_bumps_the_next_request_count() {
    let mut harness = Harness::build(None);
    harness.person.set(true);
    assert_eq!(harness.step(0, &flat()).unwrap(), 1);
    assert_eq!(harness.step(1, &flat()).unwrap(), 2);
    assert_eq!(harness.step(2, &flat()).unwrap(), 3);
    let lines = harness.lines();
    assert!(lines[1].contains(EDGE_BOX));
    assert!(lines[2].contains(OBJ2_BOX));
}
#[test]
fn two_pipelines_on_the_same_frames_and_engine_write_identical_bytes() {
    let mut runs = Vec::new();
    for _ in 0..2 {
        let mut harness = Harness::build(None);
        harness.person.set(true);
        harness.step(0, &flat()).unwrap();
        harness.person.set(false);
        for time in 1u32..20 {
            harness.step(time, &flat()).unwrap();
        }
        runs.push(harness.pipeline.finish());
    }
    assert!(!runs[0].is_empty() && runs[0] == runs[1]);
}
#[test]
fn engine_error_stops_at_the_frame_and_a_short_frame_buffer_is_a_region_error() {
    let mut harness = Harness::build(Some(2));
    harness.person.set(true);
    harness.step(0, &flat()).unwrap();
    let error = harness.step(1, &flat()).unwrap_err();
    assert!(matches!(error, ReplayError::Candidate(_)), "got {error:?}");
    assert!(std::error::Error::source(&error).is_some());
    let lines = harness.lines();
    assert_eq!(lines.len(), 2, "the failed frame wrote no line");
    assert_eq!(lines[0], HEADER_LINE);
    assert!(lines[1].contains("\"id\":\"1\""));
    let mut fresh = Harness::build(None);
    let short = &flat()[..FRAME_BYTES / 2];
    let error = fresh.step(0, short).unwrap_err();
    let ReplayError::Region(RegionStageError::WrongFrameLength { expected, got }) = error else {
        panic!("wrong error: {error:?}");
    };
    assert_eq!((expected, got), (28_800, 14_400));
}
#[test]
fn writer_failure_after_the_header_surfaces_as_a_record_error() {
    let mut ids = IdSource::default();
    let objects = ObjectStage::new(tracker(), &mut ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    let sink = BudgetWriter {
        budget: HEADER_LINE.len() + 1,
        written: Vec::new(),
    };
    let writer = RecordWriter::new(sink, &header()).expect("header fits the budget");
    let engine = engine(Rc::new(Cell::new(false)), Rc::new(Cell::new(0)), None);
    let mut pipeline =
        ReplayPipeline::new(stage_with(true), candidate_stage(), objects, engine, writer);
    let error = pipeline.step(0.0, &flat()).unwrap_err();
    assert!(matches!(error, ReplayError::Record(_)), "got {error:?}");
    let text = String::from_utf8(pipeline.finish().written).unwrap();
    assert_eq!(text, format!("{HEADER_LINE}\n"));
}
