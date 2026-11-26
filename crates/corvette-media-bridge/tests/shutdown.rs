//! A stop signal must end the process cleanly: the open piece is finalized and
//! its index row committed, and the real binary exits zero within the deadline.

use corvette_media_bridge::record_config::{CameraRecord, RecordConfig, Storage, Streams};
use corvette_media_bridge::{record_role, shutdown};
use corvette_rtsp_client::depacketize::{Codec, Frame as ClientFrame};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::broadcast;

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

fn temp_root(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("corvette-shutdown-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp root created");
    root
}

fn recording_config(root: &Path) -> RecordConfig {
    RecordConfig {
        cameras: vec![CameraRecord {
            name: "gate".to_owned(),
            enabled: true,
            record_enabled: true,
            piece_seconds: 4,
            retention_days: 7,
            event_retention_days: 30,
            streams: Streams {
                main: true,
                sub: false,
            },
            review: corvette_media_bridge::record_config::Review::default(),
        }],
        storage: Storage {
            block_path: Some(root.to_path_buf()),
            retention_enforce: false,
        },
        warnings: Vec::new(),
    }
}

fn video_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            video_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "mp4") {
            found.push(path);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_after_frames_finalizes_the_open_piece_and_its_index_row() {
    let root = temp_root("piece");
    let runtime = record_role::prepare(&recording_config(&root))
        .expect("prepare succeeds")
        .expect("a recording camera prepares");
    let recorder = runtime.recorder_for("gate").expect("gate records");
    let (sender, receiver) = broadcast::channel::<ClientFrame>(64);
    sender.send(frame(SPS, 0)).expect("accepted");
    sender.send(frame(PPS, 0)).expect("accepted");
    sender.send(frame(IDR, 0)).expect("accepted");
    sender.send(frame(NON_IDR, 3_000)).expect("accepted");
    sender.send(frame(NON_IDR, 6_000)).expect("accepted");

    let (trigger, stop) = shutdown::Shutdown::channel();
    let (settled_tx, mut settled_rx) = tokio::sync::watch::channel(false);
    let role = tokio::spawn(record_role::run_record(
        "gate".to_string(),
        receiver,
        recorder,
        record_role::WATCHDOG_SILENCE,
        stop,
        settled_tx,
    ));
    trigger.trigger();
    // Same signal the stop drain waits on: the role parks after finalizing.
    tokio::time::timeout(Duration::from_secs(5), settled_rx.wait_for(|done| *done))
        .await
        .expect("the record role settles on the stop signal")
        .expect("the settle watch is alive");
    role.abort();

    let mut pieces = Vec::new();
    video_files(&root.join("recordings"), &mut pieces);
    assert_eq!(pieces.len(), 1, "the open piece reached the visible tree");
    let part_dir = root.join("recordings/.part");
    let parts_left = part_dir.read_dir().map_or(0, std::iter::Iterator::count);
    assert_eq!(parts_left, 0, "the finalized piece left no .part file");

    let index = record_role::RecordingRuntime::into_writer(runtime)
        .join()
        .expect("the index writer thread joins");
    let rows = index.all_pieces().expect("pieces readable");
    assert_eq!(rows.len(), 1, "the piece's index row was committed");
    assert_eq!(rows[0].camera, "gate");
    assert!(
        root.join(&rows[0].path).is_file(),
        "the committed row names a file on disk"
    );
    std::fs::remove_dir_all(&root).expect("temp root removed");
}

fn send_sigterm_and_expect_zero(child: &mut std::process::Child) {
    std::thread::sleep(Duration::from_millis(300));
    if let Ok(Some(status)) = child.try_wait() {
        panic!("the bridge exited before the signal: {status}");
    }
    // SAFETY: the pid belongs to the child process this test spawned and owns.
    let sent = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    assert_eq!(sent, 0, "sending SIGTERM to the bridge process");
    let started = std::time::Instant::now();
    loop {
        let polled = child.try_wait().expect("child status readable");
        if let Some(status) = polled {
            let elapsed = started.elapsed();
            assert!(
                status.success(),
                "the bridge exited {status} after {} ms",
                elapsed.as_millis()
            );
            assert!(
                elapsed < Duration::from_secs(8),
                "the bridge took {elapsed:?} to exit after SIGTERM"
            );
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the bridge is still running 10 s after SIGTERM"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn spawn_bridge(dir: &Path, cameras_json: &str, camera_toml: Option<&str>) -> std::process::Child {
    let bridge_cfg = dir.join("bridge.json");
    std::fs::write(
        &bridge_cfg,
        format!(
            "{{\"rtsp_bind_addr\":\"127.0.0.1:0\",\"fmp4_ws_bind_addr\":\"127.0.0.1:0\",\"hls_bind_addr\":\"127.0.0.1:0\",\"cameras\":{cameras_json}}}"
        ),
    )
    .expect("bridge config written");
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_corvette-media-bridge"));
    command
        .env(
            corvette_media_bridge::config::CONFIG_PATH_ENV_VAR,
            &bridge_cfg,
        )
        .env_remove(corvette_media_bridge::record_config::CAMERA_TOML_PATH_ENV_VAR)
        .env_remove(corvette_media_bridge::event_consumer::TRACK_EVENTS_SOCKET_ENV_VAR)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(toml) = camera_toml {
        command.env(
            corvette_media_bridge::record_config::CAMERA_TOML_PATH_ENV_VAR,
            toml,
        );
    }
    command.spawn().expect("the bridge binary starts")
}

#[test]
fn the_bridge_process_exits_zero_on_sigterm_without_recording() {
    let dir = temp_root("sigterm-off");
    let mut child = spawn_bridge(&dir, "[]", None);
    send_sigterm_and_expect_zero(&mut child);
    std::fs::remove_dir_all(&dir).expect("temp dir removed");
}

#[test]
fn the_bridge_process_exits_zero_on_sigterm_with_recording_configured() {
    // No frame source: the mock camera server carries no real parameter sets
    // (see g2_integration's own note), so no piece opens. What this proves is
    // the recording path's shutdown: record role, index writer thread and all.
    let dir = temp_root("sigterm-on");
    let block_root = dir.join("blocks");
    std::fs::create_dir_all(&block_root).expect("block root created");
    let camera_toml = dir.join("cameras.toml");
    std::fs::write(
        &camera_toml,
        format!(
            "[[cameras]]\nname = \"gate\"\n\n[cameras.source]\nmain = \"rtsp://127.0.0.1:1/\"\n\n[storage.block]\npath = \"{}\"\n",
            block_root.display()
        ),
    )
    .expect("camera toml written");
    let mut child = spawn_bridge(
        &dir,
        r#"[{"name":"gate","host":"127.0.0.1","port":1,"path":"/"}]"#,
        Some(camera_toml.to_str().expect("utf-8 path")),
    );
    send_sigterm_and_expect_zero(&mut child);
    assert!(
        block_root.join("index.sqlite").is_file(),
        "the recording index was created while the bridge ran"
    );
    std::fs::remove_dir_all(&dir).expect("temp dir removed");
}
