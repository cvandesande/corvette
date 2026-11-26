//! The events table and its consumer: the schema upgrade, the label filter, the moved-at-least-once rule, the refresh throttle, the cutoff sweep and the socket reconnect.

use corvette_detect_proto::{TrackFrame, TrackInfo, encode_track_line};
use corvette_media_bridge::event_consumer::EventWriter;
use corvette_media_bridge::index::{self, EventWrite, Index, IndexWrite, PIECE_CHANNEL_CAPACITY};
use corvette_media_bridge::record_config::Review;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::SyncSender;
use std::thread::JoinHandle;

const BASE_EPOCH_MS: i64 = 1_780_000_000_000;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("corvette-events-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp root created");
    root
}

fn rig(name: &str) -> (PathBuf, SyncSender<IndexWrite>, JoinHandle<Index>) {
    let root = temp_root(name);
    let index = Index::open(&root.join("index.sqlite"), &[("front".to_owned(), 4u32)])
        .expect("index opens");
    let (sender, handle) = index::spawn_writer(index, PIECE_CHANNEL_CAPACITY);
    (root, sender, handle)
}

fn flush(sender: SyncSender<IndexWrite>, handle: JoinHandle<Index>) -> Index {
    drop(sender);
    handle.join().expect("writer thread returned the index")
}

fn review_all() -> HashMap<String, Review> {
    let mut map = HashMap::new();
    map.insert("front".to_owned(), Review::default());
    map
}

fn review_person_car() -> HashMap<String, Review> {
    let mut map = HashMap::new();
    map.insert(
        "front".to_owned(),
        Review {
            alert_labels: vec!["person".to_owned(), "car".to_owned()],
            detection_labels: Some(vec!["person".to_owned(), "car".to_owned()]),
        },
    );
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

fn open_write(track_id: &str, label: &str, last_motion_ts: i64) -> EventWrite {
    EventWrite::Open {
        camera: "front".to_owned(),
        track_id: track_id.to_owned(),
        label: label.to_owned(),
        start_ts: last_motion_ts,
        first_motion_ts: last_motion_ts,
        last_motion_ts,
        box_json: "[0.1,0.2,0.3,0.4]".to_owned(),
        zones_json: "[]".to_owned(),
    }
}

fn alert_map() -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    map.insert("front".to_owned(), vec!["person".to_owned()]);
    map
}

fn open_index_file_with_open_event(
    name: &str,
    track_id: &str,
    label: &str,
    last_motion_ts: i64,
) -> (Index, PathBuf) {
    let root = temp_root(name);
    let path = root.join("index.sqlite");
    let mut index = Index::open(&path, &[("front".to_owned(), 4u32)]).expect("index opens");
    index
        .apply_event_write(&open_write(track_id, label, last_motion_ts))
        .expect("open event row lands");
    (index, path)
}

fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(BASE_EPOCH_MS, |elapsed| {
            let millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
            i64::try_from(millis).unwrap_or(i64::MAX)
        })
}

async fn send_frames_over(listener: &tokio::net::UnixListener, frames: &[TrackFrame]) {
    use tokio::io::AsyncWriteExt;
    let (mut stream, _) = listener.accept().await.expect("connection accepted");
    for frame in frames {
        stream
            .write_all(&encode_track_line(frame))
            .await
            .expect("line written");
    }
    stream.flush().await.expect("lines flushed");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

#[test]
fn a_label_outside_detection_labels_never_opens_a_row() {
    let mut writer = EventWriter::new(review_person_car());
    let bird = TrackFrame::Update(track("bird", "1", Some(100.0), 100.0));
    assert!(writer.apply(&bird, BASE_EPOCH_MS).0.is_empty());
    let person = TrackFrame::Update(track("person", "2", Some(100.0), 100.0));
    assert_eq!(writer.apply(&person, BASE_EPOCH_MS).0.len(), 1);
}

#[test]
fn a_never_moved_track_has_no_row_and_a_moved_track_has_one() {
    let (root, sender, handle) = rig("moved");
    let mut writer = EventWriter::new(review_all());
    let frames = [
        TrackFrame::Start(track("person", "still", None, 1000.0)),
        TrackFrame::Update(track("person", "still", None, 1500.0)),
        TrackFrame::Update(track("person", "mover", Some(1200.0), 1300.0)),
    ];
    for frame in frames {
        for write in writer.apply(&frame, BASE_EPOCH_MS).0 {
            sender.send(IndexWrite::Event(write)).expect("write queued");
        }
    }
    let index = flush(sender, handle);
    let rows = index.events("front").expect("events read");
    assert_eq!(rows.len(), 1, "only the moved track gets a row");
    assert_eq!(rows[0].track_id, "mover");
    assert_eq!(rows[0].start_ts, 1_200_000);
    assert_eq!(rows[0].first_motion, Some(1_200_000));
    assert!(rows[0].end_ts.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn two_updates_inside_five_seconds_write_once_and_past_five_seconds_write_again() {
    let (root, sender, handle) = rig("throttle");
    let mut writer = EventWriter::new(review_all());
    let mut write_at = |last_seen: f64, now_ms: i64| {
        let frame = TrackFrame::Update(track("person", "7", Some(0.0), last_seen));
        for write in writer.apply(&frame, now_ms).0 {
            sender.send(IndexWrite::Event(write)).expect("queued");
        }
    };
    write_at(0.0, BASE_EPOCH_MS);
    write_at(1.0, BASE_EPOCH_MS + 1_000);
    write_at(4.0, BASE_EPOCH_MS + 4_000);
    write_at(6.0, BASE_EPOCH_MS + 6_000);
    let index = flush(sender, handle);
    let rows = index.events("front").expect("events read");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].last_motion_ts, 6_000,
        "the update past five seconds was the second write"
    );
    assert_ne!(
        rows[0].last_motion_ts, 1_000,
        "the update inside five seconds wrote nothing"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_end_line_sets_end_ts_on_the_open_row() {
    let (root, sender, handle) = rig("end");
    let mut writer = EventWriter::new(review_all());
    for write in writer
        .apply(
            &TrackFrame::Update(track("person", "7", Some(10.0), 10.0)),
            BASE_EPOCH_MS,
        )
        .0
    {
        sender.send(IndexWrite::Event(write)).expect("open queued");
    }
    let closes = writer
        .apply(
            &TrackFrame::End {
                camera: "front".to_owned(),
                track_id: "7".to_owned(),
            },
            BASE_EPOCH_MS + 2_000,
        )
        .0;
    assert_eq!(closes.len(), 1);
    for write in closes {
        sender.send(IndexWrite::Event(write)).expect("close queued");
    }
    let index = flush(sender, handle);
    let rows = index.events("front").expect("events read");
    assert_eq!(rows[0].end_ts, Some(BASE_EPOCH_MS + 2_000));
    assert_eq!(index.open_event_count().expect("count reads"), 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn reopening_the_index_keeps_open_rows_open_and_counts_them_for_recovery() {
    let (index, path) = open_index_file_with_open_event("reopen", "7", "person", BASE_EPOCH_MS);
    assert_eq!(index.open_event_count().expect("count reads"), 1);
    let rows = index.events("front").expect("events read");
    assert!(rows[0].end_ts.is_none(), "a restart never closes an event");
    drop(index);
    let reopened =
        Index::open(&path, &[("front".to_owned(), 4u32)]).expect("reopen succeeds after open");
    assert_eq!(reopened.open_event_count().expect("count reads"), 1);
    let rows = reopened.events("front").expect("events read");
    assert_eq!(rows[0].track_id, "7");
    assert!(
        rows[0].end_ts.is_none(),
        "the row stays open across the reopen"
    );
}

#[test]
fn a_v1_database_reopens_as_version_two_with_pieces_intact() {
    let root = temp_root("v1");
    let path = root.join("index.sqlite");
    {
        let conn = rusqlite::Connection::open(&path).expect("v1 file opens");
        conn.execute_batch(
            "
CREATE TABLE pieces (
    camera TEXT NOT NULL,
    seq INTEGER NOT NULL,
    start_ts INTEGER NOT NULL,
    end_ts INTEGER NOT NULL,
    duration_s REAL NOT NULL,
    path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    codec TEXT NOT NULL,
    upload_state TEXT NOT NULL DEFAULT 'local',
    PRIMARY KEY (camera, seq)
);
CREATE INDEX pieces_camera_start ON pieces(camera, start_ts);
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
INSERT INTO meta (key, value) VALUES ('schema_version', '1');
INSERT INTO pieces (camera, seq, start_ts, end_ts, duration_s, path, size_bytes, codec)
VALUES ('front', 1, 0, 4000, 4.0, 'recordings/front/2026-11-25/00/0-1.mp4', 10, 'h264');
",
        )
        .expect("v1 schema and row land");
    }
    let index = Index::open(&path, &[("front".to_owned(), 4u32)]).expect("v1 database migrates");
    let raw = rusqlite::Connection::open(&path).expect("raw reopen");
    let version: String = raw
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("meta readable");
    assert_eq!(version, "2");
    let new_tables: i64 = raw
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('events', 'motion')",
            [],
            |row| row.get(0),
        )
        .expect("sqlite_master readable");
    assert_eq!(
        new_tables, 2,
        "the new tables were created by the migration"
    );
    let pieces: i64 = raw
        .query_row("SELECT COUNT(*) FROM pieces", [], |row| row.get(0))
        .expect("pieces readable");
    assert_eq!(pieces, 1, "the v1 piece row survived");
    assert!(index.events("front").expect("events read").is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn reopening_a_version_two_index_twice_creates_no_new_objects() {
    let root = temp_root("twice");
    let path = root.join("index.sqlite");
    let _ = Index::open(&path, &[("front".to_owned(), 4u32)]).expect("first open");
    let count = || -> i64 {
        let raw = rusqlite::Connection::open(&path).expect("raw reopen");
        raw.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))
            .expect("sqlite_master readable")
    };
    let first = count();
    let _ = Index::open(&path, &[("front".to_owned(), 4u32)]).expect("second open");
    assert_eq!(count(), first);
    let _ = Index::open(&path, &[("front".to_owned(), 4u32)]).expect("third open");
    assert_eq!(count(), first);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_alert_label_event_closes_at_41_seconds_of_silence_and_stays_open_at_39() {
    let (mut index, root) = open_index_file_with_open_event("sweep-alert", "7", "person", 0);
    assert_eq!(
        index
            .close_stale_events(39_000, &alert_map())
            .expect("sweep runs"),
        0,
        "39 seconds of silence is inside the alert cutoff"
    );
    assert_eq!(index.open_event_count().expect("count reads"), 1);
    assert_eq!(
        index
            .close_stale_events(41_000, &alert_map())
            .expect("sweep runs"),
        1,
        "41 seconds of silence passes the alert cutoff"
    );
    let rows = index.events("front").expect("events read");
    assert_eq!(rows[0].end_ts, Some(40_000));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_detection_label_event_closes_at_31_seconds_of_silence_and_stays_open_at_29() {
    let (mut index, root) = open_index_file_with_open_event("sweep-detection", "8", "bird", 0);
    assert_eq!(
        index
            .close_stale_events(29_000, &alert_map())
            .expect("sweep runs"),
        0,
        "29 seconds of silence is inside the detection cutoff"
    );
    assert_eq!(
        index
            .close_stale_events(31_000, &alert_map())
            .expect("sweep runs"),
        1,
        "31 seconds of silence passes the detection cutoff"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_restart_opened_event_is_closed_by_the_sweep_and_not_at_startup() {
    let (index, path) = open_index_file_with_open_event("sweep-restart", "7", "person", 0);
    let rows = index.events("front").expect("events read");
    assert!(
        rows[0].end_ts.is_none(),
        "the first open leaves the event open"
    );
    drop(index);
    let mut reopened = Index::open(&path, &[("front".to_owned(), 4u32)]).expect("reopen");
    assert_eq!(
        reopened.open_event_count().expect("count reads"),
        1,
        "the restart left the event open"
    );
    assert_eq!(
        reopened
            .close_stale_events(41_000, &alert_map())
            .expect("sweep runs"),
        1,
        "the sweep closes what the restart left open"
    );
}

#[tokio::test]
async fn the_consumer_dials_again_after_the_socket_closes() {
    let dir = temp_root("reconnect");
    let socket = dir.join("events.sock");
    let _ = std::fs::remove_file(&socket);
    let (root, sender, handle) = rig("reconnect-index");
    let writer = EventWriter::new(review_all());
    tokio::spawn(corvette_media_bridge::event_consumer::run(
        socket.clone(),
        sender.clone(),
        writer,
        0,
        epoch_ms,
        std::time::Duration::from_millis(50),
    ));

    let listener = tokio::net::UnixListener::bind(&socket).expect("first listener binds");
    send_frames_over(
        &listener,
        &[TrackFrame::Update(track("person", "one", Some(10.0), 10.0))],
    )
    .await;
    drop(listener);
    let _ = std::fs::remove_file(&socket);

    let listener = tokio::net::UnixListener::bind(&socket).expect("second listener binds");
    send_frames_over(
        &listener,
        &[TrackFrame::Update(track("person", "two", Some(20.0), 20.0))],
    )
    .await;
    drop(listener);

    let mut second_line_applied = false;
    for _ in 0..100 {
        let raw = rusqlite::Connection::open_with_flags(
            root.join("index.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("read-only reopen");
        let count: i64 = raw
            .query_row(
                "SELECT COUNT(*) FROM events WHERE camera = 'front' AND track_id = 'two'",
                [],
                |row| row.get(0),
            )
            .expect("events readable");
        drop(raw);
        if count == 1 {
            second_line_applied = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        second_line_applied,
        "the consumer reconnected and applied the second socket's line"
    );
    drop(sender);
    drop(handle);
}

#[tokio::test]
async fn fixture_lines_over_a_socket_open_filter_and_close_events() {
    let dir = temp_root("fixture");
    let socket = dir.join("events.sock");
    let _ = std::fs::remove_file(&socket);
    let (root, sender, handle) = rig("fixture-index");
    let writer = EventWriter::new(review_person_car());
    tokio::spawn(corvette_media_bridge::event_consumer::run(
        socket.clone(),
        sender.clone(),
        writer,
        0,
        epoch_ms,
        std::time::Duration::from_millis(50),
    ));

    let listener = tokio::net::UnixListener::bind(&socket).expect("listener binds");
    send_frames_over(
        &listener,
        &[
            TrackFrame::Start(track("person", "p", None, 10.0)),
            TrackFrame::Update(track("person", "p", Some(11.0), 11.0)),
            TrackFrame::Update(track("bird", "b", Some(11.5), 11.5)),
            TrackFrame::End {
                camera: "front".to_owned(),
                track_id: "p".to_owned(),
            },
        ],
    )
    .await;
    drop(listener);

    let mut closed = None;
    for _ in 0..100 {
        let raw = rusqlite::Connection::open_with_flags(
            root.join("index.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("read-only reopen");
        closed = raw
            .query_row(
                "SELECT end_ts FROM events WHERE camera = 'front' AND track_id = 'p'",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten();
        drop(raw);
        if closed.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(closed.is_some(), "the end line closed the person event");
    let raw = rusqlite::Connection::open_with_flags(
        root.join("index.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("read-only reopen");
    let birds: i64 = raw
        .query_row(
            "SELECT COUNT(*) FROM events WHERE camera = 'front' AND label = 'bird'",
            [],
            |row| row.get(0),
        )
        .expect("events readable");
    drop(raw);
    assert_eq!(
        birds, 0,
        "the bird label is outside the camera's detection labels"
    );
    drop(sender);
    drop(handle);
}

#[test]
fn a_track_id_reused_after_its_close_gets_a_second_row() {
    let (root, sender, handle) = rig("reuse-closed");
    let mut writer = EventWriter::new(review_all());
    for write in writer
        .apply(
            &TrackFrame::Update(track("person", "7", Some(10.0), 10.0)),
            BASE_EPOCH_MS,
        )
        .0
    {
        sender
            .send(IndexWrite::Event(write))
            .expect("first open queued");
    }
    for write in writer
        .apply(
            &TrackFrame::End {
                camera: "front".to_owned(),
                track_id: "7".to_owned(),
            },
            BASE_EPOCH_MS + 2_000,
        )
        .0
    {
        sender.send(IndexWrite::Event(write)).expect("close queued");
    }
    for write in writer
        .apply(
            &TrackFrame::Update(track("person", "7", Some(90.0), 90.0)),
            BASE_EPOCH_MS + 60_000,
        )
        .0
    {
        sender
            .send(IndexWrite::Event(write))
            .expect("reused open queued");
    }
    let index = flush(sender, handle);
    let rows = index.events("front").expect("events read");
    assert_eq!(rows.len(), 2, "the reused id opened its own row");
    assert_eq!(
        rows[0].start_ts, 10_000,
        "the first row kept its own first-seen"
    );
    assert_eq!(
        rows[0].end_ts,
        Some(BASE_EPOCH_MS + 2_000),
        "the first row kept its own close"
    );
    assert_eq!(rows[1].start_ts, 90_000);
    assert!(rows[1].end_ts.is_none(), "the reused track is open");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_reused_track_id_opens_its_own_row_while_the_lost_end_row_waits_for_the_sweep() {
    let (root, sender, handle) = rig("reuse-open");
    let mut writer = EventWriter::new(review_all());
    for write in writer
        .apply(
            &TrackFrame::Update(track("person", "7", Some(10.0), 10.0)),
            10_000,
        )
        .0
    {
        sender
            .send(IndexWrite::Event(write))
            .expect("first open queued");
    }
    for write in writer
        .apply(
            &TrackFrame::Update(track("person", "7", Some(20.0), 20.0)),
            45_000,
        )
        .0
    {
        sender
            .send(IndexWrite::Event(write))
            .expect("the id reuse opened a second row");
    }
    sender.send(writer.sweep(50_001)).expect("sweep queued");
    let index = flush(sender, handle);
    let rows = index.events("front").expect("events read");
    assert_eq!(rows.len(), 2, "the id reuse got its own row");
    assert_eq!(rows[0].start_ts, 10_000);
    assert_eq!(
        rows[0].end_ts,
        Some(50_000),
        "the sweep closed the old row at last motion plus cutoff"
    );
    assert_eq!(rows[1].start_ts, 20_000);
    assert!(
        rows[1].end_ts.is_none(),
        "the new row is untouched by the sweep"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_repeated_open_of_the_same_triple_writes_nothing_and_cannot_reopen_a_closed_row() {
    let (mut index, root) = open_index_file_with_open_event("repeat-open", "7", "person", 10_000);
    let repeat = EventWrite::Open {
        camera: "front".to_owned(),
        track_id: "7".to_owned(),
        label: "person".to_owned(),
        start_ts: 10_000,
        first_motion_ts: 10_000,
        last_motion_ts: 90_000,
        box_json: "[0.5,0.5,0.6,0.6]".to_owned(),
        zones_json: "[\"shed\"]".to_owned(),
    };
    index
        .apply_event_write(&repeat)
        .expect("a repeat of the same triple is ignored");
    let rows = index.events("front").expect("events read");
    assert_eq!(rows.len(), 1, "the repeat wrote no second row");
    assert_eq!(rows[0].last_motion_ts, 10_000, "the repeat changed nothing");
    index
        .apply_event_write(&EventWrite::Close {
            camera: "front".to_owned(),
            track_id: "7".to_owned(),
            start_ts: 10_000,
            end_ts: 30_000,
        })
        .expect("close lands");
    index
        .apply_event_write(&repeat)
        .expect("a repeat after the close is ignored too");
    let rows = index.events("front").expect("events read");
    assert_eq!(
        rows[0].end_ts,
        Some(30_000),
        "a repeat cannot reopen a closed row"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_alert_event_stays_open_at_exactly_the_cutoff_and_closes_one_millisecond_later() {
    let (mut index, root) = open_index_file_with_open_event("sweep-alert-exact", "7", "person", 0);
    assert_eq!(
        index
            .close_stale_events(40_000, &alert_map())
            .expect("sweep runs"),
        0,
        "exactly forty seconds of silence keeps the row open"
    );
    assert_eq!(
        index
            .close_stale_events(40_001, &alert_map())
            .expect("sweep runs"),
        1,
        "one millisecond past forty seconds closes"
    );
    let rows = index.events("front").expect("events read");
    assert_eq!(rows[0].end_ts, Some(40_000));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_detection_event_stays_open_at_exactly_the_cutoff_and_closes_one_millisecond_later() {
    let (mut index, root) =
        open_index_file_with_open_event("sweep-detection-exact", "8", "bird", 0);
    assert_eq!(
        index
            .close_stale_events(30_000, &alert_map())
            .expect("sweep runs"),
        0,
        "exactly thirty seconds of silence keeps the row open"
    );
    assert_eq!(
        index
            .close_stale_events(30_001, &alert_map())
            .expect("sweep runs"),
        1,
        "one millisecond past thirty seconds closes"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn the_consumer_leaves_a_preexisting_open_row_open_until_the_sweep_passes_its_cutoff() {
    let (index, path) = open_index_file_with_open_event("startup-open", "7", "person", 0);
    let dir = temp_root("startup-open-sock");
    let socket = dir.join("events.sock");
    let _ = std::fs::remove_file(&socket);
    let (sender, handle) = index::spawn_writer(index, PIECE_CHANNEL_CAPACITY);
    tokio::spawn(corvette_media_bridge::event_consumer::run(
        socket.clone(),
        sender.clone(),
        EventWriter::new(review_all()),
        1,
        || 30_000,
        std::time::Duration::from_millis(50),
    ));

    let listener = tokio::net::UnixListener::bind(&socket).expect("first listener binds");
    tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
        .await
        .expect("the consumer connected at startup")
        .expect("the startup connection was read");
    drop(listener);
    let _ = std::fs::remove_file(&socket);

    let listener = tokio::net::UnixListener::bind(&socket).expect("second listener binds");
    tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
        .await
        .expect("the consumer reconnected")
        .expect("the reconnection was read");
    drop(listener);

    let raw =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("read-only reopen");
    let end_ts: Option<i64> = raw
        .query_row(
            "SELECT end_ts FROM events WHERE camera = 'front' AND track_id = '7'",
            [],
            |row| row.get(0),
        )
        .expect("the pre-existing row is present");
    drop(raw);
    assert!(
        end_ts.is_none(),
        "startup and the reconnect left the pre-existing row open with end_ts NULL"
    );

    sender
        .send(IndexWrite::Sweep {
            now_ms: 40_001,
            alert_labels: alert_map(),
        })
        .expect("sweep queued");
    let mut closed = None;
    for _ in 0..100 {
        let raw = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("read-only reopen");
        closed = raw
            .query_row(
                "SELECT end_ts FROM events WHERE camera = 'front' AND track_id = '7'",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten();
        drop(raw);
        if closed.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        closed,
        Some(40_000),
        "the sweep past the cutoff is what closed the row"
    );
    drop(sender);
    drop(handle);
}
