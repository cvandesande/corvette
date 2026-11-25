//! The stitched download answer: one MP4 body built from the stored pieces of one run.

use crate::config::ConfigSource;
use crate::playlist::{self, Answer, MediaPath, PieceRead, PlanError, PlannedPiece};
use crate::rebase;
use crate::recordings::to_millis;
use crate::{Request, Response, log_event, recording_unavailable};
use std::borrow::Cow;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

const MEDIA_TYPE_DOWNLOAD: &str = "application/mp4";

/// Each streamed-body socket write must complete within this bound; there is no total deadline.
pub(crate) const WRITE_INACTIVITY: Duration = Duration::from_secs(15);

struct PendingPiece {
    seq: i64,
    path: PathBuf,
    offset_ticks: u64,
}

/// The streamed body: the first piece already read, the rest as paths plus one shared read count.
pub(crate) struct StreamBody {
    camera: String,
    head: Vec<u8>,
    tail: Vec<PendingPiece>,
    reads: Arc<AtomicUsize>,
}

pub(crate) async fn handle(
    request: &Request,
    config: &ConfigSource,
    index_path: &Path,
    media: MediaPath<'_>,
) -> Response {
    let (camera, start, end) = match playlist::validated_bounds(request, config, &media).await {
        Ok(bounds) => bounds,
        Err(response) => return response,
    };
    if !camera_name_is_plain(&camera) {
        return playlist::answer_response(Answer::NotFound, &camera);
    }
    let blocking_index = index_path.to_path_buf();
    let blocking_camera = camera.clone();
    let start_ms = to_millis(start);
    let end_ms = to_millis(end);
    let prepared = tokio::task::spawn_blocking(move || {
        prepare(&blocking_index, &blocking_camera, start_ms, end_ms)
    })
    .await;
    let prepared = match prepared {
        Ok(Ok(prepared)) => prepared,
        Ok(Err(answer)) => return playlist::answer_response(answer, &camera),
        Err(_) => return recording_unavailable(),
    };
    let filename = format!(
        "attachment; filename=\"{camera}-{}-{}.mp4\"",
        seconds_floor(start),
        seconds_floor(end),
    );
    let mut extra = vec![(Cow::Borrowed("Content-Disposition"), filename)];
    if let Some(next_start_ms) = prepared.next_start_ms {
        let next_start = format!(
            "{}.{:03}",
            next_start_ms.div_euclid(1000),
            next_start_ms.rem_euclid(1000)
        );
        extra.push((Cow::Borrowed("X-Corvette-Next-Start"), next_start));
    }
    Response {
        status: Cow::Borrowed("200 OK"),
        body: Vec::new(),
        content_type: Cow::Borrowed(MEDIA_TYPE_DOWNLOAD),
        cache_control: Cow::Borrowed("no-store"),
        extra,
        stream: Some(StreamBody {
            camera,
            head: prepared.head,
            tail: prepared.tail,
            reads: Arc::new(AtomicUsize::new(0)),
        }),
    }
}

struct Prepared {
    head: Vec<u8>,
    tail: Vec<PendingPiece>,
    next_start_ms: Option<i64>,
}

/// Everything cheap to validate before any header is sent: the plan, then the
/// first piece's readability and rebase at offset zero.
fn prepare(
    index_path: &Path,
    camera: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<Prepared, Answer> {
    let plan =
        playlist::plan_blocking(index_path, camera, start_ms, end_ms).map_err(|err| match err {
            PlanError::IndexUnreadable => Answer::IndexUnreadable,
            PlanError::TooManyPieces => Answer::NotFound,
            PlanError::PieceUnreadable { seq } => Answer::PieceUnreadable { seq },
            PlanError::PieceMalformed { seq } => Answer::PieceMalformed { seq },
        })?;
    if plan.pieces.is_empty() {
        return Err(Answer::NotFound);
    }
    let Some(first) = plan.pieces.first() else {
        return Err(Answer::NotFound);
    };
    let read_answer = |err| match err {
        PieceRead::Unreadable => Answer::PieceUnreadable { seq: first.seq },
        PieceRead::Malformed => Answer::PieceMalformed { seq: first.seq },
    };
    let init = playlist::read_init_prefix(&first.path).map_err(read_answer)?;
    let timescale = rebase::fingerprint(&init)
        .map_err(|_| Answer::PieceMalformed { seq: first.seq })?
        .timescale;
    let mut head = init;
    head.extend(playlist::segment_body(&first.path, 0).map_err(read_answer)?);
    // A gap inside a run is collapsed: the offset keeps accumulating across it.
    let run_end = plan
        .timeline
        .run_first_pieces()
        .get(1)
        .copied()
        .unwrap_or(plan.pieces.len());
    let mut tail = Vec::new();
    let mut cumulative = 0_u64;
    let mut previous: Option<&PlannedPiece> = None;
    for piece in plan.pieces.iter().take(run_end) {
        if let Some(previous) = previous {
            cumulative = cumulative
                .checked_add(playlist::duration_ticks(previous.duration_s, timescale))
                .ok_or(Answer::PieceMalformed { seq: piece.seq })?;
            tail.push(PendingPiece {
                seq: piece.seq,
                path: piece.path.clone(),
                offset_ticks: cumulative,
            });
        }
        previous = Some(piece);
    }
    let next_start_ms = match (
        plan.timeline.run_first_pieces().get(1),
        plan.pieces.get(run_end - 1),
    ) {
        (Some(&next), Some(last_included)) => plan
            .pieces
            .get(next)
            .map(|next_piece| next_piece.start_ts.max(last_included.end_ts)),
        _ => None,
    };
    Ok(Prepared {
        head,
        tail,
        next_start_ms,
    })
}

/// Writes the whole streamed body. `inactivity` bounds each logical socket write —
/// head, chunks, terminator — with no total deadline; production passes [`WRITE_INACTIVITY`].
pub(crate) async fn write_streamed(
    socket: &mut UnixStream,
    response: &Response,
    mut body: StreamBody,
    inactivity: Duration,
) -> std::io::Result<()> {
    // Chunk framing is only defined by HTTP/1.1; every buffered route keeps its HTTP/1.0 answer.
    let mut head = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nCache-Control: {}\r\n",
        response.status, response.content_type, response.cache_control
    );
    for (name, value) in &response.extra {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    head.push_str("Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
    match write_frame(socket, head.as_bytes(), inactivity).await {
        StreamWrite::Done => {}
        StreamWrite::Stall => {
            stalled(&body.camera);
            return Ok(());
        }
        StreamWrite::Failed(err) => return Err(err),
    }
    // The head bytes are free once their chunk is out: tail writes hold one piece plus its rebased copy.
    let first_chunk = std::mem::take(&mut body.head);
    let written = write_chunk(socket, &first_chunk, inactivity).await;
    drop(first_chunk);
    match written {
        StreamWrite::Done => {}
        StreamWrite::Stall => {
            stalled(&body.camera);
            return Ok(());
        }
        StreamWrite::Failed(err) => return Err(err),
    }
    for piece in &body.tail {
        body.reads.fetch_add(1, Ordering::Relaxed);
        let path = piece.path.clone();
        let offset_ticks = piece.offset_ticks;
        let rebased =
            tokio::task::spawn_blocking(move || playlist::segment_body(&path, offset_ticks)).await;
        let bytes = match rebased {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(PieceRead::Unreadable)) | Err(_) => {
                log_event(
                    "playlist",
                    "recording-piece-unreadable",
                    format!("camera={} seq={}", body.camera, piece.seq),
                );
                return Ok(());
            }
            Ok(Err(PieceRead::Malformed)) => {
                log_event(
                    "playlist",
                    "recording-piece-malformed",
                    format!("camera={} seq={}", body.camera, piece.seq),
                );
                return Ok(());
            }
        };
        match write_chunk(socket, &bytes, inactivity).await {
            StreamWrite::Done => {}
            StreamWrite::Stall => {
                stalled(&body.camera);
                return Ok(());
            }
            StreamWrite::Failed(err) => return Err(err),
        }
    }
    match write_frame(socket, b"0\r\n\r\n", inactivity).await {
        StreamWrite::Done => {}
        StreamWrite::Stall => {
            stalled(&body.camera);
            return Ok(());
        }
        StreamWrite::Failed(err) => return Err(err),
    }
    socket.shutdown().await
}

enum StreamWrite {
    Done,
    Stall,
    Failed(std::io::Error),
}

async fn write_frame(socket: &mut UnixStream, bytes: &[u8], inactivity: Duration) -> StreamWrite {
    settle(tokio::time::timeout(inactivity, socket.write_all(bytes)).await)
}

/// One chunk: size line, data and CRLF count as a single inactivity-bounded write.
async fn write_chunk(socket: &mut UnixStream, data: &[u8], inactivity: Duration) -> StreamWrite {
    let framed = write_chunk_framed(socket, data);
    settle(tokio::time::timeout(inactivity, framed).await)
}

fn settle(outcome: Result<std::io::Result<()>, tokio::time::error::Elapsed>) -> StreamWrite {
    match outcome {
        Err(_elapsed) => StreamWrite::Stall,
        Ok(Err(err)) => StreamWrite::Failed(err),
        Ok(Ok(())) => StreamWrite::Done,
    }
}

async fn write_chunk_framed(socket: &mut UnixStream, data: &[u8]) -> std::io::Result<()> {
    let size = format!("{:x}\r\n", data.len());
    socket.write_all(size.as_bytes()).await?;
    socket.write_all(data).await?;
    socket.write_all(b"\r\n").await
}

/// The fixed-word stall line; the stream ends without the terminating chunk.
fn stalled(camera: &str) {
    log_event(
        "playlist",
        "recording-download-stalled",
        format!("camera={camera}"),
    );
}

/// The camera characters this route echoes into `Content-Disposition` and log lines.
fn camera_name_is_plain(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// The request's Unix seconds as the integer-second floor the file name carries.
fn seconds_floor(seconds: f64) -> i64 {
    format!("{}", seconds.floor())
        .parse::<i64>()
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    fn put_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn put_u64(out: &mut Vec<u8>, value: u64) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn mp4_box(kind: [u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        put_u32(
            &mut out,
            u32::try_from(body.len() + 8).expect("fixture box fits u32"),
        );
        out.extend_from_slice(&kind);
        out.extend_from_slice(body);
        out
    }

    fn fixture_init() -> Vec<u8> {
        let mut stsd = Vec::new();
        put_u32(&mut stsd, 0);
        put_u32(&mut stsd, 1);
        let mut entry = vec![0_u8; 78];
        entry.extend_from_slice(&mp4_box(*b"avcC", &[0x01]));
        stsd.extend_from_slice(&mp4_box(*b"avc1", &entry));
        let stbl = mp4_box(*b"stbl", &mp4_box(*b"stsd", &stsd));
        let minf = mp4_box(*b"minf", &stbl);
        let mut mdhd = vec![0_u8; 12];
        put_u32(&mut mdhd, 90_000);
        let mdia = mp4_box(*b"mdia", &[mp4_box(*b"mdhd", &mdhd), minf].concat());
        let trak = mp4_box(*b"trak", &mdia);
        mp4_box(*b"moov", &trak)
    }

    fn fixture_fragment(payload_len: usize) -> Vec<u8> {
        let mut mfhd_body = Vec::new();
        put_u32(&mut mfhd_body, 0);
        put_u32(&mut mfhd_body, 1);
        let mut trun = Vec::new();
        put_u32(&mut trun, 0x0000_0101);
        put_u32(&mut trun, 1);
        put_u32(&mut trun, 0);
        put_u32(&mut trun, 360_000);
        let mut tfdt = Vec::new();
        put_u32(&mut tfdt, 1 << 24);
        put_u64(&mut tfdt, 0);
        let traf = mp4_box(
            *b"traf",
            &[
                mp4_box(*b"tfhd", &[0, 2, 0, 0, 0, 0, 0, 1]),
                mp4_box(*b"tfdt", &tfdt),
                mp4_box(*b"trun", &trun),
            ]
            .concat(),
        );
        let moof = mp4_box(*b"moof", &[mp4_box(*b"mfhd", &mfhd_body), traf].concat());
        let mut mdat_body = Vec::new();
        put_u32(
            &mut mdat_body,
            u32::try_from(payload_len).expect("fixture payload fits u32"),
        );
        mdat_body.extend_from_slice(&vec![0x5A_u8; payload_len]);
        [moof, mp4_box(*b"mdat", &mdat_body)].concat()
    }

    #[tokio::test]
    async fn the_writer_has_read_only_the_next_piece_when_the_first_tail_chunk_arrives() {
        let dir =
            std::env::temp_dir().join(format!("corvette-download-lazy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let init = fixture_init();
        let mut head = init.clone();
        head.extend_from_slice(&fixture_fragment(1024));
        let mut tail = Vec::new();
        for index in 1..40_i64 {
            let mut bytes = init.clone();
            bytes.extend_from_slice(&fixture_fragment(1 << 20));
            let path = dir.join(format!("piece-{index}.mp4"));
            std::fs::write(&path, bytes).expect("write piece");
            tail.push(PendingPiece {
                seq: index,
                path,
                offset_ticks: u64::try_from(index).expect("index fits") * 360_000,
            });
        }
        let response = Response {
            status: Cow::Borrowed("200 OK"),
            body: Vec::new(),
            content_type: Cow::Borrowed(MEDIA_TYPE_DOWNLOAD),
            cache_control: Cow::Borrowed("no-store"),
            extra: Vec::new(),
            stream: None,
        };
        let head_len = head.len();
        let reads = Arc::new(AtomicUsize::new(0));
        let body = StreamBody {
            camera: "front".to_owned(),
            head,
            tail,
            reads: Arc::clone(&reads),
        };
        let (mut tx, mut rx) = UnixStream::pair().expect("stream pair");
        let writer = tokio::spawn(async move {
            write_streamed(&mut tx, &response, body, WRITE_INACTIVITY).await
        });
        let mut one = [0_u8; 1];
        let mut head_buf: Vec<u8> = Vec::new();
        loop {
            rx.read_exact(&mut one).await.expect("header byte");
            head_buf.push(one[0]);
            if head_buf.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let mut size_line: Vec<u8> = Vec::new();
        loop {
            rx.read_exact(&mut one).await.expect("chunk size byte");
            size_line.push(one[0]);
            if one[0] == b'\n' {
                break;
            }
        }
        let chunk_text = std::str::from_utf8(&size_line)
            .expect("chunk size")
            .trim()
            .to_owned();
        let first_chunk = usize::from_str_radix(&chunk_text, 16).expect("hex chunk size");
        assert_eq!(
            first_chunk, head_len,
            "the preloaded head is the first chunk"
        );
        let mut skip = vec![0_u8; first_chunk + 2];
        rx.read_exact(&mut skip).await.expect("first chunk bytes");
        rx.read_exact(&mut one).await.expect("first tail byte");
        assert!(
            reads.load(Ordering::Relaxed) <= 2,
            "the first tail chunk arrived after {} piece reads, not all 39",
            reads.load(Ordering::Relaxed)
        );
        let mut rest = Vec::new();
        rx.read_to_end(&mut rest).await.expect("drain");
        assert!(
            rest.ends_with(b"0\r\n\r\n"),
            "the stream terminates when complete"
        );
        assert_eq!(reads.load(Ordering::Relaxed), 39, "one read per tail piece");
        writer.await.expect("writer task").expect("stream written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    const STALL_CHILD_ENV: &str = "CORVETTE_DOWNLOAD_STALL_CHILD";

    #[tokio::test]
    async fn a_client_that_stops_reading_ends_the_stream_without_the_terminator() {
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--exact",
                "download::tests::write_stall_child",
                "--nocapture",
            ])
            .env(STALL_CHILD_ENV, "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .expect("run the stall child");
        assert!(output.status.success(), "stall child:\n{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        let line = "corvette-api-server unit=\"playlist\" event=recording-download-stalled detail=\"camera=front\"";
        assert_eq!(stderr.matches(line).count(), 1, "{stderr}");
    }

    /// Child of the stall test: a client that never reads; the injected bound ends the stream.
    #[tokio::test]
    async fn write_stall_child() {
        if std::env::var(STALL_CHILD_ENV).is_err() {
            return;
        }
        let mut head = fixture_init();
        head.extend_from_slice(&fixture_fragment(1 << 20));
        let response = Response {
            status: Cow::Borrowed("200 OK"),
            body: Vec::new(),
            content_type: Cow::Borrowed(MEDIA_TYPE_DOWNLOAD),
            cache_control: Cow::Borrowed("no-store"),
            extra: Vec::new(),
            stream: None,
        };
        let body = StreamBody {
            camera: "front".to_owned(),
            head,
            tail: Vec::new(),
            reads: Arc::new(AtomicUsize::new(0)),
        };
        let (mut tx, mut rx) = UnixStream::pair().expect("stream pair");
        let started = std::time::Instant::now();
        let writer = tokio::spawn(async move {
            tokio::time::timeout(
                Duration::from_secs(5),
                write_streamed(&mut tx, &response, body, Duration::from_millis(150)),
            )
            .await
        });
        let written = writer.await.expect("stall writer task");
        let elapsed = started.elapsed();
        assert!(
            matches!(written, Ok(Ok(()))),
            "the stalled write must return, got {written:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "the stream gave up after {elapsed:?}, not on the injected bound"
        );
        let mut received = Vec::new();
        rx.read_to_end(&mut received).await.expect("drain");
        assert!(
            received.starts_with(b"HTTP/1.1 200 OK"),
            "the head arrived before the stall"
        );
        assert!(
            received.len() > 64 * 1024,
            "the stalled chunk carried into the socket buffer, got {} bytes",
            received.len()
        );
        assert!(
            !received.ends_with(b"0\r\n\r\n"),
            "a stalled stream ends without the terminating chunk"
        );
    }
}
