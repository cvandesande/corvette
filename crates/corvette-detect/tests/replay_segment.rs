use std::cell::RefCell;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use corvette_detect::{
    CandidateError, EngineError, Labels, ReplayConfig, ReplayError, ReplaySegmentError,
    ReplaySummary, SegmentName, replay_segment,
};
use corvette_detect_proto::{HEADER_LEN, Reply, Request, Row, parse_header};

static COUNTER: AtomicU32 = AtomicU32::new(0);

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
    let rows = vec![person_row()];
    let (first, first_out) = replay_case("det-one", rows.clone());
    let (second, second_out) = replay_case("det-two", rows);
    assert_eq!(first.frames, second.frames);
    assert_eq!(first_out.bytes(), second_out.bytes());
}

#[test]
fn a_person_row_from_the_worker_is_tracked_and_listed_on_later_frames() {
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
