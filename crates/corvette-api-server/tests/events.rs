//! Tests the events list and detail routes against a fixture config and a review index.

use corvette_api::ErrorBody;
use corvette_api_server::config::{ConfigSource, DEFAULT_CACHE_TTL};
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixStream};
use tokio::task::JoinHandle;

/// Tables as crates/corvette-media-bridge/src/index.rs creates them.
const INDEX_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS pieces (
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
CREATE INDEX IF NOT EXISTS pieces_camera_start ON pieces(camera, start_ts);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    camera TEXT NOT NULL,
    track_id TEXT NOT NULL,
    label TEXT NOT NULL,
    start_ts INTEGER NOT NULL,
    end_ts INTEGER,
    last_motion_ts INTEGER NOT NULL,
    first_motion INTEGER,
    box_json TEXT NOT NULL,
    zones_json TEXT NOT NULL,
    UNIQUE(camera, track_id, start_ts)
);
CREATE TABLE IF NOT EXISTS reviews (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    camera TEXT NOT NULL,
    severity TEXT NOT NULL CHECK (severity IN ('alert', 'detection')),
    start_ts INTEGER NOT NULL,
    end_ts INTEGER,
    last_activity_ts INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS reviews_camera_end ON reviews(camera, end_ts);
CREATE TABLE IF NOT EXISTS motion (
    camera TEXT NOT NULL,
    second INTEGER NOT NULL,
    UNIQUE(camera, second)
);
";

/// The request moment every clock-controlled server answers.
const NOW: f64 = 2000.0;

/// One seeded event: camera, track id, label, start, end, first motion, in seconds.
type EventSeed<'a> = (&'a str, &'a str, &'a str, i64, Option<i64>, Option<i64>);

struct TestEnv {
    socket: PathBuf,
    dir: PathBuf,
    index_path: PathBuf,
    handle: JoinHandle<()>,
}

impl TestEnv {
    async fn start(tag: &str) -> Self {
        let config_addr = start_config_backing().await;
        let dir = unique_dir(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test directory");
        let socket = dir.join("api.sock");
        let index_path = dir.join("index.sqlite");
        let config = Arc::new(ConfigSource::new(config_addr, DEFAULT_CACHE_TTL));
        let server = ApiServer::bind(&socket, CONNECTION_TIMEOUT, config, &dir.join("absent.db"))
            .expect("bind")
            .with_index_path(&index_path)
            .with_clock(|| NOW);
        let socket = server.socket_path().to_path_buf();
        let handle = tokio::spawn(server.serve_until(std::future::pending::<()>()));
        Self {
            socket,
            dir,
            index_path,
            handle,
        }
    }

    fn finish(self) {
        self.handle.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    fn create_index(&self) -> Connection {
        let index = Connection::open(&self.index_path).expect("create index");
        index
            .execute_batch(INDEX_SCHEMA)
            .expect("create index schema");
        index
    }

    /// Inserts pieces of `(camera, start_seconds, end_seconds)`.
    fn seed_index(&self, pieces: &[(&str, i64, i64)]) {
        let index = self.create_index();
        for (n, (camera, start, end)) in pieces.iter().enumerate() {
            let seq = i64::try_from(n).expect("piece count fits i64") + 1;
            index
                .execute(
                    "INSERT INTO pieces \
                     (camera, seq, start_ts, end_ts, duration_s, path, size_bytes, codec) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 'h264')",
                    params![
                        camera,
                        seq,
                        start * 1000,
                        end * 1000,
                        1.0,
                        format!("/{camera}/{start}.mp4")
                    ],
                )
                .expect("insert piece");
        }
    }

    /// Inserts reviews of `(camera, severity, start_seconds, end_seconds)`.
    fn seed_reviews(&self, reviews: &[(&str, &str, i64, Option<i64>)]) {
        let index = Connection::open(&self.index_path).expect("open index");
        for (camera, severity, start, end) in reviews {
            index
                .execute(
                    "INSERT INTO reviews (camera, severity, start_ts, end_ts, last_activity_ts) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        camera,
                        severity,
                        start * 1000,
                        end.map(|end| end * 1000),
                        end.unwrap_or(*start) * 1000
                    ],
                )
                .expect("insert review");
        }
    }

    /// Inserts events of `(camera, track_id, label, start_seconds, end_seconds, first_motion_seconds)`.
    fn seed_events(&self, events: &[EventSeed]) {
        let index = Connection::open(&self.index_path).expect("open index");
        for (camera, track_id, label, start, end, first_motion) in events {
            index
                .execute(
                    "INSERT INTO events \
                     (camera, track_id, label, start_ts, end_ts, last_motion_ts, first_motion, \
                      box_json, zones_json) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '[0.1,0.2,0.3,0.4]', '[\"yard\"]')",
                    params![
                        camera,
                        track_id,
                        label,
                        start * 1000,
                        end.map(|end| end * 1000),
                        end.unwrap_or(*start) * 1000,
                        first_motion.map(|fm| fm * 1000)
                    ],
                )
                .expect("insert event");
        }
    }

    async fn raw(&self, request: Vec<u8>) -> Vec<u8> {
        exchange(&self.socket, &request).await
    }

    /// The parsed `reviews` of a `200` list answer.
    async fn list(&self, role: &str, query: &str) -> Value {
        let raw = self
            .raw(get(
                &format!("/corvette/api/v1/events?{query}"),
                &["Remote-User: alice", &format!("Remote-Role: {role}")],
            ))
            .await;
        let (status, _headers, body) = http_parts(&raw);
        assert_eq!(status_code(&status), "200", "status line {status:?}");
        let answer: Value = serde_json::from_slice(&body).expect("events answer is JSON");
        answer["reviews"].clone()
    }

    /// The parsed detail answer of a `200` response.
    async fn detail(&self, role: &str, id: i64) -> Value {
        let raw = self.detail_raw(role, id).await;
        let (status, _headers, body) = http_parts(&raw);
        assert_eq!(status_code(&status), "200", "status line {status:?}");
        serde_json::from_slice(&body).expect("detail answer is JSON")
    }

    async fn detail_raw(&self, role: &str, id: i64) -> Vec<u8> {
        self.raw(get(
            &format!("/corvette/api/v1/events/{id}"),
            &["Remote-User: alice", &format!("Remote-Role: {role}")],
        ))
        .await
    }
}

fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "corvette-events-test-{}-{tag}-{id}",
        std::process::id()
    ))
}

fn get(path: &str, headers: &[&str]) -> Vec<u8> {
    let mut request = format!("GET {path} HTTP/1.0\r\n");
    for header in headers {
        request.push_str(header);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.into_bytes()
}

async fn exchange(socket: &Path, request: &[u8]) -> Vec<u8> {
    let mut stream = UnixStream::connect(socket).await.expect("connect");
    stream.write_all(request).await.expect("write");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read to end");
    response
}

fn http_parts(raw: &[u8]) -> (String, HashMap<String, String>, Vec<u8>) {
    let separator = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("response has no header/body separator");
    let head = &raw[..separator];
    let body = raw[separator + 4..].to_vec();
    let mut lines = std::str::from_utf8(head)
        .expect("response head is ASCII")
        .split("\r\n");
    let status = lines.next().unwrap_or_default().to_owned();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    (status, headers, body)
}

fn status_code(status: &str) -> &str {
    status.split_whitespace().nth(1).unwrap_or_default()
}

fn assert_error_shape(raw: &[u8], status_prefix: &str, code: &str) {
    let (status, _headers, body) = http_parts(raw);
    assert_eq!(
        status_code(&status),
        status_prefix,
        "status line {status:?}"
    );
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(error.code, code);
}

/// Enables `front` and `back`; `frontonly` sees only `front`.
fn fixture_config() -> String {
    json!({
        "auth": { "roles": {
            "admin": [],
            "frontonly": ["front"]
        } },
        "cameras": {
            "front": { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                       "detect": { "width": 704, "height": 576 } },
            "back":  { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                       "detect": { "width": 704, "height": 576 } }
        }
    })
    .to_string()
}

/// Answers only `GET /api/config` with the fixture configuration.
async fn start_config_backing() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind config backing");
    let addr = listener
        .local_addr()
        .expect("config backing addr")
        .to_string();
    let body = format!(
        "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        fixture_config().len(),
        fixture_config()
    );
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _peer)) = listener.accept().await else {
                break;
            };
            let response = body.clone().into_bytes();
            tokio::spawn(async move {
                let mut request = [0_u8; 512];
                let _ = stream.read(&mut request).await;
                let _ = stream.write_all(&response).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    addr
}

fn review_ids(reviews: &Value) -> Vec<i64> {
    reviews
        .as_array()
        .expect("reviews is an array")
        .iter()
        .map(|review| review["id"].as_i64().expect("review has an id"))
        .collect()
}

#[tokio::test]
async fn the_list_orders_reviews_by_start_then_id() {
    let env = TestEnv::start("order").await;
    env.seed_index(&[]);
    env.seed_reviews(&[
        ("front", "detection", 1000, Some(1010)),
        ("back", "alert", 900, Some(950)),
        ("front", "alert", 1000, Some(1020)),
    ]);

    let reviews = env.list("admin", "after=0&before=2000").await;
    assert_eq!(review_ids(&reviews), vec![2, 1, 3]);
    let starts: Vec<f64> = reviews
        .as_array()
        .unwrap()
        .iter()
        .map(|review| review["start"].as_f64().unwrap())
        .collect();
    assert_eq!(starts, vec![900.0, 1000.0, 1000.0]);
    env.finish();
}

#[tokio::test]
async fn window_bounds_are_half_open_on_both_sides() {
    let env = TestEnv::start("bounds").await;
    env.seed_index(&[]);
    env.seed_reviews(&[
        ("front", "detection", 0, Some(1000)),
        ("front", "detection", 2000, Some(2100)),
        ("front", "detection", 1500, Some(1600)),
    ]);

    assert_eq!(
        review_ids(&env.list("admin", "after=1000&before=2000").await),
        vec![3]
    );
    assert_eq!(
        review_ids(&env.list("admin", "after=0&before=1000").await),
        vec![1]
    );
    assert_eq!(
        review_ids(&env.list("admin", "after=2000&before=3000").await),
        vec![2]
    );
    env.finish();
}

#[tokio::test]
async fn a_review_crossing_the_window_edge_appears_once() {
    let env = TestEnv::start("crossing").await;
    env.seed_index(&[]);
    env.seed_reviews(&[("front", "detection", 900, Some(1050))]);

    assert_eq!(
        review_ids(&env.list("admin", "after=1000&before=2000").await),
        vec![1]
    );
    env.finish();
}

#[tokio::test]
async fn exactly_touching_reviews_are_listed_in_only_one_of_two_windows() {
    let env = TestEnv::start("touching").await;
    env.seed_index(&[]);
    env.seed_reviews(&[
        ("front", "detection", 900, Some(1000)),
        ("front", "detection", 1000, Some(1100)),
    ]);

    assert_eq!(
        review_ids(&env.list("admin", "after=0&before=1000").await),
        vec![1]
    );
    assert_eq!(
        review_ids(&env.list("admin", "after=1000&before=2000").await),
        vec![2]
    );
    env.finish();
}

#[tokio::test]
async fn an_open_review_appears_in_a_window_after_its_start() {
    let env = TestEnv::start("open-review").await;
    env.seed_index(&[]);
    env.seed_reviews(&[("front", "detection", 1500, None)]);

    let reviews = env.list("admin", "after=1600&before=1900").await;
    assert_eq!(review_ids(&reviews), vec![1]);
    assert_eq!(reviews[0]["end"], Value::Null);
    env.finish();
}

#[tokio::test]
async fn media_clip_answers_a_playlist_link_when_a_piece_overlaps() {
    let env = TestEnv::start("clip-overlap").await;
    env.seed_index(&[("front", 1050, 1090)]);
    env.seed_reviews(&[("front", "alert", 1000, Some(1100))]);

    let reviews = env.list("admin", "after=0&before=2000").await;
    assert_eq!(
        reviews[0]["media"]["clip"],
        json!({"url": "/corvette/api/v1/media/front/start/996/end/1104/master.m3u8"})
    );
    env.finish();
}

#[tokio::test]
async fn media_clip_is_null_without_a_piece() {
    let env = TestEnv::start("clip-none").await;
    env.seed_index(&[("front", 1400, 1500)]);
    env.seed_reviews(&[("front", "alert", 1000, Some(1100))]);

    let reviews = env.list("admin", "after=0&before=2000").await;
    assert_eq!(reviews[0]["media"]["clip"], Value::Null);
    env.finish();
}

#[tokio::test]
async fn media_clip_is_null_when_a_piece_only_touches_the_boundary() {
    let env = TestEnv::start("clip-touching").await;
    env.seed_index(&[("front", 900, 1000), ("front", 1100, 1200)]);
    env.seed_reviews(&[("front", "alert", 1000, Some(1100))]);

    let reviews = env.list("admin", "after=0&before=2000").await;
    assert_eq!(reviews[0]["media"]["clip"], Value::Null);
    env.finish();
}

#[tokio::test]
async fn media_clip_for_an_open_review_ends_at_the_request_moment() {
    let env = TestEnv::start("clip-open").await;
    env.seed_index(&[("front", 1400, 1600)]);
    env.seed_reviews(&[("front", "detection", 1500, None)]);

    let reviews = env.list("admin", "after=1450&before=1900").await;
    assert_eq!(
        reviews[0]["media"]["clip"],
        json!({"url": "/corvette/api/v1/media/front/start/1496/end/2000/master.m3u8"})
    );
    env.finish();
}

#[tokio::test]
async fn media_clip_is_null_for_an_open_review_the_index_cannot_reach_yet() {
    let env = TestEnv::start("clip-future-open").await;
    env.seed_index(&[]);
    env.seed_reviews(&[("front", "detection", 2500, None)]);

    let reviews = env.list("admin", "after=2400&before=2900").await;
    assert_eq!(review_ids(&reviews), vec![1]);
    assert_eq!(reviews[0]["media"]["clip"], Value::Null);
    env.finish();
}

#[tokio::test]
async fn the_list_never_answers_a_review_of_a_camera_the_role_cannot_see() {
    let env = TestEnv::start("role-list").await;
    env.seed_index(&[]);
    env.seed_reviews(&[
        ("front", "detection", 1000, Some(1010)),
        ("back", "alert", 1050, Some(1060)),
    ]);

    assert_eq!(
        review_ids(&env.list("frontonly", "after=0&before=2000").await),
        vec![1]
    );
    assert_eq!(
        env.list("frontonly", "after=0&before=2000&cameras=back")
            .await,
        json!([])
    );
    assert_eq!(env.list("ghost", "after=0&before=2000").await, json!([]));
    env.finish();
}

#[tokio::test]
async fn camera_and_severity_filters_narrow_the_window() {
    let env = TestEnv::start("filters").await;
    env.seed_index(&[]);
    env.seed_reviews(&[
        ("front", "alert", 1000, Some(1010)),
        ("back", "detection", 1050, Some(1060)),
    ]);

    assert_eq!(
        review_ids(&env.list("admin", "after=0&before=2000&cameras=front").await),
        vec![1]
    );
    assert_eq!(
        review_ids(
            &env.list("admin", "after=0&before=2000&cameras=back,front")
                .await
        ),
        vec![1, 2]
    );
    assert_eq!(
        env.list("admin", "after=0&before=2000&cameras=ghost").await,
        json!([])
    );
    assert_eq!(
        review_ids(
            &env.list("admin", "after=0&before=2000&severity=alert")
                .await
        ),
        vec![1]
    );
    assert_eq!(
        review_ids(
            &env.list("admin", "after=0&before=2000&severity=detection")
                .await
        ),
        vec![2]
    );
    assert_eq!(
        review_ids(
            &env.list("admin", "after=0&before=2000&severity=alert,detection")
                .await
        ),
        vec![1, 2]
    );
    env.finish();
}

#[tokio::test]
async fn a_window_without_reviews_answers_an_empty_list() {
    let env = TestEnv::start("empty-window").await;
    env.seed_index(&[]);
    env.seed_reviews(&[("front", "detection", 5000, Some(5100))]);

    let raw = env
        .raw(get(
            "/corvette/api/v1/events?after=0&before=2000",
            &["Remote-User: alice", "Remote-Role: admin"],
        ))
        .await;
    let (status, _headers, body) = http_parts(&raw);
    assert_eq!(status_code(&status), "200", "status line {status:?}");
    let answer: Value = serde_json::from_slice(&body).expect("answer is JSON");
    assert_eq!(answer, json!({"reviews": []}));
    env.finish();
}

#[tokio::test]
async fn the_detail_answers_the_review_and_only_its_own_events() {
    let env = TestEnv::start("detail-own-events").await;
    env.seed_index(&[]);
    env.seed_reviews(&[
        ("front", "alert", 1000, Some(1100)),
        ("front", "detection", 1100, Some(1200)),
    ]);
    env.seed_events(&[
        ("front", "7", "person", 1050, Some(1080), Some(1050)),
        ("front", "8", "car", 1100, Some(1150), Some(1105)),
    ]);

    let first = env.detail("admin", 1).await;
    assert_eq!(first["id"], json!(1));
    assert_eq!(first["camera"], json!("front"));
    assert_eq!(first["severity"], json!("alert"));
    assert_eq!(first["start"], json!(1000.0));
    assert_eq!(first["end"], json!(1100.0));
    assert_eq!(
        first["events"],
        json!([{
            "label": "person",
            "box": [0.1, 0.2, 0.3, 0.4],
            "zones": ["yard"],
            "first_motion": 1050.0,
        }])
    );

    let second = env.detail("admin", 2).await;
    assert_eq!(second["events"][0]["label"], json!("car"));
    env.finish();
}

#[tokio::test]
async fn the_detail_excludes_events_of_another_camera() {
    let env = TestEnv::start("detail-camera").await;
    env.seed_index(&[]);
    env.seed_reviews(&[("front", "detection", 1000, Some(1100))]);
    env.seed_events(&[
        ("front", "7", "person", 1050, None, None),
        ("back", "7", "person", 1050, None, None),
    ]);

    let detail = env.detail("admin", 1).await;
    assert_eq!(detail["events"].as_array().unwrap().len(), 1);
    env.finish();
}

#[tokio::test]
async fn an_event_open_inside_a_review_that_started_earlier_is_listed() {
    let env = TestEnv::start("detail-open-event").await;
    env.seed_index(&[]);
    env.seed_reviews(&[("front", "detection", 1000, Some(1100))]);
    let index = Connection::open(&env.index_path).expect("open index");
    index
        .execute(
            "INSERT INTO events \
             (camera, track_id, label, start_ts, end_ts, last_motion_ts, first_motion, \
              box_json, zones_json) \
             VALUES ('front', '7', 'person', 950000, NULL, 1050000, 950000, \
                     '[0.1,0.2,0.3,0.4]', '[\"yard\"]')",
            params![],
        )
        .expect("insert open event");

    let detail = env.detail("admin", 1).await;
    assert_eq!(detail["events"].as_array().unwrap().len(), 1);
    assert_eq!(detail["events"][0]["label"], json!("person"));
    env.finish();
}

#[tokio::test]
async fn an_event_active_across_two_adjacent_reviews_is_listed_in_both() {
    let env = TestEnv::start("detail-spanning").await;
    env.seed_index(&[]);
    env.seed_reviews(&[
        ("front", "detection", 1000, Some(1100)),
        ("front", "detection", 1100, Some(1200)),
    ]);
    env.seed_events(&[("front", "7", "person", 1050, Some(1150), Some(1050))]);

    let first = env.detail("admin", 1).await;
    assert_eq!(first["events"].as_array().unwrap().len(), 1);
    assert_eq!(first["events"][0]["label"], json!("person"));
    let second = env.detail("admin", 2).await;
    assert_eq!(second["events"].as_array().unwrap().len(), 1);
    assert_eq!(second["events"][0]["label"], json!("person"));
    env.finish();
}

#[tokio::test]
async fn an_event_ending_exactly_at_the_review_start_is_not_listed() {
    let env = TestEnv::start("detail-touching-event").await;
    env.seed_index(&[]);
    env.seed_reviews(&[("front", "detection", 1000, Some(1100))]);
    env.seed_events(&[("front", "7", "person", 900, Some(1000), Some(900))]);

    assert_eq!(env.detail("admin", 1).await["events"], json!([]));
    env.finish();
}

#[tokio::test]
async fn a_single_frame_event_inside_the_window_is_listed() {
    let env = TestEnv::start("detail-one-frame").await;
    env.seed_index(&[]);
    env.seed_reviews(&[("front", "detection", 1000, Some(1100))]);
    env.seed_events(&[("front", "7", "cat", 1050, None, None)]);

    let detail = env.detail("admin", 1).await;
    assert_eq!(detail["events"].as_array().unwrap().len(), 1);
    assert_eq!(detail["events"][0]["label"], json!("cat"));
    env.finish();
}

#[tokio::test]
async fn an_open_review_on_a_hidden_camera_is_absent_and_its_detail_is_not_found() {
    let env = TestEnv::start("role-open-hidden").await;
    env.seed_index(&[]);
    env.seed_reviews(&[
        ("front", "detection", 1000, Some(1010)),
        ("back", "alert", 1050, None),
    ]);

    assert_eq!(
        review_ids(&env.list("frontonly", "after=0&before=2000").await),
        vec![1]
    );
    assert_eq!(env.list("ghost", "after=0&before=2000").await, json!([]));

    let hidden = env.detail_raw("frontonly", 2).await;
    assert_error_shape(&hidden, "404", "not_found");
    assert_eq!(
        hidden,
        env.detail_raw("frontonly", 999).await,
        "the hidden open review answers identically to an unknown id"
    );
    assert_error_shape(&env.detail_raw("ghost", 1).await, "404", "not_found");
    env.finish();
}

#[tokio::test]
async fn the_detail_reports_unknown_ids_as_not_found() {
    let env = TestEnv::start("detail-unknown").await;
    env.seed_index(&[]);
    env.seed_reviews(&[("front", "detection", 1000, Some(1100))]);

    assert_error_shape(&env.detail_raw("admin", 999).await, "404", "not_found");
    assert_error_shape(&env.detail_raw("admin", 0).await, "404", "not_found");
    env.finish();
}

#[tokio::test]
async fn the_detail_never_answers_a_review_the_role_cannot_see() {
    let env = TestEnv::start("detail-role").await;
    env.seed_index(&[]);
    env.seed_reviews(&[
        ("front", "detection", 1000, Some(1100)),
        ("back", "alert", 1050, Some(1060)),
    ]);

    let front = env.detail("frontonly", 1).await;
    assert_eq!(front["camera"], json!("front"));
    let (status, _headers, body) = http_parts(&env.detail_raw("frontonly", 2).await);
    assert_eq!(status_code(&status), "404", "status line {status:?}");
    let error: ErrorBody = serde_json::from_slice(&body).expect("error body parses");
    assert_eq!(error.code, "not_found");
    env.finish();
}

#[tokio::test]
async fn malformed_list_queries_answer_bad_request() {
    let env = TestEnv::start("bad-queries").await;
    let cases: [&str; 11] = [
        "",
        "after=0",
        "before=100",
        "after=0&before=0",
        "after=-1&before=100",
        "after=0&before=3000000",
        "after=abc&before=100",
        "after=0&before=100&unknown=1",
        "after=0&before=100&after=5",
        "after=0&before=100&&",
        "after=0&before=100&severity=bird",
    ];
    for query in cases {
        let raw = env
            .raw(get(
                &format!("/corvette/api/v1/events?{query}"),
                &["Remote-User: alice", "Remote-Role: admin"],
            ))
            .await;
        assert_error_shape(&raw, "400", "bad_request");
    }
    let detail_raw = env
        .raw(get(
            "/corvette/api/v1/events/1?unknown=1",
            &["Remote-User: alice", "Remote-Role: admin"],
        ))
        .await;
    assert_error_shape(&detail_raw, "400", "bad_request");
    env.finish();
}

#[tokio::test]
async fn an_unreadable_index_answers_recording_unavailable() {
    let env = TestEnv::start("absent-index").await;

    let list = env
        .raw(get(
            "/corvette/api/v1/events?after=0&before=2000",
            &["Remote-User: alice", "Remote-Role: admin"],
        ))
        .await;
    assert_error_shape(&list, "503", "recording_unavailable");
    assert!(
        !env.index_path.exists(),
        "the read path must never create the index file"
    );

    let detail = env.detail_raw("admin", 1).await;
    assert_error_shape(&detail, "503", "recording_unavailable");

    std::fs::write(&env.index_path, "these bytes are not a database").expect("write file");
    let list = env
        .raw(get(
            "/corvette/api/v1/events?after=0&before=2000",
            &["Remote-User: alice", "Remote-Role: admin"],
        ))
        .await;
    assert_error_shape(&list, "503", "recording_unavailable");
    env.finish();
}

#[tokio::test]
async fn a_role_with_no_cameras_answers_empty_without_reading_the_index() {
    let env = TestEnv::start("no-cameras").await;

    assert_eq!(env.list("ghost", "after=0&before=2000").await, json!([]));
    assert!(
        !env.index_path.exists(),
        "the route must not open an index it will not answer from"
    );
    env.finish();
}

#[tokio::test]
async fn the_routes_read_an_index_the_service_cannot_write() {
    use std::os::unix::fs::PermissionsExt;
    let env = TestEnv::start("write-refused-open").await;
    env.seed_index(&[("front", 1050, 1090)]);
    env.seed_reviews(&[("front", "detection", 1000, Some(1100))]);
    // The deployed media mount is read-only for this service; a route that
    // opened read-write could not answer on it at all.
    std::fs::set_permissions(&env.index_path, std::fs::Permissions::from_mode(0o444))
        .expect("make index read-only");

    assert_eq!(
        review_ids(&env.list("admin", "after=0&before=2000").await),
        vec![1]
    );
    assert_eq!(env.detail("admin", 1).await["id"], json!(1));
    env.finish();
}
