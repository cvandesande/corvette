//! Tests the two detection routes against a fixture config backing and a fake
//! detection service answering canned bytes on a Unix socket.

use corvette_api_server::config::ConfigSource;
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio::task::JoinHandle;

const CACHE_TTL: Duration = Duration::from_mins(5);
const STATUS_PATH: &str = "/corvette/api/v1/detection/status";
const MARKER: &str = "MARKER-LEAK-7";
const ADMIN: [&str; 2] = ["Remote-User: alice", "Remote-Role: admin"];
const FRONT_ONLY: [&str; 2] = ["Remote-User: carol", "Remote-Role: frontonly"];
const THREE_ROLE: [&str; 2] = ["Remote-User: bob", "Remote-Role: threerole"];

enum ServiceAnswer {
    Bytes(Vec<u8>),
    Silent,
    Stall,
}

fn fixture_config() -> String {
    let camera = |width: u32, order: i32, enabled: bool| {
        json!({ "enabled": enabled, "friendly_name": null, "ui": { "order": order },
                "detect": { "width": width, "height": 576 } })
    };
    json!({
        "auth": { "roles": { "admin": [], "frontonly": ["front"],
                             "threerole": ["front", "back", "back yard", "front+porch"] } },
        "cameras": {
            "front": camera(704, 0, true),
            "back": camera(704, 1, true),
            "back yard": camera(640, 2, true),
            "front+porch": camera(640, 3, true),
            "living": camera(640, 4, false)
        }
    })
    .to_string()
}

fn http_answer(status: &str, body: &[u8]) -> Vec<u8> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    [head.into_bytes(), body.to_vec()].concat()
}

async fn read_request_block<S>(stream: &mut S) -> Vec<u8>
where
    S: AsyncRead + std::marker::Unpin + std::marker::Send,
{
    let mut block = Vec::new();
    let mut chunk = [0_u8; 512];
    while !block.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => block.extend_from_slice(&chunk[..read]),
        }
    }
    block
}

fn first_line(block: &[u8]) -> String {
    String::from_utf8_lossy(block)
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_owned()
}

struct Service {
    socket: PathBuf,
    answer: Arc<tokio::sync::Mutex<ServiceAnswer>>,
    requests: Arc<Mutex<Vec<String>>>,
    handle: JoinHandle<()>,
}

impl Service {
    fn start(dir: &Path) -> Self {
        let socket = dir.join("detection.sock");
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind fake detection service");
        let answer = Arc::new(tokio::sync::Mutex::new(ServiceAnswer::Bytes(Vec::new())));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (answer_loop, requests_loop) = (Arc::clone(&answer), Arc::clone(&requests));
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _peer)) = listener.accept().await else {
                    break;
                };
                let answer = Arc::clone(&answer_loop);
                let requests = Arc::clone(&requests_loop);
                tokio::spawn(async move {
                    let block = read_request_block(&mut stream).await;
                    requests.lock().expect("requests").push(first_line(&block));
                    match &*answer.lock().await {
                        ServiceAnswer::Bytes(bytes) => {
                            let _ = stream.write_all(bytes).await;
                        }
                        ServiceAnswer::Silent => {}
                        ServiceAnswer::Stall => tokio::time::sleep(Duration::from_secs(10)).await,
                    }
                });
            }
        });
        Self {
            socket,
            answer,
            requests,
            handle,
        }
    }

    async fn set(&self, answer: ServiceAnswer) {
        *self.answer.lock().await = answer;
    }

    fn request_lines(&self) -> Vec<String> {
        self.requests.lock().expect("requests").clone()
    }
}

struct TestEnv {
    socket: PathBuf,
    dir: PathBuf,
    server: JoinHandle<()>,
    service: Option<Service>,
}

impl TestEnv {
    /// `with_service` binds the fake service; `detection_socket` overrides
    /// the path given to the server, and an absent path tests connect failure.
    /// `dead_config` points the config source at an address nothing listens on.
    async fn build(
        tag: &str,
        with_service: bool,
        detection_socket: Option<PathBuf>,
        timeouts: Option<(Duration, Duration)>,
        dead_config: bool,
    ) -> Self {
        let dir = unique_dir(tag);
        std::fs::create_dir_all(&dir).expect("create test directory");
        let service = with_service.then(|| Service::start(&dir));
        let socket = detection_socket
            .clone()
            .or_else(|| service.as_ref().map(|s| s.socket.clone()));
        let backing = if dead_config {
            dead_config_addr().await
        } else {
            start_backing(http_answer("200 OK", fixture_config().as_bytes())).await
        };
        let source = Arc::new(ConfigSource::new(backing, CACHE_TTL));
        let mut server = ApiServer::bind(
            &dir.join("api.sock"),
            CONNECTION_TIMEOUT,
            source,
            &dir.join("frigate.db"),
        )
        .expect("bind");
        if let Some(path) = socket {
            server = server.with_detection_socket(path);
        }
        if let Some((connect, total)) = timeouts {
            server = server.with_detection_timeouts(connect, total);
        }
        let api_socket = server.socket_path().to_path_buf();
        let handle = tokio::spawn(server.serve_until(std::future::pending::<()>()));
        Self {
            socket: api_socket,
            dir,
            server: handle,
            service,
        }
    }

    const fn service(&self) -> &Service {
        self.service.as_ref().expect("test has a fake service")
    }

    fn finish(self) {
        self.server.abort();
        if let Some(service) = self.service {
            service.handle.abort();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "corvette-detection-test-{}-{tag}-{id}",
        std::process::id()
    ))
}

async fn start_backing(config: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("backing");
    let addr = listener.local_addr().expect("backing address").to_string();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _peer)) = listener.accept().await else {
                break;
            };
            let config = config.clone();
            tokio::spawn(async move {
                let block = read_request_block(&mut stream).await;
                if first_line(&block).starts_with("GET /api/config") {
                    let _ = stream.write_all(&config).await;
                }
            });
        }
    });
    addr
}

/// An address that nothing listens on, so every config fetch fails.
async fn dead_config_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("dead config address");
    let addr = listener.local_addr().expect("dead config addr").to_string();
    drop(listener);
    addr
}

fn get(path: &str, headers: &[&str]) -> Vec<u8> {
    let mut request = format!("GET {path} HTTP/1.0\r\n{}", headers.join("\r\n"));
    request.push_str("\r\n\r\n");
    request.into_bytes()
}

fn post(path: &str) -> Vec<u8> {
    format!("POST {path} HTTP/1.0\r\nRemote-User: alice\r\nRemote-Role: admin\r\n\r\n").into_bytes()
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

/// Status code and body of a response.
fn http_split(raw: &[u8]) -> (String, Vec<u8>) {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("no separator");
    let head = String::from_utf8_lossy(&raw[..sep]);
    let code = head.split(' ').nth(1).unwrap_or_default().to_owned();
    (code, raw[sep + 4..].to_vec())
}

async fn fetch(env: &TestEnv, path: &str, headers: &[&str]) -> (String, Vec<u8>) {
    http_split(&exchange(&env.socket, &get(path, headers)).await)
}

fn objects_path(camera: &str) -> String {
    format!("/corvette/api/v1/cameras/{camera}/objects")
}

fn assert_unavailable(body: &[u8]) {
    assert_eq!(
        body,
        br#"{"code":"detection_unavailable","message":"The detection service could not be reached."}"#.as_slice()
    );
}

#[tokio::test]
async fn allowed_camera_gets_the_service_object_list_verbatim() {
    let env = TestEnv::build("objects-passthrough", true, None, None, false).await;
    let listed =
        json!({ "objects": [{ "id": "7", "label": "person", "box": [0.1, 0.2, 0.3, 0.4] }] });
    env.service()
        .set(ServiceAnswer::Bytes(http_answer(
            "200 OK",
            listed.to_string().as_bytes(),
        )))
        .await;
    let (code, body) = fetch(&env, &objects_path("front"), &ADMIN).await;
    assert_eq!(code, "200");
    assert_eq!(body, listed.to_string().into_bytes());
    assert_eq!(
        env.service().request_lines(),
        vec!["GET /corvette/api/v1/cameras/front/objects HTTP/1.1".to_owned()]
    );
    env.finish();
}

#[tokio::test]
async fn unusable_service_answers_are_all_unavailable() {
    let env = TestEnv::build("answer-shape", true, None, None, false).await;
    let padding = "A".repeat(1024 * 1024);
    let oversized = format!(r#"{{"objects":["{MARKER}{padding}"]}}"#);
    let unknown_body = format!(r#"{{"reason":"objects_unknown {MARKER}"}}"#);
    let cases = vec![
        ServiceAnswer::Bytes(http_answer(
            "503 Service Unavailable",
            unknown_body.as_bytes(),
        )),
        ServiceAnswer::Bytes(http_answer(
            "200 OK",
            format!("not json {MARKER}").as_bytes(),
        )),
        ServiceAnswer::Bytes(http_answer("200 OK", br#"{"no_objects":[]}"#)),
        ServiceAnswer::Bytes(http_answer("200 OK", oversized.as_bytes())),
        ServiceAnswer::Silent,
    ];
    for answer in cases {
        env.service().set(answer).await;
        let (code, body) = fetch(&env, &objects_path("front"), &ADMIN).await;
        assert_eq!(code, "503");
        assert_unavailable(&body);
    }
    env.finish();
}

#[tokio::test]
async fn no_socket_refused_close_and_stall_are_all_unavailable() {
    let absent = unique_dir("absent").join("absent.sock");
    for env in [
        TestEnv::build("socket-unset", false, None, None, false).await,
        TestEnv::build("socket-refused", false, Some(absent), None, false).await,
        TestEnv::build(
            "stall",
            true,
            None,
            Some((Duration::from_millis(200), Duration::from_millis(300))),
            false,
        )
        .await,
    ] {
        if let Some(service) = env.service.as_ref() {
            service.set(ServiceAnswer::Stall).await;
        }
        for path in [objects_path("front"), STATUS_PATH.to_owned()] {
            let (code, body) = fetch(&env, &path, &ADMIN).await;
            assert_eq!(code, "503");
            assert_unavailable(&body);
        }
        env.finish();
    }
}

#[tokio::test]
async fn service_404_answers_detection_off_and_hidden_cameras_answer_from_config_only() {
    let env = TestEnv::build("detection-off", true, None, None, false).await;
    env.service()
        .set(ServiceAnswer::Bytes(http_answer(
            "404 Not Found",
            br#"{"reason":"camera_unregistered"}"#,
        )))
        .await;
    for camera in ["back", "back%20yard"] {
        let (code, body) = fetch(&env, &objects_path(camera), &ADMIN).await;
        assert_eq!(code, "200", "camera {camera:?}");
        assert_eq!(body, br#"{"objects":[]}"#.as_slice());
    }
    let (code, hidden) = fetch(&env, &objects_path("back"), &FRONT_ONLY).await;
    let (_, unknown) = fetch(&env, &objects_path("nope"), &FRONT_ONLY).await;
    assert_eq!(code, "404");
    assert_eq!(
        hidden,
        br#"{"code":"not_found","message":"No such camera."}"#.as_slice()
    );
    assert_eq!(
        hidden, unknown,
        "hidden and unknown cameras are byte-identical"
    );
    assert_eq!(
        env.service().request_lines(),
        vec![
            "GET /corvette/api/v1/cameras/back/objects HTTP/1.1".to_owned(),
            "GET /corvette/api/v1/cameras/back%20yard/objects HTTP/1.1".to_owned(),
        ],
        "allowed cameras always reach the service; hidden and unknown cameras reach none"
    );
    env.finish();
}

#[tokio::test]
async fn hidden_and_unknown_cameras_are_identical_and_reach_no_service() {
    let env = TestEnv::build("hidden-camera", true, None, None, false).await;
    let (code, hidden) = fetch(&env, &objects_path("back"), &FRONT_ONLY).await;
    let (_, unknown) = fetch(&env, &objects_path("nope"), &FRONT_ONLY).await;
    assert_eq!(code, "404");
    assert_eq!(
        hidden,
        br#"{"code":"not_found","message":"No such camera."}"#.as_slice()
    );
    assert_eq!(
        hidden, unknown,
        "hidden and unknown cameras are byte-identical"
    );
    assert!(env.service().request_lines().is_empty());
    env.finish();
}

#[tokio::test]
async fn camera_names_reach_the_service_percent_encoded_and_bodies_come_back_verbatim() {
    let env = TestEnv::build("wire-encoding", true, None, None, false).await;
    let listed = json!({ "objects": [{ "id": "9", "label": "person" }] });
    env.service()
        .set(ServiceAnswer::Bytes(http_answer(
            "200 OK",
            listed.to_string().as_bytes(),
        )))
        .await;
    for public in ["back%20yard", "front%2Bporch"] {
        let (code, body) = fetch(&env, &objects_path(public), &ADMIN).await;
        assert_eq!(code, "200", "for {public}");
        assert_eq!(body, listed.to_string().into_bytes(), "for {public}");
    }
    assert_eq!(
        env.service().request_lines(),
        vec![
            "GET /corvette/api/v1/cameras/back%20yard/objects HTTP/1.1".to_owned(),
            "GET /corvette/api/v1/cameras/front%2Bporch/objects HTTP/1.1".to_owned(),
        ],
        "every byte outside the unreserved set arrives escaped in upper case"
    );
    env.finish();
}

#[tokio::test]
async fn both_routes_answer_config_unavailable_when_the_config_backing_is_dead() {
    let env = TestEnv::build("config-dead", false, None, None, true).await;
    for path in [objects_path("front"), STATUS_PATH.to_owned()] {
        let (code, body) = fetch(&env, &path, &ADMIN).await;
        assert_eq!(code, "503", "for {path}");
        let parsed: Value = serde_json::from_slice(&body).expect("error body");
        assert_eq!(parsed["code"], "config_unavailable", "for {path}");
    }
    env.finish();
}

#[tokio::test]
async fn status_keeps_config_order_drops_hidden_and_adds_off_cameras() {
    let env = TestEnv::build("status-order", true, None, None, false).await;
    let front = json!({ "camera": "front", "state": "running", "fps_in": 5.0 });
    let back = json!({ "camera": "back", "state": "waiting_for_frames" });
    let living = json!({ "camera": "living", "state": "running" });
    let listed = json!({ "cameras": [back.clone(), living, front.clone()] });
    env.service()
        .set(ServiceAnswer::Bytes(http_answer(
            "200 OK",
            listed.to_string().as_bytes(),
        )))
        .await;
    let (code, body) = fetch(&env, STATUS_PATH, &THREE_ROLE).await;
    assert_eq!(code, "200");
    let answer: Value = serde_json::from_slice(&body).expect("status answer is JSON");
    assert_eq!(
        answer,
        json!({ "cameras": [front, back, { "camera": "back yard", "state": "off" },
                            { "camera": "front+porch", "state": "off" }] })
    );
    for shape in [
        br#"{"cameras":"not an array"}"#.as_slice(),
        br#"{"cameras":[{"state":"running"}]}"#.as_slice(),
        br#"{"cameras":[7]}"#.as_slice(),
    ] {
        env.service()
            .set(ServiceAnswer::Bytes(http_answer("200 OK", shape)))
            .await;
        let (code, body) = fetch(&env, STATUS_PATH, &THREE_ROLE).await;
        assert_eq!(code, "503");
        assert_unavailable(&body);
    }
    env.service()
        .set(ServiceAnswer::Bytes(http_answer(
            "500 Internal Server Error",
            br#"{"cameras":[]}"#,
        )))
        .await;
    let (code, body) = fetch(&env, STATUS_PATH, &THREE_ROLE).await;
    assert_eq!(code, "503");
    assert_unavailable(&body);
    env.finish();
}

#[tokio::test]
async fn detection_routes_gate_identity_and_method() {
    let env = TestEnv::build("gate", true, None, None, false).await;
    for path in [objects_path("front"), STATUS_PATH.to_owned()] {
        let (code, body) = fetch(&env, &path, &[]).await;
        assert_eq!(code, "401");
        let parsed: Value = serde_json::from_slice(&body).expect("error body");
        assert_eq!(parsed["code"], "unauthorized");
    }
    let (_, expected) = http_split(&exchange(&env.socket, &post("/corvette/api/v1/health")).await);
    for path in [objects_path("front"), STATUS_PATH.to_owned()] {
        let (code, body) = http_split(&exchange(&env.socket, &post(&path)).await);
        assert_eq!(code, "400");
        assert_eq!(body, expected, "same answer the existing routes give");
    }
    env.finish();
}

#[tokio::test]
async fn failure_bodies_leak_no_socket_path_or_marker() {
    let env = TestEnv::build("leak", true, None, None, false).await;
    let service_socket = env.service().socket.display().to_string();
    env.service()
        .set(ServiceAnswer::Bytes(http_answer(
            "503 Service Unavailable",
            format!(r#"{{"reason":"objects_unknown {MARKER} service={service_socket} "}}"#)
                .as_bytes(),
        )))
        .await;
    let (_code, body) = fetch(&env, &objects_path("front"), &ADMIN).await;
    let text = String::from_utf8(body).expect("body is UTF-8");
    assert!(!text.contains(MARKER), "marker leaked into the answer");
    assert!(
        !text.contains(&env.dir.display().to_string()),
        "socket path leaked"
    );
    env.finish();
}
