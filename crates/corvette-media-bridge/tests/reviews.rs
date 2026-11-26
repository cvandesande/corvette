//! Review items: per-camera aggregation of event activity, the cutoff sweep and restart persistence.

use corvette_detect_proto::{TrackFrame, TrackInfo};
use corvette_media_bridge::event_consumer::EventWriter;
use corvette_media_bridge::index::{
    self, EventWrite, Index, IndexWrite, PIECE_CHANNEL_CAPACITY, ReviewActivity,
};
use corvette_media_bridge::record_config::Review;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::thread::JoinHandle;

const BRIDGE_MS: i64 = 1_780_000_000_000;
const BRIDGE_SECONDS: f64 = 1_780_000_000.0;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("corvette-reviews-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp root created");
    root
}

fn rig(name: &str) -> (PathBuf, SyncSender<IndexWrite>, JoinHandle<Index>) {
    let root = temp_root(name);
    let index = Index::open(
        &root.join("index.sqlite"),
        &[("front".to_owned(), 4u32), ("back".to_owned(), 4u32)],
    )
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

fn activity(camera: &str, activity_ts: i64, is_alert: bool) -> IndexWrite {
    IndexWrite::Review(ReviewActivity {
        camera: camera.to_owned(),
        activity_ts,
        is_alert,
    })
}

fn review_rows(root: &Path) -> Vec<(String, String, i64, Option<i64>, i64)> {
    let raw = rusqlite::Connection::open_with_flags(
        root.join("index.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("read-only reopen");
    let mut statement = raw
        .prepare(
            "SELECT camera, severity, start_ts, end_ts, last_activity_ts FROM reviews ORDER BY id",
        )
        .expect("reviews readable");
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .expect("reviews query");
    rows.map(|row| row.expect("review row")).collect()
}

#[test]
fn two_events_ten_seconds_apart_share_one_open_review() {
    let (root, sender, handle) = rig("ten");
    sender.send(activity("front", 0, true)).expect("queued");
    sender
        .send(activity("front", 10_000, true))
        .expect("queued");
    flush(sender, handle);
    let rows = review_rows(&root);
    assert_eq!(rows.len(), 1, "two close activities share one item");
    assert_eq!(
        rows[0],
        ("front".to_owned(), "alert".to_owned(), 0, None, 10_000)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn two_events_sixty_seconds_apart_close_the_first_at_its_cutoff_and_open_a_second() {
    let (root, sender, handle) = rig("sixty");
    sender.send(activity("front", 0, false)).expect("queued");
    sender
        .send(activity("front", 60_000, false))
        .expect("queued");
    flush(sender, handle);
    let rows = review_rows(&root);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0],
        (
            "front".to_owned(),
            "detection".to_owned(),
            0,
            Some(30_000),
            0
        ),
        "the first item closed at its own rule instant, last activity plus cutoff"
    );
    assert_eq!(
        rows[1],
        (
            "front".to_owned(),
            "detection".to_owned(),
            60_000,
            None,
            60_000
        )
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_detection_item_upgrades_on_an_alert_event_and_stays_alert() {
    let (root, sender, handle) = rig("upgrade");
    sender.send(activity("front", 0, false)).expect("queued");
    sender
        .send(activity("front", 10_000, true))
        .expect("queued");
    sender
        .send(activity("front", 20_000, false))
        .expect("queued");
    flush(sender, handle);
    let rows = review_rows(&root);
    assert_eq!(rows.len(), 1, "the upgrade stays inside one item");
    assert_eq!(rows[0].1, "alert", "the alert event upgraded the review");
    assert_eq!(rows[0].3, None);
    assert_eq!(rows[0].4, 20_000, "the later detection event extended it");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_alert_item_stays_alert_when_a_later_event_is_detection() {
    let (root, sender, handle) = rig("no-downgrade");
    sender.send(activity("front", 0, true)).expect("queued");
    sender
        .send(activity("front", 10_000, false))
        .expect("queued");
    flush(sender, handle);
    let rows = review_rows(&root);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, "alert", "a detection event never downgrades");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_sweep_leaves_an_alert_item_open_at_exactly_forty_seconds_and_closes_one_millisecond_later() {
    let root = temp_root("sweep-alert");
    let mut index = Index::open(&root.join("index.sqlite"), &[("front".to_owned(), 4u32)])
        .expect("index opens");
    index
        .apply_review_activity(&ReviewActivity {
            camera: "front".to_owned(),
            activity_ts: 0,
            is_alert: true,
        })
        .expect("item opens");
    assert_eq!(
        index.close_stale_reviews(40_000).expect("sweep runs"),
        0,
        "exactly forty seconds of silence keeps the review open"
    );
    assert_eq!(
        index.close_stale_reviews(40_001).expect("sweep runs"),
        1,
        "one millisecond past forty seconds closes"
    );
    let rows = review_rows(&root);
    assert_eq!(rows[0].3, Some(40_000), "closed at the rule instant");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_sweep_leaves_a_detection_item_open_at_exactly_thirty_seconds_and_closes_one_millisecond_later()
 {
    let root = temp_root("sweep-detection");
    let mut index = Index::open(&root.join("index.sqlite"), &[("front".to_owned(), 4u32)])
        .expect("index opens");
    index
        .apply_review_activity(&ReviewActivity {
            camera: "front".to_owned(),
            activity_ts: 0,
            is_alert: false,
        })
        .expect("item opens");
    assert_eq!(
        index.close_stale_reviews(30_000).expect("sweep runs"),
        0,
        "exactly thirty seconds of silence keeps the review open"
    );
    assert_eq!(
        index.close_stale_reviews(30_001).expect("sweep runs"),
        1,
        "one millisecond past thirty seconds closes"
    );
    let rows = review_rows(&root);
    assert_eq!(rows[0].3, Some(30_000), "closed at the rule instant");
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn the_consumer_leaves_a_preexisting_open_review_open_until_the_sweep_closes_it() {
    let root = temp_root("startup-open");
    let path = root.join("index.sqlite");
    let mut index = Index::open(&path, &[("front".to_owned(), 4u32)]).expect("index opens");
    index
        .apply_review_activity(&ReviewActivity {
            camera: "front".to_owned(),
            activity_ts: 0,
            is_alert: true,
        })
        .expect("item opens");
    let dir = temp_root("startup-open-sock");
    let socket = dir.join("reviews.sock");
    let _ = std::fs::remove_file(&socket);
    let (sender, handle) = index::spawn_writer(index, PIECE_CHANNEL_CAPACITY);
    tokio::spawn(corvette_media_bridge::event_consumer::run(
        socket.clone(),
        sender.clone(),
        EventWriter::new(review_all()),
        0,
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
        .query_row("SELECT end_ts FROM reviews", [], |row| row.get(0))
        .expect("the pre-existing item is present");
    drop(raw);
    assert!(
        end_ts.is_none(),
        "startup and the reconnect left the pre-existing item open with end_ts NULL"
    );

    sender
        .send(IndexWrite::Sweep {
            now_ms: 40_001,
            alert_labels: HashMap::new(),
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
            .query_row("SELECT end_ts FROM reviews", [], |row| {
                row.get::<_, Option<i64>>(0)
            })
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
        "the sweep past the cutoff is what closed the review"
    );
    drop(sender);
    drop(handle);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn events_for_two_cameras_never_share_a_review() {
    let (root, sender, handle) = rig("two-cameras");
    sender.send(activity("front", 0, true)).expect("queued");
    sender.send(activity("back", 10_000, true)).expect("queued");
    flush(sender, handle);
    let rows = review_rows(&root);
    assert_eq!(rows.len(), 2, "each camera keeps its own open item");
    assert_eq!(rows[0].0, "front");
    assert_eq!(rows[0].3, None);
    assert_eq!(rows[1].0, "back");
    assert_eq!(rows[1].2, 10_000);
    assert_eq!(rows[1].3, None);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_refresh_after_the_cutoff_closes_the_old_item_at_last_activity_plus_cutoff() {
    let (root, sender, handle) = rig("late-refresh");
    sender.send(activity("front", 0, false)).expect("queued");
    sender
        .send(activity("front", 45_000, false))
        .expect("queued");
    flush(sender, handle);
    let rows = review_rows(&root);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].3,
        Some(30_000),
        "the arriving activity closed the old item at its own rule instant, not at arrival"
    );
    assert_eq!(
        rows[1].2, 45_000,
        "the new item starts at the arriving activity"
    );
    assert_eq!(rows[1].3, None);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_reused_track_id_after_a_consumer_restart_extends_the_open_item() {
    let (root, sender, handle) = rig("reuse");
    sender.send(activity("front", 0, true)).expect("queued");
    let mut writer = EventWriter::new(review_all());
    let frame = TrackFrame::Update(TrackInfo {
        camera: "front".to_owned(),
        track_id: "7".to_owned(),
        label: "person".to_owned(),
        bounds: [0.1, 0.2, 0.3, 0.4],
        zones: vec![],
        first_motion: Some(10.0),
        last_seen: 10.0,
    });
    let (event_writes, review_activity) = writer.apply(&frame, 10_000);
    assert_eq!(
        event_writes.len(),
        1,
        "the reused id opened its own event row"
    );
    assert!(
        review_activity.is_some(),
        "the same line is review activity"
    );
    for event in event_writes {
        sender.send(IndexWrite::Event(event)).expect("queued");
    }
    sender
        .send(IndexWrite::Review(
            review_activity.expect("the reuse carries its activity"),
        ))
        .expect("queued");
    flush(sender, handle);
    let rows = review_rows(&root);
    assert_eq!(rows.len(), 1, "the reused track attached to the open item");
    assert_eq!(rows[0].4, 10_000, "the review extended to the new track");
    assert_eq!(rows[0].3, None);
    let raw = rusqlite::Connection::open_with_flags(
        root.join("index.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("read-only reopen");
    let events: i64 = raw
        .query_row(
            "SELECT COUNT(*) FROM events WHERE track_id = '7'",
            [],
            |row| row.get(0),
        )
        .expect("events readable");
    assert_eq!(events, 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn review_rows_never_carry_a_path_column() {
    let root = temp_root("columns");
    let index = Index::open(&root.join("index.sqlite"), &[("front".to_owned(), 4u32)])
        .expect("index opens");
    let raw = rusqlite::Connection::open_with_flags(
        root.join("index.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("read-only reopen");
    let names: Vec<String> = raw
        .prepare("SELECT name FROM pragma_table_info('reviews')")
        .expect("columns readable")
        .query_map([], |row| row.get(0))
        .expect("columns query")
        .map(|name| name.expect("column name"))
        .collect();
    assert_eq!(
        names,
        vec![
            "id".to_owned(),
            "camera".to_owned(),
            "severity".to_owned(),
            "start_ts".to_owned(),
            "end_ts".to_owned(),
            "last_activity_ts".to_owned(),
        ],
        "a review row carries no media reference"
    );
    drop(index);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_end_line_sets_review_activity_to_its_bridge_arrival_time() {
    let (root, sender, handle) = rig("end-activity");
    let mut writer = EventWriter::new(review_all());
    let start = TrackFrame::Start(TrackInfo {
        camera: "front".to_owned(),
        track_id: "7".to_owned(),
        label: "person".to_owned(),
        bounds: [0.1, 0.2, 0.3, 0.4],
        zones: vec![],
        first_motion: Some(BRIDGE_SECONDS),
        last_seen: BRIDGE_SECONDS,
    });
    let (event_writes, review_activity) = writer.apply(&start, BRIDGE_MS);
    for event in event_writes {
        sender.send(IndexWrite::Event(event)).expect("queued");
    }
    sender
        .send(IndexWrite::Review(
            review_activity.expect("the open line is activity"),
        ))
        .expect("queued");
    let (end_writes, end_activity) = writer.apply(
        &TrackFrame::End {
            camera: "front".to_owned(),
            track_id: "7".to_owned(),
        },
        BRIDGE_MS + 30_000,
    );
    let activity = end_activity.expect("the end line is activity");
    assert_eq!(
        activity.activity_ts,
        BRIDGE_MS + 30_000,
        "the end activity is the bridge arrival time itself"
    );
    for event in end_writes {
        sender.send(IndexWrite::Event(event)).expect("queued");
    }
    sender.send(IndexWrite::Review(activity)).expect("queued");
    let mut index = flush(sender, handle);
    let rows = review_rows(&root);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].4,
        BRIDGE_MS + 30_000,
        "the review's last activity is exactly the end line's bridge time"
    );
    assert_eq!(rows[0].3, None);
    let closed_at = BRIDGE_MS + 70_000;
    assert_eq!(
        index.close_stale_reviews(closed_at).expect("sweep runs"),
        0,
        "at exactly the cutoff the review stays open"
    );
    assert_eq!(
        index
            .close_stale_reviews(closed_at + 1)
            .expect("sweep runs"),
        1,
        "one millisecond past the cutoff closes it"
    );
    let rows = review_rows(&root);
    assert_eq!(
        rows[0].3,
        Some(closed_at),
        "the review closes at its last activity plus the cutoff"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_line_dated_an_hour_ahead_is_used_at_the_bridge_clock_and_closes_by_its_cutoff() {
    let (root, sender, handle) = rig("future-line");
    let mut writer = EventWriter::new(review_all());
    let ahead = BRIDGE_SECONDS + 3_600.0;
    let line = TrackFrame::Start(TrackInfo {
        camera: "front".to_owned(),
        track_id: "7".to_owned(),
        label: "person".to_owned(),
        bounds: [0.1, 0.2, 0.3, 0.4],
        zones: vec![],
        first_motion: Some(ahead),
        last_seen: ahead,
    });
    let (event_writes, review_activity) = writer.apply(&line, BRIDGE_MS);
    assert_eq!(event_writes.len(), 1);
    match &event_writes[0] {
        EventWrite::Open {
            last_motion_ts,
            start_ts,
            ..
        } => {
            assert_eq!(
                *last_motion_ts, BRIDGE_MS,
                "a future line time is clamped to the bridge clock"
            );
            assert_eq!(
                *start_ts,
                BRIDGE_MS + 3_600_000,
                "the stored start stays exactly as received"
            );
        }
        other => panic!("the start line opens an event row, not {other:?}"),
    }
    let activity = review_activity.expect("the open line is activity");
    assert_eq!(
        activity.activity_ts, BRIDGE_MS,
        "the review activity is clamped to the bridge clock"
    );
    for event in event_writes {
        sender.send(IndexWrite::Event(event)).expect("queued");
    }
    sender.send(IndexWrite::Review(activity)).expect("queued");
    let mut index = flush(sender, handle);
    let rows = review_rows(&root);
    assert_eq!(rows[0].4, BRIDGE_MS);
    assert_eq!(
        index
            .close_stale_reviews(BRIDGE_MS + 40_001)
            .expect("sweep runs"),
        1,
        "the review closes by the cutoff measured from the bridge clock"
    );
    assert_eq!(review_rows(&root)[0].3, Some(BRIDGE_MS + 40_000));
    assert_eq!(
        index
            .close_stale_events(BRIDGE_MS + 40_001, &HashMap::new())
            .expect("event sweep runs"),
        1,
        "the event row closes by the same bridge-clock rule"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_older_dated_line_never_moves_activity_or_motion_backwards() {
    let (root, sender, handle) = rig("old-line");
    let mut writer = EventWriter::new(review_all());
    let forward_line = |writer: &mut EventWriter,
                        line: TrackFrame,
                        bridge: i64,
                        sender: &SyncSender<IndexWrite>| {
        let (event_writes, review_activity) = writer.apply(&line, bridge);
        assert!(!event_writes.is_empty(), "the line wrote an event row");
        for event in event_writes {
            sender.send(IndexWrite::Event(event)).expect("queued");
        }
        sender
            .send(IndexWrite::Review(
                review_activity.expect("the line is activity"),
            ))
            .expect("queued");
    };
    let first = TrackFrame::Start(TrackInfo {
        camera: "front".to_owned(),
        track_id: "7".to_owned(),
        label: "person".to_owned(),
        bounds: [0.1, 0.2, 0.3, 0.4],
        zones: vec![],
        first_motion: Some(BRIDGE_SECONDS),
        last_seen: BRIDGE_SECONDS,
    });
    forward_line(&mut writer, first, BRIDGE_MS, &sender);
    let second = TrackFrame::Update(TrackInfo {
        camera: "front".to_owned(),
        track_id: "7".to_owned(),
        label: "person".to_owned(),
        bounds: [0.1, 0.2, 0.3, 0.4],
        zones: vec![],
        first_motion: Some(BRIDGE_SECONDS),
        last_seen: BRIDGE_SECONDS - 50.0,
    });
    forward_line(&mut writer, second, BRIDGE_MS + 6_000, &sender);
    let index = flush(sender, handle);
    let rows = review_rows(&root);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].4, BRIDGE_MS,
        "the older-dated line did not move the review's last activity backwards"
    );
    assert_eq!(rows[0].3, None);
    let events = index.events("front").expect("events read");
    assert_eq!(
        events[0].last_motion_ts, BRIDGE_MS,
        "the older-dated line did not move last motion backwards"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_line_at_or_behind_the_bridge_clock_is_stored_exactly_as_received() {
    let mut writer = EventWriter::new(review_all());
    let line = TrackFrame::Start(TrackInfo {
        camera: "front".to_owned(),
        track_id: "7".to_owned(),
        label: "person".to_owned(),
        bounds: [0.1, 0.2, 0.3, 0.4],
        zones: vec![],
        first_motion: Some(BRIDGE_SECONDS - 5.0),
        last_seen: BRIDGE_SECONDS - 5.0,
    });
    let (event_writes, review_activity) = writer.apply(&line, BRIDGE_MS);
    match &event_writes[0] {
        EventWrite::Open { last_motion_ts, .. } => assert_eq!(
            *last_motion_ts,
            BRIDGE_MS - 5_000,
            "a line at or behind the bridge clock is stored as received"
        ),
        other => panic!("the start line opens an event row, not {other:?}"),
    }
    assert_eq!(
        review_activity
            .expect("the open line is activity")
            .activity_ts,
        BRIDGE_MS - 5_000
    );
}
