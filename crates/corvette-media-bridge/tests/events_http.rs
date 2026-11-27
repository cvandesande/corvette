//! The events route over HTTP: chunk reading on an in-memory stream, head validation, fault and EOF redials, kept open-track state across redials, the sweep on a stalled stream, the kept write half, and the index end-to-end on a temp directory.

use corvette_detect_proto::{TrackFrame, TrackInfo, encode_track_line};
use corvette_media_bridge::event_consumer::{self, EventWriter, StreamOutcome};
use corvette_media_bridge::index::{EventWrite, Index, IndexWrite, PIECE_CHANNEL_CAPACITY};
use corvette_media_bridge::record_config::Review;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::SyncSender;
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

const HEAD: &[u8] = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\n\r\n";
const REQUEST: &[u8] = b"GET /corvette/api/v1/events HTTP/1.1\r\n\r\n";
const BACKOFF: Duration = Duration::from_millis(50);

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "corvette-events-http-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp root created");
    root
}

fn rig(name: &str) -> (PathBuf, SyncSender<IndexWrite>, JoinHandle<Index>) {
    let root = temp_root(name);
    let index = Index::open(&root.join("index.sqlite"), &[("front".to_owned(), 4u32)])
        .expect("index opens");
    let (sender, handle) =
        corvette_media_bridge::index::spawn_writer(index, PIECE_CHANNEL_CAPACITY);
    (root, sender, handle)
}

fn review_all() -> HashMap<String, Review> {
    let mut map = HashMap::new();
    map.insert("front".to_owned(), Review::default());
    map
}

fn track(label: &str, track_id: &str, first_motion: Option<f64>, last_seen: f64) -> TrackInfo {
    TrackInfo {
        camera: "front".to_owned(),
        track_id: track_id.to_owned(),
        label: label.to_owned(),
        bounds: [0.1, 0.2, 0.3, 0.4],
        zones: vec!["yard".to_owned()],
        first_motion,
        last_seen,
    }
}

fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            let millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
            i64::try_from(millis).unwrap_or(i64::MAX)
        })
}

fn chunk_frame(line: &[u8]) -> Vec<u8> {
    let mut framed = format!("{:x}\r\n", line.len()).into_bytes();
    framed.extend_from_slice(line);
    framed.extend_from_slice(b"\r\n");
    framed
}

fn now_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

fn start_frame(track_id: &str) -> Vec<u8> {
    // Times track the wall clock so the startup sweep of the sweep interval leaves the row open.
    let now_seconds = now_seconds();
    encode_track_line(&TrackFrame::Start(track(
        "person",
        track_id,
        Some(now_seconds),
        now_seconds,
    )))
}

fn end_frame(track_id: &str) -> Vec<u8> {
    encode_track_line(&TrackFrame::End {
        camera: "front".to_owned(),
        track_id: track_id.to_owned(),
    })
}

/// Drives one handshake like the service does and hands back the connection; the bridge's request bytes are checked here once per test.
async fn serve_stream(listener: &UnixListener) -> UnixStream {
    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
        .await
        .expect("a connection arrives")
        .expect("connection accepted");
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await.expect("request bytes read");
        assert_ne!(read, 0, "the request head arrives");
        request.extend_from_slice(&chunk[..read]);
    }
    assert!(
        request.strip_prefix(REQUEST).is_some(),
        "the bridge sends the events request: {request:?}"
    );
    stream.write_all(HEAD).await.expect("head written");
    stream
}

async fn row_count_matches(root: &std::path::Path, query: &str, want: i64) -> bool {
    for _ in 0..100 {
        let raw = rusqlite::Connection::open_with_flags(
            root.join("index.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("read-only reopen");
        let count: i64 = raw
            .query_row(query, [], |row| row.get(0))
            .expect("events readable");
        drop(raw);
        if count == want {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn chunked_frames_over_an_in_memory_stream_open_and_close_a_track() {
    let (mut server, client) = tokio::io::duplex(64 * 1024);
    let (tx, rx) = std::sync::mpsc::sync_channel(64);
    let pumping = tokio::spawn(async move {
        let mut writer = EventWriter::new(review_all());
        event_consumer::pump_events_http(client, &tx, &mut writer, &epoch_ms).await
    });
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = server.read(&mut chunk).await.expect("request bytes read");
        assert_ne!(read, 0, "the request head arrives");
        request.extend_from_slice(&chunk[..read]);
    }
    assert!(request.strip_prefix(REQUEST).is_some());
    server.write_all(HEAD).await.expect("head written");
    server
        .write_all(&chunk_frame(&start_frame("one")))
        .await
        .expect("start frame written");
    server
        .write_all(&chunk_frame(&end_frame("one")))
        .await
        .expect("end frame written");
    drop(server);
    assert_eq!(
        pumping.await.expect("pump task joins"),
        StreamOutcome::Ended
    );
    let mut saw_open = false;
    let mut saw_close = false;
    while let Ok(write) = rx.try_recv() {
        if let IndexWrite::Event(event) = write {
            match event {
                EventWrite::Open { track_id, .. } if track_id == "one" => saw_open = true,
                EventWrite::Close { track_id, .. } if track_id == "one" => saw_close = true,
                _ => {}
            }
        }
    }
    assert!(saw_open, "the start chunk reached the writer");
    assert!(saw_close, "the end chunk reached the writer");
}

#[tokio::test]
async fn a_chunk_over_the_line_limit_faults_and_the_consumer_redials() {
    let dir = temp_root("overcap");
    let socket = dir.join("events.sock");
    let _ = std::fs::remove_file(&socket);
    let (root, sender, handle) = rig("overcap-index");
    tokio::spawn(event_consumer::run_http(
        socket.clone(),
        sender.clone(),
        EventWriter::new(review_all()),
        0,
        epoch_ms,
        BACKOFF,
    ));
    let listener = UnixListener::bind(&socket).expect("listener binds");
    let mut first = serve_stream(&listener).await;
    first
        .write_all(b"5000\r\n")
        .await
        .expect("over-cap size announced");
    let second = tokio::time::timeout(Duration::from_secs(3), listener.accept())
        .await
        .expect("the consumer redials while the first stream is still open");
    assert!(second.is_ok(), "a redial connection arrives");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        row_count_matches(&root, "SELECT COUNT(*) FROM events", 0).await,
        "the announced over-cap chunk never reached the index"
    );
    drop(second.ok());
    drop(first);
    drop(listener);
    drop(sender);
    drop(handle);
}

#[tokio::test]
async fn an_over_cap_size_line_ending_in_crlf_faults_and_forwards_nothing() {
    let (mut server, client) = tokio::io::duplex(64 * 1024);
    let (tx, rx) = std::sync::mpsc::sync_channel(64);
    let pumping = tokio::spawn(async move {
        let mut writer = EventWriter::new(review_all());
        event_consumer::pump_events_http(client, &tx, &mut writer, &epoch_ms).await
    });
    server.write_all(HEAD).await.expect("head written");
    let frame = start_frame("one");
    let mut announced = "0".repeat(16_437).into_bytes();
    announced.extend_from_slice(format!("{:x}\r\n", frame.len()).as_bytes());
    announced.extend_from_slice(&frame);
    announced.extend_from_slice(b"\r\n");
    server
        .write_all(&announced)
        .await
        .expect("over-cap size line announced");
    let outcome = tokio::time::timeout(Duration::from_secs(3), pumping)
        .await
        .expect("the pump ends on the over-cap size line instead of parsing it")
        .expect("pump task joins");
    assert_eq!(outcome, StreamOutcome::Ended);
    assert!(
        rx.try_recv().is_err(),
        "the line behind the over-cap size line reaches nobody"
    );
}

#[tokio::test]
async fn a_zero_padded_size_line_over_the_cap_faults_instead_of_parsing_small() {
    let (mut server, client) = tokio::io::duplex(64 * 1024);
    let (tx, rx) = std::sync::mpsc::sync_channel(64);
    let pumping = tokio::spawn(async move {
        let mut writer = EventWriter::new(review_all());
        event_consumer::pump_events_http(client, &tx, &mut writer, &epoch_ms).await
    });
    server.write_all(HEAD).await.expect("head written");
    let mut announced = "0".repeat(16_437).into_bytes();
    announced.extend_from_slice(b"10\r\nABCDEFGHIJKLMNOP\r\n");
    server
        .write_all(&announced)
        .await
        .expect("zero-padded over-cap size line announced");
    let outcome = tokio::time::timeout(Duration::from_secs(3), pumping)
        .await
        .expect("the pump ends on the zero-padded over-cap size line instead of parsing small")
        .expect("pump task joins");
    assert_eq!(outcome, StreamOutcome::Ended);
    assert!(
        rx.try_recv().is_err(),
        "the frame behind the zero-padded size line reaches nobody"
    );
}

#[tokio::test]
async fn a_close_mid_chunk_discards_the_partial_line_and_the_consumer_redials() {
    let dir = temp_root("midline");
    let socket = dir.join("events.sock");
    let _ = std::fs::remove_file(&socket);
    let (root, sender, handle) = rig("midline-index");
    tokio::spawn(event_consumer::run_http(
        socket.clone(),
        sender.clone(),
        EventWriter::new(review_all()),
        0,
        epoch_ms,
        BACKOFF,
    ));
    let listener = UnixListener::bind(&socket).expect("listener binds");
    let mut first = serve_stream(&listener).await;
    first
        .write_all(&chunk_frame(&start_frame("one")))
        .await
        .expect("first line delivered");
    let partial = start_frame("two");
    first
        .write_all(format!("{:x}\r\n", partial.len()).as_bytes())
        .await
        .expect("partial size announced");
    first
        .write_all(&partial[..5])
        .await
        .expect("partial payload announced");
    drop(first);
    let _second = serve_stream(&listener).await;
    assert!(
        row_count_matches(
            &root,
            "SELECT COUNT(*) FROM events WHERE track_id = 'one' AND end_ts IS NULL",
            1
        )
        .await,
        "the complete line opened its row"
    );
    assert!(
        row_count_matches(
            &root,
            "SELECT COUNT(*) FROM events WHERE track_id = 'two'",
            0
        )
        .await,
        "the partial line was discarded"
    );
    drop(listener);
    drop(sender);
    drop(handle);
}

#[tokio::test]
async fn an_open_track_survives_a_redial_so_the_second_stream_closes_it() {
    let dir = temp_root("keepstate");
    let socket = dir.join("events.sock");
    let _ = std::fs::remove_file(&socket);
    let (root, sender, handle) = rig("keepstate-index");
    tokio::spawn(event_consumer::run_http(
        socket.clone(),
        sender.clone(),
        EventWriter::new(review_all()),
        0,
        epoch_ms,
        BACKOFF,
    ));
    let listener = UnixListener::bind(&socket).expect("listener binds");
    let mut first = serve_stream(&listener).await;
    first
        .write_all(&chunk_frame(&start_frame("one")))
        .await
        .expect("start line delivered");
    drop(first);
    let mut second = serve_stream(&listener).await;
    second
        .write_all(&chunk_frame(&end_frame("one")))
        .await
        .expect("end line delivered");
    assert!(
        row_count_matches(
            &root,
            "SELECT COUNT(*) FROM events WHERE track_id = 'one' AND end_ts IS NOT NULL",
            1
        )
        .await,
        "the end line closed the row the previous stream opened"
    );
    drop(second);
    drop(listener);
    drop(sender);
    drop(handle);
}

#[tokio::test]
async fn the_consumer_keeps_its_write_half_open_while_the_stream_is_wanted() {
    let dir = temp_root("writehalf");
    let socket = dir.join("events.sock");
    let _ = std::fs::remove_file(&socket);
    let (_root, sender, handle) = rig("writehalf-index");
    tokio::spawn(event_consumer::run_http(
        socket.clone(),
        sender.clone(),
        EventWriter::new(review_all()),
        0,
        epoch_ms,
        BACKOFF,
    ));
    let listener = UnixListener::bind(&socket).expect("listener binds");
    let stream = serve_stream(&listener).await;
    let mut saw_eof = false;
    let mut probe = [0_u8; 1];
    for _ in 0..30 {
        match stream.try_read(&mut probe) {
            Ok(0) => saw_eof = true,
            Ok(_) => {}
            Err(source) => assert_ne!(
                source.kind(),
                std::io::ErrorKind::NotConnected,
                "the connection stays usable from the service side"
            ),
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !saw_eof,
        "the bridge never half-closes its write side while it still wants the stream"
    );
    drop(stream);
    drop(listener);
    drop(sender);
    drop(handle);
}

#[tokio::test]
async fn the_sweep_keeps_running_while_the_stream_stalls() {
    let dir = temp_root("stall");
    let socket = dir.join("events.sock");
    let _ = std::fs::remove_file(&socket);
    let (tx, rx) = std::sync::mpsc::sync_channel(64);
    tokio::spawn(event_consumer::run_http(
        socket.clone(),
        tx,
        EventWriter::new(review_all()),
        0,
        epoch_ms,
        BACKOFF,
    ));
    let listener = UnixListener::bind(&socket).expect("listener binds");
    let stream = serve_stream(&listener).await;
    let mut saw_sweep = false;
    for _ in 0..40 {
        while let Ok(write) = rx.try_recv() {
            if matches!(write, IndexWrite::Sweep { .. }) {
                saw_sweep = true;
            }
        }
        if saw_sweep {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        saw_sweep,
        "the sweep fires while no stream bytes have arrived"
    );
    drop(stream);
    drop(listener);
}

#[tokio::test]
async fn index_rows_open_and_close_across_a_server_restart_on_a_temp_directory() {
    let dir = temp_root("e2e");
    let socket = dir.join("events.sock");
    let _ = std::fs::remove_file(&socket);
    let (root, sender, handle) = rig("e2e-index");
    tokio::spawn(event_consumer::run_http(
        socket.clone(),
        sender.clone(),
        EventWriter::new(review_all()),
        0,
        epoch_ms,
        BACKOFF,
    ));
    let listener = UnixListener::bind(&socket).expect("first listener binds");
    let mut stream = serve_stream(&listener).await;
    stream
        .write_all(&chunk_frame(&start_frame("one")))
        .await
        .expect("start line delivered");
    assert!(
        row_count_matches(
            &root,
            "SELECT COUNT(*) FROM events WHERE track_id = 'one' AND end_ts IS NULL",
            1
        )
        .await,
        "the start line opened an index row"
    );
    drop(stream);
    drop(listener);
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("the redial rebinds");
    let mut stream = serve_stream(&listener).await;
    stream
        .write_all(&chunk_frame(&end_frame("one")))
        .await
        .expect("end line delivered");
    assert!(
        row_count_matches(
            &root,
            "SELECT COUNT(*) FROM events WHERE track_id = 'one' AND end_ts IS NOT NULL",
            1
        )
        .await,
        "the end line after the restart closed the row"
    );
    drop(stream);
    drop(listener);
    drop(sender);
    drop(handle);
}
