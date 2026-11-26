//! Tests both activity backings: the absent setting and the Corvette-index setting.

use corvette_api::ErrorBody;
use corvette_api_server::activity::ActivityBacking;
use corvette_api_server::config::ConfigSource;
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::task::JoinHandle;

const CACHE_TTL: Duration = Duration::from_mins(5);

const REVIEWSEGMENT_DDL: &str = r#"CREATE TABLE "reviewsegment" ("id" VARCHAR(30) NOT NULL PRIMARY KEY, "camera" VARCHAR(20) NOT NULL, "start_time" DATETIME NOT NULL, "end_time" DATETIME, "severity" VARCHAR(30) NOT NULL, "thumb_path" VARCHAR(255) NOT NULL, "data" JSON NOT NULL)"#;
const RECORDINGS_DDL: &str = r#"CREATE TABLE "recordings" ("id" VARCHAR(30) NOT NULL PRIMARY KEY, "camera" VARCHAR(20) NOT NULL, "path" VARCHAR(255) NOT NULL, "start_time" DATETIME NOT NULL, "end_time" DATETIME NOT NULL, "duration" REAL NOT NULL, "motion" INTEGER)"#;

/// Tables as `crates/corvette-media-bridge/src/index.rs` creates them.
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

struct TestEnv {
    socket: PathBuf,
    dir: PathBuf,
    handle: JoinHandle<()>,
}

impl TestEnv {
    async fn start(tag: &str, backing: Option<ActivityBacking>, now: f64) -> Self {
        let (_fixture, config_addr) =
            Fixture::start(vec![Behavior::Respond(config_response(&fixture_config()))]).await;
        let dir = unique_dir(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test directory");
        let socket = dir.join("api.sock");
        let db_path = dir.join("frigate.db");
        let index_path = dir.join("index.sqlite");
        let config = Arc::new(ConfigSource::new(config_addr, CACHE_TTL));
        let server = ApiServer::bind(&socket, CONNECTION_TIMEOUT, config, &db_path)
            .expect("bind")
            .with_index_path(&index_path)
            .with_clock(move || now);
        let server = match backing {
            Some(backing) => server.with_activity_backing(backing),
            None => server,
        };
        let socket = server.socket_path().to_path_buf();
        let handle = tokio::spawn(server.serve_until(std::future::pending::<()>()));
        Self {
            socket,
            dir,
            handle,
        }
    }

    fn finish(self) {
        self.handle.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    fn seed_frigate_store(&self) {
        let db_path = self.dir.join("frigate.db");
        let db = Connection::open(&db_path).expect("create test database");
        db.execute_batch(REVIEWSEGMENT_DDL)
            .expect("create reviewsegment");
        db.execute_batch(RECORDINGS_DDL).expect("create recordings");
        db.execute(
            "INSERT INTO reviewsegment \
             (id, camera, start_time, end_time, severity, thumb_path, data) \
             VALUES ('r-front', 'front', 1500.0, 1600.0, 'alert', \
                     '/media/frigate/thumb-front.webp', '{}')",
            [],
        )
        .expect("insert review");
        db.execute(
            "INSERT INTO recordings \
             (id, camera, path, start_time, end_time, duration, motion) \
             VALUES ('motion-front', 'front', '/front/1500.mp4', 1500.0, 1530.0, 30.0, 3)",
            [],
        )
        .expect("insert recording");
    }

    fn seed_index(&self) {
        let index_path = self.dir.join("index.sqlite");
        let index = Connection::open(&index_path).expect("create index");
        index
            .execute_batch(INDEX_SCHEMA)
            .expect("create index schema");
        for (camera, start_ms, end_ms) in [("front", 0, 200_000), ("back", 0, 200_000)] {
            index
                .execute(
                    "INSERT INTO pieces \
                     (camera, seq, start_ts, end_ts, duration_s, path, size_bytes, codec) \
                     VALUES (?1, 1, ?2, ?3, 200.0, 'p', 1, 'h264')",
                    params![camera, start_ms, end_ms],
                )
                .expect("insert piece");
        }
        let reviews = [
            ("front", "alert", 50_000, Some(70_000), 70_000),
            ("back", "detection", 90_000, None, 110_000),
            ("front", "alert", 130_000, Some(140_000), 140_000),
        ];
        for (camera, severity, start_ms, end_ms, last_activity_ms) in reviews {
            index
                .execute(
                    "INSERT INTO reviews \
                     (camera, severity, start_ts, end_ts, last_activity_ts) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![camera, severity, start_ms, end_ms, last_activity_ms],
                )
                .expect("insert review");
        }
        for (camera, second) in [
            ("front", 29),
            ("front", 30),
            ("front", 59),
            ("front", 60),
            ("front", 89),
            ("front", 120),
            ("back", 30),
            ("back", 31),
            ("back", 119),
        ] {
            index
                .execute(
                    "INSERT INTO motion (camera, second) VALUES (?1, ?2)",
                    params![camera, second],
                )
                .expect("insert motion tick");
        }
    }

    async fn request_raw(&self, role: &str, query: &str) -> Vec<u8> {
        let request = format!(
            "GET /corvette/api/v1/activity?{query} HTTP/1.0\r\nRemote-User: alice\r\n\
             Remote-Role: {role}\r\n\r\n"
        );
        let mut stream = UnixStream::connect(&self.socket).await.expect("connect");
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("read to end");
        response
    }

    async fn activity_json(&self, role: &str, query: &str) -> Value {
        let raw = self.request_raw(role, query).await;
        let (status, headers, body) = http_parts(&raw);
        assert_eq!(
            status.split_whitespace().nth(1).unwrap_or_default(),
            "200",
            "status line {status:?} body {}",
            String::from_utf8_lossy(&body)
        );
        assert_eq!(
            headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        serde_json::from_slice(&body).expect("activity body parses as JSON")
    }
}

fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "corvette-activity-backing-test-{}-{tag}-{id}",
        std::process::id()
    ))
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

#[derive(Clone)]
enum Behavior {
    Respond(Vec<u8>),
}

struct Fixture {
    behaviors: Mutex<VecDeque<Behavior>>,
}

impl Fixture {
    async fn start(behaviors: Vec<Behavior>) -> (Arc<Self>, String) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fixture server");
        let addr = listener.local_addr().expect("fixture addr").to_string();
        let fixture = Arc::new(Self {
            behaviors: Mutex::new(behaviors.into()),
        });
        let served = Arc::clone(&fixture);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _peer)) = listener.accept().await else {
                    break;
                };
                let behavior = served.next_behavior();
                tokio::spawn(async move {
                    match behavior {
                        Behavior::Respond(response) => {
                            read_request(&mut stream).await;
                            let _ = stream.write_all(&response).await;
                            let _ = stream.shutdown().await;
                        }
                    }
                });
            }
        });
        (fixture, addr)
    }

    fn next_behavior(&self) -> Behavior {
        let behaviors = self.behaviors.lock().expect("behaviors lock");
        behaviors.front().expect("at least one behavior").clone()
    }
}

async fn read_request(stream: &mut TcpStream) {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 512];
    loop {
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return;
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => request.extend_from_slice(&chunk[..read]),
        }
    }
}

fn config_response(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn fixture_config() -> String {
    serde_json::json!({
        "auth": { "roles": { "admin": [], "frontonly": ["front"] } },
        "cameras": {
            "front": { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                       "detect": { "width": 704, "height": 576 } },
            "back":  { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                       "detect": { "width": 704, "height": 576 } }
        }
    })
    .to_string()
}

#[test]
fn the_setting_accepts_only_frigate_and_corvette() {
    assert_eq!(
        ActivityBacking::from_setting(None),
        Ok(ActivityBacking::Frigate)
    );
    assert_eq!(
        ActivityBacking::from_setting(Some("frigate")),
        Ok(ActivityBacking::Frigate)
    );
    assert_eq!(
        ActivityBacking::from_setting(Some("corvette")),
        Ok(ActivityBacking::Corvette)
    );
    assert!(ActivityBacking::from_setting(Some("Frigate")).is_err());
    assert!(ActivityBacking::from_setting(Some("")).is_err());
    assert!(ActivityBacking::from_setting(Some("index")).is_err());
}

#[tokio::test]
async fn an_absent_setting_answers_the_frigate_fixture_answer() {
    let env = TestEnv::start("default-frigate", None, 1_800.0).await;
    env.seed_frigate_store();

    let body = env.activity_json("admin", "after=1000&before=2000").await;
    assert_eq!(body["motion_bucket_seconds"], json!(30));
    let reviews = body["reviews"].as_array().expect("reviews array");
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0]["id"], json!("r-front"));
    assert_eq!(reviews[0]["camera"], json!("front"));
    assert!(reviews[0]["media"].get("thumbnail").is_some());
    let motion = body["motion"].as_array().expect("motion array");
    assert_eq!(
        motion.as_slice(),
        [json!({"start": 1500, "level": 3, "cameras": ["front"]})]
    );
    env.finish();
}

#[tokio::test]
async fn corvette_backing_answers_the_activity_shape_from_the_index() {
    let env = TestEnv::start("corvette-shape", Some(ActivityBacking::Corvette), 200.0).await;
    env.seed_index();

    let body = env.activity_json("admin", "after=0&before=120").await;
    assert_eq!(
        top_level_keys(&body),
        [
            "after",
            "before",
            "motion",
            "motion_bucket_seconds",
            "reviews"
        ]
    );
    assert_eq!(body["motion_bucket_seconds"], json!(30));

    let reviews = body["reviews"].as_array().expect("reviews array");
    assert_eq!(reviews.len(), 2);
    let [front, back]: [Value; 2] = reviews.clone().try_into().expect("two reviews");
    assert_eq!(
        review_keys(&front),
        ["camera", "end", "id", "media", "severity", "start"]
    );
    assert_eq!(front["camera"], json!("front"));
    assert_eq!(front["severity"], json!("alert"));
    assert_eq!(front["start"], json!(50.0));
    assert_eq!(front["end"], json!(70.0));
    assert_eq!(
        front["media"],
        json!({"clip": {"url": "/corvette/api/v1/media/front/start/46/end/74/master.m3u8"}})
    );
    assert_eq!(back["camera"], json!("back"));
    assert_eq!(back["severity"], json!("detection"));
    assert_eq!(back["start"], json!(90.0));
    assert!(back["end"].is_null());
    assert_eq!(
        back["media"],
        json!({"clip": {"url": "/corvette/api/v1/media/back/start/86/end/200/master.m3u8"}})
    );

    assert_eq!(
        body["motion"],
        json!([
            {"start": 0, "level": 1, "cameras": ["front"]},
            {"start": 30, "level": 2, "cameras": ["back", "front"]},
            {"start": 60, "level": 2, "cameras": ["front"]},
            {"start": 90, "level": 1, "cameras": ["back"]},
        ])
    );
    env.finish();
}

#[tokio::test]
async fn no_review_of_the_corvette_backing_carries_a_thumbnail_key() {
    let env = TestEnv::start(
        "corvette-no-thumbnail",
        Some(ActivityBacking::Corvette),
        200.0,
    )
    .await;
    env.seed_index();

    let body = env.activity_json("admin", "after=0&before=120").await;
    let reviews = body["reviews"].as_array().expect("reviews array");
    assert_eq!(reviews.len(), 2);
    for review in reviews {
        assert!(
            review["media"].get("thumbnail").is_none(),
            "review carries {review}"
        );
    }
    env.finish();
}

#[tokio::test]
async fn corvette_backing_filters_reviews_and_motion_by_role() {
    let env = TestEnv::start("corvette-role", Some(ActivityBacking::Corvette), 200.0).await;
    env.seed_index();

    let body = env.activity_json("frontonly", "after=0&before=120").await;
    let reviews = body["reviews"].as_array().expect("reviews array");
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0]["camera"], json!("front"));
    let motion = body["motion"].as_array().expect("motion array");
    assert_eq!(
        motion.as_slice(),
        [
            json!({"start": 0, "level": 1, "cameras": ["front"]}),
            json!({"start": 30, "level": 2, "cameras": ["front"]}),
            json!({"start": 60, "level": 2, "cameras": ["front"]}),
        ]
    );
    env.finish();
}

#[tokio::test]
async fn an_unreadable_index_answers_recording_unavailable() {
    let env = TestEnv::start(
        "corvette-unreadable",
        Some(ActivityBacking::Corvette),
        200.0,
    )
    .await;

    let raw = env.request_raw("admin", "after=0&before=120").await;
    let (status, _headers, body) = http_parts(&raw);
    assert_eq!(status.split_whitespace().nth(1).unwrap_or_default(), "503");
    let error: ErrorBody = serde_json::from_slice(&body).expect("error body parses");
    assert_eq!(error.code, "recording_unavailable");
    env.finish();
}

#[tokio::test]
async fn include_narrows_both_kinds_on_the_corvette_backing() {
    let env = TestEnv::start("corvette-include", Some(ActivityBacking::Corvette), 200.0).await;
    env.seed_index();

    let reviews_only = env
        .activity_json("admin", "after=0&before=120&include=reviews")
        .await;
    assert_eq!(reviews_only["motion"].as_array().expect("motion").len(), 0);
    assert_eq!(
        reviews_only["reviews"].as_array().expect("reviews").len(),
        2
    );
    let motion_only = env
        .activity_json("admin", "after=0&before=120&include=motion")
        .await;
    assert_eq!(motion_only["reviews"].as_array().expect("reviews").len(), 0);
    assert_eq!(motion_only["motion"].as_array().expect("motion").len(), 4);
    env.finish();
}

#[tokio::test]
async fn a_role_with_no_cameras_answers_empty_on_both_backings() {
    let frigate_env = TestEnv::start("empty-frigate", None, 1_800.0).await;
    let frigate_body = frigate_env
        .activity_json("nosuchrole", "after=0&before=120")
        .await;
    assert!(
        frigate_body["reviews"]
            .as_array()
            .expect("reviews")
            .is_empty()
    );
    assert!(
        frigate_body["motion"]
            .as_array()
            .expect("motion")
            .is_empty()
    );
    frigate_env.finish();

    let corvette_env =
        TestEnv::start("empty-corvette", Some(ActivityBacking::Corvette), 1_800.0).await;
    let corvette_body = corvette_env
        .activity_json("nosuchrole", "after=0&before=120")
        .await;
    assert_eq!(corvette_body, frigate_body);
    corvette_env.finish();
}

fn top_level_keys(body: &Value) -> Vec<&str> {
    body.as_object()
        .expect("body is an object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn review_keys(review: &Value) -> Vec<&str> {
    review
        .as_object()
        .expect("review is an object")
        .keys()
        .map(String::as_str)
        .collect()
}
