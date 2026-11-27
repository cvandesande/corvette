use std::cell::RefCell;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use corvette_detect::{
    Area, CandidateError, EngineError, Labels, ReplayConfig, ReplayError, ReplaySegmentError,
    ReplaySummary, SegmentName, replay_segment,
};
use corvette_detect_proto::{HEADER_LEN, Reply, Request, Row, parse_header};

static COUNTER: AtomicU32 = AtomicU32::new(0);

// ffmpeg children are process-wide and the ffmpeg-leak check scans all of them, so ffmpeg tests never overlap.
static FFMPEG_GATE: Mutex<()> = Mutex::new(());

/// Returns the process-wide ffmpeg lock for one test, recovered from poisoning so a panicking test does not stop the others.
fn ffmpeg_gate() -> MutexGuard<'static, ()> {
    FFMPEG_GATE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "replay-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn make_segment(dir: &Path) -> PathBuf {
    let path = dir.join("cam_main-20261101T120000Z.ts");
    let output = std::process::Command::new("ffmpeg")
        .args(
            "-nostdin -loglevel error -y -f lavfi -i testsrc=size=704x576:rate=5 \
             -t 3 -c:v mpeg2video"
                .split_whitespace(),
        )
        .arg(&path)
        .output()
        .expect("ffmpeg is installed; these tests must fail, not skip, without it");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "segment gen failed: {stderr}");
    path
}

const fn person_row() -> Row {
    Row {
        class_id: 0,
        score: 0.9,
        y_min: 0.25,
        x_min: 0.25,
        y_max: 0.75,
        x_max: 0.75,
    }
}

fn read_request_id(stream: &mut UnixStream) -> Option<u64> {
    let mut header = [0u8; HEADER_LEN];
    if stream.read_exact(&mut header).is_err() {
        return None;
    }
    let parsed = parse_header(&header).expect("request header parses");
    let mut frame = vec![0u8; HEADER_LEN + parsed.body_len];
    frame[..HEADER_LEN].copy_from_slice(&header);
    if stream.read_exact(&mut frame[HEADER_LEN..]).is_err() {
        return None;
    }
    let Ok(Request::InferRequest { id, .. }) = Request::decode(&frame) else {
        return None;
    };
    Some(id)
}

fn serve(listener: UnixListener, rows: Vec<Row>, close_after: usize) -> JoinHandle<()> {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("worker accept");
        let mut answered: usize = 0;
        while let Some(id) = read_request_id(&mut stream) {
            let reply = Reply::Rows {
                id,
                worker_ms: 0,
                rows: rows.clone(),
            };
            let bytes = reply.encode().expect("reply encodes");
            stream.write_all(&bytes).expect("reply write");
            answered += 1;
            if answered >= close_after {
                break;
            }
        }
    })
}

const fn same(actual: f64, expected: f64) -> bool {
    actual.to_bits() == expected.to_bits()
}

#[derive(Clone, Default)]
struct SharedOutput {
    pending: Rc<RefCell<Vec<u8>>>,
    flushed: Rc<RefCell<Vec<u8>>>,
}

impl SharedOutput {
    fn bytes(&self) -> Vec<u8> {
        self.flushed.borrow().clone()
    }

    fn text(&self) -> String {
        String::from_utf8(self.bytes()).expect("the record is utf-8")
    }
}

impl Write for SharedOutput {
    fn write(&mut self, buffer: &[u8]) -> Result<usize, std::io::Error> {
        self.pending.borrow_mut().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        self.flushed
            .borrow_mut()
            .append(&mut self.pending.borrow_mut());
        Ok(())
    }
}

fn config(socket: PathBuf) -> ReplayConfig {
    ReplayConfig {
        camera: "cam_replay".to_string(),
        frame_width: 704,
        frame_height: 576,
        fps: 5,
        socket,
        deadline: Duration::from_secs(30),
        labels: Labels::parse("0 person\n").expect("labels parse"),
        confirm_threshold: 0.7,
        area: None,
    }
}

fn area_config(socket: PathBuf, area: Option<Area>) -> ReplayConfig {
    ReplayConfig {
        area,
        ..config(socket)
    }
}

fn replay_case(tag: &str, rows: Vec<Row>) -> (ReplaySummary, SharedOutput) {
    let dir = temp_dir(tag);
    let segment = make_segment(&dir);
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("worker socket binds");
    let server = serve(listener, rows, usize::MAX);
    let out = SharedOutput::default();
    let summary = replay_segment(&segment, &config(socket), out.clone()).expect("replay succeeds");
    server.join().expect("worker thread joins");
    (summary, out)
}

const fn is_candidate_io(error: &ReplaySegmentError) -> bool {
    let ReplaySegmentError::Stage(inner) = error else {
        return false;
    };
    let ReplayError::Candidate(CandidateError::Engine(EngineError::Io(_))) = inner else {
        return false;
    };
    true
}

fn frame_time(line: &str) -> f64 {
    line.strip_prefix("{\"frame_time\":")
        .expect("frame line")
        .split(',')
        .next()
        .expect("frame_time field")
        .parse::<f64>()
        .expect("frame_time number")
}

fn ffmpeg_children() -> Vec<String> {
    let mut found = Vec::new();
    for entry in fs::read_dir("/proc").expect("proc is readable") {
        let path = entry.expect("proc entry").path();
        let Ok(stat) = fs::read_to_string(path.join("stat")) else {
            continue;
        };
        let mut fields = stat.split(' ');
        let comm = fields.nth(1).unwrap_or("");
        let state = fields.next().unwrap_or("");
        let ppid = fields.next().unwrap_or("");
        if comm == "(ffmpeg)"
            && state != "Z"
            && ppid.parse::<u32>().unwrap_or(0) == std::process::id()
        {
            found.push(path.display().to_string());
        }
    }
    found
}

#[test]
fn the_record_holds_a_header_and_one_line_per_decoded_frame() {
    let _ffmpeg_gate = ffmpeg_gate();
    let (summary, out) = replay_case("lines", Vec::new());
    let text = out.text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(summary.frames, 15, "fifteen frames from three seconds");
    assert_eq!(lines.len(), 16, "one header line plus one line per frame");
    assert!(lines[0].contains("cam_replay"), "header camera");
    assert!(
        lines[0].contains("cam_main-20261101T120000Z.ts"),
        "segment name"
    );
    assert!(lines[0].contains("\"frame_width\":704"), "header width");
    assert!(lines[0].contains("\"frame_height\":576"), "header height");
    assert!(lines[0].contains("\"detect_fps\":5.0"), "fps in header");
    let name = SegmentName::parse("cam_main-20261101T120000Z.ts").expect("name");
    let start = name.start().as_secs_f64();
    assert!(same(frame_time(lines[1]), start), "first frame time");
    assert!(same(frame_time(lines[2]), start + 0.2), "second time");
}

#[test]
fn two_runs_over_the_same_segment_give_byte_identical_records() {
    let _ffmpeg_gate = ffmpeg_gate();
    let rows = vec![person_row()];
    let (first, first_out) = replay_case("det-one", rows.clone());
    let (second, second_out) = replay_case("det-two", rows);
    assert_eq!(first.frames, second.frames);
    assert_eq!(first_out.bytes(), second_out.bytes());
}

#[test]
fn a_person_row_from_the_worker_is_tracked_and_listed_on_later_frames() {
    let _ffmpeg_gate = ffmpeg_gate();
    let (_summary, out) = replay_case("person", vec![person_row()]);
    let text = out.text();
    let lines: Vec<&str> = text.lines().collect();
    let first = lines
        .iter()
        .position(|line| line.contains("\"id\":\"1\""))
        .expect("id one appears");
    assert!(first >= 1, "frame lines start after the header");
    let later = lines[first + 1..]
        .iter()
        .filter(|line| line.contains("\"id\":\"1\""))
        .count();
    assert!(later >= 1, "id one is listed on later frame lines too");
}

#[test]
fn a_missing_segment_a_bad_name_and_an_unopened_socket_each_fail_before_any_output() {
    let _ffmpeg_gate = ffmpeg_gate();
    let dir = temp_dir("fail");
    let out = SharedOutput::default();
    let missing = dir.join("cam_gone-20261101T120000Z.ts");
    let error = replay_segment(&missing, &config(dir.join("s1.sock")), out.clone())
        .expect_err("a missing segment fails");
    assert!(matches!(error, ReplaySegmentError::Io(_)), "io: {error}");
    let bad = dir.join("not-a-segment.txt");
    fs::write(&bad, b"x").expect("file with an unparseable name");
    let error = replay_segment(&bad, &config(dir.join("s2.sock")), out.clone())
        .expect_err("an unparseable name fails");
    assert!(matches!(error, ReplaySegmentError::Segment(_)), "bad name");
    let segment = make_segment(&dir);
    let error = replay_segment(
        &segment,
        &config(dir.join("nobody-listens.sock")),
        out.clone(),
    )
    .expect_err("a socket nobody listens on fails");
    assert!(
        matches!(error, ReplaySegmentError::Worker(_)),
        "no listener"
    );
    assert!(out.bytes().is_empty(), "nothing written before failure");
}

#[test]
fn the_call_returns_when_the_source_ends_and_leaves_no_ffmpeg_child() {
    let _ffmpeg_gate = ffmpeg_gate();
    let dir = temp_dir("return");
    let segment = make_segment(&dir);
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("worker socket binds");
    let server = serve(listener, Vec::new(), usize::MAX);
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let out = SharedOutput::default();
        let summary = replay_segment(&segment, &config(socket), out);
        sender.send(summary).expect("the waiting thread is alive");
    });
    let summary = receiver
        .recv_timeout(Duration::from_mins(1))
        .expect("replay returned within 60s")
        .expect("replay succeeds");
    server.join().expect("worker thread joins");
    assert_eq!(summary.frames, 15, "the segment decoded to fifteen frames");
    let mut children = ffmpeg_children();
    for _ in 0..50 {
        if children.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
        children = ffmpeg_children();
    }
    assert!(children.is_empty(), "no ffmpeg left: {children:?}");
}

#[test]
fn a_worker_that_closes_the_connection_surfaces_as_a_stage_failure_at_a_complete_line() {
    let _ffmpeg_gate = ffmpeg_gate();
    let dir = temp_dir("closed");
    let segment = make_segment(&dir);
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("worker socket binds");
    let server = serve(listener, vec![person_row()], 1);
    let out = SharedOutput::default();
    let error = replay_segment(&segment, &config(socket), out.clone())
        .expect_err("a worker that closes mid-run fails the replay");
    server.join().expect("worker thread joins");
    assert!(is_candidate_io(&error), "candidate io: {error}");
    let text = out.text();
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        !lines.is_empty() && lines.len() <= 2,
        "header plus one line"
    );
    assert!(text.ends_with('\n'), "record ends at a complete line");
    assert!(lines[0].contains("\"schema\":1"), "first line is header");
}

#[test]
fn a_segment_that_decodes_no_frames_fails_after_writing_the_header() {
    let _ffmpeg_gate = ffmpeg_gate();
    let dir = temp_dir("noframes");
    for (tag, body) in [("empty", ""), ("text", "not video data")] {
        let segment = dir.join(format!("cam_{tag}-20261101T120000Z.ts"));
        fs::write(&segment, body).expect("segment file with a valid name");
        let socket = dir.join(format!("{tag}.sock"));
        let listener = UnixListener::bind(&socket).expect("worker socket binds");
        let server = serve(listener, Vec::new(), usize::MAX);
        let out = SharedOutput::default();
        let error = replay_segment(&segment, &config(socket), out.clone())
            .expect_err("a segment that decodes no frame fails");
        assert!(
            matches!(error, ReplaySegmentError::NoFrames),
            "{tag}: {error}"
        );
        assert_eq!(out.text().lines().count(), 1, "{tag}: header only");
        server.join().expect("worker thread joins");
    }
}

fn replay_case_bytes(
    segment: &Path,
    sock: &str,
    rows: Vec<Row>,
    area: Option<Area>,
) -> (ReplaySummary, Vec<u8>) {
    let socket = segment.parent().expect("segment directory").join(sock);
    let listener = UnixListener::bind(&socket).expect("worker socket binds");
    let server = serve(listener, rows, usize::MAX);
    let out = SharedOutput::default();
    let summary =
        replay_segment(segment, &area_config(socket, area), out.clone()).expect("replay succeeds");
    server.join().expect("worker thread joins");
    (summary, out.bytes())
}

fn counted_requests(segment: &Path, sock: &str, area: Option<Area>) -> usize {
    let socket = segment.parent().expect("segment directory").join(sock);
    let listener = UnixListener::bind(&socket).expect("worker socket binds");
    let counter = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&counter);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("worker accept");
        while let Some(id) = read_request_id(&mut stream) {
            let bytes = Reply::Rows {
                id,
                worker_ms: 0,
                rows: vec![person_row()],
            }
            .encode()
            .expect("reply encodes");
            stream.write_all(&bytes).expect("reply write");
            observed.fetch_add(1, Ordering::Relaxed);
        }
    });
    let out = SharedOutput::default();
    replay_segment(segment, &area_config(socket, area), out).expect("replay succeeds");
    server.join().expect("counting worker joins");
    counter.load(Ordering::Relaxed)
}

#[test]
fn an_area_polygon_containing_the_object_bottom_centre_carries_the_zone_and_one_away_carries_none()
{
    let _ffmpeg_gate = ffmpeg_gate();
    let dir = temp_dir("area-zone");
    let segment = make_segment(&dir);
    let inside = Area {
        name: "back-area".to_owned(),
        polygon: vec![(0.45, 0.0), (1.0, 0.0), (1.0, 1.0), (0.45, 1.0)],
        pad_pixels: 100,
    };
    let outside = Area {
        name: "back-area".to_owned(),
        polygon: vec![(0.0, 0.0), (0.3, 0.0), (0.3, 1.0), (0.0, 1.0)],
        pad_pixels: 100,
    };
    let ((summary, bytes), outside_bytes) = thread::scope(|scope| {
        let inside_run = scope
            .spawn(|| replay_case_bytes(&segment, "inside.sock", vec![person_row()], Some(inside)));
        let outside_run = scope.spawn(|| {
            replay_case_bytes(&segment, "outside.sock", vec![person_row()], Some(outside))
        });
        (
            inside_run.join().expect("inside run"),
            outside_run.join().expect("outside run").1,
        )
    });
    assert_eq!(summary.frames, 15, "fifteen frames");
    let inside_text = String::from_utf8(bytes).expect("the record is utf-8");
    let lines: Vec<&str> = inside_text.lines().collect();
    let first_object = lines
        .iter()
        .position(|line| line.contains("\"id\":\"1\""))
        .expect("id one appears");
    let entered = lines
        .iter()
        .position(|line| line.contains("\"back-area\""))
        .expect("the zone name is entered");
    assert!(
        entered > first_object,
        "entry follows the first object line"
    );
    assert_eq!(
        entered, 5,
        "zone line index after inertia three counted frames"
    );
    let outside_text = String::from_utf8(outside_bytes).expect("the record is utf-8");
    assert!(
        outside_text.contains("\"id\":\"1\""),
        "the object is tracked with the area away from it"
    );
    assert!(
        !outside_text.contains("back-area"),
        "no entry outside the polygon"
    );
}

fn without_zones(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(position) = rest.find("\"zones_now\":[") {
        out.push_str(&rest[..position]);
        let after = &rest[position + "\"zones_now\":[".len()..];
        let close = after.find(']').expect("zone array closes") + 1;
        rest = &after[close..];
        if let Some(entered) = rest.strip_prefix(",\"zones_entered\":[") {
            let close = entered.find(']').expect("zone array closes") + 1;
            rest = &entered[close..];
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn an_area_polygon_over_the_whole_frame_changes_only_the_zone_fields_of_the_record() {
    let _ffmpeg_gate = ffmpeg_gate();
    let dir = temp_dir("area-whole");
    let segment = make_segment(&dir);
    let whole = Area {
        name: "whole".to_owned(),
        polygon: vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
        pad_pixels: 16,
    };
    let (plain_bytes, grown_bytes) = thread::scope(|scope| {
        let plain_run =
            scope.spawn(|| replay_case_bytes(&segment, "none.sock", vec![person_row()], None));
        let grown_run = scope
            .spawn(|| replay_case_bytes(&segment, "whole.sock", vec![person_row()], Some(whole)));
        (
            plain_run.join().expect("plain run").1,
            grown_run.join().expect("grown run").1,
        )
    });
    let plain_text = String::from_utf8(plain_bytes).expect("the record is utf-8");
    let grown_text = String::from_utf8(grown_bytes).expect("the record is utf-8");
    let plain_lines: Vec<String> = plain_text.lines().map(without_zones).collect();
    let grown_lines: Vec<String> = grown_text.lines().map(without_zones).collect();
    assert_eq!(plain_lines.len(), 16, "header plus fifteen frames");
    assert_eq!(plain_lines.len(), grown_lines.len(), "same line count");
    assert_eq!(
        plain_lines, grown_lines,
        "every line matches once zone fields are dropped"
    );
    assert!(
        grown_text.contains("\"whole\""),
        "the area run carries the zone name"
    );
    assert!(
        !plain_text.contains("\"whole\""),
        "the plain run carries no zone name"
    );
}

#[test]
fn an_area_polygon_at_a_quiet_corner_sends_fewer_worker_requests_than_no_area() {
    let _ffmpeg_gate = ffmpeg_gate();
    let dir = temp_dir("area-corner");
    let segment = make_segment(&dir);
    let corner = Area {
        name: "corner".to_owned(),
        polygon: vec![(0.0, 0.85), (0.15, 1.0), (0.0, 1.0)],
        pad_pixels: 0,
    };
    let (without, with) = thread::scope(|scope| {
        let none_run = scope.spawn(|| counted_requests(&segment, "none.sock", None));
        let corner_run = scope.spawn(|| counted_requests(&segment, "corner.sock", Some(corner)));
        (
            none_run.join().expect("none run"),
            corner_run.join().expect("corner run"),
        )
    });
    assert_eq!(without, 35, "no-area count: 27 motion plus 8 startup");
    assert_eq!(
        with, 23,
        "bottom-left-corner count: 15 motion plus 8 startup"
    );
    assert!(
        with < without,
        "the mask drops the motion outside the polygon"
    );
}

#[test]
fn growing_the_keep_mask_by_the_pad_sweeps_in_motion_and_sends_more_requests() {
    let _ffmpeg_gate = ffmpeg_gate();
    let dir = temp_dir("area-pad");
    let segment = make_segment(&dir);
    let polygon = vec![(0.0, 0.85), (0.15, 1.0), (0.0, 1.0)];
    let (tight, grown) = thread::scope(|scope| {
        let tight_run = scope.spawn(|| {
            counted_requests(
                &segment,
                "tight.sock",
                Some(Area {
                    name: "corner".to_owned(),
                    polygon: polygon.clone(),
                    pad_pixels: 0,
                }),
            )
        });
        let grown_run = scope.spawn(|| {
            counted_requests(
                &segment,
                "grown.sock",
                Some(Area {
                    name: "corner".to_owned(),
                    polygon: polygon.clone(),
                    pad_pixels: 150,
                }),
            )
        });
        (
            tight_run.join().expect("tight run"),
            grown_run.join().expect("grown run"),
        )
    });
    assert_eq!(tight, 23, "unpadded count keeps only the quiet corner");
    assert_eq!(grown, 35, "a pad of 150 reaches the moving region");
}

#[test]
fn the_pad_boundary_at_frame_pixels_flips_the_keep_mask_and_the_request_count() {
    // Frame 704x576 shrinks to the motion shape 122x100 (5.7704x5.76 frame px per
    // motion px). A sweep measured the sweep boundary of this corner triangle at pad
    // 46: at pad 45 the first motion column is still ignored, at pad 46 it is kept.
    let _ffmpeg_gate = ffmpeg_gate();
    let dir = temp_dir("pad-boundary");
    let segment = make_segment(&dir);
    let polygon = vec![(0.0, 0.85), (0.15, 1.0), (0.0, 1.0)];
    let before = counted_requests(
        &segment,
        "before.sock",
        Some(Area {
            name: "corner".to_owned(),
            polygon: polygon.clone(),
            pad_pixels: 45,
        }),
    );
    let at = counted_requests(
        &segment,
        "at.sock",
        Some(Area {
            name: "corner".to_owned(),
            polygon,
            pad_pixels: 46,
        }),
    );
    assert_eq!(before, 23, "pad 45 has not reached the motion yet");
    assert_eq!(at, 35, "pad 46 keeps the first motion column");
    assert!(
        at > before,
        "one frame pixel of radius changes the request count"
    );
}
