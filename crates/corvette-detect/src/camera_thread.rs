//! Runs one camera as two operating-system threads: a frame producer owning the decode child, and a pipeline runner that rebuilds and restarts after a fatal error.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Mutex, MutexGuard};
use std::thread::Builder;
use std::time::{Duration, Instant};

use corvette_track::IdSource;
use tokio::process::Command;
use tokio::sync::watch;

use crate::backoff::{Backoff, Policy};
use crate::camera_runner::{CameraHandle, CameraRunner, CameraState, CameraStatus};
use crate::clock::FrameClock;
use crate::ffmpeg::FrameSpec;
use crate::frame_slot::{SlotCloser, SlotReceiver, SlotSender, frame_slot};
use crate::live_pipeline::{LivePipeline, PipelineCounters};
use crate::log_event;
use crate::object_stage::{CameraObjects, FrameObjects};
use crate::shared_link::{SharedInfer, SharedLink};
use crate::source::{FrameSource, SourceError, SourceStopper};

const STOPPER_WAIT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug)]
pub struct CameraThreadSettings {
    pub frame_timeout: Duration,
    pub backoff: Policy,
}

#[derive(Debug)]
pub enum StartError {
    ThreadStart {
        role: &'static str,
        source: std::io::Error,
    },
    ProducerRuntime(std::io::Error),
    NoStopper,
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ThreadStart { role, source } => {
                write!(f, "the camera {role} thread failed to start: {source}")
            }
            Self::ProducerRuntime(source) => {
                write!(
                    f,
                    "the camera producer could not build its runtime: {source}"
                )
            }
            Self::NoStopper => {
                f.write_str("the camera producer handed back no frame-source stopper")
            }
        }
    }
}

impl std::error::Error for StartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ThreadStart { source, .. } | Self::ProducerRuntime(source) => Some(source),
            Self::NoStopper => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadEnd {
    Finished,
    Panicked,
    TimedOut,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JoinReport {
    pub runner: ThreadEnd,
    pub producer: ThreadEnd,
}

/// The status and objects channels of one camera, alive for the process: a toggle swaps the threads, never these channels, so a publisher spawned at startup keeps live receivers across a toggle.
#[derive(Clone, Debug)]
pub struct CameraChannels {
    status_tx: watch::Sender<CameraStatus>,
    objects_tx: watch::Sender<CameraObjects>,
}

impl Default for CameraChannels {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraChannels {
    #[must_use]
    pub fn new() -> Self {
        Self {
            status_tx: watch::channel(initial_status()).0,
            objects_tx: watch::channel(CameraObjects::Cleared).0,
        }
    }

    #[must_use]
    pub fn handle(&self) -> CameraHandle {
        CameraHandle {
            status: self.status_tx.subscribe(),
            objects: self.objects_tx.subscribe(),
        }
    }

    #[must_use]
    pub fn objects_receiver(&self) -> watch::Receiver<CameraObjects> {
        self.objects_tx.subscribe()
    }

    /// Places one frame's objects on the channel, as the runner thread does for every processed frame.
    pub fn publish_frame(&self, objects: FrameObjects) {
        let _ = self.objects_tx.send(CameraObjects::Frame(objects));
    }

    pub(crate) fn mark_stopped(&self) {
        let _ = self.objects_tx.send(CameraObjects::Stopped);
    }
}

/// One camera thread's end: a delivered value survives a timed-out wait, so a later wait reports the same end instead of waiting twice on a thread that already ended.
#[derive(Debug)]
struct EndWatcher {
    done: Receiver<()>,
    ended: Option<ThreadEnd>,
}

impl EndWatcher {
    fn wait_until(&mut self, deadline: Instant) -> ThreadEnd {
        self.end_or_wait(Some(deadline))
    }

    fn wait_finished(&mut self) -> ThreadEnd {
        self.end_or_wait(None)
    }

    fn end_or_wait(&mut self, deadline: Option<Instant>) -> ThreadEnd {
        if let Some(ended) = self.ended {
            return ended;
        }
        let ended = match deadline {
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                match self.done.recv_timeout(remaining) {
                    Ok(()) => ThreadEnd::Finished,
                    Err(RecvTimeoutError::Disconnected) => ThreadEnd::Panicked,
                    Err(RecvTimeoutError::Timeout) => return ThreadEnd::TimedOut,
                }
            }
            None => match self.done.recv() {
                Ok(()) => ThreadEnd::Finished,
                Err(_sender_dropped) => ThreadEnd::Panicked,
            },
        };
        self.ended = Some(ended);
        ended
    }
}

/// The producer inputs one start takes from its plan: the frame geometry the reader expects and the clock it stamps.
#[derive(Clone, Copy, Debug)]
pub struct ProducerPlan {
    pub spec: FrameSpec,
    pub clock: FrameClock,
}

#[derive(Debug)]
pub struct CameraThreads {
    stop_tx: SyncSender<()>,
    slot_closer: SlotCloser,
    stopper: SourceStopper,
    runner_end: Mutex<EndWatcher>,
    producer_end: Mutex<EndWatcher>,
}

impl CameraThreads {
    /// Starts one camera's two OS threads and the decode child they supervise, publishing on the persistent `channels`; `stop` ends them and `join` waits for both.
    ///
    /// # Errors
    /// [`StartError::ThreadStart`] when an OS thread cannot be created, [`StartError::ProducerRuntime`] when the producer's runtime cannot be built, [`StartError::NoStopper`] when the producer hands no stopper back within [`STOPPER_WAIT`].
    pub fn start<M, B, E>(
        unit: &str,
        settings: CameraThreadSettings,
        channels: &CameraChannels,
        producer: ProducerPlan,
        make_command: M,
        link: &SharedLink,
        build: B,
    ) -> Result<Self, StartError>
    where
        M: FnMut() -> Command + Send + 'static,
        B: for<'a> Fn(&'a mut IdSource, SharedInfer) -> Result<LivePipeline<'a, SharedInfer>, E>
            + Send
            + 'static,
        E: std::fmt::Display + Send + 'static,
    {
        let ProducerPlan { spec, clock } = producer;
        let (slot_sender, slot_receiver) = frame_slot();
        let slot_closer = slot_receiver.closer();
        let status_tx = channels.status_tx.clone();
        let objects_tx = channels.objects_tx.clone();
        let (stop_tx, stop_rx) = mpsc::sync_channel(1);
        let (handback_tx, handback_rx) = mpsc::sync_channel(1);
        let (producer_done_tx, producer_done) = mpsc::sync_channel(1);
        let (runner_done_tx, runner_done) = mpsc::sync_channel(1);

        let producer_unit = unit.to_string();
        Builder::new()
            .spawn(move || {
                run_producer(
                    producer_unit,
                    settings.backoff,
                    spec,
                    clock,
                    make_command,
                    &slot_sender,
                    handback_tx,
                );
                slot_sender.close();
                let _ = producer_done_tx.send(());
            })
            .map_err(|source| StartError::ThreadStart {
                role: "producer",
                source,
            })?;

        let stopper = match handback_rx.recv_timeout(STOPPER_WAIT) {
            Ok(Ok(stopper)) => stopper,
            Ok(Err(source)) => return Err(StartError::ProducerRuntime(source)),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                return Err(StartError::NoStopper);
            }
        };

        let thread_link = link.clone();
        let runner_unit = unit.to_string();
        let runner = Builder::new().spawn(move || {
            run_runner(
                &runner_unit,
                &slot_receiver,
                &thread_link,
                settings,
                (&status_tx, &objects_tx),
                &stop_rx,
                build,
            );
            let _ = runner_done_tx.send(());
        });
        if let Err(source) = runner {
            slot_closer.close();
            stopper.stop();
            return Err(StartError::ThreadStart {
                role: "runner",
                source,
            });
        }

        Ok(Self {
            stop_tx,
            slot_closer,
            stopper,
            runner_end: Mutex::new(EndWatcher {
                done: runner_done,
                ended: None,
            }),
            producer_end: Mutex::new(EndWatcher {
                done: producer_done,
                ended: None,
            }),
        })
    }

    /// Stops the frame source, closes the slot and wakes a runner waiting in back-off; idempotent and callable from any thread.
    pub fn stop(&self) {
        self.stopper.stop();
        self.slot_closer.close();
        let _ = self.stop_tx.try_send(());
    }

    /// Waits for both threads under one shared `timeout` deadline and reports how each ended; call `stop` first, `join` only waits.
    #[must_use]
    pub fn join(self, timeout: Duration) -> JoinReport {
        self.wait_ends(timeout)
    }

    /// Waits like `join` without consuming the threads; a thread that ends after a timed-out wait is reported by a later wait.
    #[must_use]
    pub fn wait_ends(&self, timeout: Duration) -> JoinReport {
        let deadline = Instant::now() + timeout;
        let runner = end_of(&self.runner_end).wait_until(deadline);
        let producer = end_of(&self.producer_end).wait_until(deadline);
        JoinReport { runner, producer }
    }

    /// Waits for both threads to end, however long that takes, after a timed-out wait left one running.
    #[must_use]
    pub fn wait_finished(&self) -> JoinReport {
        let runner = end_of(&self.runner_end).wait_finished();
        let producer = end_of(&self.producer_end).wait_finished();
        JoinReport { runner, producer }
    }
}

fn end_of(end: &Mutex<EndWatcher>) -> MutexGuard<'_, EndWatcher> {
    end.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn initial_status() -> CameraStatus {
    CameraStatus {
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
    }
}

fn run_producer<M>(
    unit: String,
    policy: Policy,
    spec: FrameSpec,
    clock: FrameClock,
    make_command: M,
    slot: &SlotSender,
    handback: SyncSender<Result<SourceStopper, std::io::Error>>,
) where
    M: FnMut() -> Command + Send + 'static,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(source) => {
            let _ = handback.send(Err(source));
            return;
        }
    };
    runtime.block_on(produce(
        unit,
        policy,
        spec,
        clock,
        make_command,
        slot,
        handback,
    ));
}

async fn produce<M>(
    unit: String,
    policy: Policy,
    spec: FrameSpec,
    clock: FrameClock,
    make_command: M,
    slot: &SlotSender,
    handback: SyncSender<Result<SourceStopper, std::io::Error>>,
) where
    M: FnMut() -> Command + Send + 'static,
{
    let mut source = FrameSource::spawn(unit, policy, spec, clock, make_command);
    let stopper = source.stopper();
    if handback.send(Ok(stopper)).is_err() {
        return;
    }
    loop {
        match source.next_frame().await {
            Ok(frame) => slot.put(frame),
            Err(SourceError::Stopped) => break,
            // The source's supervisor restarts the child itself, so these errors are gaps in the stream, not its end.
            Err(SourceError::Ended | SourceError::Read(_) | SourceError::SpawnFailed { .. }) => {
                slot.gap();
            }
        }
    }
}

fn run_runner<B, E>(
    unit: &str,
    slot: &SlotReceiver,
    link: &SharedLink,
    settings: CameraThreadSettings,
    channels: (&watch::Sender<CameraStatus>, &watch::Sender<CameraObjects>),
    stop_rx: &Receiver<()>,
    build: B,
) where
    B: for<'a> Fn(&'a mut IdSource, SharedInfer) -> Result<LivePipeline<'a, SharedInfer>, E>,
    E: std::fmt::Display,
{
    let (status_tx, objects_tx) = channels;
    let mut restarts: u32 = 0;
    let mut backoff = Backoff::new(settings.backoff);
    loop {
        let mut ids = IdSource::default();
        let infer = link.infer_handle();
        let timings = infer.timings();
        let pipeline = match build(&mut ids, infer) {
            Ok(pipeline) => pipeline,
            Err(_error) => {
                // A failed build is a runner failure with no runner: the same failed status and back-off restart, published in place because no runner exists to publish it; as with a runner error the status carries the failure and the error value is spent.
                status_tx.send_modify(|status| status.state = CameraState::Failed);
                restarts = restarts.saturating_add(1);
                let delay = backoff.after_run(Duration::ZERO);
                let delay_ms = delay.as_millis();
                log_event(
                    unit,
                    "runner-restart",
                    format!("restarts={restarts} delay_ms={delay_ms} reason=build-failed"),
                );
                match stop_rx.recv_timeout(delay) {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                }
                continue;
            }
        };
        let (runner, _handle) = CameraRunner::new(slot.clone(), pipeline, settings.frame_timeout);
        let runner = runner
            .publishing_to(status_tx.clone(), objects_tx.clone(), restarts)
            .logged_as(unit)
            .with_timings(timings);
        let started = Instant::now();
        if matches!(runner.run(), Ok(())) {
            break;
        }
        // The runner already published the failed state; the restart below is the recovery, so the error value is spent.
        restarts = restarts.saturating_add(1);
        let delay = backoff.after_run(started.elapsed());
        let delay_ms = delay.as_millis();
        log_event(
            unit,
            "runner-restart",
            format!("restarts={restarts} delay_ms={delay_ms} reason=pipeline-error"),
        );
        match stop_rx.recv_timeout(delay) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backoff::DEFAULT_POLICY;
    use crate::candidate_stage::CandidateStage;
    use crate::engine::{LabelError, Labels};
    use crate::ffmpeg::Decode;
    use crate::live_pipeline::LivePipeline;
    use crate::object_stage::ObjectStage;
    use crate::reader::Frame;
    use crate::region_stage::{RegionStage, RegionStageSettings};
    use crate::worker_link::WorkerLink;
    use corvette_detect_proto::{HEADER_LEN, Reply, Request, parse_header};
    use corvette_motion::{MotionSettings, Shape, SizeGrid};
    use corvette_track::{ObjectFilter, ObjectFilters, TrackerSettings, ZoneSet};
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;

    fn flat_frame(index: u64) -> Frame {
        let spec = FrameSpec::new(160, 120, 10, Decode::Cpu).expect("valid frame spec");
        Frame {
            index,
            time: Duration::from_millis(100 * (index + 1)),
            data: vec![0x80; spec.frame_bytes()],
        }
    }

    fn build_pipeline(
        ids: &mut IdSource,
        infer: SharedInfer,
    ) -> Result<LivePipeline<'_, SharedInfer>, LabelError> {
        let settings = RegionStageSettings {
            motion: MotionSettings::default(),
            floor: 32,
            grid: SizeGrid::empty(),
            model: Shape::new(640, 640).expect("model shape"),
            motion_enabled: true,
        };
        let regions = RegionStage::new(settings, Shape::new(160, 120).expect("frame shape"), None)
            .expect("region stage");
        let mut filters = BTreeMap::new();
        filters.insert("person".to_owned(), ObjectFilter::default());
        let candidates = CandidateStage::new(
            Labels::parse("0 person")?,
            ObjectFilters::new(vec!["person".to_owned()], filters),
            160,
            120,
        )
        .expect("candidate stage");
        let objects =
            ObjectStage::new(tracker(), ids, ZoneSet::new(Vec::new()), 0.5).expect("object stage");
        Ok(LivePipeline::new(regions, candidates, objects, infer))
    }

    fn tracker() -> TrackerSettings {
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

    // Answers every infer request on every connection with the given worker time, until the socket dies.
    fn answer_infer_requests(listener: UnixListener, worker_ms: u32) {
        std::thread::spawn(move || {
            while let Ok((mut stream, _)) = listener.accept() {
                loop {
                    let mut header = [0u8; HEADER_LEN];
                    if stream.read_exact(&mut header).is_err() {
                        break;
                    }
                    let parsed = parse_header(&header).expect("header decodes");
                    let mut body = vec![0u8; parsed.body_len];
                    if stream.read_exact(&mut body).is_err() {
                        break;
                    }
                    let mut request = header.to_vec();
                    request.extend_from_slice(&body);
                    let id = match Request::decode(&request) {
                        Ok(Request::InferRequest { id, .. }) => id,
                        other => panic!("expected an infer request, got {other:?}"),
                    };
                    let reply = Reply::Rows {
                        id,
                        worker_ms,
                        rows: Vec::new(),
                    };
                    if stream
                        .write_all(&reply.encode().expect("reply encodes"))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        });
    }

    #[test]
    fn run_runner_publishes_the_worker_time_recorded_on_its_link() {
        let dir = std::env::temp_dir().join(format!("camthread-timings-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let socket = dir.join("worker.sock");
        let _ = fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind worker socket");
        answer_infer_requests(listener, 41);
        let link = SharedLink::new(WorkerLink::new(
            socket,
            Duration::from_secs(5),
            DEFAULT_POLICY,
        ));
        let (sender, receiver) = frame_slot();
        let closer = receiver.closer();
        let (status_tx, mut status_rx) = watch::channel(initial_status());
        let (objects_tx, _objects_rx) = watch::channel(CameraObjects::Cleared);
        let (_stop_tx, stop_rx) = mpsc::channel();
        let runner_thread = std::thread::spawn(move || {
            let settings = CameraThreadSettings {
                frame_timeout: Duration::from_millis(200),
                backoff: DEFAULT_POLICY,
            };
            run_runner(
                "timings-test",
                &receiver,
                &link,
                settings,
                (&status_tx, &objects_tx),
                &stop_rx,
                build_pipeline,
            );
        });
        sender.put(flat_frame(0));
        let deadline = Instant::now() + Duration::from_secs(10);
        while status_rx.borrow_and_update().worker_ms != 41 {
            assert!(
                Instant::now() < deadline,
                "worker_ms never reached 41 in the published status"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        closer.close();
        assert!(
            runner_thread.join().is_ok(),
            "the runner thread ends on the closed slot"
        );
        assert_eq!(
            status_rx.borrow().worker_ms,
            41,
            "the published status carries the worker's own time"
        );
        fs::remove_dir_all(&dir).expect("remove temp dir");
    }
}
