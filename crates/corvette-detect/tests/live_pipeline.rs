// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and `clippy.toml` does not forbid them.
use corvette_detect::{
    CandidateStage, DEFAULT_POLICY, EngineError, FrameObjects, FrameOutcome, Infer, Labels,
    LiveError, LivePipeline, ObjectStage, RegionStage, RegionStageError, RegionStageSettings,
    TrackedObject, WorkerLink,
};
use corvette_detect_proto::{Row, TENSOR_BYTES};
use corvette_motion::{MotionSettings, Region, Shape, SizeGrid};
use corvette_track::{IdSource, ObjectFilter, ObjectFilters, TrackerSettings, ZoneSet};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

const WIDTH: usize = 160;
const HEIGHT: usize = 120;
const FRAME_BYTES: usize = WIDTH * HEIGHT * 3 / 2;

fn flat() -> Vec<u8> {
    vec![128; FRAME_BYTES]
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

#[derive(Clone, Copy)]
enum Failure {
    Io,
    Worker,
}

struct Script {
    person: Rc<Cell<bool>>,
    failure: Rc<Cell<Option<Failure>>>,
    calls: Rc<Cell<usize>>,
}

impl Infer for Script {
    fn infer(&mut self, _tensor: &[u8; TENSOR_BYTES]) -> Result<Vec<Row>, EngineError> {
        self.calls.set(self.calls.get() + 1);
        if let Some(kind) = self.failure.get() {
            return Err(match kind {
                Failure::Io => EngineError::Io("scripted engine failure".to_owned()),
                Failure::Worker => EngineError::Worker("scripted worker failure".to_owned()),
            });
        }
        if self.person.get() {
            Ok(vec![person_row()])
        } else {
            Ok(Vec::new())
        }
    }
}

struct Harness {
    pipeline: LivePipeline<'static, Script>,
    person: Rc<Cell<bool>>,
    failure: Rc<Cell<Option<Failure>>>,
    calls: Rc<Cell<usize>>,
}

impl Harness {
    fn build() -> Self {
        let person = Rc::new(Cell::new(false));
        let failure = Rc::new(Cell::new(None));
        let calls = Rc::new(Cell::new(0));
        let ids: &'static mut IdSource = Box::leak(Box::new(IdSource::default()));
        let objects = ObjectStage::new(tracker(), ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
        let pipeline = LivePipeline::new(
            stage_with(true),
            candidate_stage(),
            objects,
            Script {
                person: Rc::clone(&person),
                failure: Rc::clone(&failure),
                calls: Rc::clone(&calls),
            },
        );
        Self {
            pipeline,
            person,
            failure,
            calls,
        }
    }

    fn step(&mut self, time: u32, frame: &[u8]) -> Result<(FrameOutcome, usize), LiveError> {
        let before = self.calls.get();
        let outcome = self.pipeline.step(f64::from(time), frame)?;
        Ok((outcome, self.calls.get() - before))
    }
}

#[test]
fn good_frames_all_report_objects_and_none_count_as_inference_failure() {
    let mut harness = Harness::build();
    harness.person.set(true);
    for time in 0u32..5 {
        let (outcome, _) = harness.step(time, &flat()).unwrap();
        assert!(
            matches!(outcome, FrameOutcome::Objects(_)),
            "frame {time} lost its tracking"
        );
    }
    let counters = harness.pipeline.counters();
    assert_eq!(counters.frames_in, 5);
    assert_eq!(counters.frames_tracked, 5);
    assert_eq!(counters.frames_inference_failed, 0);
}

#[test]
fn inference_failure_between_good_frames_reports_unavailable_and_keeps_the_object_id() {
    let mut harness = Harness::build();
    harness.person.set(true);
    let (outcome, calls) = harness.step(0, &flat()).unwrap();
    assert!(matches!(outcome, FrameOutcome::Objects(_)));
    assert_eq!(calls, 1, "the first flat frame requests one region");
    let (outcome, calls) = harness.step(1, &flat()).unwrap();
    assert_eq!(calls, 2, "the kept object requests its region too");
    let FrameOutcome::Objects(frame_objects) = outcome else {
        panic!("the second frame must track");
    };
    let id = frame_objects.objects[0].id.clone();
    harness.person.set(false);
    let (outcome, _) = harness.step(2, &flat()).unwrap();
    assert!(matches!(outcome, FrameOutcome::Objects(_)));
    harness.failure.set(Some(Failure::Io));
    let (outcome, calls) = harness.step(3, &flat()).unwrap();
    assert_eq!(calls, 1, "the failing frame stops at its first tensor");
    assert!(
        matches!(outcome, FrameOutcome::InferenceUnavailable),
        "a failed inference must not list objects"
    );
    harness.failure.set(None);
    let (outcome, calls) = harness.step(4, &flat()).unwrap();
    assert!(
        calls > 1,
        "the failure must keep the object region requested before it"
    );
    let FrameOutcome::Objects(frame_objects) = outcome else {
        panic!("the frame after the failure must track");
    };
    assert!(
        frame_objects.objects.iter().any(|object| object.id == id),
        "the tracker must keep the id it had before the failure"
    );
    let counters = harness.pipeline.counters();
    assert_eq!(
        (
            counters.frames_in,
            counters.frames_tracked,
            counters.frames_inference_failed
        ),
        (5, 4, 1)
    );
}

#[test]
fn every_frame_failing_reports_only_inference_unavailable() {
    let mut harness = Harness::build();
    harness.failure.set(Some(Failure::Io));
    for time in 0u32..4 {
        let (outcome, _) = harness.step(time, &flat()).unwrap();
        assert!(
            !matches!(outcome, FrameOutcome::Objects(_)),
            "frame {time} must not report objects"
        );
    }
    let counters = harness.pipeline.counters();
    assert_eq!(counters.frames_in, 4);
    assert_eq!(counters.frames_tracked, 0);
    assert_eq!(counters.frames_inference_failed, 4);
}

#[test]
fn a_missing_worker_socket_reports_unavailable_on_every_frame() {
    let ids: &'static mut IdSource = Box::leak(Box::new(IdSource::default()));
    let objects = ObjectStage::new(tracker(), ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
    let link = WorkerLink::new(
        PathBuf::from("/no/such/dir/corvette-worker.sock"),
        Duration::from_secs(5),
        DEFAULT_POLICY,
    );
    let mut pipeline = LivePipeline::new(stage_with(true), candidate_stage(), objects, link);
    for time in 0u32..3 {
        let outcome = pipeline.step(f64::from(time), &flat()).unwrap();
        assert!(
            matches!(outcome, FrameOutcome::InferenceUnavailable),
            "frame {time} saw a live worker"
        );
    }
    let counters = pipeline.counters();
    assert_eq!(
        (counters.frames_in, counters.frames_inference_failed),
        (3, 3)
    );
    assert_eq!(counters.frames_tracked, 0);
}

#[test]
fn a_worker_error_reply_counts_as_inference_unavailable() {
    let mut harness = Harness::build();
    harness.failure.set(Some(Failure::Worker));
    let (outcome, _) = harness.step(0, &flat()).unwrap();
    assert!(matches!(outcome, FrameOutcome::InferenceUnavailable));
    let counters = harness.pipeline.counters();
    assert_eq!(
        (counters.frames_in, counters.frames_inference_failed),
        (1, 1)
    );
}

#[test]
fn a_short_frame_is_a_fatal_error_and_not_inference_unavailable() {
    let mut harness = Harness::build();
    let short = &flat()[..FRAME_BYTES / 2];
    let error = harness.step(0, short).unwrap_err();
    let LiveError::Region(RegionStageError::WrongFrameLength { expected, got }) = error else {
        panic!("wrong error: {error:?}");
    };
    assert_eq!((expected, got), (28_800, 14_400));
    assert!(std::error::Error::source(&error).is_some());
    assert_eq!(harness.pipeline.counters().frames_in, 1);
}

#[test]
fn regions_maps_object_edges_in_object_order() {
    let tracked = |id: &str, edges: [i32; 4]| TrackedObject {
        id: id.to_owned(),
        label: "person".to_owned(),
        score: 0.5,
        confirmed: true,
        bounds: [0.0; 4],
        edges,
        first_seen: 0.0,
        last_seen: 0.0,
        zones_now: Vec::new(),
        zones_entered: Vec::new(),
        stationary: false,
    };
    let frame_objects = FrameObjects {
        frame_time: 0.0,
        objects: vec![tracked("1", [1, 2, 3, 4]), tracked("2", [5, 6, 7, 8])],
        ended: Vec::new(),
    };
    assert_eq!(
        frame_objects.regions(),
        vec![
            Region {
                x1: 1,
                y1: 2,
                x2: 3,
                y2: 4
            },
            Region {
                x1: 5,
                y1: 6,
                x2: 7,
                y2: 8
            },
        ]
    );
}
