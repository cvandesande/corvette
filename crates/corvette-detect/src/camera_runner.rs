use std::time::Duration;

use tokio::sync::watch;

use crate::candidate_stage::Infer;
use crate::frame_slot::{SlotEvent, SlotReceiver};
use crate::live_pipeline::{FrameOutcome, LiveError, LivePipeline, PipelineCounters};
use crate::object_stage::FrameObjects;

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
    status: watch::Sender<CameraStatus>,
    objects: watch::Sender<Option<FrameObjects>>,
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
        };
        let (status, status_rx) = watch::channel(initial);
        let (objects, objects_rx) = watch::channel(None);
        (
            Self {
                slot,
                pipeline,
                frame_timeout,
                source_gaps: 0,
                status,
                objects,
            },
            CameraHandle {
                status: status_rx,
                objects: objects_rx,
            },
        )
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

    fn publish(&self, state: CameraState) {
        let counters = self.pipeline.counters();
        self.status.send_replace(CameraStatus {
            state,
            counters,
            source_gaps: self.source_gaps,
            frames_dropped: self.slot.dropped(),
        });
    }
}
