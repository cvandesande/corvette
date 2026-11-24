use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::camera_events::transition_event;
use crate::candidate_stage::Infer;
use crate::frame_slot::{SlotEvent, SlotReceiver};
use crate::live_pipeline::{FrameOutcome, LiveError, LivePipeline, PipelineCounters};
use crate::log_event;
use crate::object_events::ObjectLog;
use crate::object_stage::FrameObjects;
use crate::rate_window::RateWindow;
use crate::shared_link::LinkTimings;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CameraState {
    Running,
    WaitingForFrames,
    InferenceUnavailable,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CameraStatus {
    pub state: CameraState,
    pub counters: PipelineCounters,
    pub source_gaps: u64,
    pub frames_dropped: u64,
    pub runner_restarts: u32,
    pub fps_in_tenths: u32,
    pub fps_processed_tenths: u32,
    pub worker_ms: u32,
    pub roundtrip_ms: u32,
    pub lock_wait_ms: u32,
}

#[derive(Clone, Debug)]
pub struct CameraHandle {
    pub status: watch::Receiver<CameraStatus>,
    pub objects: watch::Receiver<Option<FrameObjects>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepEnd {
    Frame,
    Gap,
    Stopped,
}

#[derive(Debug)]
pub enum RunnerError {
    Pipeline(LiveError),
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pipeline(error) => write!(f, "camera runner: {error}"),
        }
    }
}

impl std::error::Error for RunnerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pipeline(error) => Some(error),
        }
    }
}

impl From<LiveError> for RunnerError {
    fn from(error: LiveError) -> Self {
        Self::Pipeline(error)
    }
}

#[derive(Debug)]
pub struct CameraRunner<'a, E: Infer> {
    slot: SlotReceiver,
    pipeline: LivePipeline<'a, E>,
    frame_timeout: Duration,
    source_gaps: u64,
    runner_restarts: u32,
    status: watch::Sender<CameraStatus>,
    objects: watch::Sender<Option<FrameObjects>>,
    window: RateWindow,
    timings: Option<LinkTimings>,
    log_unit: Option<String>,
    object_log: ObjectLog,
    last_state: Option<CameraState>,
    left_running: Option<Instant>,
}

impl<'a, E: Infer> CameraRunner<'a, E> {
    #[must_use]
    pub fn new(
        slot: SlotReceiver,
        pipeline: LivePipeline<'a, E>,
        frame_timeout: Duration,
    ) -> (Self, CameraHandle) {
        let initial = CameraStatus {
            state: CameraState::WaitingForFrames,
            counters: PipelineCounters::default(),
            source_gaps: 0,
            frames_dropped: 0,
            runner_restarts: 0,
            fps_in_tenths: 0,
            fps_processed_tenths: 0,
            worker_ms: 0,
            roundtrip_ms: 0,
            lock_wait_ms: 0,
        };
        let (status, status_rx) = watch::channel(initial);
        let (objects, objects_rx) = watch::channel(None);
        let arrived_at_start = slot.arrived();
        (
            Self {
                slot,
                pipeline,
                frame_timeout,
                source_gaps: 0,
                runner_restarts: 0,
                status,
                objects,
                window: RateWindow::new(Instant::now(), 0, arrived_at_start),
                timings: None,
                log_unit: None,
                object_log: ObjectLog::new(),
                last_state: None,
                left_running: None,
            },
            CameraHandle {
                status: status_rx,
                objects: objects_rx,
            },
        )
    }

    /// Carries this link's timings into every published status; without it the timings publish as zero.
    #[must_use]
    pub fn with_timings(mut self, timings: LinkTimings) -> Self {
        self.timings = Some(timings);
        self
    }

    /// Publishes the waiting status once on these channels; every status this runner publishes carries `restarts`.
    #[must_use]
    pub fn publishing_to(
        mut self,
        status: watch::Sender<CameraStatus>,
        objects: watch::Sender<Option<FrameObjects>>,
        restarts: u32,
    ) -> Self {
        self.status = status;
        self.objects = objects;
        self.runner_restarts = restarts;
        self.publish(CameraState::WaitingForFrames);
        self
    }

    /// Emits a lifecycle log line for each state transition this runner publishes, under `unit`; without it no line is emitted.
    #[must_use]
    pub fn logged_as(mut self, unit: &str) -> Self {
        self.log_unit = Some(unit.to_string());
        self
    }

    /// # Errors
    /// A slot timeout or gap is reported as a gap, not an error; only a fatal stage error is returned, wrapped in [`RunnerError::Pipeline`], and it leaves the published status and objects untouched.
    pub fn step_once(&mut self) -> Result<StepEnd, RunnerError> {
        match self.slot.take(self.frame_timeout) {
            SlotEvent::Timeout | SlotEvent::Gap => Ok(self.record_gap()),
            SlotEvent::Closed => Ok(StepEnd::Stopped),
            SlotEvent::Frame(frame) => {
                match self.pipeline.step(frame.time.as_secs_f64(), &frame.data) {
                    Ok(FrameOutcome::Objects(objects)) => {
                        let object_lines = self.object_log.observe(&objects);
                        if let Some(unit) = self.log_unit.as_deref() {
                            for (event, detail) in object_lines {
                                log_event(unit, event, detail);
                            }
                        }
                        let _ = self.objects.send_replace(Some(objects));
                        self.publish(CameraState::Running);
                        Ok(StepEnd::Frame)
                    }
                    Ok(FrameOutcome::InferenceUnavailable) => {
                        let _ = self.objects.send_replace(None);
                        self.publish(CameraState::InferenceUnavailable);
                        Ok(StepEnd::Frame)
                    }
                    Err(error) => Err(RunnerError::Pipeline(error)),
                }
            }
        }
    }

    /// # Errors
    /// The first fatal pipeline error, wrapped in [`RunnerError::Pipeline`].
    pub fn run(mut self) -> Result<(), RunnerError> {
        loop {
            match self.step_once() {
                Ok(StepEnd::Stopped) => return Ok(()),
                Ok(StepEnd::Frame | StepEnd::Gap) => {}
                Err(error) => {
                    self.publish(CameraState::Failed);
                    return Err(error);
                }
            }
        }
    }

    fn record_gap(&mut self) -> StepEnd {
        self.source_gaps = self.source_gaps.saturating_add(1);
        let _ = self.objects.send_replace(None);
        self.publish(CameraState::WaitingForFrames);
        StepEnd::Gap
    }

    fn publish(&mut self, state: CameraState) {
        let counters = self.pipeline.counters();
        self.window
            .observe(Instant::now(), counters.frames_in, self.slot.arrived());
        let rates = self.window.rates();
        let (worker_ms, roundtrip_ms, lock_wait_ms) =
            self.timings.as_ref().map_or((0, 0, 0), |timings| {
                (
                    timings.worker_ms(),
                    timings.roundtrip_ms(),
                    timings.lock_wait_ms(),
                )
            });
        self.status.send_replace(CameraStatus {
            state,
            counters,
            source_gaps: self.source_gaps,
            frames_dropped: self.slot.dropped(),
            runner_restarts: self.runner_restarts,
            fps_in_tenths: rates.fps_in_tenths,
            fps_processed_tenths: rates.fps_processed_tenths,
            worker_ms,
            roundtrip_ms,
            lock_wait_ms,
        });
        let down_ms = self
            .left_running
            .map(|left| u64::try_from(left.elapsed().as_millis()).unwrap_or(u64::MAX));
        let timeout_ms = u64::try_from(self.frame_timeout.as_millis()).unwrap_or(u64::MAX);
        let transition = transition_event(self.last_state, state, down_ms, timeout_ms);
        if let Some((unit, (event, detail))) = self.log_unit.as_deref().zip(transition) {
            log_event(unit, event, detail);
        }
        if state == CameraState::Running {
            self.left_running = None;
        } else if self.last_state == Some(CameraState::Running) {
            self.left_running = Some(Instant::now());
        }
        self.last_state = Some(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::{Decode, FrameSpec};
    use crate::frame_slot::{SlotSender, frame_slot};
    use crate::reader::Frame;
    use crate::{
        CandidateStage, EngineError, Labels, LivePipeline, ObjectStage, RegionStage,
        RegionStageSettings,
    };
    use corvette_detect_proto::{Row, TENSOR_BYTES};
    use corvette_motion::{MotionSettings, Shape, SizeGrid};
    use corvette_track::{IdSource, ObjectFilter, ObjectFilters, TrackerSettings, ZoneSet};
    use std::cell::Cell;
    use std::rc::Rc;

    fn black_frame(index: u64) -> Frame {
        let d = FrameSpec::new(160, 120, 10, Decode::Cpu)
            .unwrap()
            .frame_bytes();
        Frame {
            index,
            time: Duration::from_millis(100 * (index + 1)),
            data: vec![0; d],
        }
    }

    struct QuietEngine;

    impl Infer for QuietEngine {
        fn infer(&mut self, _tensor: &[u8; TENSOR_BYTES]) -> Result<Vec<Row>, EngineError> {
            Ok(Vec::new())
        }
    }

    fn pipeline() -> LivePipeline<'static, QuietEngine> {
        pipeline_with_engine(QuietEngine)
    }

    fn pipeline_with_engine<E: Infer>(engine: E) -> LivePipeline<'static, E> {
        let stage = RegionStage::new(
            RegionStageSettings {
                motion: MotionSettings::default(),
                floor: 32,
                grid: SizeGrid::empty(),
                model: Shape::new(640, 640).expect("model shape"),
                motion_enabled: true,
            },
            Shape::new(160, 120).expect("frame shape"),
            None,
        )
        .expect("region stage");
        let mut filters = std::collections::BTreeMap::new();
        filters.insert("person".to_owned(), ObjectFilter::default());
        let candidates = CandidateStage::new(
            Labels::parse("0 person").expect("labels"),
            ObjectFilters::new(vec!["person".to_owned()], filters),
            160,
            120,
        )
        .expect("candidate stage");
        let ids: &'static mut IdSource = Box::leak(Box::new(IdSource::default()));
        let tracker = TrackerSettings {
            width: 160,
            height: 120,
            min_initialized: 0,
            max_disappeared: 5,
            stationary_threshold: 100,
            max_frames_default: None,
            max_frames_overrides: Vec::new(),
            own_labels: Vec::new(),
        };
        let objects = ObjectStage::new(tracker, ids, ZoneSet::new(Vec::new()), 0.5).unwrap();
        LivePipeline::new(stage, candidates, objects, engine)
    }

    fn drive(runner: &mut CameraRunner<'_, QuietEngine>, sender: &SlotSender) {
        sender.put(black_frame(0));
        assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
        sender.gap();
        assert!(matches!(runner.step_once(), Ok(StepEnd::Gap)));
        sender.put(black_frame(1));
        assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
    }

    fn lines_for(unit: &str) -> Vec<String> {
        let log = crate::TEST_LOG
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        log.iter()
            .filter(|l| l.contains(&format!("unit=\"{unit}\"")))
            .cloned()
            .collect()
    }

    fn assert_line(lines: &[String], i: usize, needle: &str) {
        assert!(lines[i].contains(needle), "{lines:?}");
    }

    #[test]
    fn camera_runner_logs_its_transitions_through_publish() {
        let (sender, receiver) = frame_slot();
        let (runner, _handle) = CameraRunner::new(receiver, pipeline(), Duration::from_millis(250));
        let mut runner = runner.logged_as("wiring-cam");
        drive(&mut runner, &sender);
        let lines = lines_for("wiring-cam");
        assert_eq!(lines.len(), 3, "one line per transition: {lines:?}");
        assert_line(&lines, 0, "event=connected detail=\"first_frame\"");
        assert_line(
            &lines,
            1,
            "event=source-gap detail=\"no_frames_for_ms=250\"",
        );
        assert_line(&lines, 2, "event=connected detail=\"down_ms=");
        let n = lines[2].rsplit('=').next().unwrap().trim_end_matches('"');
        assert!(n.parse::<u64>().is_ok(), "{n} is not an integer");
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

    struct PersonEngine {
        person: Rc<Cell<bool>>,
    }

    impl Infer for PersonEngine {
        fn infer(&mut self, _tensor: &[u8; TENSOR_BYTES]) -> Result<Vec<Row>, EngineError> {
            if self.person.get() {
                Ok(vec![person_row()])
            } else {
                Ok(Vec::new())
            }
        }
    }

    fn flat_frame(index: u64) -> Frame {
        let d = FrameSpec::new(160, 120, 10, Decode::Cpu)
            .unwrap()
            .frame_bytes();
        Frame {
            index,
            time: Duration::from_millis(100 * (index + 1)),
            data: vec![0x80; d],
        }
    }

    #[test]
    fn object_lines_reach_the_log_through_step_once() {
        let (sender, receiver) = frame_slot();
        let person = Rc::new(Cell::new(true));
        let engine = PersonEngine {
            person: Rc::clone(&person),
        };
        let (runner, _handle) = CameraRunner::new(
            receiver,
            pipeline_with_engine(engine),
            Duration::from_millis(250),
        );
        let mut runner = runner.logged_as("objects-cam");
        sender.put(flat_frame(0));
        assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
        person.set(false);
        for index in 1..5 {
            sender.put(flat_frame(index));
            assert!(matches!(runner.step_once(), Ok(StepEnd::Frame)));
        }
        let lines = lines_for("objects-cam");
        assert_eq!(
            lines,
            [
                "corvette-detect unit=\"objects-cam\" event=object-confirmed \
                 detail=\"id=1 label=person score_pct=90 zones=0\"",
                "corvette-detect unit=\"objects-cam\" event=connected detail=\"first_frame\"",
                "corvette-detect unit=\"objects-cam\" event=object-ended \
                 detail=\"id=1 label=person seconds=0\"",
            ]
        );
    }
}
