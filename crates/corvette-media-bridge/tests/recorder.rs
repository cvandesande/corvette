//! What the recorder owes the recording tree: `.part`-then-rename pieces with
//! contiguous rows, and recovery-compatible cleanup when a piece never closes.

use corvette_media_bridge::index::{Index, IndexWrite, PieceRow};
use corvette_media_bridge::piece_walk;
use corvette_media_bridge::recorder::{PieceRecorder, RecorderSettings};
use corvette_media_bridge::recovery;
use corvette_rtsp_client::depacketize::{Codec, Frame as ClientFrame};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};

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

const CLOCK_BASE: i64 = 1_795_478_400_000;
const CLOCK_STEP: i64 = 5_000;
const CLOCK_DATE: &str = "2026-11-24";
const CLOCK_HOUR: &str = "00";

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

fn rig(
    test: &str,
    camera: &str,
    piece_seconds: u32,
    channel_capacity: usize,
) -> (PathBuf, PieceRecorder, Receiver<IndexWrite>) {
    let root =
        std::env::temp_dir().join(format!("corvette-recorder-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp root created");
    let index = Index::open(
        &root.join("index.sqlite"),
        &[(camera.to_owned(), piece_seconds)],
    )
    .expect("index opens");
    let seqs = index.allocator();
    drop(index);
    let (sender, rows) = sync_channel(channel_capacity);
    let clock = AtomicI64::new(CLOCK_BASE);
    let recorder = PieceRecorder::new(
        RecorderSettings {
            camera: camera.to_owned(),
            root: root.clone(),
            piece_seconds,
        },
        seqs,
        sender,
        Box::new(move || clock.fetch_add(CLOCK_STEP, Ordering::SeqCst)),
    );
    (root, recorder, rows)
}

fn cleanup(root: &Path) {
    std::fs::remove_dir_all(root).expect("temp root removed");
}

fn push_video(recorder: &mut PieceRecorder, video_frames: usize, keyframe_every: usize) {
    recorder.push(&frame(SPS, 0));
    recorder.push(&frame(PPS, 0));
    let mut timestamp = 0u32;
    for index in 0..video_frames {
        let nal = if index % keyframe_every == 0 {
            IDR
        } else {
            NON_IDR
        };
        recorder.push(&frame(nal, timestamp));
        timestamp += 3000;
    }
}

fn drain(rows: &Receiver<IndexWrite>) -> Vec<PieceRow> {
    let mut received = Vec::new();
    while let Ok(write) = rows.try_recv() {
        if let IndexWrite::Piece(row) = write {
            received.push(row);
        }
    }
    received
}

fn part_dir(root: &Path) -> PathBuf {
    root.join("recordings/.part")
}

fn part_files(root: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(part_dir(root)) {
        Ok(entries) => entries
            .map(|entry| entry.expect("readable entry").path())
            .collect(),
        Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(failed) => panic!("part directory unreadable: {failed}"),
    }
}

fn read_box(bytes: &[u8], offset: usize) -> ([u8; 4], usize) {
    let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
    let kind = bytes[offset + 4..offset + 8].try_into().unwrap();
    (kind, size)
}

fn first_fragment_at(bytes: &[u8]) -> usize {
    let (_, ftyp_len) = read_box(bytes, 0);
    let (_, moov_len) = read_box(bytes, ftyp_len);
    ftyp_len + moov_len
}

fn first_tfdt(bytes: &[u8]) -> u64 {
    let moof_at = first_fragment_at(bytes);
    let (moof_kind, moof_len) = read_box(bytes, moof_at);
    assert_eq!(&moof_kind, b"moof");
    let mut at = moof_at + 8;
    while at < moof_at + moof_len {
        let (kind, len) = read_box(bytes, at);
        if kind == *b"traf" {
            let mut inner = at + 8;
            while inner < at + len {
                let (inner_kind, inner_len) = read_box(bytes, inner);
                if inner_kind == *b"tfdt" {
                    return u64::from_be_bytes(bytes[inner + 12..inner + 20].try_into().unwrap());
                }
                inner += inner_len;
            }
            panic!("fragment has no tfdt");
        }
        at += len;
    }
    panic!("moof has no traf")
}

fn first_sample_flags(bytes: &[u8]) -> u32 {
    let flags_at = first_fragment_at(bytes) + 96;
    u32::from_be_bytes(bytes[flags_at..flags_at + 4].try_into().unwrap())
}

#[test]
fn pieces_start_at_keyframes_cover_the_stream_and_reach_the_channel() {
    let (root, mut recorder, rows) = rig("cut", "cam-cut", 4, 8);
    push_video(&mut recorder, 450, 150);
    recorder.finish();

    let rows = drain(&rows);
    assert_eq!(rows.len(), 3);
    let expected_starts = [
        CLOCK_BASE,
        CLOCK_BASE + CLOCK_STEP,
        CLOCK_BASE + 2 * CLOCK_STEP,
    ];
    for (row, (start_ms, expected_seq)) in rows.iter().zip(expected_starts.into_iter().zip(1u64..))
    {
        assert_eq!(row.camera, "cam-cut");
        assert_eq!(row.seq, expected_seq);
        assert_eq!(row.start_ms, start_ms);
        assert_eq!(row.codec, "h264");
        assert_eq!(
            row.path,
            format!(
                "recordings/cam-cut/{CLOCK_DATE}/{CLOCK_HOUR}/{start_ms}-{}.mp4",
                row.seq
            )
        );
        let bytes = std::fs::read(root.join(&row.path)).expect("piece file at the row path");
        assert_eq!(
            u64::try_from(bytes.len()).expect("length fits"),
            row.size_bytes
        );
        let facts = piece_walk::walk(&bytes).expect("stored piece parses");
        assert_eq!(facts.duration_ms, row.end_ms - row.start_ms);
        assert!(facts.duration_ms >= 4_000, "the cut waited for a keyframe");
        assert_eq!(facts.codec, "h264");
        assert_eq!(first_tfdt(&bytes), 0);
        assert_eq!(first_sample_flags(&bytes), 0x0200_0000);
    }
    for pair in rows.windows(2) {
        let gap = (pair[1].start_ms - pair[0].end_ms).abs();
        assert!(gap <= CLOCK_STEP, "piece {pair:?} runs past the next start");
    }
    assert!(part_files(&root).is_empty(), "every piece renamed away");
    cleanup(&root);
}

#[test]
fn an_open_piece_lives_in_the_part_dir_until_it_closes() {
    let (root, mut recorder, rows) = rig("part", "cam-part", 4, 4);
    push_video(&mut recorder, 40, 150);

    let open_parts = part_files(&root);
    assert_eq!(open_parts.len(), 1, "one open piece sits in .part");
    let open_part = &open_parts[0];
    assert!(
        open_part
            .file_name()
            .expect("named part file")
            .to_string_lossy()
            .ends_with(".mp4.part")
    );
    assert!(
        std::fs::metadata(open_part)
            .expect("open part statable")
            .len()
            > 0,
        "the open part file already carries bytes"
    );
    assert!(
        !root.join("recordings/cam-part").exists(),
        "nothing visible yet"
    );
    assert!(rows.try_recv().is_err(), "an open piece offers no row");

    recorder.finish();
    assert!(part_files(&root).is_empty(), "the closed piece left .part");
    let rows = drain(&rows);
    assert_eq!(rows.len(), 1);
    assert!(
        root.join(&rows[0].path).exists(),
        "the row names a real file"
    );
    cleanup(&root);
}

#[test]
fn pieces_replay_their_rows_and_a_probe_sees_a_video_stream() {
    let (root, mut recorder, rows) = rig("walk", "cam-walk", 4, 8);
    push_video(&mut recorder, 310, 150);
    recorder.finish();
    let rows = drain(&rows);
    assert!(!rows.is_empty());
    for row in &rows {
        let bytes = std::fs::read(root.join(&row.path)).expect("piece file exists");
        let facts = piece_walk::walk(&bytes).expect("stored piece parses");
        assert_eq!(facts.duration_ms, row.end_ms - row.start_ms);
        assert_eq!(first_tfdt(&bytes), 0);
    }

    let (probe_root, mut probe, probe_rows) = rig("probe", "cam-probe", 4, 8);
    push_video(&mut probe, 250, 1);
    probe.finish();
    let probed = drain(&probe_rows)
        .into_iter()
        .next()
        .expect("the keyframe-only stream closed a piece");
    let probe_file = probe_root.join(&probed.path);
    match std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "default=nw=1",
        ])
        .arg(&probe_file)
        .output()
    {
        Ok(output) => {
            assert!(
                output.status.success(),
                "ffprobe rejected a stored piece: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("codec_type=video"),
                "ffprobe must report a video stream"
            );
        }
        Err(absent) => {
            println!("note: ffprobe is not on PATH ({absent}); skipping the probe check");
        }
    }
    cleanup(&root);
    cleanup(&probe_root);
}

#[test]
fn frames_before_a_keyframe_or_parameter_sets_produce_nothing() {
    let (root, mut recorder, rows) = rig("prekey", "cam-prekey", 4, 4);
    for index in 0..5 {
        recorder.push(&frame(NON_IDR, index * 3000));
    }
    recorder.push(&frame(SPS, 0));
    recorder.push(&frame(PPS, 0));
    for index in 0..100 {
        recorder.push(&frame(NON_IDR, index * 3000));
    }
    recorder.finish();
    assert!(!root.join("recordings").exists());
    assert!(drain(&rows).is_empty());
    cleanup(&root);

    let (bare_root, mut bare, bare_rows) = rig("noparams", "cam-noparams", 4, 4);
    for index in 0..200 {
        bare.push(&frame(IDR, index * 3000));
    }
    bare.finish();
    assert!(!bare_root.join("recordings").exists());
    assert!(drain(&bare_rows).is_empty());
    cleanup(&bare_root);
}

#[test]
fn a_part_directory_obstacle_fails_no_row_and_recording_resumes() {
    let (root, mut recorder, rows) = rig("obstacle", "cam-obstacle", 4, 4);
    std::fs::create_dir_all(root.join("recordings")).expect("recordings dir created");
    std::fs::write(root.join("recordings/.part"), b"obstacle").expect("obstacle in place");
    push_video(&mut recorder, 10, 150);
    recorder.finish();
    assert!(drain(&rows).is_empty(), "the blocked piece offers no row");
    assert!(!root.join("recordings/cam-obstacle").exists());

    std::fs::remove_file(root.join("recordings/.part")).expect("obstacle removed");
    push_video(&mut recorder, 10, 150);
    recorder.finish();
    let rows = drain(&rows);
    assert_eq!(rows.len(), 1, "the next keyframe resumed recording");
    assert!(root.join(&rows[0].path).exists());
    cleanup(&root);
}

#[test]
fn a_full_row_channel_keeps_the_files_and_never_blocks() {
    let (root, mut recorder, rows) = rig("full", "cam-full", 4, 1);
    push_video(&mut recorder, 450, 150);
    recorder.finish();

    let received = drain(&rows);
    assert_eq!(
        received.len(),
        1,
        "the one-place channel held the first row"
    );
    for piece_index in 0..3i64 {
        let start_ms = CLOCK_BASE + piece_index * CLOCK_STEP;
        let path = root.join(format!(
            "recordings/cam-full/{CLOCK_DATE}/{CLOCK_HOUR}/{start_ms}-{}.mp4",
            piece_index + 1
        ));
        assert!(path.exists(), "the piece at {path:?} stays on disk");
    }
    cleanup(&root);
}

#[test]
fn dropping_without_finish_leaves_exactly_one_part_for_the_sweep() {
    let (root, mut recorder, rows) = rig("drop", "cam-drop", 4, 8);
    push_video(&mut recorder, 50, 150);
    drop(recorder);

    let open_parts = part_files(&root);
    assert_eq!(open_parts.len(), 1, "the dropped piece left one .part file");
    assert!(
        !root.join("recordings/cam-drop").exists(),
        "nothing entered the visible tree"
    );
    assert!(drain(&rows).is_empty());

    let mut index = Index::open(&root.join("index.sqlite"), &[("cam-drop".to_owned(), 4)])
        .expect("index reopens");
    let report = recovery::scan(&root, &mut index).expect("scan succeeds");
    assert_eq!(report.part_removed, 1);
    assert_eq!(report.inserted, 0);
    assert!(part_files(&root).is_empty());
    cleanup(&root);
}

#[test]
fn two_cameras_sharing_an_allocator_never_repeat_a_pair() {
    let root = std::env::temp_dir().join(format!("corvette-recorder-pair-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp root created");
    let mut index = Index::open(
        &root.join("index.sqlite"),
        &[("alpha".to_owned(), 4), ("beta".to_owned(), 4)],
    )
    .expect("index opens");
    let seqs = index.allocator();

    let (alpha_tx, alpha_rows) = sync_channel(4);
    let (beta_tx, beta_rows) = sync_channel(4);
    let mut alpha = PieceRecorder::new(
        RecorderSettings {
            camera: "alpha".to_owned(),
            root: root.clone(),
            piece_seconds: 4,
        },
        seqs.clone(),
        alpha_tx,
        Box::new(move || CLOCK_BASE),
    );
    let mut beta = PieceRecorder::new(
        RecorderSettings {
            camera: "beta".to_owned(),
            root: root.clone(),
            piece_seconds: 4,
        },
        seqs.clone(),
        beta_tx,
        Box::new(move || CLOCK_BASE),
    );
    push_video(&mut alpha, 30, 150);
    push_video(&mut beta, 30, 150);
    alpha.finish();
    beta.finish();

    let mut rows = drain(&alpha_rows);
    rows.extend(drain(&beta_rows));
    assert_eq!(rows.len(), 2);
    let mut pairs: Vec<(String, u64)> = rows
        .iter()
        .map(|row| (row.camera.clone(), row.seq))
        .collect();
    pairs.sort();
    pairs.dedup();
    assert_eq!(pairs.len(), 2, "(camera, seq) pairs stayed unique");

    assert_eq!(index.next_seq("alpha").expect("alpha allocates on"), 2);
    assert_eq!(seqs.next("beta").expect("beta allocates on"), 2);
    cleanup(&root);
}

#[test]
fn a_forward_timestamp_jump_closes_at_the_previous_frame_and_the_next_keyframe_starts_a_fresh_piece()
 {
    let (root, mut recorder, rows) = rig("jumpfwd", "cam-jumpfwd", 4, 8);
    recorder.push(&frame(SPS, 0));
    recorder.push(&frame(PPS, 0));
    recorder.push(&frame(IDR, 0));
    recorder.push(&frame(NON_IDR, 3_000));
    recorder.push(&frame(NON_IDR, 6_000));
    // +10 s beyond the expected 3000-tick step: closes the piece at 6000.
    recorder.push(&frame(NON_IDR, 909_000));
    recorder.push(&frame(NON_IDR, 912_000));
    recorder.push(&frame(IDR, 915_000));
    recorder.push(&frame(NON_IDR, 918_000));
    recorder.finish();

    let rows = drain(&rows);
    assert_eq!(rows.len(), 2, "the jump closed the first piece");
    let first = std::fs::read(root.join(&rows[0].path)).expect("first piece on disk");
    assert_eq!(
        rows[0].end_ms - rows[0].start_ms,
        100,
        "ends at the frame before the jump"
    );
    assert_eq!(first_sample_flags(&first), 0x0200_0000);
    let first_facts = piece_walk::walk(&first).expect("first piece parses");
    assert_eq!(first_facts.duration_ms, rows[0].end_ms - rows[0].start_ms);
    assert_eq!(first_tfdt(&first), 0);

    let second = std::fs::read(root.join(&rows[1].path)).expect("second piece on disk");
    assert_eq!(
        rows[1].end_ms - rows[1].start_ms,
        66,
        "the next piece holds only frames from its keyframe on"
    );
    assert_eq!(
        first_sample_flags(&second),
        0x0200_0000,
        "the next piece starts at the keyframe"
    );
    let second_facts = piece_walk::walk(&second).expect("second piece parses");
    assert_eq!(second_facts.duration_ms, rows[1].end_ms - rows[1].start_ms);
    assert_eq!(first_tfdt(&second), 0);
    assert!(part_files(&root).is_empty());
    cleanup(&root);
}

#[test]
fn a_backward_timestamp_jump_closes_at_the_previous_frame_like_a_forward_one() {
    let (root, mut recorder, rows) = rig("jumpback", "cam-jumpback", 4, 8);
    recorder.push(&frame(SPS, 0));
    recorder.push(&frame(PPS, 0));
    recorder.push(&frame(IDR, 600_000));
    recorder.push(&frame(NON_IDR, 603_000));
    recorder.push(&frame(NON_IDR, 606_000));
    let jumped = 606_000u32.wrapping_sub(903_000);
    recorder.push(&frame(NON_IDR, jumped));
    recorder.push(&frame(NON_IDR, jumped.wrapping_add(3_000)));
    recorder.push(&frame(IDR, 700_000));
    recorder.push(&frame(NON_IDR, 703_000));
    recorder.finish();

    let rows = drain(&rows);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].end_ms - rows[0].start_ms,
        100,
        "ends at the frame before the jump"
    );
    assert_eq!(
        rows[1].end_ms - rows[1].start_ms,
        66,
        "starts at the later keyframe"
    );
    for row in &rows {
        let bytes = std::fs::read(root.join(&row.path)).expect("piece on disk");
        let facts = piece_walk::walk(&bytes).expect("piece parses");
        assert_eq!(facts.duration_ms, row.end_ms - row.start_ms);
        assert_eq!(first_tfdt(&bytes), 0);
    }
    assert!(part_files(&root).is_empty());
    cleanup(&root);
}

#[test]
fn timestamps_across_the_u32_wrap_record_one_continuous_piece() {
    let (root, mut recorder, rows) = rig("wrap", "cam-wrap", 4, 8);
    let start = u32::MAX - 200;
    recorder.push(&frame(SPS, start));
    recorder.push(&frame(PPS, start));
    let mut timestamp = start;
    for index in 0..60 {
        let nal = if index % 15 == 0 { IDR } else { NON_IDR };
        recorder.push(&frame(nal, timestamp));
        timestamp = timestamp.wrapping_add(3_000);
    }
    recorder.finish();

    let rows = drain(&rows);
    assert_eq!(rows.len(), 1, "the wrap closed nothing");
    assert_eq!(rows[0].end_ms - rows[0].start_ms, 2_000);
    let bytes = std::fs::read(root.join(&rows[0].path)).expect("piece on disk");
    let facts = piece_walk::walk(&bytes).expect("piece parses");
    assert_eq!(facts.duration_ms, rows[0].end_ms - rows[0].start_ms);
    assert_eq!(first_tfdt(&bytes), 0);
    cleanup(&root);
}

#[test]
fn a_step_of_exactly_the_two_second_threshold_keeps_the_piece_and_one_tick_over_closes_it() {
    let (root, mut recorder, rows) = rig("threshold", "cam-threshold", 4, 8);
    recorder.push(&frame(SPS, 0));
    recorder.push(&frame(PPS, 0));
    recorder.push(&frame(IDR, 0));
    recorder.push(&frame(NON_IDR, 3_000));
    recorder.push(&frame(NON_IDR, 6_000));
    recorder.push(&frame(NON_IDR, 189_000));
    recorder.push(&frame(NON_IDR, 192_000));
    recorder.finish();
    let rows = drain(&rows);
    assert_eq!(
        rows.len(),
        1,
        "a step of exactly the threshold keeps the piece open"
    );
    assert_eq!(rows[0].end_ms - rows[0].start_ms, 2_166);
    cleanup(&root);

    let (over_root, mut over, over_rows) = rig("over", "cam-over", 4, 8);
    over.push(&frame(SPS, 0));
    over.push(&frame(PPS, 0));
    over.push(&frame(IDR, 0));
    over.push(&frame(NON_IDR, 3_000));
    over.push(&frame(NON_IDR, 6_000));
    over.push(&frame(NON_IDR, 189_001));
    over.push(&frame(NON_IDR, 192_001));
    over.finish();
    let over_rows = drain(&over_rows);
    assert_eq!(
        over_rows.len(),
        1,
        "one tick over closed at the previous frame"
    );
    assert_eq!(over_rows[0].end_ms - over_rows[0].start_ms, 100);
    cleanup(&over_root);
}

#[test]
fn a_backward_step_of_one_tick_closes_at_the_previous_frame_like_a_forward_one() {
    let (root, mut recorder, rows) = rig("backtick", "cam-back-tick", 4, 8);
    recorder.push(&frame(SPS, 0));
    recorder.push(&frame(PPS, 0));
    recorder.push(&frame(IDR, 600_000));
    recorder.push(&frame(NON_IDR, 603_000));
    recorder.push(&frame(NON_IDR, 606_000));
    // one tick back: any negative step closes, however small.
    recorder.push(&frame(NON_IDR, 605_999));
    recorder.push(&frame(NON_IDR, 608_999));
    recorder.push(&frame(IDR, 609_000));
    recorder.push(&frame(NON_IDR, 612_000));
    recorder.finish();

    let rows = drain(&rows);
    assert_eq!(rows.len(), 2, "the backward step closed the first piece");
    assert_eq!(
        rows[0].end_ms - rows[0].start_ms,
        100,
        "ends at the frame before the step"
    );
    assert_eq!(rows[1].end_ms - rows[1].start_ms, 66);
    for (index, row) in rows.iter().enumerate() {
        let bytes = std::fs::read(root.join(&row.path)).expect("piece on disk");
        let facts = piece_walk::walk(&bytes).expect("piece parses");
        assert_eq!(facts.duration_ms, row.end_ms - row.start_ms);
        assert!(facts.duration_ms < 60_000, "no wrapped sample duration");
        assert_eq!(first_tfdt(&bytes), 0);
        if index > 0 {
            assert_eq!(first_sample_flags(&bytes), 0x0200_0000);
        }
    }
    assert!(part_files(&root).is_empty());
    cleanup(&root);
}

#[test]
fn a_backward_step_of_one_second_closes_at_the_previous_frame_like_a_forward_one() {
    let (root, mut recorder, rows) = rig("backsecond", "cam-back-second", 4, 8);
    recorder.push(&frame(SPS, 0));
    recorder.push(&frame(PPS, 0));
    recorder.push(&frame(IDR, 600_000));
    recorder.push(&frame(NON_IDR, 603_000));
    recorder.push(&frame(NON_IDR, 606_000));
    // one second back: far under the forward threshold, yet still a close.
    recorder.push(&frame(NON_IDR, 606_000 - 90_000));
    recorder.push(&frame(NON_IDR, 609_000 - 90_000));
    recorder.push(&frame(IDR, 609_000));
    recorder.push(&frame(NON_IDR, 612_000));
    recorder.finish();

    let rows = drain(&rows);
    assert_eq!(rows.len(), 2, "the backward step closed the first piece");
    assert_eq!(
        rows[0].end_ms - rows[0].start_ms,
        100,
        "ends at the frame before the step"
    );
    assert_eq!(rows[1].end_ms - rows[1].start_ms, 66);
    for (index, row) in rows.iter().enumerate() {
        let bytes = std::fs::read(root.join(&row.path)).expect("piece on disk");
        let facts = piece_walk::walk(&bytes).expect("piece parses");
        assert_eq!(facts.duration_ms, row.end_ms - row.start_ms);
        assert!(facts.duration_ms < 60_000, "no wrapped sample duration");
        assert_eq!(first_tfdt(&bytes), 0);
        if index > 0 {
            assert_eq!(first_sample_flags(&bytes), 0x0200_0000);
        }
    }
    assert!(part_files(&root).is_empty());
    cleanup(&root);
}

#[test]
fn a_backward_step_of_ten_seconds_closes_at_the_previous_frame_like_a_forward_one() {
    let (root, mut recorder, rows) = rig("backten", "cam-back-ten", 4, 8);
    recorder.push(&frame(SPS, 0));
    recorder.push(&frame(PPS, 0));
    recorder.push(&frame(IDR, 600_000));
    recorder.push(&frame(NON_IDR, 603_000));
    recorder.push(&frame(NON_IDR, 606_000));
    // ten seconds back, wrapping the u32 clock: closes like any negative step.
    let jumped = 606_000u32.wrapping_sub(900_000);
    recorder.push(&frame(NON_IDR, jumped));
    recorder.push(&frame(NON_IDR, jumped.wrapping_add(3_000)));
    recorder.push(&frame(IDR, 609_000));
    recorder.push(&frame(NON_IDR, 612_000));
    recorder.finish();

    let rows = drain(&rows);
    assert_eq!(rows.len(), 2, "the backward step closed the first piece");
    assert_eq!(
        rows[0].end_ms - rows[0].start_ms,
        100,
        "ends at the frame before the step"
    );
    assert_eq!(rows[1].end_ms - rows[1].start_ms, 66);
    for (index, row) in rows.iter().enumerate() {
        let bytes = std::fs::read(root.join(&row.path)).expect("piece on disk");
        let facts = piece_walk::walk(&bytes).expect("piece parses");
        assert_eq!(facts.duration_ms, row.end_ms - row.start_ms);
        assert!(facts.duration_ms < 60_000, "no wrapped sample duration");
        assert_eq!(first_tfdt(&bytes), 0);
        if index > 0 {
            assert_eq!(first_sample_flags(&bytes), 0x0200_0000);
        }
    }
    assert!(part_files(&root).is_empty());
    cleanup(&root);
}
