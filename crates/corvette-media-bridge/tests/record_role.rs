//! What the record role owes the bridge: `prepare` brings up exactly what the
//! record config asked for and nothing else, and the role turns a live frame
//! feed into pieces whose index rows match the files on disk.

use corvette_media_bridge::index::Index;
use corvette_media_bridge::piece_walk;
use corvette_media_bridge::record_config::{self, CameraRecord, RecordConfig, Storage, Streams};
use corvette_media_bridge::record_role;
use corvette_media_bridge::recorder::PieceRecorder;
use corvette_media_bridge::shutdown;
use corvette_rtsp_client::depacketize::{Codec, Frame as ClientFrame};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::{broadcast, watch};

const SPS: &[u8] = &[
    0x67, 0x42, 0xc0, 0x1f, 0xdc, 0x42, 0x6c, 0x04, 0x40, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00,
    0x40, 0x00, 0x00, 0x0f, 0x03, 0xc6, 0x0c, 0xe0,
];
const PPS: &[u8] = &[0x68, 0xce, 0x0f, 0x2c, 0x80];
const IDR: &[u8] = &[
    0x65, 0x88, 0x84, 0x04, 0xbc, 0x46, 0x28, 0x00, 0x0c, 0xc4, 0xc7, 0x00, 0x01, 0x7d, 0x68, 0xe0,
    0x00, 0x22, 0x43, 0x27, 0x27, 0x27, 0x5d, 0x75, 0xd7, 0x5d, 0x75, 0xd7, 0x5d, 0x75, 0xd7, 0x80,
];
const NON_IDR: &[u8] = &[0x41, 0x9a, 0x24, 0x6c];

fn annex_b(nal: &[u8]) -> bytes::Bytes {
    let mut payload = Vec::with_capacity(nal.len() + 4);
    payload.extend_from_slice(&[0, 0, 0, 1]);
    payload.extend_from_slice(nal);
    bytes::Bytes::from(payload)
}

fn frame(nal: &[u8], timestamp: u32) -> ClientFrame {
    ClientFrame {
        codec: Codec::H264,
        timestamp,
        payload: annex_b(nal),
    }
}

fn camera_record(name: &str, record_enabled: bool, piece_seconds: u32) -> CameraRecord {
    CameraRecord {
        name: name.to_owned(),
        enabled: true,
        record_enabled,
        piece_seconds,
        retention_days: 7,
        event_retention_days: 30,
        streams: Streams {
            main: true,
            sub: false,
        },
        review: corvette_media_bridge::record_config::Review::default(),
    }
}

const fn config(cameras: Vec<CameraRecord>, block_path: Option<PathBuf>) -> RecordConfig {
    RecordConfig {
        cameras,
        storage: Storage {
            block_path,
            retention_enforce: false,
        },
        warnings: Vec::new(),
    }
}

fn temp_path(test: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "corvette-record-role-{test}-{}",
        std::process::id()
    ))
}

fn fresh_root(test: &str) -> PathBuf {
    let root = temp_path(test);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp root created");
    root
}

fn cleanup(root: &Path) {
    std::fs::remove_dir_all(root).expect("temp root removed");
}

fn part_files(root: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(root.join("recordings/.part")) {
        Ok(entries) => entries
            .map(|entry| entry.expect("readable entry").path())
            .collect(),
        Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(failed) => panic!("part directory unreadable: {failed}"),
    }
}

fn visible_pieces(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut dirs = vec![root.join("recordings")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir)
            .unwrap_or_else(|failed| panic!("{} unreadable: {failed}", dir.display()))
        {
            let entry = entry.expect("readable entry");
            let path = entry.path();
            if entry.file_type().expect("readable type").is_dir() {
                if path.file_name().is_some_and(|name| name != ".part") {
                    dirs.push(path);
                }
            } else if path.extension().is_some_and(|ext| ext == "mp4") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn feed_video(sender: &broadcast::Sender<ClientFrame>, video_frames: usize, keyframe_every: usize) {
    sender
        .send(frame(SPS, 0))
        .expect("the channel accepts the setup frames");
    sender.send(frame(PPS, 0)).expect("accepted");
    let mut timestamp = 0u32;
    for index in 0..video_frames {
        let nal = if index % keyframe_every == 0 {
            IDR
        } else {
            NON_IDR
        };
        sender
            .send(frame(nal, timestamp))
            .expect("the channel accepts the video frames");
        timestamp += 3000;
    }
}

fn push_video(recorder: &mut PieceRecorder, video_frames: usize) {
    recorder.push(&frame(SPS, 0));
    recorder.push(&frame(PPS, 0));
    recorder.push(&frame(IDR, 0));
    let mut timestamp = 3000u32;
    for _ in 1..video_frames {
        recorder.push(&frame(NON_IDR, timestamp));
        timestamp += 3000;
    }
}

fn join_writer(runtime: record_role::RecordingRuntime) -> Index {
    runtime
        .into_writer()
        .join()
        .expect("the index writer thread joins")
}

#[test]
fn a_record_config_that_records_nothing_creates_nothing() {
    let root = fresh_root("none");
    let record = config(vec![camera_record("cam-off", false, 4)], Some(root.clone()));
    let runtime =
        record_role::prepare(&record).expect("a config with no recording camera prepares");
    assert!(runtime.is_none(), "no camera records, so no runtime exists");
    assert!(
        std::fs::read_dir(&root)
            .expect("root readable")
            .next()
            .is_none(),
        "prepare created nothing under the root"
    );
    cleanup(&root);
}

#[test]
fn a_recording_camera_without_a_block_path_is_refused_naming_the_setting() {
    let record = config(vec![camera_record("cam-on", true, 4)], None);
    let err =
        record_role::prepare(&record).expect_err("a recording camera without a block path fails");
    assert!(
        err.to_string().contains("storage.block.path"),
        "the error names the setting the operator must fix: {err}"
    );
}

#[test]
fn prepare_clears_a_stale_part_before_returning() {
    let root = fresh_root("stale");
    let part_dir = root.join("recordings/.part");
    std::fs::create_dir_all(&part_dir).expect("part dir created");
    let stale = part_dir.join("x-1.mp4.part");
    std::fs::write(&stale, b"half a piece").expect("stale part written");
    let record = config(vec![camera_record("cam-x", true, 4)], Some(root.clone()));
    let runtime = record_role::prepare(&record)
        .expect("prepare succeeds")
        .expect("a recording camera prepares");
    assert!(
        !stale.exists(),
        "the startup scan removed the stale part before prepare returned"
    );
    assert!(root.join("index.sqlite").exists(), "the index file exists");
    assert!(
        root.join("recordings").is_dir(),
        "the recordings dir exists"
    );
    drop(join_writer(runtime));
    cleanup(&root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_whole_frame_feed_lands_as_pieces_the_index_and_disk_agree_on() {
    let root = fresh_root("stream");
    let record = config(vec![camera_record("cam-d", true, 4)], Some(root.clone()));
    let runtime = record_role::prepare(&record)
        .expect("prepare succeeds")
        .expect("a recording camera prepares");
    let recorder = runtime.recorder_for("cam-d").expect("cam-d records");
    let (sender, receiver) = broadcast::channel::<ClientFrame>(600);
    feed_video(&sender, 450, 150);
    drop(sender);
    let (_stop_trigger, stop) = shutdown::Shutdown::channel();
    let (settled_tx, _settled_rx) = watch::channel(false);
    record_role::run_record(
        "cam-d".to_string(),
        receiver,
        recorder,
        Duration::from_hours(1),
        stop,
        settled_tx,
    )
    .await;

    drop(join_writer(runtime));
    let index = Index::open(&root.join("index.sqlite"), &[("cam-d".to_owned(), 4)])
        .expect("the index reopens after the writer joins");
    let mut row_paths: Vec<String> = index
        .all_pieces()
        .expect("rows read")
        .into_iter()
        .map(|row| row.path)
        .collect();
    row_paths.sort();
    assert_eq!(
        row_paths.len(),
        3,
        "the same cut shape as the recorder unit tests"
    );
    let mut disk_paths: Vec<String> = visible_pieces(&root)
        .iter()
        .map(|path| {
            path.strip_prefix(&root)
                .expect("piece under the root")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    disk_paths.sort();
    assert_eq!(row_paths, disk_paths, "index rows equal the files on disk");
    assert!(part_files(&root).is_empty(), "nothing left in .part");
    drop(index);
    cleanup(&root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lagged_receiver_survives_and_later_frames_still_produce_a_piece() {
    let root = fresh_root("lagged");
    let record = config(vec![camera_record("cam-e", true, 4)], Some(root.clone()));
    let runtime = record_role::prepare(&record)
        .expect("prepare succeeds")
        .expect("a recording camera prepares");
    let recorder = runtime.recorder_for("cam-e").expect("cam-e records");
    let (sender, receiver) = broadcast::channel::<ClientFrame>(2);
    sender.send(frame(NON_IDR, 0)).expect("accepted");
    sender.send(frame(NON_IDR, 3000)).expect("accepted");
    sender.send(frame(NON_IDR, 6000)).expect("accepted");
    let (_stop_trigger, stop) = shutdown::Shutdown::channel();
    let (settled_tx, _settled_rx) = watch::channel(false);
    let role = tokio::spawn(record_role::run_record(
        "cam-e".to_string(),
        receiver,
        recorder,
        Duration::from_hours(1),
        stop,
        settled_tx,
    ));

    // The later frames go in one at a time, each followed by a pause long
    // enough for the role to have consumed it, so only the first three sends
    // overflow the two-slot ring.
    let mut timestamp = 9_000u32;
    for nal in [SPS, PPS, IDR, NON_IDR, NON_IDR, NON_IDR, NON_IDR, NON_IDR] {
        sender.send(frame(nal, timestamp)).expect("accepted");
        timestamp += 3000;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    drop(sender);
    role.await
        .expect("the record role finishes without panicking");

    drop(join_writer(runtime));
    let index = Index::open(&root.join("index.sqlite"), &[("cam-e".to_owned(), 4)])
        .expect("the index reopens after the writer joins");
    let rows = index.all_pieces().expect("rows read");
    assert_eq!(
        rows.len(),
        1,
        "the frames after the overflow produced exactly one piece"
    );
    assert!(root.join(&rows[0].path).is_file(), "row names a real file");
    assert!(part_files(&root).is_empty(), "nothing left in .part");
    drop(index);
    cleanup(&root);
}

#[test]
fn recorder_for_answers_none_for_cameras_it_does_not_record() {
    let root = fresh_root("map");
    let record = config(
        vec![
            camera_record("cam-on", true, 4),
            camera_record("cam-off", false, 4),
        ],
        Some(root.clone()),
    );
    let runtime = record_role::prepare(&record)
        .expect("prepare succeeds")
        .expect("a recording camera prepares");
    assert!(runtime.recorder_for("cam-absent").is_none());
    assert!(runtime.recorder_for("cam-off").is_none());
    assert!(runtime.recorder_for("cam-on").is_some());
    drop(join_writer(runtime));
    cleanup(&root);
}

#[test]
fn a_camera_with_record_disabled_in_the_toml_gets_no_runtime() {
    let text = "
[[cameras]]
name = \"cam-toml-off\"
record.enabled = false

[cameras.source.main]
url = \"rtsp://127.0.0.1:554/main\"
";
    let record = record_config::parse(text).expect("the TOML parses");
    let root = temp_path("toml-off");
    let _ = std::fs::remove_dir_all(&root);
    let runtime = record_role::prepare(&record).expect("a non-recording config prepares");
    assert!(runtime.is_none(), "a disabled camera records nothing");
    assert!(!root.exists(), "nothing was created for a disabled camera");
}

#[test]
fn a_restarted_role_completes_its_piece_above_the_abandoned_one() {
    let root = fresh_root("restart");
    let record = config(vec![camera_record("cam-r", true, 4)], Some(root.clone()));
    let runtime = record_role::prepare(&record)
        .expect("prepare succeeds")
        .expect("a recording camera prepares");
    let make_recorder = || runtime.recorder_for("cam-r");

    let mut first = make_recorder().expect("cam-r records");
    push_video(&mut first, 50);
    drop(first);
    assert_eq!(
        part_files(&root).len(),
        1,
        "the abandoned piece left one part"
    );

    let mut second = make_recorder().expect("cam-r records again");
    push_video(&mut second, 50);
    second.finish();
    drop(second);

    let index = join_writer(runtime);
    let rows = index.all_pieces().expect("rows read");
    assert_eq!(rows.len(), 1, "only the second run closed a piece");
    assert_eq!(
        rows[0].seq, 2,
        "the restarted run continued the shared allocator"
    );
    assert!(root.join(&rows[0].path).is_file());
    assert_eq!(
        part_files(&root).len(),
        1,
        "the abandoned part is still there"
    );
    assert_eq!(visible_pieces(&root).len(), 1);
    drop(index);
    cleanup(&root);
}

#[test]
fn a_scan_failure_prints_no_path_and_keeps_its_cause_behind_source() {
    let root = fresh_root("SECRETPATH");
    std::fs::write(root.join(".trash"), b"not a directory").expect("trash planted as a file");
    let record = config(vec![camera_record("cam-s", true, 4)], Some(root.clone()));
    let err = record_role::prepare(&record)
        .expect_err("scanning a regular file where .trash belongs fails");
    let printed = err.to_string();
    let root_text = root.to_string_lossy().into_owned();
    assert!(
        !printed.contains("SECRETPATH") && !printed.contains(&root_text),
        "the printed error carries no path from the tree: {printed}"
    );
    assert!(
        <record_role::RecordSetupError as std::error::Error>::source(&err).is_some(),
        "the full cause stays reachable through source()"
    );
    cleanup(&root);
}

#[test]
fn an_index_open_failure_prints_no_path_and_keeps_its_cause() {
    let root = fresh_root("SECRETPATH-index");
    std::fs::create_dir(root.join("index.sqlite")).expect("index path planted as a directory");
    let record = config(vec![camera_record("cam-i", true, 4)], Some(root.clone()));
    let err = record_role::prepare(&record)
        .expect_err("an index path that is a directory cannot be opened");
    let printed = err.to_string();
    let root_text = root.to_string_lossy().into_owned();
    assert!(
        !printed.contains("SECRETPATH") && !printed.contains(&root_text),
        "the printed error carries no path from the tree: {printed}"
    );
    assert!(
        <record_role::RecordSetupError as std::error::Error>::source(&err).is_some(),
        "the full cause stays reachable through source()"
    );
    cleanup(&root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_record_task_run_builds_a_fresh_recorder_above_the_abandoned_piece() {
    let root = fresh_root("task");
    let record = config(vec![camera_record("cam-t", true, 4)], Some(root.clone()));
    let runtime = std::sync::Arc::new(
        record_role::prepare(&record)
            .expect("prepare succeeds")
            .expect("a recording camera prepares"),
    );
    let (sender, _) = broadcast::channel::<ClientFrame>(600);

    let (_stop_trigger, stop) = shutdown::Shutdown::channel();
    let (settled_tx, _settled_rx) = watch::channel(false);
    let first = tokio::spawn(record_role::record_task(
        std::sync::Arc::clone(&runtime),
        "cam-t".to_owned(),
        sender.subscribe(),
        stop.clone(),
        settled_tx.clone(),
    ));
    sender.send(frame(SPS, 0)).expect("accepted");
    sender.send(frame(PPS, 0)).expect("accepted");
    sender.send(frame(IDR, 0)).expect("accepted");
    sender.send(frame(NON_IDR, 3000)).expect("accepted");
    let mut started = false;
    for _ in 0..400 {
        if !part_files(&root).is_empty() {
            started = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(started, "the first run started its piece");
    assert_eq!(
        part_files(&root).len(),
        1,
        "exactly one part left by the first run"
    );
    first.abort();
    first
        .await
        .expect_err("the first run is dropped mid-piece, feed still open");

    let second = tokio::spawn(record_role::record_task(
        std::sync::Arc::clone(&runtime),
        "cam-t".to_owned(),
        sender.subscribe(),
        stop,
        settled_tx,
    ));
    feed_video(&sender, 151, 150);
    drop(sender);
    second
        .await
        .expect("the second run finishes without panicking");

    let inner = std::sync::Arc::try_unwrap(runtime).expect("the runtime has no other owner");
    let index = join_writer(inner);
    let rows = index.all_pieces().expect("rows read");
    assert_eq!(
        rows.len(),
        2,
        "the second run closed a piece and its last keyframe opened a stub the feed-close finished"
    );
    assert_eq!(
        rows.iter().map(|row| row.seq).min(),
        Some(2),
        "the fresh recorder completed above the sequence the first run started on"
    );
    assert_eq!(
        part_files(&root).len(),
        1,
        "the abandoned part is untouched"
    );
    assert_eq!(visible_pieces(&root).len(), 2);
    drop(index);
    cleanup(&root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lagged_overflow_closes_the_open_piece_short_and_a_later_keyframe_starts_a_clean_piece() {
    let root = fresh_root("lagclose");
    let record = config(vec![camera_record("cam-l", true, 4)], Some(root.clone()));
    let runtime = record_role::prepare(&record)
        .expect("prepare succeeds")
        .expect("a recording camera prepares");
    let recorder = runtime.recorder_for("cam-l").expect("cam-l records");
    let (sender, receiver) = broadcast::channel::<ClientFrame>(2);
    let (_stop_trigger, stop) = shutdown::Shutdown::channel();
    let (settled_tx, _settled_rx) = watch::channel(false);
    let role = tokio::spawn(record_role::run_record(
        "cam-l".to_string(),
        receiver,
        recorder,
        Duration::from_hours(1),
        stop,
        settled_tx,
    ));

    for (nal, timestamp) in [
        (SPS, 0u32),
        (PPS, 0),
        (IDR, 0),
        (NON_IDR, 3_000),
        (NON_IDR, 6_000),
    ] {
        sender.send(frame(nal, timestamp)).expect("accepted");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    for _ in 0..400 {
        if !part_files(&root).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        !part_files(&root).is_empty(),
        "the piece opened before the overflow"
    );

    // more sends than the two-slot ring holds, back to back: the receiver's
    // next read reports the skip.
    for timestamp in [9_000u32, 12_000, 15_000, 18_000, 21_000] {
        sender.send(frame(NON_IDR, timestamp)).expect("accepted");
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    for (nal, timestamp) in [
        (SPS, 30_000u32),
        (PPS, 30_000),
        (IDR, 30_000),
        (NON_IDR, 33_000),
    ] {
        sender.send(frame(nal, timestamp)).expect("accepted");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    drop(sender);
    role.await.expect("the role finishes");

    drop(join_writer(runtime));
    let index = Index::open(&root.join("index.sqlite"), &[("cam-l".to_owned(), 4)])
        .expect("the index reopens after the writer joins");
    let rows = index.all_pieces().expect("rows read");
    assert_eq!(rows.len(), 2, "the lag closed the open piece short");
    for row in &rows {
        let bytes = std::fs::read(root.join(&row.path)).expect("piece on disk");
        piece_walk::walk(&bytes).expect("each piece walks with no stitched corruption");
    }
    assert!(part_files(&root).is_empty(), "nothing left in .part");
    drop(index);
    cleanup(&root);
}

const WATCHDOG_TEST_NAME: &str =
    "a_silent_feed_closes_the_open_piece_once_and_recording_resumes_at_a_keyframe";
const RECORD_WATCHDOG_CHILD_RUN: &str = "RECORD_WATCHDOG_CHILD_RUN";
const RECORD_WATCHDOG_CHILD_TOKEN: &str = "watchdog-child";

#[test]
fn a_silent_feed_closes_the_open_piece_once_and_recording_resumes_at_a_keyframe() {
    let child_run = std::env::var(RECORD_WATCHDOG_CHILD_RUN).unwrap_or_default();
    if child_run != RECORD_WATCHDOG_CHILD_TOKEN {
        let child = std::process::Command::new(std::env::current_exe().expect("test binary path"))
            .arg("--exact")
            .arg(WATCHDOG_TEST_NAME)
            .arg("--nocapture")
            .env_remove(RECORD_WATCHDOG_CHILD_RUN)
            .env(RECORD_WATCHDOG_CHILD_RUN, RECORD_WATCHDOG_CHILD_TOKEN)
            .output()
            .expect("the child test runs");
        let stderr = String::from_utf8_lossy(&child.stderr);
        assert!(child.status.success(), "the child run failed:\n{stderr}");
        let fires = stderr.matches("event=record-watchdog").count();
        assert_eq!(fires, 1, "one watchdog event per silent stretch\n{stderr}");
        return;
    }

    let root = fresh_root("watchdog");
    let record = config(vec![camera_record("cam-w", true, 4)], Some(root.clone()));
    let runtime = record_role::prepare(&record)
        .expect("prepare succeeds")
        .expect("a recording camera prepares");
    let recorder = runtime.recorder_for("cam-w").expect("cam-w records");
    let (sender, receiver) = broadcast::channel::<ClientFrame>(64);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime builds");
    rt.block_on(async {
        let (_stop_trigger, stop) = shutdown::Shutdown::channel();
        let (settled_tx, _settled_rx) = watch::channel(false);
        let role = tokio::spawn(record_role::run_record(
            "cam-w".to_string(),
            receiver,
            recorder,
            Duration::from_millis(100),
            stop,
            settled_tx,
        ));
        for (nal, timestamp) in [
            (SPS, 0u32),
            (PPS, 0),
            (IDR, 0),
            (NON_IDR, 3_000),
            (NON_IDR, 6_000),
        ] {
            sender.send(frame(nal, timestamp)).expect("accepted");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        for _ in 0..400 {
            if !part_files(&root).is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(!part_files(&root).is_empty(), "the piece opened");
        // several silence windows with no frame at all
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(
            part_files(&root).is_empty(),
            "the watchdog closed the piece short"
        );
        for (nal, timestamp) in [(NON_IDR, 9_000u32), (IDR, 12_000), (NON_IDR, 15_000)] {
            sender.send(frame(nal, timestamp)).expect("accepted");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        drop(sender);
        role.await.expect("the role finishes");
    });
    drop(join_writer(runtime));
    let index = Index::open(&root.join("index.sqlite"), &[("cam-w".to_owned(), 4)])
        .expect("the index reopens after the writer joins");
    let mut rows = index.all_pieces().expect("rows read");
    rows.sort_by_key(|row| (row.start_ms, row.seq));
    assert_eq!(rows.len(), 2);
    assert!(
        rows[1].seq > rows[0].seq,
        "the next piece has a fresh sequence"
    );
    assert_eq!(
        rows[0].end_ms - rows[0].start_ms,
        100,
        "the closed row carries its true duration"
    );
    assert!(
        rows[1].start_ms > rows[0].end_ms,
        "the silent stretch shows as a gap between the rows"
    );
    assert_eq!(
        rows[1].end_ms - rows[1].start_ms,
        66,
        "the resumed piece starts only at the keyframe"
    );
    for row in &rows {
        let bytes = std::fs::read(root.join(&row.path)).expect("piece on disk");
        piece_walk::walk(&bytes).expect("piece parses");
    }
    assert!(part_files(&root).is_empty(), "nothing left in .part");
    drop(index);
    cleanup(&root);
}
