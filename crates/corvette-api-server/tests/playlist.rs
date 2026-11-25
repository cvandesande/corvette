//! Exercises the playlist route directory against a fixture index and hand-built piece files.

use corvette_api::ErrorBody;
use corvette_api_server::config::{ConfigSource, DEFAULT_CACHE_TTL};
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT};
use rusqlite::{Connection, params};
use serde_json::json;
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixStream};
use tokio::task::JoinHandle;

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
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

const TICKS_4S: u32 = 360_000;
const FOUR_SECONDS: f64 = 4.0;

struct TestEnv {
    socket: PathBuf,
    dir: PathBuf,
    root: PathBuf,
    index_path: PathBuf,
    handle: JoinHandle<()>,
}

impl TestEnv {
    async fn start(tag: &str) -> Self {
        let config_addr = start_config_backing().await;
        let dir = unique_dir(tag);
        let root = dir.join("storage");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&root).expect("create storage root");
        let socket = dir.join("api.sock");
        let index_path = root.join("index.sqlite");
        let config = Arc::new(ConfigSource::new(config_addr, DEFAULT_CACHE_TTL));
        let server = ApiServer::bind(&socket, CONNECTION_TIMEOUT, config, &dir.join("absent.db"))
            .expect("bind")
            .with_index_path(&index_path);
        let socket = server.socket_path().to_path_buf();
        let handle = tokio::spawn(server.serve_until(std::future::pending::<()>()));
        Self {
            socket,
            dir,
            root,
            index_path,
            handle,
        }
    }

    fn finish(self) {
        self.handle.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    fn open_index(&self) -> Connection {
        let index = Connection::open(&self.index_path).expect("create index");
        index
            .execute_batch(INDEX_SCHEMA)
            .expect("create index schema");
        index
    }

    fn insert_row(
        &self,
        camera: &str,
        seq: i64,
        start: i64,
        end: i64,
        path: &str,
        size_bytes: i64,
    ) {
        let index = Connection::open(&self.index_path).expect("open index");
        index
            .execute_batch(INDEX_SCHEMA)
            .expect("create index schema");
        index
            .execute(
                "INSERT INTO pieces \
                 (camera, seq, start_ts, end_ts, duration_s, path, size_bytes, codec) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'h264')",
                params![camera, seq, start, end, FOUR_SECONDS, path, size_bytes],
            )
            .expect("insert piece");
    }

    /// Writes the piece file under the storage root and indexes the row; returns the stored path.
    fn seed_piece(
        &self,
        camera: &str,
        seq: i64,
        start_ms: i64,
        end_ms: i64,
        bytes: &[u8],
    ) -> PathBuf {
        let relative = piece_relative(camera, seq, start_ms);
        let path = self.root.join(&relative);
        std::fs::create_dir_all(path.parent().expect("piece directory")).expect("piece dirs");
        std::fs::write(&path, bytes).expect("write piece");
        let size_bytes = i64::try_from(bytes.len()).expect("fixture size fits i64");
        self.insert_row(camera, seq, start_ms, end_ms, &relative, size_bytes);
        path
    }

    /// A row whose file is deliberately absent; the stored path is valid-looking.
    fn seed_missing_piece(&self, camera: &str, seq: i64, start_ms: i64, end_ms: i64) {
        let relative = piece_relative(camera, seq, start_ms);
        self.insert_row(camera, seq, start_ms, end_ms, &relative, 1);
    }

    /// Inserts a row with an arbitrary stored path and no file of its own.
    fn seed_bad_path(&self, camera: &str, seq: i64, start_ms: i64, end_ms: i64, path: &str) {
        self.insert_row(camera, seq, start_ms, end_ms, path, 1);
    }

    async fn media(&self, role: &str, camera: &str, start: &str, end: &str, file: &str) -> Vec<u8> {
        self.exchange(media_request(role, camera, start, end, file, &[]))
            .await
    }

    async fn media_range(
        &self,
        role: &str,
        camera: &str,
        start: &str,
        end: &str,
        file: &str,
        range: &str,
    ) -> Vec<u8> {
        let header = format!("Range: {range}");
        self.exchange(media_request(
            role,
            camera,
            start,
            end,
            file,
            &[header.as_str()],
        ))
        .await
    }

    async fn exchange(&self, request: Vec<u8>) -> Vec<u8> {
        exchange(&self.socket, &request).await
    }
}

fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "corvette-playlist-test-{}-{tag}-{id}",
        std::process::id()
    ))
}

fn piece_relative(camera: &str, seq: i64, start_ms: i64) -> String {
    format!("recordings/{camera}/1970-01-01/00/{start_ms}-{seq}.mp4")
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

fn media_request(
    role: &str,
    camera: &str,
    start: &str,
    end: &str,
    file: &str,
    extra: &[&str],
) -> Vec<u8> {
    let path = format!("/corvette/api/v1/media/{camera}/start/{start}/end/{end}/{file}");
    let role_header = format!("Remote-Role: {role}");
    let mut headers = vec!["Remote-User: alice", role_header.as_str()];
    headers.extend(extra);
    get(&path, &headers)
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

fn assert_error(raw: &[u8], status_prefix: &str, code: &str) {
    let (status, headers, body) = http_parts(raw);
    assert_eq!(status_code(&status), status_prefix, "status {status:?}");
    assert_eq!(
        headers_content_type(&headers),
        "application/json",
        "error body is JSON"
    );
    let error: ErrorBody = serde_json::from_slice(&body).expect("error body parses");
    assert_eq!(error.code, code);
}

fn headers_content_type(headers: &HashMap<String, String>) -> String {
    headers.get("content-type").cloned().unwrap_or_default()
}

fn assert_text_headers(headers: &HashMap<String, String>, content_type: &str) {
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some(content_type)
    );
    assert_eq!(
        headers.get("cache-control").map(String::as_str),
        Some("no-store"),
        "every playlist-route answer carries no-store"
    );
}

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

/// Builds stored-piece bytes by hand, as the tests for the rebase library do.
struct InitSpec {
    creation_time: u32,
    mdhd_version: u8,
    timescale: u32,
    fourcc: [u8; 4],
    width: u16,
    height: u16,
    config_kind: [u8; 4],
    config_body: Vec<u8>,
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

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

fn ftyp_box() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"iso5");
    put_u32(&mut body, 0);
    body.extend_from_slice(b"iso5iso6isommp41");
    mp4_box(*b"ftyp", &body)
}

fn default_init_spec() -> InitSpec {
    InitSpec {
        creation_time: 0,
        mdhd_version: 0,
        timescale: 90_000,
        fourcc: *b"avc1",
        width: 1920,
        height: 1080,
        config_kind: *b"avcC",
        config_body: vec![0x01, 0x42, 0xC0, 0x1F, 0xE1, 0xFF],
    }
}

fn init_with_width(width: u16) -> InitSpec {
    InitSpec {
        width,
        ..default_init_spec()
    }
}

fn matrix_and_tail(out: &mut Vec<u8>) {
    for value in [0x0001_0000i32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        put_u32(out, value.cast_unsigned());
    }
}

fn mvhd_body(creation_time: u32) -> Vec<u8> {
    let mut body = Vec::new();
    put_u32(&mut body, 0);
    put_u32(&mut body, creation_time);
    put_u32(&mut body, creation_time);
    put_u32(&mut body, 90_000);
    put_u32(&mut body, 0);
    put_u32(&mut body, 0x0001_0000);
    put_u16(&mut body, 0x0100);
    put_u16(&mut body, 0);
    put_u64(&mut body, 0);
    matrix_and_tail(&mut body);
    for _ in 0..6 {
        put_u32(&mut body, 0);
    }
    put_u32(&mut body, 2);
    body
}

fn tkhd_body(creation_time: u32, width: u16, height: u16) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0);
    body.extend_from_slice(&[0, 0, 0x07]);
    put_u32(&mut body, creation_time);
    put_u32(&mut body, creation_time);
    put_u32(&mut body, 1);
    put_u32(&mut body, 0);
    put_u32(&mut body, 0);
    put_u64(&mut body, 0);
    put_u16(&mut body, 0);
    put_u16(&mut body, 0);
    put_u16(&mut body, 0);
    put_u16(&mut body, 0);
    matrix_and_tail(&mut body);
    put_u32(&mut body, u32::from(width) << 16);
    put_u32(&mut body, u32::from(height) << 16);
    body
}

fn mdhd_body(version: u8, timescale: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(version);
    body.extend_from_slice(&[0, 0, 0]);
    if version == 1 {
        put_u64(&mut body, 0);
        put_u64(&mut body, 0);
        put_u32(&mut body, timescale);
        put_u64(&mut body, 0);
    } else {
        put_u32(&mut body, 0);
        put_u32(&mut body, 0);
        put_u32(&mut body, timescale);
        put_u32(&mut body, 0);
    }
    put_u16(&mut body, 0x55C4);
    put_u16(&mut body, 0);
    body
}

fn hdlr_body() -> Vec<u8> {
    let mut body = Vec::new();
    put_u32(&mut body, 0);
    put_u32(&mut body, 0);
    body.extend_from_slice(b"vide");
    put_u64(&mut body, 0);
    put_u32(&mut body, 0);
    body.extend_from_slice(b"test\0");
    body
}

fn sample_entry_body(width: u16, height: u16) -> Vec<u8> {
    let mut body = vec![0u8; 6];
    put_u16(&mut body, 1);
    put_u16(&mut body, 0);
    put_u16(&mut body, 0);
    put_u32(&mut body, 0);
    put_u64(&mut body, 0);
    put_u16(&mut body, width);
    put_u16(&mut body, height);
    put_u32(&mut body, 0x0048_0000);
    put_u32(&mut body, 0x0048_0000);
    put_u32(&mut body, 0);
    put_u16(&mut body, 1);
    body.extend_from_slice(&[0u8; 32]);
    put_u16(&mut body, 0x0018);
    put_u16(&mut body, 0xFFFF);
    body
}

fn stsd_body(spec: &InitSpec) -> Vec<u8> {
    let mut entry_body = sample_entry_body(spec.width, spec.height);
    entry_body.extend_from_slice(&mp4_box(spec.config_kind, &spec.config_body));
    let mut body = Vec::new();
    put_u32(&mut body, 0);
    put_u32(&mut body, 1);
    body.extend_from_slice(&mp4_box(spec.fourcc, &entry_body));
    body
}

fn init_bytes(spec: &InitSpec) -> Vec<u8> {
    let stbl = mp4_box(*b"stbl", &mp4_box(*b"stsd", &stsd_body(spec)));
    let mut minf_body = mp4_box(*b"vmhd", &[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
    minf_body.extend_from_slice(&stbl);
    let minf = mp4_box(*b"minf", &minf_body);
    let mut mdia_body = mp4_box(*b"mdhd", &mdhd_body(spec.mdhd_version, spec.timescale));
    mdia_body.extend_from_slice(&mp4_box(*b"hdlr", &hdlr_body()));
    mdia_body.extend_from_slice(&minf);
    let mdia = mp4_box(*b"mdia", &mdia_body);
    let mut trak_body = mp4_box(
        *b"tkhd",
        &tkhd_body(spec.creation_time, spec.width, spec.height),
    );
    trak_body.extend_from_slice(&mdia);
    let trak = mp4_box(*b"trak", &trak_body);
    let mut moov_body = mp4_box(*b"mvhd", &mvhd_body(spec.creation_time));
    moov_body.extend_from_slice(&trak);
    let mut out = ftyp_box();
    out.extend_from_slice(&mp4_box(*b"moov", &moov_body));
    out
}

fn tfdt_box(version: u8, value: u64) -> Vec<u8> {
    let mut body = Vec::new();
    if version == 1 {
        put_u32(&mut body, 1 << 24);
        put_u64(&mut body, value);
    } else {
        put_u32(&mut body, 0);
        put_u32(
            &mut body,
            u32::try_from(value).expect("fixture v0 time fits u32"),
        );
    }
    mp4_box(*b"tfdt", &body)
}

fn tfhd_box(default_duration: Option<u32>) -> Vec<u8> {
    let mut body = Vec::new();
    let mut flags = 0x0002_0000u32;
    if default_duration.is_some() {
        flags |= 0x0000_0008;
    }
    put_u32(&mut body, flags);
    put_u32(&mut body, 1);
    if let Some(duration) = default_duration {
        put_u32(&mut body, duration);
    }
    mp4_box(*b"tfhd", &body)
}

fn trun_box(durations: &[u32]) -> Vec<u8> {
    let mut body = Vec::new();
    put_u32(&mut body, 0x0000_0101);
    put_u32(
        &mut body,
        u32::try_from(durations.len()).expect("fixture count fits u32"),
    );
    put_u32(&mut body, 0);
    for duration in durations {
        put_u32(&mut body, *duration);
    }
    mp4_box(*b"trun", &body)
}

fn mdat_box(payload_len: usize) -> Vec<u8> {
    let mut body = Vec::new();
    put_u32(
        &mut body,
        u32::try_from(payload_len).expect("fixture payload fits u32"),
    );
    body.extend_from_slice(&vec![0x5Au8; payload_len]);
    mp4_box(*b"mdat", &body)
}

fn fragment(durations: &[u32], tfdt_value: u64) -> Vec<u8> {
    let mut mfhd_body = Vec::new();
    put_u32(&mut mfhd_body, 0);
    put_u32(&mut mfhd_body, 1);
    let mut traf_body = tfhd_box(None);
    traf_body.extend_from_slice(&tfdt_box(1, tfdt_value));
    traf_body.extend_from_slice(&trun_box(durations));
    let mut moof_body = mp4_box(*b"mfhd", &mfhd_body);
    moof_body.extend_from_slice(&mp4_box(*b"traf", &traf_body));
    let mut out = mp4_box(*b"moof", &moof_body);
    out.extend_from_slice(&mdat_box(durations.len() * 8 + 4));
    out
}

/// A stored piece: its init followed by one rebased-to-zero fragment; returned with the
/// fragment byte range so segment answers can be compared against the stored bytes.
struct FixturePiece {
    bytes: Vec<u8>,
    fragments_start: usize,
    size_bytes: i64,
}

fn fixture_piece(spec: &InitSpec, durations: &[u32]) -> FixturePiece {
    let init = init_bytes(spec);
    let mut bytes = init.clone();
    bytes.extend_from_slice(&fragment(durations, 0));
    FixturePiece {
        fragments_start: init.len(),
        size_bytes: i64::try_from(bytes.len()).expect("fixture piece fits i64"),
        bytes,
    }
}

fn first_tfdt(body: &[u8]) -> u64 {
    let marker = body
        .windows(4)
        .position(|window| window == *b"tfdt")
        .expect("served body carries a tfdt");
    let at = marker + 8;
    u64::from_be_bytes(body[at..at + 8].try_into().expect("tfdt field"))
}

/// Three contiguous pieces of 4 seconds each at seconds 1000..1012 for `front`.
fn seed_three_contiguous(env: &TestEnv) -> [FixturePiece; 3] {
    let pieces = [
        fixture_piece(&default_init_spec(), &[TICKS_4S]),
        fixture_piece(&default_init_spec(), &[TICKS_4S]),
        fixture_piece(&default_init_spec(), &[TICKS_4S]),
    ];
    for (index, piece) in pieces.iter().enumerate() {
        let seq = i64::try_from(index).expect("fixture index fits i64");
        let start = 1_000_000 + i64::try_from(index).expect("fixture fits") * 4_000;
        env.seed_piece("front", seq, start, start + 4_000, &piece.bytes);
    }
    pieces
}

#[tokio::test]
async fn contiguous_pieces_answer_master_playlist_and_segments_on_one_timeline() {
    let env = TestEnv::start("contiguous").await;
    let pieces = seed_three_contiguous(&env);

    let total_bytes: u64 = pieces
        .iter()
        .map(|piece| u64::try_from(piece.size_bytes).expect("fixture size fits u64"))
        .sum();
    let bandwidth = (total_bytes * 8 * 1_000) / 12_000;
    let master = env
        .media("admin", "front", "1000", "1012", "master.m3u8")
        .await;
    let (status, headers, body) = http_parts(&master);
    assert_eq!(status_code(&status), "200");
    assert_text_headers(&headers, "application/vnd.apple.mpegurl");
    assert_eq!(
        String::from_utf8(body).expect("master text"),
        format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH={bandwidth}\nindex.m3u8\n")
    );

    let index = env
        .media("admin", "front", "1000", "1012", "index.m3u8")
        .await;
    let (status, headers, body) = http_parts(&index);
    assert_eq!(status_code(&status), "200");
    assert_text_headers(&headers, "application/vnd.apple.mpegurl");
    let expected = concat!(
        "#EXTM3U\n",
        "#EXT-X-VERSION:7\n",
        "#EXT-X-PLAYLIST-TYPE:VOD\n",
        "#EXT-X-INDEPENDENT-SEGMENTS\n",
        "#EXT-X-TARGETDURATION:4\n",
        "#EXT-X-MAP:URI=\"init-0.mp4\"\n",
        "#EXT-X-PROGRAM-DATE-TIME:1970-01-01T00:16:40.000Z\n",
        "#EXTINF:4.000,\n",
        "seg-0.m4s\n",
        "#EXTINF:4.000,\n",
        "seg-1.m4s\n",
        "#EXTINF:4.000,\n",
        "seg-2.m4s\n",
        "#EXT-X-ENDLIST\n",
    );
    assert_eq!(String::from_utf8(body).expect("playlist text"), expected);

    let init = env
        .media("admin", "front", "1000", "1012", "init-0.mp4")
        .await;
    let (status, headers, body) = http_parts(&init);
    assert_eq!(status_code(&status), "200");
    assert_text_headers(&headers, "video/mp4");
    assert_eq!(body, init_bytes(&default_init_spec()));

    let seg0 = env
        .media("admin", "front", "1000", "1012", "seg-0.m4s")
        .await;
    let (status, headers, body) = http_parts(&seg0);
    assert_eq!(status_code(&status), "200");
    assert_text_headers(&headers, "video/mp4");
    assert_eq!(
        headers.get("accept-ranges").map(String::as_str),
        Some("bytes")
    );
    assert_eq!(body, pieces[0].bytes[pieces[0].fragments_start..]);
    assert_eq!(first_tfdt(&body), 0);

    let seg1 = env
        .media("admin", "front", "1000", "1012", "seg-1.m4s")
        .await;
    let (_status, _headers, body) = http_parts(&seg1);
    assert_eq!(first_tfdt(&body), u64::from(TICKS_4S));

    let seg2 = env
        .media("admin", "front", "1000", "1012", "seg-2.m4s")
        .await;
    let (_status, _headers, body) = http_parts(&seg2);
    assert_eq!(first_tfdt(&body), u64::from(TICKS_4S) * 2);
    env.finish();
}

#[tokio::test]
async fn a_gap_answers_a_discontinuity_a_map_and_a_restarted_offset() {
    let env = TestEnv::start("gap").await;
    let piece = fixture_piece(&default_init_spec(), &[TICKS_4S]);
    env.seed_piece("front", 0, 1_000_000, 1_004_000, &piece.bytes);
    env.seed_piece("front", 1, 1_006_000, 1_010_000, &piece.bytes);

    let index = env
        .media("admin", "front", "1000", "1010", "index.m3u8")
        .await;
    let (_status, _headers, body) = http_parts(&index);
    let expected = concat!(
        "#EXTM3U\n",
        "#EXT-X-VERSION:7\n",
        "#EXT-X-PLAYLIST-TYPE:VOD\n",
        "#EXT-X-INDEPENDENT-SEGMENTS\n",
        "#EXT-X-TARGETDURATION:4\n",
        "#EXT-X-MAP:URI=\"init-0.mp4\"\n",
        "#EXT-X-PROGRAM-DATE-TIME:1970-01-01T00:16:40.000Z\n",
        "#EXTINF:4.000,\n",
        "seg-0.m4s\n",
        "#EXT-X-DISCONTINUITY\n",
        "#EXT-X-MAP:URI=\"init-0.mp4\"\n",
        "#EXT-X-PROGRAM-DATE-TIME:1970-01-01T00:16:46.000Z\n",
        "#EXTINF:4.000,\n",
        "seg-1.m4s\n",
        "#EXT-X-ENDLIST\n",
    );
    assert_eq!(String::from_utf8(body).expect("playlist text"), expected);

    let seg1 = env
        .media("admin", "front", "1000", "1010", "seg-1.m4s")
        .await;
    let (_status, _headers, body) = http_parts(&seg1);
    assert_eq!(first_tfdt(&body), 0, "offsets restart at the discontinuity");
    env.finish();
}

#[tokio::test]
async fn a_parameter_change_answers_a_second_run_with_its_own_init() {
    let env = TestEnv::start("parameter-change").await;
    let first = fixture_piece(&default_init_spec(), &[TICKS_4S]);
    let second = fixture_piece(&init_with_width(640), &[TICKS_4S]);
    env.seed_piece("front", 0, 1_000_000, 1_004_000, &first.bytes);
    env.seed_piece("front", 1, 1_004_000, 1_008_000, &second.bytes);

    let index = env
        .media("admin", "front", "1000", "1008", "index.m3u8")
        .await;
    let (_status, _headers, body) = http_parts(&index);
    let expected = concat!(
        "#EXTM3U\n",
        "#EXT-X-VERSION:7\n",
        "#EXT-X-PLAYLIST-TYPE:VOD\n",
        "#EXT-X-INDEPENDENT-SEGMENTS\n",
        "#EXT-X-TARGETDURATION:4\n",
        "#EXT-X-MAP:URI=\"init-0.mp4\"\n",
        "#EXT-X-PROGRAM-DATE-TIME:1970-01-01T00:16:40.000Z\n",
        "#EXTINF:4.000,\n",
        "seg-0.m4s\n",
        "#EXT-X-DISCONTINUITY\n",
        "#EXT-X-MAP:URI=\"init-1.mp4\"\n",
        "#EXT-X-PROGRAM-DATE-TIME:1970-01-01T00:16:44.000Z\n",
        "#EXTINF:4.000,\n",
        "seg-1.m4s\n",
        "#EXT-X-ENDLIST\n",
    );
    assert_eq!(String::from_utf8(body).expect("playlist text"), expected);

    let init0 = env
        .media("admin", "front", "1000", "1008", "init-0.mp4")
        .await;
    let (_status, _headers, body) = http_parts(&init0);
    assert_eq!(body, init_bytes(&default_init_spec()));

    let init1 = env
        .media("admin", "front", "1000", "1008", "init-1.mp4")
        .await;
    let (_status, _headers, body) = http_parts(&init1);
    assert_eq!(body, init_bytes(&init_with_width(640)));
    env.finish();
}

#[tokio::test]
async fn segment_ranges_answer_206_416_and_multi_range_whole() {
    let env = TestEnv::start("ranges").await;
    seed_three_contiguous(&env);
    let full = env
        .media("admin", "front", "1000", "1012", "seg-0.m4s")
        .await;
    let (_status, _headers, full_body) = http_parts(&full);
    let length = full_body.len();

    let case0 = env
        .media_range("admin", "front", "1000", "1012", "seg-0.m4s", "bytes=0-9")
        .await;
    let (status, headers, body) = http_parts(&case0);
    assert_eq!(status_code(&status), "206");
    assert_eq!(body, &full_body[..10]);
    assert_eq!(
        headers.get("content-range").map(String::as_str),
        Some(&format!("bytes 0-9/{length}")[..])
    );

    let suffix = env
        .media_range("admin", "front", "1000", "1012", "seg-0.m4s", "bytes=-5")
        .await;
    let (status, headers, body) = http_parts(&suffix);
    assert_eq!(status_code(&status), "206");
    assert_eq!(body, &full_body[length - 5..]);
    assert_eq!(
        headers.get("content-range").map(String::as_str),
        Some(&format!("bytes {}-{}/{}", length - 5, length - 1, length)[..])
    );

    let open = env
        .media_range("admin", "front", "1000", "1012", "seg-0.m4s", "bytes=5-")
        .await;
    let (status, headers, body) = http_parts(&open);
    assert_eq!(status_code(&status), "206");
    assert_eq!(body, &full_body[5..]);
    assert_eq!(
        headers.get("content-range").map(String::as_str),
        Some(&format!("bytes 5-{}/{}", length - 1, length)[..])
    );

    let past = env
        .media_range(
            "admin",
            "front",
            "1000",
            "1012",
            "seg-0.m4s",
            &format!("bytes={length}-{}", length + 5),
        )
        .await;
    let (status, headers, body) = http_parts(&past);
    assert_eq!(status_code(&status), "416");
    assert_eq!(
        headers.get("content-range").map(String::as_str),
        Some(&format!("bytes */{length}")[..])
    );
    let error: ErrorBody = serde_json::from_slice(&body).expect("416 body is JSON");
    assert_eq!(error.code, "range_unsatisfiable");

    let multi = env
        .media_range(
            "admin",
            "front",
            "1000",
            "1012",
            "seg-0.m4s",
            "bytes=0-1,4-5",
        )
        .await;
    let (status, _headers, body) = http_parts(&multi);
    assert_eq!(status_code(&status), "200");
    assert_eq!(body, full_body);
    env.finish();
}

#[tokio::test]
async fn missing_plans_bad_numbers_and_inverted_ranges_answer_not_found() {
    let env = TestEnv::start("not-found").await;
    seed_three_contiguous(&env);

    let empty_plan = env
        .media("admin", "front", "5000", "5004", "master.m3u8")
        .await;
    assert_error(&empty_plan, "404", "not_found");

    let inverted = env
        .media("admin", "front", "2000", "1000", "master.m3u8")
        .await;
    assert_error(&inverted, "404", "not_found");

    for file in [
        "seg-1x.m4s",
        "seg-+1.m4s",
        "seg-9999999999999999999999.m4s",
        "seg-3.m4s",
        "init-x.mp4",
        "init-+0.mp4",
        "init-9.mp4",
        "other.mp4",
    ] {
        let raw = env.media("admin", "front", "1000", "1012", file).await;
        assert_error(&raw, "404", "not_found");
    }
    env.finish();
}

#[tokio::test]
async fn an_absent_index_answers_recording_unavailable() {
    let env = TestEnv::start("absent-index").await;
    let raw = env
        .media("admin", "front", "1000", "1012", "master.m3u8")
        .await;
    assert_error(&raw, "503", "recording_unavailable");
    assert!(
        !env.index_path.exists(),
        "the route must not create the index"
    );
    env.finish();
}

#[tokio::test]
async fn a_piece_row_without_a_file_answers_recording_unavailable_as_json() {
    let env = TestEnv::start("missing-piece").await;
    env.open_index();
    env.seed_missing_piece("front", 0, 1_000_000, 1_004_000);
    let raw = env
        .media("admin", "front", "1000", "1012", "index.m3u8")
        .await;
    assert_error(&raw, "503", "recording_unavailable");
    env.finish();
}

#[tokio::test]
async fn piece_rows_with_bad_stored_paths_answer_503_and_leak_no_file_content() {
    let env = TestEnv::start("bad-paths").await;
    env.open_index();
    let decoy = fixture_piece(&default_init_spec(), &[TICKS_4S]);
    for decoy_path in [
        "index.sqlite",
        "recordings/front/a/b/c.mp4/x",
        "recordings/othercam/1970-01-01/00/1-9.mp4",
    ] {
        let path = env.dir.join(decoy_path);
        std::fs::create_dir_all(path.parent().expect("decoy dirs")).expect("decoy dirs");
        std::fs::write(&path, &decoy.bytes).expect("write decoy piece");
    }

    let bad_paths = [
        "../index.sqlite",
        "/etc/passwd",
        "recordings/front/..",
        "recordings/front/a/b/c.mp4/x",
        "recordings/othercam/1970-01-01/00/1-9.mp4",
    ];
    for (index, path) in bad_paths.iter().enumerate() {
        let seq = i64::try_from(index).expect("case count fits i64");
        env.seed_bad_path("front", seq, 1_000_000, 1_004_000, path);
        let raw = env
            .media("admin", "front", "1000", "1012", "master.m3u8")
            .await;
        assert_error(&raw, "503", "recording_unavailable");
    }
    env.finish();
}

#[tokio::test]
async fn a_camera_outside_the_role_answers_like_the_recordings_route() {
    let env = TestEnv::start("role-filter").await;
    let piece = fixture_piece(&default_init_spec(), &[TICKS_4S]);
    env.seed_piece("back", 0, 1_000_000, 1_004_000, &piece.bytes);

    let refused = env
        .media("frontonly", "back", "1000", "1004", "master.m3u8")
        .await;
    let (status, _headers, body) = http_parts(&refused);
    assert_eq!(status_code(&status), "404");
    let error: ErrorBody = serde_json::from_slice(&body).expect("error body parses");
    assert_eq!(error.code, "not_found");
    assert_eq!(
        error.message,
        "No recorded footage is available for this camera and range."
    );

    let ghost = env
        .media("admin", "ghostcam", "1000", "1004", "master.m3u8")
        .await;
    assert_error(&ghost, "404", "not_found");
    env.finish();
}

#[tokio::test]
async fn a_request_sequence_never_modifies_the_stored_bytes() {
    let env = TestEnv::start("byte-stability").await;
    let pieces = seed_three_contiguous(&env);
    let stored: Vec<Vec<u8>> = (0..3_i64)
        .map(|seq| {
            let start = 1_000_000 + seq * 4_000;
            std::fs::read(env.root.join(piece_relative("front", seq, start))).expect("read piece")
        })
        .collect();
    let index_before = std::fs::read(&env.index_path).expect("read index");

    for file in [
        "master.m3u8",
        "index.m3u8",
        "init-0.mp4",
        "seg-0.m4s",
        "seg-1.m4s",
    ] {
        let raw = env.media("admin", "front", "1000", "1012", file).await;
        let (status, _headers, _body) = http_parts(&raw);
        assert_eq!(status_code(&status), "200");
    }
    let ranged = env
        .media_range("admin", "front", "1000", "1012", "seg-0.m4s", "bytes=0-9")
        .await;
    assert_eq!(status_code(&http_parts(&ranged).0), "206");

    assert_eq!(
        std::fs::read(&env.index_path).expect("reread index"),
        index_before
    );
    for (index, bytes) in stored.iter().enumerate() {
        let seq = i64::try_from(index).expect("fixture fits");
        let start = 1_000_000 + seq * 4_000;
        let after = std::fs::read(env.root.join(piece_relative("front", seq, start)))
            .expect("reread piece");
        assert_eq!(after, *bytes, "piece bytes unchanged");
    }
    let seg0 = env
        .media("admin", "front", "1000", "1012", "seg-0.m4s")
        .await;
    let (_status, _headers, body) = http_parts(&seg0);
    assert_eq!(
        body,
        stored[0][pieces[0].fragments_start..],
        "the first served segment equals the stored fragments byte for byte"
    );
    env.finish();
}

#[tokio::test]
async fn an_init_prefix_claiming_more_than_one_mebibyte_answers_503() {
    let env = TestEnv::start("huge-init").await;
    env.open_index();
    let mut piece_bytes = padded_init(1 << 20);
    piece_bytes.extend_from_slice(&fragment(&[TICKS_4S], 0));
    env.seed_piece("front", 0, 1_000_000, 1_004_000, &piece_bytes);

    let raw = env
        .media("admin", "front", "1000", "1012", "master.m3u8")
        .await;
    assert_error(&raw, "503", "recording_unavailable");
    let seg = env
        .media("admin", "front", "1000", "1012", "seg-0.m4s")
        .await;
    assert_error(&seg, "503", "recording_unavailable");
    env.finish();
}

/// A valid init longer than one mebibyte: a `free` box sits between `ftyp` and `moov`.
fn padded_init(pad: usize) -> Vec<u8> {
    let init = init_bytes(&default_init_spec());
    let ftyp_len = u32::from_be_bytes(init[..4].try_into().expect("ftyp size")) as usize;
    let mut out = init[..ftyp_len].to_vec();
    out.extend_from_slice(&mp4_box(*b"free", &vec![0_u8; pad]));
    out.extend_from_slice(&init[ftyp_len..]);
    out
}

#[tokio::test]
async fn a_row_naming_another_cameras_stored_piece_answers_recording_unavailable() {
    let env = TestEnv::start("cross-camera").await;
    env.open_index();
    let piece = fixture_piece(&default_init_spec(), &[TICKS_4S]);
    env.seed_piece("back", 0, 1_000_000, 1_004_000, &piece.bytes);
    env.insert_row(
        "front",
        0,
        1_000_000,
        1_004_000,
        &piece_relative("back", 0, 1_000_000),
        piece.size_bytes,
    );
    let raw = env
        .media("admin", "front", "1000", "1012", "master.m3u8")
        .await;
    assert_error(&raw, "503", "recording_unavailable");
    env.finish();
}

#[tokio::test]
async fn a_range_longer_than_one_month_answers_not_found_without_reading_pieces() {
    let env = TestEnv::start("long-range").await;
    env.open_index();
    env.seed_missing_piece("front", 0, 1_000_000, 1_004_000);
    let raw = env
        .media("admin", "front", "1000", "2765800", "master.m3u8")
        .await;
    assert_error(&raw, "404", "not_found");
    env.finish();
}

#[tokio::test]
async fn a_plan_over_the_piece_count_cap_answers_not_found_without_reading_pieces() {
    let env = TestEnv::start("plan-cap").await;
    let mut index = env.open_index();
    let sql = "INSERT INTO pieces \
               (camera, seq, start_ts, end_ts, duration_s, path, size_bytes, codec) \
               VALUES ('front', ?1, ?2, ?3, 4.0, ?4, 1, 'h264')";
    {
        let tx = index.transaction().expect("transaction");
        for seq in 0..=25_000_i64 {
            let start = seq * 4_000;
            tx.execute(
                sql,
                params![
                    seq,
                    start,
                    start + 4_000,
                    piece_relative("front", seq, start)
                ],
            )
            .expect("insert piece");
        }
        tx.commit().expect("commit");
    }
    drop(index);
    let raw = env
        .media("admin", "front", "0", "100004", "index.m3u8")
        .await;
    assert_error(&raw, "404", "not_found");
    env.finish();
}

#[tokio::test]
async fn a_segment_file_larger_than_sixty_four_mebibytes_answers_recording_unavailable() {
    let env = TestEnv::start("huge-segment").await;
    env.open_index();
    let init = init_bytes(&default_init_spec());
    let frag = fragment(&[TICKS_4S], 0);
    let moof_len = u32::from_be_bytes(frag[..4].try_into().expect("moof size")) as usize;
    let mdat_size = 64 * 1024 * 1024 + 1024 - init.len() - moof_len;
    let total = init.len() + moof_len + mdat_size;
    let relative = piece_relative("front", 0, 1_000_000);
    let path = env.root.join(&relative);
    std::fs::create_dir_all(path.parent().expect("piece directory")).expect("piece dirs");
    let mut file = std::fs::File::create(&path).expect("create piece file");
    file.write_all(&init).expect("write init");
    file.write_all(&frag[..moof_len]).expect("write moof");
    let mut mdat_header = Vec::new();
    put_u32(
        &mut mdat_header,
        u32::try_from(mdat_size).expect("mdat size fits u32"),
    );
    mdat_header.extend_from_slice(b"mdat");
    file.write_all(&mdat_header).expect("write mdat header");
    file.set_len(u64::try_from(total).expect("size fits u64"))
        .expect("sparse extend");
    drop(file);
    env.insert_row(
        "front",
        0,
        1_000_000,
        1_004_000,
        &relative,
        i64::try_from(total).expect("size fits i64"),
    );
    let raw = env
        .media("admin", "front", "1000", "1004", "seg-0.m4s")
        .await;
    assert_error(&raw, "503", "recording_unavailable");
    env.finish();
}

const LOG_CHILD_ENV: &str = "CORVETTE_PLAYLIST_LOG_CHILD";

#[tokio::test]
async fn each_piece_failure_logs_exactly_one_fixed_word_line() {
    for event in ["recording-piece-unreadable", "recording-piece-malformed"] {
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args(["--exact", "playlist_piece_failure_log_child", "--nocapture"])
            .env(LOG_CHILD_ENV, event)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .expect("run the log child");
        assert!(output.status.success(), "event {event}:\n{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        let line = format!(
            "corvette-api-server unit=\"playlist\" event={event} detail=\"camera=front seq=0\""
        );
        assert_eq!(
            stderr.matches(line.as_str()).count(),
            1,
            "event {event}: {stderr}"
        );
    }
}

/// Child of the log-line test: one request whose piece fails one way per mode.
#[tokio::test]
async fn playlist_piece_failure_log_child() {
    let Ok(event) = std::env::var(LOG_CHILD_ENV) else {
        return;
    };
    let env = TestEnv::start("log-child").await;
    env.open_index();
    match event.as_str() {
        "recording-piece-unreadable" => env.seed_missing_piece("front", 0, 1_000_000, 1_004_000),
        "recording-piece-malformed" => {
            env.seed_piece("front", 0, 1_000_000, 1_004_000, &[0_u8; 16]);
        }
        other => panic!("unknown log child mode {other}"),
    }
    let raw = env
        .media("admin", "front", "1000", "1012", "master.m3u8")
        .await;
    assert_error(&raw, "503", "recording_unavailable");
    env.finish();
}
