//! Turns the camera TOML's recording intent into a running recorder.

use crate::event_consumer::{self, EventWriter};
use crate::expiry::{self, ExpiryError};
use crate::index::{self, Index, IndexError, IndexWrite, PIECE_CHANNEL_CAPACITY, SeqAllocator};
use crate::record_config::{RecordConfig, Review};
use crate::recorder::{PieceRecorder, RecorderSettings};
use crate::recovery::{self, RecoveryError};
use crate::supervise::{self, Supervised, log_event};
use corvette_rtsp_client::client::Client;
use corvette_rtsp_client::depacketize::Frame as ClientFrame;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::SyncSender;
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::broadcast;

/// Silence that closes the open piece short in `run_record`. RTSP reconnects are invisible to subscribers, so frame arrival is the only liveness signal here.
pub const WATCHDOG_SILENCE: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub enum RecordSetupError {
    MissingBlockPath,
    CreateRoot { source: std::io::Error },
    Index { source: IndexError },
    Scan { source: RecoveryError },
    Expiry { source: ExpiryError },
}

impl std::fmt::Display for RecordSetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingBlockPath => {
                write!(f, "storage.block.path is required when a camera records")
            }
            Self::CreateRoot { source } => {
                write!(
                    f,
                    "recording storage could not be prepared: {}",
                    source.kind()
                )
            }
            Self::Index { .. } => write!(f, "recording index could not be opened"),
            Self::Scan { source } => match source {
                RecoveryError::Io { source: cause, .. } => {
                    write!(f, "recording recovery scan failed: {}", cause.kind())
                }
                RecoveryError::Index { .. } => write!(f, "recording recovery scan failed"),
            },
            Self::Expiry { .. } => write!(f, "recording startup expiry pass failed"),
        }
    }
}

impl std::error::Error for RecordSetupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MissingBlockPath => None,
            Self::CreateRoot { source } => Some(source),
            Self::Index { source } => Some(source),
            Self::Scan { source } => Some(source),
            Self::Expiry { source } => Some(source),
        }
    }
}

#[derive(Debug)]
pub struct RecordingRuntime {
    seqs: SeqAllocator,
    rows: SyncSender<IndexWrite>,
    writer: JoinHandle<Index>,
    root: PathBuf,
    piece_seconds: HashMap<String, u32>,
    review: HashMap<String, Review>,
    open_events: i64,
}

impl RecordingRuntime {
    /// Starts the track-events consumer writing into this runtime's index writer; the contract comment lives on [`event_consumer::run`].
    pub fn spawn_event_consumer(self: &Arc<Self>, socket: std::path::PathBuf) {
        let runtime = Arc::clone(self);
        tokio::spawn(async move {
            let writer = EventWriter::new(runtime.review.clone());
            event_consumer::run(
                socket,
                runtime.rows.clone(),
                writer,
                runtime.open_events,
                PieceRecorder::wall_clock_ms,
                event_consumer::EVENT_RECONNECT_BACKOFF,
            )
            .await;
        });
    }
}

impl RecordingRuntime {
    /// Returns a fresh recorder for `camera` if this runtime records it.
    #[must_use]
    pub fn recorder_for(&self, camera: &str) -> Option<PieceRecorder> {
        let piece_seconds = *self.piece_seconds.get(camera)?;
        Some(PieceRecorder::new(
            RecorderSettings {
                camera: camera.to_owned(),
                root: self.root.clone(),
                piece_seconds,
            },
            self.seqs.clone(),
            self.rows.clone(),
            Box::new(PieceRecorder::wall_clock_ms),
        ))
    }

    #[must_use]
    pub fn maybe_spawn_record(
        self: &Arc<Self>,
        camera: &str,
        client: &Arc<Client>,
    ) -> Option<Supervised> {
        self.recorder_for(camera)?;
        let camera = camera.to_owned();
        let client = Arc::clone(client);
        let runtime = Arc::clone(self);
        Some(supervise::spawn_supervised_with(
            format!("{camera}/record"),
            move || record_task(Arc::clone(&runtime), camera.clone(), client.subscribe()),
        ))
    }

    pub fn warn_unknown_cameras<'a>(&self, dialled: impl Iterator<Item = &'a str>) {
        let dialled: HashSet<&str> = dialled.collect();
        for camera in self.piece_seconds.keys().map(String::as_str) {
            if !dialled.contains(camera) {
                log_event(
                    camera,
                    "record-camera-unknown",
                    "in the record config, not among the cameras to dial",
                );
            }
        }
    }

    #[must_use]
    pub fn into_writer(self) -> JoinHandle<Index> {
        self.writer
    }
}

/// Runs one record pass for `name` on a freshly built recorder, up to a closed feed.
pub async fn record_task(
    runtime: Arc<RecordingRuntime>,
    name: String,
    frames: broadcast::Receiver<ClientFrame>,
) {
    let Some(recorder) = runtime.recorder_for(&name) else {
        return;
    };
    run_record(name, frames, recorder, WATCHDOG_SILENCE).await;
}

/// # Errors
pub fn prepare(record: &RecordConfig) -> Result<Option<RecordingRuntime>, RecordSetupError> {
    let cameras: Vec<(String, u32)> = record
        .cameras
        .iter()
        .filter(|camera| camera.records())
        .map(|camera| (camera.name.clone(), camera.piece_seconds))
        .collect();
    if cameras.is_empty() {
        return Ok(None);
    }
    let Some(block_path) = &record.storage.block_path else {
        return Err(RecordSetupError::MissingBlockPath);
    };
    let root = block_path.clone();
    std::fs::create_dir_all(root.join("recordings"))
        .map_err(|source| RecordSetupError::CreateRoot { source })?;
    let mut index = Index::open(&root.join("index.sqlite"), &cameras)
        .map_err(|source| RecordSetupError::Index { source })?;
    let report =
        recovery::scan(&root, &mut index).map_err(|source| RecordSetupError::Scan { source })?;
    eprintln!(
        "corvette-media-bridge: record recovery inserted={} rows-dropped={} parts-removed={} trash-removed={} quarantined={} skipped-dirs={} rows-refused={}",
        report.inserted,
        report.rows_dropped,
        report.part_removed,
        report.trash_removed,
        report.quarantined,
        report.skipped_dirs,
        report.rows_refused
    );
    let retention_days: HashMap<String, u32> = record
        .cameras
        .iter()
        .filter(|camera| camera.records())
        .map(|camera| (camera.name.clone(), camera.retention_days))
        .collect();
    let enforce = record.storage.retention_enforce;
    let expiry_report = expiry::run_pass(
        &root,
        &mut index,
        PieceRecorder::wall_clock_ms(),
        &retention_days,
        enforce,
    )
    .map_err(|source| RecordSetupError::Expiry { source })?;
    eprintln!(
        "corvette-media-bridge: expiry startup pass would-delete={} deleted={} refused-cameras={} step-failures={}",
        expiry_report.would_delete,
        expiry_report.deleted,
        expiry_report.refused_cameras,
        expiry_report.step_failures.iter().sum::<usize>()
    );
    let review: HashMap<String, Review> = record
        .cameras
        .iter()
        .map(|camera| (camera.name.clone(), camera.review.clone()))
        .collect();
    let open_events = index
        .open_event_count()
        .map_err(|source| RecordSetupError::Index { source })?;
    let seqs = index.allocator();
    let (rows, writer) = index::spawn_writer_with_expiry(
        index,
        PIECE_CHANNEL_CAPACITY,
        index::ExpiryWiring {
            root: root.clone(),
            retention_days,
            enforce,
            clock: Box::new(PieceRecorder::wall_clock_ms),
            recording_cameras: cameras.len(),
            free_space: Box::new(crate::disk_free::free_bytes),
        },
    );
    Ok(Some(RecordingRuntime {
        seqs,
        rows,
        writer,
        root,
        piece_seconds: cameras.into_iter().collect(),
        review,
        open_events,
    }))
}

/// Streams one camera's frames into `recorder`, parking a worker for its blocking I/O.
pub async fn run_record(
    name: String,
    mut frames: broadcast::Receiver<ClientFrame>,
    mut recorder: PieceRecorder,
    silence: Duration,
) {
    let mut silent = false;
    loop {
        match tokio::time::timeout(silence, frames.recv()).await {
            Ok(Ok(frame)) => {
                silent = false;
                tokio::task::block_in_place(|| recorder.push(&frame));
            }
            Ok(Err(broadcast::error::RecvError::Lagged(skipped))) => {
                silent = false;
                tokio::task::block_in_place(|| recorder.close_short());
                log_event(&name, "record-lagged", format_args!("skipped={skipped}"));
            }
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                tokio::task::block_in_place(|| recorder.finish());
                return;
            }
            Err(_elapsed) => {
                if !silent {
                    silent = true;
                    tokio::task::block_in_place(|| recorder.close_short());
                    log_event(&name, "record-watchdog", "");
                }
            }
        }
    }
}
