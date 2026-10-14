//! Integration tests for `GET /corvette/api/v1/activity` against a fixture
//! configuration server and a Frigate-shaped database (issue #5 — I1 role
//! filtering, I3 media rules, and the error codes).

use corvette_api::{ActivityResponse, ActivityReview, ErrorBody, MediaLink};
use corvette_api_server::config::ConfigSource;
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use rusqlite::{Connection, params};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::task::JoinHandle;

/// Long enough that every request in one test shares a single config fetch.
const CACHE_TTL: Duration = Duration::from_mins(5);

/// Frigate's own DDL for the two tables the read uses, as `activity.rs`'s
/// unit tests state them.
const REVIEWSEGMENT_DDL: &str = r#"CREATE TABLE "reviewsegment" ("id" VARCHAR(30) NOT NULL PRIMARY KEY, "camera" VARCHAR(20) NOT NULL, "start_time" DATETIME NOT NULL, "end_time" DATETIME, "severity" VARCHAR(30) NOT NULL, "thumb_path" VARCHAR(255) NOT NULL, "data" JSON NOT NULL)"#;
const RECORDINGS_DDL: &str = r#"CREATE TABLE "recordings" ("id" VARCHAR(30) NOT NULL PRIMARY KEY, "camera" VARCHAR(20) NOT NULL, "path" VARCHAR(255) NOT NULL, "start_time" DATETIME NOT NULL, "end_time" DATETIME NOT NULL, "duration" REAL NOT NULL, "motion" INTEGER)"#;

/// Every request uses this range; the seeded activity lies inside it.
const RANGE: &str = "after=1000&before=2000";

/// A running service plus its temp dir and the paths it was bound with.
struct TestEnv {
    socket: PathBuf,
    dir: PathBuf,
    db_path: PathBuf,
    media_root: PathBuf,
    handle: JoinHandle<()>,
}

impl TestEnv {
    /// A service whose configuration source answers with the fixture config.
    /// The database file and media root exist as directories only: a test
    /// calls `seed_database` when it wants rows.
    async fn start(tag: &str) -> Self {
        Self::with_config_behaviors(
            tag,
            vec![Behavior::Respond(config_response(&fixture_config()))],
        )
        .await
    }

    /// A service whose configuration source refuses every connection.
    async fn refusing(tag: &str) -> Self {
        Self::with_config_behaviors(tag, vec![Behavior::Refuse]).await
    }

    async fn with_config_behaviors(tag: &str, behaviors: Vec<Behavior>) -> Self {
        let (_fixture, config_addr) = Fixture::start(behaviors).await;
        let dir = unique_dir(tag);
        let _ = std::fs::remove_dir_all(&dir);
        let media_root = dir.join("media");
        std::fs::create_dir_all(&media_root).expect("create media root");
        let socket = dir.join("api.sock");
        let db_path = dir.join("frigate.db");
        let config = Arc::new(ConfigSource::new(config_addr, CACHE_TTL));
        let server = ApiServer::bind(&socket, CONNECTION_TIMEOUT, config, &db_path)
            .expect("bind")
            .with_media_root(&media_root);
        let socket = server.socket_path().to_path_buf();
        let handle = tokio::spawn(server.serve_until(std::future::pending::<()>()));
        Self {
            socket,
            dir,
            db_path,
            media_root,
            handle,
        }
    }

    fn finish(self) {
        self.handle.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// Creates the database file holding the seeded activity rows.
    fn seed_database(&self) {
        let db = Connection::open(&self.db_path).expect("create test database");
        db.execute_batch(REVIEWSEGMENT_DDL)
            .expect("create reviewsegment");
        db.execute_batch(RECORDINGS_DDL).expect("create recordings");
        seed_activity(&db);
    }

    /// Writes `rel` under the media root, creating parent directories.
    fn write_media(&self, rel: &str) {
        write_file(&self.media_root, rel);
    }

    /// Writes `name` directly under the temp dir, which is the parent of the
    /// media root: where `media_root/../name` resolves.
    fn write_outside_media(&self, name: &str) {
        write_file(&self.dir, name);
    }

    /// Sends the request and returns the raw response bytes.
    async fn request_raw(&self, role: &str, query: &str) -> Vec<u8> {
        exchange(&self.socket, &activity_request(role, query)).await
    }

    /// Sends the request, asserts the `200` framing, and parses the body.
    async fn activity(&self, role: &str, query: &str) -> ActivityResponse {
        let raw = self.request_raw(role, query).await;
        let (status, headers, body) = http_parts(&raw);
        let actual = status.split_whitespace().nth(1).unwrap_or_default();
        assert_eq!(actual, "200", "status line {status:?}");
        assert_framing(&headers, &body);
        serde_json::from_slice(&body).expect("body parses as ActivityResponse")
    }
}

/// A fresh temp directory path, unique within this test process.
fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "corvette-activity-test-{}-{tag}-{id}",
        std::process::id()
    ))
}

/// Writes `rel` under `root`, creating parent directories.
fn write_file(root: &Path, rel: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().expect("file parent")).expect("create dirs");
    std::fs::write(&path, "bytes").expect("write file");
}

/// A well-formed `GET` request line followed by the given header lines.
fn get(path: &str, headers: &[&str]) -> Vec<u8> {
    let mut request = format!("GET {path} HTTP/1.0\r\n");
    for header in headers {
        request.push_str(header);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.into_bytes()
}

fn activity_request(role: &str, query: &str) -> Vec<u8> {
    get(
        &format!("/corvette/api/v1/activity?{query}"),
        &["Remote-User: alice", &format!("Remote-Role: {role}")],
    )
}

/// Sends `request` and reads the whole response.
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

/// Splits a raw response into its status line, header map (lower-cased names),
/// and body bytes.
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

/// Asserts I7's framing on every response: exactly the headers the service
/// promises, and a `Content-Length` that matches the body.
fn assert_framing(headers: &HashMap<String, String>, body: &[u8]) {
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json"),
        "Content-Type must be exactly application/json"
    );
    assert_eq!(
        headers.get("cache-control").map(String::as_str),
        Some("no-store")
    );
    assert_eq!(headers.get("connection").map(String::as_str), Some("close"));
    let declared: usize = headers
        .get("content-length")
        .expect("response carries a Content-Length")
        .parse()
        .expect("Content-Length is a number");
    assert_eq!(declared, body.len(), "Content-Length must match the body");
}

/// Asserts the single error shape (I7).
fn assert_error_shape(raw: &[u8], status_prefix: &str, code: &str) {
    let (status, headers, body) = http_parts(raw);
    let actual = status.split_whitespace().nth(1).unwrap_or_default();
    assert_eq!(actual, status_prefix, "status line {status:?}");
    assert_framing(&headers, &body);
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(error.code, code);
}

/// How the fixture config server answers a connection.
#[derive(Clone)]
enum Behavior {
    /// Read the request, write these response bytes, close.
    Respond(Vec<u8>),
    /// Close at once, as nothing behind the address would.
    Refuse,
}

/// A fixture for Frigate's config HTTP interface: it answers connections with
/// the queued behaviors (the last one repeats).
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
                        Behavior::Refuse => drop(stream),
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
        let mut behaviors = self.behaviors.lock().expect("behaviors lock");
        if behaviors.len() > 1 {
            behaviors.pop_front().expect("more than one behavior")
        } else {
            behaviors.front().expect("at least one behavior").clone()
        }
    }
}

/// Reads until the request's `\r\n\r\n` terminator; `None` if the peer closes
/// first.
async fn read_request(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 512];
    loop {
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return Some(request);
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return None,
            Ok(read) => request.extend_from_slice(&chunk[..read]),
        }
    }
}

/// The `HTTP/1.0` `200` framing the config source must accept around `body`.
fn config_response(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Cameras `front`, `back`, and `side` enabled plus `living` disabled; roles
/// `admin` (everything), `frontonly` (front), and `ghostonly` (a camera that
/// does not exist).
fn fixture_config() -> String {
    serde_json::json!({
        "auth": { "roles": {
            "admin": [],
            "frontonly": ["front"],
            "ghostonly": ["ghost"]
        } },
        "cameras": {
            "front":  { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                        "detect": { "width": 704, "height": 576 } },
            "back":   { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                        "detect": { "width": 704, "height": 576 } },
            "side":   { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
                        "detect": { "width": 704, "height": 576 } },
            "living": { "enabled": false, "friendly_name": null, "ui": { "order": 0 },
                        "detect": { "width": 640, "height": 480 } }
        }
    })
    .to_string()
}

/// One review and one motion recording per camera, all inside [`RANGE`],
/// seeded so the media rules each have a case:
/// - `r-front` ended with an overlapping recording and a well-formed
///   `thumb_path` whose file a test may or may not create;
/// - `r-back` ended without an overlapping recording and its `thumb_path`
///   sits outside the media prefix;
/// - `r-side` is ongoing and its `thumb_path` tries to escape the media root;
/// - `r-living` belongs to the disabled camera.
fn seed_activity(db: &Connection) {
    let reviews = [
        (
            "r-front",
            "front",
            "alert",
            1_500.0,
            Some(1_600.0),
            "/media/frigate/clips/review/thumb-front.webp",
        ),
        (
            "r-back",
            "back",
            "detection",
            1_510.0,
            Some(1_610.0),
            "/media/other/thumb-back.webp",
        ),
        (
            "r-side",
            "side",
            "significant_motion",
            1_520.0,
            None,
            "/media/frigate/../secret.webp",
        ),
        (
            "r-living",
            "living",
            "detection",
            1_530.0,
            Some(1_560.0),
            "/media/frigate/clips/review/thumb-living.webp",
        ),
    ];
    for (id, camera, severity, start, end, thumb_path) in reviews {
        db.execute(
            "INSERT INTO reviewsegment \
             (id, camera, start_time, end_time, severity, thumb_path, data) \
             VALUES (?, ?, ?, ?, ?, ?, '{}')",
            params![id, camera, start, end, severity, thumb_path],
        )
        .expect("insert review");
    }
    // Each recording only overlaps the review of its own camera whose clip
    // rule expects `true`.
    let recordings = [
        ("motion-front", "front", 1_500.0, 1_530.0, Some(3)),
        ("motion-back", "back", 1_800.0, 1_830.0, Some(4)),
        ("motion-side", "side", 1_650.0, 1_680.0, Some(2)),
        ("motion-living", "living", 1_540.0, 1_570.0, Some(5)),
    ];
    for (id, camera, start, end, motion) in recordings {
        db.execute(
            "INSERT INTO recordings \
             (id, camera, path, start_time, end_time, duration, motion) \
             VALUES (?, ?, ?, ?, ?, 30.0, ?)",
            params![
                id,
                camera,
                format!("/{camera}/{start}.mp4"),
                start,
                end,
                motion
            ],
        )
        .expect("insert recording");
    }
}

/// The camera names across all reviews, in response order.
fn review_cameras(response: &ActivityResponse) -> Vec<String> {
    response
        .reviews
        .iter()
        .map(|review| review.camera.clone())
        .collect()
}

/// The distinct camera names across all motion buckets, sorted.
fn motion_cameras(response: &ActivityResponse) -> Vec<String> {
    let mut names: Vec<String> = response
        .motion
        .iter()
        .flat_map(|bucket| bucket.cameras.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn review<'a>(response: &'a ActivityResponse, id: &str) -> &'a ActivityReview {
    response
        .reviews
        .iter()
        .find(|review| review.id == id)
        .unwrap_or_else(|| panic!("review `{id}` missing"))
}

#[tokio::test]
async fn a_frontonly_role_never_sees_another_cameras_activity() {
    let env = TestEnv::start("frontonly").await;
    env.seed_database();

    let response = env.activity("frontonly", RANGE).await;
    assert_eq!(review_cameras(&response), ["front"]);
    assert_eq!(motion_cameras(&response), ["front"]);

    // Asking for a camera the role may not see answers empty, and the
    // request's list never adds a camera.
    let refused = env
        .activity("frontonly", &format!("{RANGE}&cameras=back"))
        .await;
    assert!(refused.reviews.is_empty());
    assert!(refused.motion.is_empty());

    let partly = env
        .activity("frontonly", &format!("{RANGE}&cameras=front,back"))
        .await;
    assert_eq!(review_cameras(&partly), ["front"]);
    assert_eq!(motion_cameras(&partly), ["front"]);
    env.finish();
}

#[tokio::test]
async fn admin_sees_every_enabled_camera_and_never_the_disabled_one() {
    let env = TestEnv::start("admin").await;
    env.seed_database();

    let response = env.activity("admin", RANGE).await;
    assert_eq!(review_cameras(&response), ["front", "back", "side"]);
    assert_eq!(motion_cameras(&response), ["back", "front", "side"]);
    env.finish();
}

#[tokio::test]
async fn a_thumbnail_url_appears_only_when_the_file_is_present() {
    let env = TestEnv::start("thumbnail").await;
    env.seed_database();

    // Nothing under the media root yet, so the well-formed path links to nothing.
    let response = env
        .activity("frontonly", &format!("{RANGE}&include=reviews"))
        .await;
    assert_eq!(review(&response, "r-front").media.thumbnail, None);

    env.write_media("clips/review/thumb-front.webp");
    let response = env
        .activity("frontonly", &format!("{RANGE}&include=reviews"))
        .await;
    assert_eq!(
        review(&response, "r-front").media.thumbnail,
        Some(MediaLink {
            url: "/clips/review/thumb-front.webp".to_owned()
        })
    );
    env.finish();
}

#[tokio::test]
async fn a_stored_path_outside_the_media_prefix_is_not_linked() {
    let env = TestEnv::start("thumb-outside").await;
    env.seed_database();

    let response = env
        .activity("admin", &format!("{RANGE}&include=reviews"))
        .await;
    assert_eq!(review(&response, "r-back").media.thumbnail, None);
    env.finish();
}

#[tokio::test]
async fn a_thumbnail_path_escaping_the_media_root_is_not_linked() {
    let env = TestEnv::start("thumb-escape").await;
    env.seed_database();
    // The file exists where `media_root/../secret.webp` resolves, so only
    // the component rule can refuse the link.
    env.write_outside_media("secret.webp");

    let response = env
        .activity("admin", &format!("{RANGE}&include=reviews"))
        .await;
    assert_eq!(review(&response, "r-side").media.thumbnail, None);
    env.finish();
}

#[tokio::test]
async fn a_clip_url_appears_only_for_an_ended_review_with_a_recording() {
    let env = TestEnv::start("clip").await;
    env.seed_database();

    let response = env
        .activity("admin", &format!("{RANGE}&include=reviews"))
        .await;
    assert_eq!(
        review(&response, "r-front").media.clip,
        Some(MediaLink {
            url: "/api/front/start/1500/end/1600/clip.mp4".to_owned()
        })
    );
    // Ended, but no recording of `back` overlaps it.
    assert_eq!(review(&response, "r-back").media.clip, None);
    // Ongoing reviews have no clip even with recordings present.
    assert_eq!(review(&response, "r-side").media.clip, None);
    env.finish();
}

#[tokio::test]
async fn the_response_echoes_the_range_and_include_selects_the_kinds() {
    let env = TestEnv::start("echo").await;
    env.seed_database();

    let response = env.activity("admin", RANGE).await;
    let body: serde_json::Value =
        serde_json::to_value(&response).expect("response serializes back");
    assert_eq!(body["after"], serde_json::json!(1_000.0));
    assert_eq!(body["before"], serde_json::json!(2_000.0));
    assert_eq!(response.motion_bucket_seconds, 30);
    assert!(!response.reviews.is_empty());
    assert!(!response.motion.is_empty());

    let reviews_only = env
        .activity("admin", &format!("{RANGE}&include=reviews"))
        .await;
    assert_eq!(reviews_only.motion, []);
    assert!(!reviews_only.reviews.is_empty());
    env.finish();
}

#[tokio::test]
async fn a_malformed_query_is_a_bad_request() {
    let env = TestEnv::start("bad-query").await;
    env.seed_database();

    let raw = env.request_raw("admin", "after=x&before=2000").await;
    assert_error_shape(&raw, "400", "bad_request");
    env.finish();
}

#[tokio::test]
async fn a_refusing_config_server_is_config_unavailable() {
    let env = TestEnv::refusing("config-down").await;
    env.seed_database();

    let raw = env.request_raw("admin", RANGE).await;
    assert_error_shape(&raw, "503", "config_unavailable");
    env.finish();
}

#[tokio::test]
async fn a_missing_database_file_is_database_unavailable() {
    let env = TestEnv::start("db-missing").await;
    // No `seed_database`: the file never exists, and cameras are allowed.

    let raw = env.request_raw("admin", RANGE).await;
    let (status, headers, body) = http_parts(&raw);
    assert_eq!(
        status.split_whitespace().nth(1).unwrap_or_default(),
        "503",
        "status line {status:?}"
    );
    assert_framing(&headers, &body);
    let error: ErrorBody = serde_json::from_slice(&body).expect("body parses as ErrorBody");
    assert_eq!(error.code, "database_unavailable");
    assert_eq!(error.message, "Frigate's database could not be read.");
    env.finish();
}

#[tokio::test]
async fn an_empty_allowed_set_answers_without_touching_the_database() {
    let env = TestEnv::start("empty-set").await;
    // No `seed_database`: any database access would be a `503`.

    // `ghostonly` lists only a camera that does not exist.
    let response = env.activity("ghostonly", RANGE).await;
    assert!(response.reviews.is_empty());
    assert!(response.motion.is_empty());

    // An admin asking for a nameless camera also gets an empty answer.
    let response = env
        .activity("admin", &format!("{RANGE}&cameras=nope"))
        .await;
    assert!(response.reviews.is_empty());
    assert!(response.motion.is_empty());
    env.finish();
}

#[tokio::test]
async fn a_malformed_query_is_reported_before_the_config_fetch() {
    let env = TestEnv::refusing("bad-query-first").await;

    // The config server would answer `503`; the `400` proves the query was
    // parsed first.
    let raw = env.request_raw("admin", "after=x&before=2000").await;
    assert_error_shape(&raw, "400", "bad_request");
    env.finish();
}
