use corvette_detect::{
    CameraObjects, EventFeed, FrameObjects, ServiceState, serve, serve_track_events,
};
use corvette_detect_proto::{TrackFrame, encode_track_line};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;
use tokio::time::timeout;

const DRAIN: Duration = Duration::from_millis(200);
const EVENT_REQUEST: &[u8] = b"GET /corvette/api/v1/events HTTP/1.1\r\nHost: ipc\r\n\r\n";

fn temp_socket(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "corvette-event-stream-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("ipc.sock")
}

fn tick_frame(second: u64) -> FrameObjects {
    FrameObjects {
        frame_time: f64::from(u32::try_from(second).unwrap()) + 0.5,
        objects: Vec::new(),
        ended: Vec::new(),
        motion_triggered: true,
    }
}

fn tick_line(second: u64) -> Vec<u8> {
    encode_track_line(&TrackFrame::Motion {
        camera: "front".to_owned(),
        second,
    })
}

async fn serve_pair(tag: &str) -> (PathBuf, PathBuf, watch::Sender<CameraObjects>) {
    let feed = Arc::new(EventFeed::new());
    let (sender, receiver) = watch::channel(CameraObjects::Cleared);
    feed.spawn_publishers(vec![("front".to_owned(), receiver)]);
    let ipc = temp_socket(&format!("{tag}-ipc"));
    let ipc_listener = UnixListener::bind(&ipc).unwrap();
    tokio::spawn(serve(
        ipc_listener,
        Arc::new(ServiceState::new(Vec::new())),
        Some(Arc::clone(&feed)),
        None,
    ));
    let track = temp_socket(&format!("{tag}-track"));
    let track_listener = UnixListener::bind(&track).unwrap();
    tokio::spawn(serve_track_events(track_listener, Arc::clone(&feed)));
    tokio::time::sleep(Duration::from_millis(50)).await;
    (ipc, track, sender)
}

async fn request_events(path: &Path) -> (UnixStream, Vec<u8>) {
    let mut stream = UnixStream::connect(path).await.unwrap();
    stream.write_all(EVENT_REQUEST).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let read = timeout(Duration::from_secs(5), stream.read(&mut byte))
            .await
            .expect("head bytes")
            .unwrap();
        assert_ne!(read, 0, "the head ended early");
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            return (stream, head);
        }
        assert!(head.len() < 4096, "the head ran long: {head:?}");
    }
}

async fn read_body(stream: &mut UnixStream) -> Vec<u8> {
    let mut raw = Vec::new();
    let mut buf = [0_u8; 4096];
    while let Ok(Ok(read)) = timeout(DRAIN, stream.read(&mut buf)).await {
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..read]);
    }
    raw
}

fn decode_chunks(body: &[u8]) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        let end = rest
            .windows(2)
            .position(|w| w == b"\r\n")
            .expect("a chunk size line");
        let size = usize::from_str_radix(std::str::from_utf8(&rest[..end]).unwrap(), 16)
            .expect("a hex size");
        assert!(rest.len() >= end + 4 + size, "a chunk body is complete");
        chunks.push(rest[end + 2..end + 2 + size].to_vec());
        rest = &rest[end + 4 + size..];
    }
    chunks
}

fn split_lines(body: &[u8]) -> Vec<Vec<u8>> {
    body.split_inclusive(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

#[tokio::test]
async fn the_http_stream_and_the_socket_subscriber_receive_the_same_lines_one_line_per_chunk() {
    let (ipc, track, sender) = serve_pair("parity").await;
    let mut track_stream = UnixStream::connect(&track).await.unwrap();
    let (mut http_stream, head) = request_events(&ipc).await;
    let head = String::from_utf8_lossy(&head).into_owned();
    assert!(head.starts_with("HTTP/1.1 200 OK"), "head: {head}");
    assert!(head.contains("Transfer-Encoding: chunked"), "head: {head}");
    assert!(head.contains("Content-Type: text/plain"), "head: {head}");
    assert!(head.contains("Cache-Control: no-store"), "head: {head}");
    assert!(!head.contains("Connection: close"), "head: {head}");

    let expected: Vec<Vec<u8>> = (100..103).map(tick_line).collect();
    for second in 100..103 {
        sender
            .send(CameraObjects::Frame(tick_frame(second)))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let socket_body = read_body(&mut track_stream).await;
    let http_body = read_body(&mut http_stream).await;
    assert_eq!(
        split_lines(&socket_body),
        expected,
        "the socket line stream"
    );
    assert_eq!(
        decode_chunks(&http_body),
        expected,
        "each line is exactly one chunk and the bodies are byte-identical"
    );
}

#[tokio::test]
async fn a_late_subscriber_receives_only_the_lines_after_its_registration() {
    let (ipc, _track, sender) = serve_pair("late").await;
    sender.send(CameraObjects::Frame(tick_frame(100))).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (mut late_stream, head) = request_events(&ipc).await;
    assert!(head.starts_with(b"HTTP/1.1 200 OK"), "head: {head:?}");
    sender.send(CameraObjects::Frame(tick_frame(101))).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let body = read_body(&mut late_stream).await;
    assert_eq!(
        decode_chunks(&body),
        vec![tick_line(101)],
        "no replay of the line sent before registration"
    );
}

#[tokio::test]
async fn the_seventeenth_stream_gets_503_and_a_closed_read_half_frees_the_slot() {
    let (ipc, _track, _sender) = serve_pair("limit").await;
    let mut held_streams = Vec::new();
    for _ in 0..16 {
        let (stream, head) = request_events(&ipc).await;
        assert!(head.starts_with(b"HTTP/1.1 200 OK"), "head: {head:?}");
        held_streams.push(stream);
    }
    let (_refused, head) = request_events(&ipc).await;
    let head = String::from_utf8_lossy(&head).into_owned();
    assert!(head.starts_with("HTTP/1.1 503"), "over-limit head: {head}");

    AsyncWriteExt::shutdown(&mut held_streams[0]).await.unwrap();
    let mut freed = false;
    for _ in 0..100 {
        let (stream, head) = request_events(&ipc).await;
        let accepted = head.starts_with(b"HTTP/1.1 200 OK");
        drop(stream);
        if accepted {
            freed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(freed, "the closed stream's slot was never freed");
}
