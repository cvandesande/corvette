//! Turns one camera's frame stream into the stored-piece tree: fragments land in a `.part` file, and a closed piece is synced and renamed into the visible tree before its index row is offered.

use crate::fmp4::{Fragmenter, InitSegmentTracker};
use crate::index::{PieceRow, SeqAllocator};
use crate::rtp_clock::VIDEO_CLOCK_RATE_HZ;
use crate::supervise::log_event;
use crate::utc::utc_parts;
use corvette_rtsp_client::depacketize::Frame as ClientFrame;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct RecorderSettings {
    pub camera: String,
    pub root: PathBuf,
    pub piece_seconds: u32,
}

struct OpenPiece {
    part_path: PathBuf,
    file: File,
    fragmenter: Fragmenter,
    seq: u64,
    start_ms: i64,
    ticks: u64,
    bytes_written: usize,
    codec: &'static str,
}

/// Writes one camera's pieces from frames handed to it by its owner; `clock` is read at each piece start, and dropping without `finish` leaves the open piece's `.part` file for the startup sweep.
pub struct PieceRecorder {
    settings: RecorderSettings,
    seqs: SeqAllocator,
    rows: SyncSender<PieceRow>,
    clock: Box<dyn Fn() -> i64 + Send>,
    tracker: InitSegmentTracker,
    open: Option<OpenPiece>,
    write_blocked: bool,
}

impl std::fmt::Debug for PieceRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PieceRecorder")
            .field("camera", &self.settings.camera)
            .field("open_seq", &self.open.as_ref().map(|piece| piece.seq))
            .finish_non_exhaustive()
    }
}

impl PieceRecorder {
    #[must_use]
    pub fn new(
        settings: RecorderSettings,
        seqs: SeqAllocator,
        rows: SyncSender<PieceRow>,
        clock: Box<dyn Fn() -> i64 + Send>,
    ) -> Self {
        Self {
            settings,
            seqs,
            rows,
            clock,
            tracker: InitSegmentTracker::default(),
            open: None,
            write_blocked: false,
        }
    }

    /// Returns the system time in milliseconds since the Unix epoch, saturating at both ends rather than failing.
    #[must_use]
    pub fn wall_clock_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            })
    }

    /// Feeds every frame to the long-lived parameter-set tracker, then to the open piece or a start attempt; frames before the first keyframe of a camera whose parameter sets are known produce nothing.
    pub fn push(&mut self, frame: &ClientFrame) {
        self.tracker.observe(frame);
        let cut_ticks = self.cut_ticks();
        let Some(mut piece) = self.open.take() else {
            self.maybe_start(frame);
            return;
        };
        let Some(fragment) = piece.fragmenter.next(frame) else {
            self.open = Some(piece);
            return;
        };
        if fragment.is_keyframe && piece.ticks >= cut_ticks {
            self.close_piece(piece);
            self.maybe_start(frame);
            return;
        }
        if let Err(error) = piece.file.write_all(&fragment.bytes) {
            let OpenPiece { part_path, .. } = piece;
            abandon_part(&part_path);
            self.note_write_failure(format_args!("write: {error}"));
            return;
        }
        piece.ticks += u64::from(fragment.duration_ticks);
        piece.bytes_written += fragment.bytes.len();
        self.open = Some(piece);
    }

    /// Closes the open piece the way a cut does; does nothing when none is open.
    pub fn finish(&mut self) {
        if let Some(piece) = self.open.take() {
            self.close_piece(piece);
        }
    }

    fn cut_ticks(&self) -> u64 {
        u64::from(self.settings.piece_seconds) * u64::from(VIDEO_CLOCK_RATE_HZ)
    }

    fn maybe_start(&mut self, frame: &ClientFrame) {
        let mut fresh = Fragmenter::stored();
        let Some(fragment) = fresh.next(frame) else {
            return;
        };
        if !fragment.is_keyframe {
            return;
        }
        let Some(init) = self.tracker.current_init() else {
            return;
        };
        let Some(codec) = codec_from_init(&init) else {
            self.note_write_failure("codec");
            return;
        };
        let camera = &self.settings.camera;
        let Ok(seq) = self.seqs.next(camera) else {
            self.note_write_failure("sequence");
            return;
        };
        let start_ms = (self.clock)();
        let part_dir = self.settings.root.join("recordings/.part");
        let part_path = part_dir.join(format!("{camera}-{seq}.mp4.part"));
        let opened = (|| -> std::io::Result<File> {
            fs::create_dir_all(&part_dir)?;
            let mut file = File::create_new(&part_path)?;
            file.write_all(&init)?;
            file.write_all(&fragment.bytes)?;
            Ok(file)
        })();
        let Ok(file) = opened else {
            abandon_part(&part_path);
            self.note_write_failure("start");
            return;
        };
        self.open = Some(OpenPiece {
            part_path,
            file,
            fragmenter: fresh,
            seq,
            start_ms,
            ticks: u64::from(fragment.duration_ticks),
            bytes_written: init.len() + fragment.bytes.len(),
            codec,
        });
    }

    fn close_piece(&mut self, piece: OpenPiece) {
        let OpenPiece {
            part_path,
            file,
            seq,
            start_ms,
            ticks,
            bytes_written,
            codec,
            ..
        } = piece;
        let parts = utc_parts(start_ms);
        let date = format!("{:04}-{:02}-{:02}", parts.year, parts.month, parts.day);
        let hour = format!("{:02}", parts.hour);
        let camera = &self.settings.camera;
        let final_path = self
            .settings
            .root
            .join("recordings")
            .join(camera)
            .join(&date)
            .join(&hour)
            .join(format!("{start_ms}-{seq}.mp4"));
        if close_and_move(file, &part_path, &final_path).is_err() {
            abandon_part(&part_path);
            self.note_write_failure("close");
            return;
        }
        if self.write_blocked {
            log_event(camera, "record-resumed", "");
            self.write_blocked = false;
        }
        let span_ms = ticks.saturating_mul(1_000) / u64::from(VIDEO_CLOCK_RATE_HZ);
        let duration_ms = i64::try_from(span_ms).expect("a piece's millisecond span fits i64");
        let row = PieceRow {
            camera: camera.clone(),
            seq,
            start_ms,
            end_ms: start_ms.saturating_add(duration_ms),
            duration_s: Duration::from_millis(
                u64::try_from(duration_ms).expect("a piece's duration is never negative"),
            )
            .as_secs_f64(),
            path: format!("recordings/{camera}/{date}/{hour}/{start_ms}-{seq}.mp4"),
            size_bytes: u64::try_from(bytes_written)
                .expect("a length this process could write fits u64"),
            codec: codec.to_owned(),
        };
        if self.rows.try_send(row).is_err() {
            log_event(camera, "record-index-queue-full", "");
        }
    }

    fn note_write_failure(&mut self, detail: impl std::fmt::Display) {
        if self.write_blocked {
            return;
        }
        log_event(&self.settings.camera, "record-write-failed", detail);
        self.write_blocked = true;
    }
}

/// Removes an abandoned piece's own part file, the only file this recorder ever deletes.
fn abandon_part(part_path: &Path) {
    let _ = fs::remove_file(part_path);
}

fn close_and_move(mut file: File, part_path: &Path, final_path: &Path) -> std::io::Result<()> {
    file.flush()?;
    file.sync_all()?;
    drop(file);
    let final_dir = final_path
        .parent()
        .expect("a piece's final path always names a directory component");
    fs::create_dir_all(final_dir)?;
    fs::rename(part_path, final_path)
}

/// Reads the `stsd` sample-entry fourcc, the same field `crate::piece_walk::walk` reads, so a row's codec matches what the stored file itself declares.
fn codec_from_init(init: &[u8]) -> Option<&'static str> {
    // The fourcc sits 16 bytes after the `stsd` type field: version and flags,
    // entry count, and the sample entry's own size come first.
    let fourcc = init
        .windows(4)
        .position(|window| window == *b"stsd")
        .and_then(|marker| init.get(marker + 16..marker + 20));
    match fourcc {
        Some(b"avc1") => Some("h264"),
        Some(b"hvc1" | b"hev1") => Some("h265"),
        Some(_) | None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Index;
    use corvette_rtsp_client::depacketize::Codec;
    use std::sync::atomic::{AtomicI64, Ordering};

    fn annex_b(nal: &[u8]) -> bytes::Bytes {
        let mut payload = Vec::with_capacity(nal.len() + 4);
        payload.extend_from_slice(&[0, 0, 0, 1]);
        payload.extend_from_slice(nal);
        bytes::Bytes::from(payload)
    }

    fn frame(codec: Codec, nal: &[u8]) -> ClientFrame {
        ClientFrame {
            codec,
            timestamp: 0,
            payload: annex_b(nal),
        }
    }

    #[test]
    fn codec_reads_the_sample_entry_the_init_segment_carries() {
        let mut tracker = InitSegmentTracker::default();
        tracker.observe(&frame(Codec::H264, &[0x67, 0x42, 0xc0, 0x1f]));
        tracker.observe(&frame(Codec::H264, &[0x68, 0xce, 0x3c, 0x80]));
        let init = tracker.current_init().expect("SPS+PPS complete the set");
        assert_eq!(codec_from_init(&init), Some("h264"));

        let mut h265 = InitSegmentTracker::default();
        h265.observe(&frame(Codec::H265, &[0x40, 0x01]));
        h265.observe(&frame(Codec::H265, &[0x42, 0x01]));
        h265.observe(&frame(Codec::H265, &[0x44, 0x01]));
        let init = h265.current_init().expect("VPS+SPS+PPS complete the set");
        assert_eq!(codec_from_init(&init), Some("h265"));

        assert_eq!(codec_from_init(b"no sample entry here"), None);
    }

    #[test]
    fn wall_clock_is_positive_now() {
        assert!(PieceRecorder::wall_clock_ms() > 0);
    }

    #[test]
    fn a_write_failure_streak_reports_only_its_first_entry() {
        let dir =
            std::env::temp_dir().join(format!("corvette-recorder-streak-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp directory created");
        let index = Index::open(&dir.join("index.sqlite"), &[("streak".to_owned(), 4)])
            .expect("index opens");
        let (rows, _rx) = std::sync::mpsc::sync_channel(1);
        let counter = AtomicI64::new(0);
        let mut recorder = PieceRecorder::new(
            RecorderSettings {
                camera: "streak".to_owned(),
                root: dir.clone(),
                piece_seconds: 4,
            },
            index.allocator(),
            rows,
            Box::new(move || counter.load(Ordering::SeqCst)),
        );
        recorder.note_write_failure("write");
        assert!(recorder.write_blocked);
        recorder.note_write_failure("close");
        assert!(recorder.write_blocked);
        std::fs::remove_dir_all(&dir).expect("temp directory removed");
    }
}
