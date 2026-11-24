use corvette_media_bridge::fmp4::{Fragmenter, InitSegmentTracker};
use corvette_media_bridge::index::{Index, PieceRow};
use corvette_media_bridge::recovery::{self, ScanReport};
use corvette_rtsp_client::depacketize::{Codec, Frame};
use std::path::{Path, PathBuf};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("corvette-recovery-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("test temp directory created");
    dir
}

fn cleanup(dir: &Path) {
    std::fs::remove_dir_all(dir).expect("test temp directory removed");
}

const SPS: &[u8] = &[
    0x67, 0x42, 0xc0, 0x1f, 0xda, 0x01, 0x40, 0x16, 0xe9, 0xb8, 0x08, 0x08, 0x0a, 0x00, 0x00, 0x07,
    0xd0, 0x00, 0x01, 0xd4, 0xc0, 0x80,
];
const PPS: &[u8] = &[0x68, 0xce, 0x3c, 0x80];
const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x21];
const NON_IDR: &[u8] = &[0x41, 0x9a, 0x24, 0x6c];

fn annex_b(nal: &[u8]) -> bytes::Bytes {
    let mut payload = Vec::with_capacity(nal.len() + 4);
    payload.extend_from_slice(&[0, 0, 0, 1]);
    payload.extend_from_slice(nal);
    bytes::Bytes::from(payload)
}

fn frame(timestamp: u32, nal: &[u8]) -> Frame {
    Frame {
        codec: Codec::H264,
        timestamp,
        payload: annex_b(nal),
    }
}

// The same two-fragment piece shape the index store tests use: 6000 ticks at
// 90 kHz is a 66 ms duration and one tfdt pair the walk can read.
fn valid_piece() -> Vec<u8> {
    let mut tracker = InitSegmentTracker::default();
    tracker.observe(&frame(0, SPS));
    let mut piece = tracker
        .observe(&frame(0, PPS))
        .expect("SPS and PPS complete the init segment")
        .to_vec();
    let mut fragmenter = Fragmenter::default();
    for (index, timestamp) in [0u32, 3000].iter().enumerate() {
        let nal = if index == 0 { IDR } else { NON_IDR };
        let fragment = fragmenter
            .next(&frame(*timestamp, nal))
            .expect("each VCL slice yields one fragment");
        piece.extend_from_slice(&fragment.bytes);
    }
    piece
}

fn front() -> Vec<(String, u32)> {
    vec![("front".to_owned(), 4u32)]
}

fn row(seq: u64, start_ms: i64, path: &str) -> PieceRow {
    PieceRow {
        camera: "front".to_owned(),
        seq,
        start_ms,
        end_ms: start_ms + 66,
        duration_s: 0.066,
        path: path.to_owned(),
        size_bytes: 1,
        codec: "h264".to_owned(),
    }
}

fn hour_dir(root: &Path) -> PathBuf {
    let hour = root.join("recordings/front/2026-11-24/09");
    std::fs::create_dir_all(&hour).expect("hour directory created");
    hour
}

#[test]
fn full_tree_yields_each_orphan_disposition_exactly_once() {
    let dir = temp_dir("full-tree");
    let root = dir.join("root");
    std::fs::create_dir_all(&root).expect("root created");
    let hour = hour_dir(&root);
    std::fs::create_dir_all(root.join("recordings/.part")).expect("part directory created");
    std::fs::create_dir_all(root.join(".trash")).expect("trash directory created");
    let mut index = Index::open(&root.join("index.sqlite"), &front()).expect("index opens");

    let healthy_bytes = valid_piece();
    std::fs::write(hour.join("1790000000000-5.mp4"), &healthy_bytes)
        .expect("healthy piece written");
    index
        .insert(&row(
            5,
            1_790_000_000_000,
            "recordings/front/2026-11-24/09/1790000000000-5.mp4",
        ))
        .expect("healthy row inserted");

    let orphan_bytes = valid_piece();
    std::fs::write(hour.join("1790000003000-6.mp4"), &orphan_bytes).expect("orphan piece written");

    std::fs::write(hour.join("1790000004000-9.mp4"), b"not a piece").expect("bad piece written");
    std::fs::write(hour.join("broken-name.mp4"), b"junk").expect("bad name written");

    index
        .insert(&row(
            7,
            1_790_000_005_000,
            "recordings/front/2026-11-24/09/1790000005000-7.mp4",
        ))
        .expect("dangling row inserted");

    std::fs::write(
        root.join("recordings/.part/1790000006000-8.mp4.part"),
        b"half written",
    )
    .expect("part file written");
    std::fs::write(root.join(".trash/stale.mp4"), b"expiring").expect("trash file written");

    let report = recovery::scan(&root, &mut index).expect("first scan succeeds");
    assert_eq!(
        report,
        ScanReport {
            inserted: 1,
            rows_dropped: 1,
            part_removed: 1,
            trash_removed: 1,
            quarantined: 2,
            skipped_dirs: 0,
            rows_refused: 0,
        }
    );

    let rows = index.all_pieces().expect("rows read back");
    let recovered = rows
        .iter()
        .find(|row| row.seq == 6)
        .expect("orphan file inserted its own row");
    assert_eq!(recovered.start_ms, 1_790_000_003_000);
    assert_eq!(recovered.end_ms, 1_790_000_003_066);
    assert_eq!(
        recovered.size_bytes,
        u64::try_from(orphan_bytes.len()).expect("length fits")
    );
    assert_eq!(recovered.codec, "h264");
    assert_eq!(
        recovered.path,
        "recordings/front/2026-11-24/09/1790000003000-6.mp4"
    );
    assert!((recovered.duration_s - 0.066).abs() < 1e-9);
    assert!(rows.iter().any(|row| row.seq == 5), "healthy row kept");
    assert!(!rows.iter().any(|row| row.seq == 7), "dangling row dropped");

    assert_eq!(
        std::fs::read(hour.join("1790000000000-5.mp4")).expect("healthy piece still there"),
        healthy_bytes
    );
    assert!(
        !root
            .join("recordings/.part/1790000006000-8.mp4.part")
            .exists()
    );
    assert!(!root.join(".trash/stale.mp4").exists());
    assert_eq!(
        std::fs::read(root.join("lost+found/front-1790000004000-9.mp4"))
            .expect("unparseable piece quarantined by name"),
        b"not a piece"
    );
    assert!(root.join("lost+found/front-broken-name.mp4").exists());

    let second = recovery::scan(&root, &mut index).expect("second scan succeeds");
    assert_eq!(second, ScanReport::default());
    assert_eq!(
        std::fs::read_dir(root.join("lost+found"))
            .expect("lost+found readable")
            .count(),
        2,
        "quarantined files are never re-scanned"
    );

    cleanup(&dir);
}

#[test]
fn newline_in_a_file_name_is_quarantined_without_naming_it() {
    let dir = temp_dir("newline-name");
    let root = dir.join("root");
    let hour = hour_dir(&root);
    std::fs::write(hour.join("x\nfake line.mp4"), b"junk").expect("newline name written");
    let mut index = Index::open(&root.join("index.sqlite"), &front()).expect("index opens");

    let report = recovery::scan(&root, &mut index).expect("scan succeeds");
    assert_eq!(report.quarantined, 1);
    assert_eq!(report.inserted, 0);
    assert_eq!(report.rows_dropped, 0);
    assert_eq!(report.skipped_dirs, 0);
    assert_eq!(report.rows_refused, 0);
    assert_eq!(
        std::fs::read_dir(root.join("lost+found"))
            .expect("lost+found readable")
            .count(),
        1
    );

    cleanup(&dir);
}

#[test]
fn untrusted_camera_directories_are_skipped_with_contents_intact() {
    let dir = temp_dir("skipped-dirs");
    let root = dir.join("root");
    std::fs::create_dir_all(&root).expect("root created");
    let mut index = Index::open(&root.join("index.sqlite"), &front()).expect("index opens");
    let cameras = ["camera one", "new\nline", "a b"];
    let contents = b"untouched".as_slice();
    for camera in cameras {
        let place = root.join(format!("recordings/{camera}/2026-11-24/09"));
        std::fs::create_dir_all(&place).expect("camera hour directory created");
        std::fs::write(place.join("1790000000000-1.mp4"), contents).expect("file written");
    }

    let report = recovery::scan(&root, &mut index).expect("scan succeeds");
    assert_eq!(
        report,
        ScanReport {
            skipped_dirs: 3,
            ..ScanReport::default()
        }
    );
    assert!(index.all_pieces().expect("rows read back").is_empty());
    for camera in cameras {
        let file = root.join(format!(
            "recordings/{camera}/2026-11-24/09/1790000000000-1.mp4"
        ));
        assert_eq!(
            std::fs::read(&file).expect("skipped file untouched"),
            contents
        );
    }

    cleanup(&dir);
}

#[test]
fn rows_with_paths_outside_the_root_are_refused_not_statted() {
    let dir = temp_dir("refused-rows");
    let root = dir.join("root");
    std::fs::create_dir_all(&root).expect("root created");
    std::fs::create_dir_all(root.join("recordings")).expect("recordings created");
    let outside = dir.join("outside.mp4");
    std::fs::write(&outside, b"outside").expect("outside file written");
    let mut index = Index::open(&root.join("index.sqlite"), &front()).expect("index opens");
    for (seq, path) in [
        (11u64, outside.display().to_string()),
        (12, "../outside.mp4".to_owned()),
    ] {
        index
            .insert(&PieceRow {
                camera: "front".to_owned(),
                seq,
                start_ms: 0,
                end_ms: 66,
                duration_s: 0.066,
                path,
                size_bytes: 1,
                codec: "h264".to_owned(),
            })
            .expect("escaping row inserted");
    }

    let report = recovery::scan(&root, &mut index).expect("scan succeeds");
    assert_eq!(
        report,
        ScanReport {
            rows_refused: 2,
            ..ScanReport::default()
        }
    );
    let rows = index.all_pieces().expect("rows read back");
    assert!(rows.iter().any(|row| row.seq == 11), "absolute row kept");
    assert!(
        rows.iter().any(|row| row.seq == 12),
        "parent-directory row kept"
    );
    assert_eq!(
        std::fs::read(&outside).expect("outside file untouched"),
        b"outside"
    );

    cleanup(&dir);
}

#[test]
fn oversized_piece_is_quarantined_without_being_read() {
    let dir = temp_dir("oversized");
    let root = dir.join("root");
    let hour = hour_dir(&root);
    let mut index = Index::open(&root.join("index.sqlite"), &front()).expect("index opens");
    let mut piece = valid_piece();
    let mut last_mdat: Option<(usize, usize)> = None;
    let mut at = 0;
    while at < piece.len() {
        let size = u32::from_be_bytes(piece[at..at + 4].try_into().expect("box size")) as usize;
        let mut kind = [0u8; 4];
        kind.copy_from_slice(&piece[at + 4..at + 8]);
        if kind == *b"mdat" {
            last_mdat = Some((at, size));
        }
        at += size;
    }
    let (mdat_at, mdat_size) = last_mdat.expect("fixture has an mdat");
    let hole = 256 * 1024 * 1024usize;
    let declared: u32 = (mdat_size + hole).try_into().expect("size fits u32");
    piece[mdat_at..mdat_at + 4].copy_from_slice(&declared.to_be_bytes());
    let path = hour.join("1790000009000-11.mp4");
    let mut file = std::fs::File::create(&path).expect("sparse piece created");
    std::io::Write::write_all(&mut file, &piece).expect("piece bytes written");
    file.set_len(u64::try_from(piece.len() + hole).expect("sparse length fits u64"))
        .expect("sparse length set");
    drop(file);

    let report = recovery::scan(&root, &mut index).expect("scan succeeds");
    assert_eq!(
        report,
        ScanReport {
            quarantined: 1,
            ..ScanReport::default()
        }
    );
    assert!(root.join("lost+found/front-1790000009000-11.mp4").exists());
    assert!(index.all_pieces().expect("rows read back").is_empty());

    cleanup(&dir);
}

#[test]
fn taken_quarantine_name_gains_the_smallest_free_dup_suffix() {
    let dir = temp_dir("dup-name");
    let root = dir.join("root");
    let hour = hour_dir(&root);
    std::fs::create_dir_all(root.join("lost+found")).expect("lost+found pre-created");
    std::fs::write(root.join("lost+found/front-1790000004000-9.mp4"), b"first")
        .expect("taken name pre-created");
    std::fs::write(hour.join("1790000004000-9.mp4"), b"second").expect("bad piece written");
    let mut index = Index::open(&root.join("index.sqlite"), &front()).expect("index opens");

    let report = recovery::scan(&root, &mut index).expect("scan succeeds");
    assert_eq!(
        report,
        ScanReport {
            quarantined: 1,
            ..ScanReport::default()
        }
    );
    assert_eq!(
        std::fs::read(root.join("lost+found/front-1790000004000-9.mp4"))
            .expect("existing quarantine kept"),
        b"first"
    );
    assert_eq!(
        std::fs::read(root.join("lost+found/front-1790000004000-9.mp4.dup1"))
            .expect("quarantine landed on dup1"),
        b"second"
    );

    cleanup(&dir);
}

#[test]
fn absent_tree_scans_clean_and_creates_nothing() {
    let dir = temp_dir("absent-tree");
    let mut index = Index::open(&dir.join("index.sqlite"), &[]).expect("index opens");

    let report = recovery::scan(&dir, &mut index).expect("scan of absent tree succeeds");
    assert_eq!(report, ScanReport::default());
    assert!(!dir.join("recordings").exists());
    assert!(!dir.join("lost+found").exists());

    cleanup(&dir);
}

#[test]
fn empty_tree_scans_clean() {
    let dir = temp_dir("empty-tree");
    std::fs::create_dir_all(dir.join("recordings")).expect("empty recordings created");
    let mut index = Index::open(&dir.join("index.sqlite"), &[]).expect("index opens");

    let report = recovery::scan(&dir, &mut index).expect("scan of empty tree succeeds");
    assert_eq!(report, ScanReport::default());

    cleanup(&dir);
}
