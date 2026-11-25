//! Exercises the stitched download route against a fixture index and hand-built piece files.

use corvette_api::ErrorBody;
use corvette_api_server::config::{ConfigSource, DEFAULT_CACHE_TTL};
use corvette_api_server::{ApiServer, CONNECTION_TIMEOUT, rebase};
use rusqlite::{Connection, params};
use serde_json::json;
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
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

    fn seed_missing_piece(&self, camera: &str, seq: i64, start_ms: i64, end_ms: i64) {
        let relative = piece_relative(camera, seq, start_ms);
        self.insert_row(camera, seq, start_ms, end_ms, &relative, 1);
    }

    fn seed_bad_path(&self, camera: &str, seq: i64, start_ms: i64, end_ms: i64, path: &str) {
        self.insert_row(camera, seq, start_ms, end_ms, path, 1);
    }

    async fn download(&self, camera: &str, start: &str, end: &str, extra: &[&str]) -> Vec<u8> {
        self.exchange(media_request(camera, start, end, extra))
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
        "corvette-download-test-{}-{tag}-{id}",
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

fn media_request(camera: &str, start: &str, end: &str, extra: &[&str]) -> Vec<u8> {
    let path = format!("/corvette/api/v1/media/{camera}/start/{start}/end/{end}/download.mp4");
    let role_header = "Remote-Role: admin";
    let mut headers = vec!["Remote-User: alice", role_header];
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
        headers.get("content-type").map(String::as_str),
        Some("application/json"),
        "error body is JSON"
    );
    let error: ErrorBody = serde_json::from_slice(&body).expect("error body parses");
    assert_eq!(error.code, code);
}

/// The chunk-framed body de-framed: its bytes and whether the zero chunk terminated the stream.
fn dechunk(body: &[u8]) -> Option<(Vec<u8>, bool)> {
    let mut out = Vec::new();
    let mut cursor = 0_usize;
    loop {
        let line_end = body.get(cursor..)?.windows(2).position(|w| w == b"\r\n")?;
        let text = std::str::from_utf8(body.get(cursor..cursor + line_end)?).ok()?;
        let size = usize::from_str_radix(text, 16).ok()?;
        cursor += line_end + 2;
        if size == 0 {
            return Some((out, body.get(cursor..cursor + 2)? == b"\r\n"));
        }
        out.extend_from_slice(body.get(cursor..cursor + size)?);
        cursor += size + 2;
    }
}

/// The body of a successful download: head fields checked exactly, chunks de-framed.
fn downloaded(raw: &[u8]) -> (HashMap<String, String>, Vec<u8>) {
    let (status, headers, body) = http_parts(raw);
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/mp4")
    );
    assert_eq!(
        headers.get("transfer-encoding").map(String::as_str),
        Some("chunked")
    );
    assert_eq!(
        headers.get("connection").map(String::as_str),
        Some("close"),
        "one answer per connection, as the streamed head declares"
    );
    assert_eq!(
        headers.get("cache-control").map(String::as_str),
        Some("no-store")
    );
    assert!(
        !headers.contains_key("content-length"),
        "a streamed answer carries no Content-Length"
    );
    let (data, terminated) = dechunk(&body).expect("chunk framing parses");
    assert!(terminated, "a complete stream ends with the zero chunk");
    (headers, data)
}

/// The position of the 8-byte value inside the first version-1 `tfdt` of a fragment.
fn tfdt_field(body: &[u8]) -> usize {
    let marker = body
        .windows(4)
        .position(|window| window == *b"tfdt")
        .expect("fragment carries a tfdt");
    marker + 8
}

fn first_tfdt(body: &[u8]) -> u64 {
    let at = tfdt_field(body);
    u64::from_be_bytes(body[at..at + 8].try_into().expect("tfdt field"))
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
                       "detect": { "width": 704, "height": 576 } },
            "odd\"cam": { "enabled": true, "friendly_name": null, "ui": { "order": 0 },
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

/// Builds stored-piece bytes by hand, as the playlist route's tests do.
struct InitSpec {
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

fn mdhd_body(timescale: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0);
    body.extend_from_slice(&[0, 0, 0]);
    put_u32(&mut body, 0);
    put_u32(&mut body, 0);
    put_u32(&mut body, timescale);
    put_u64(&mut body, 0);
    put_u16(&mut body, 0x55C4);
    put_u16(&mut body, 0);
    body
}

fn sample_entry_body(width: u16, height: u16) -> Vec<u8> {
    let mut body = vec![0_u8; 6];
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
    body.extend_from_slice(&[0_u8; 32]);
    put_u16(&mut body, 0x0018);
    put_u16(&mut body, 0xFFFF);
    body
}

fn init_bytes(spec: &InitSpec) -> Vec<u8> {
    let mut entry_body = sample_entry_body(spec.width, spec.height);
    entry_body.extend_from_slice(&mp4_box(spec.config_kind, &spec.config_body));
    let mut stsd = Vec::new();
    put_u32(&mut stsd, 0);
    put_u32(&mut stsd, 1);
    stsd.extend_from_slice(&mp4_box(spec.fourcc, &entry_body));
    let stbl = mp4_box(*b"stbl", &mp4_box(*b"stsd", &stsd));
    let mut minf_body = mp4_box(*b"vmhd", &[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
    minf_body.extend_from_slice(&stbl);
    let minf = mp4_box(*b"minf", &minf_body);
    let mut mdia_body = mp4_box(*b"mdhd", &mdhd_body(spec.timescale));
    mdia_body.extend_from_slice(&minf);
    let mdia = mp4_box(*b"mdia", &mdia_body);
    let mut trak_body = mp4_box(*b"tkhd", &[0_u8; 84]);
    trak_body.extend_from_slice(&mdia);
    let trak = mp4_box(*b"trak", &trak_body);
    let mut out = ftyp_box();
    out.extend_from_slice(&mp4_box(*b"moov", &trak));
    out
}

fn tfdt_box(value: u64) -> Vec<u8> {
    let mut body = Vec::new();
    put_u32(&mut body, 1 << 24);
    put_u64(&mut body, value);
    mp4_box(*b"tfdt", &body)
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
    body.extend_from_slice(&vec![0x5A_u8; payload_len]);
    mp4_box(*b"mdat", &body)
}

fn fragment(durations: &[u32], tfdt_value: u64) -> Vec<u8> {
    let mut mfhd_body = Vec::new();
    put_u32(&mut mfhd_body, 0);
    put_u32(&mut mfhd_body, 1);
    let mut traf_body = mp4_box(*b"tfhd", &[0, 2, 0, 0, 0, 0, 0, 1]);
    traf_body.extend_from_slice(&tfdt_box(tfdt_value));
    traf_body.extend_from_slice(&trun_box(durations));
    let mut moof_body = mp4_box(*b"mfhd", &mfhd_body);
    moof_body.extend_from_slice(&mp4_box(*b"traf", &traf_body));
    let mut out = mp4_box(*b"moof", &moof_body);
    out.extend_from_slice(&mdat_box(durations.len() * 8 + 4));
    out
}

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
async fn contiguous_pieces_download_as_one_file_on_one_continuous_time_base() {
    let env = TestEnv::start("contiguous").await;
    let pieces = seed_three_contiguous(&env);
    let raw = env.download("front", "1000", "1012", &[]).await;
    let (headers, data) = downloaded(&raw);
    assert_eq!(
        headers.get("content-disposition").map(String::as_str),
        Some("attachment; filename=\"front-1000-1012.mp4\"")
    );
    assert!(
        !headers.contains_key("x-corvette-next-start"),
        "one run answers without a next-start header"
    );
    let init = init_bytes(&default_init_spec());
    let parts = rebase::split_piece(&data).expect("downloaded body parses");
    assert_eq!(parts.init_len, init.len(), "one init prefix at the head");
    assert_eq!(&data[..parts.init_len], init.as_slice());
    assert_eq!(parts.fragments.len(), 3, "one fragment per stored piece");
    assert_eq!(first_tfdt(&data), 0, "the first tfdt is zero");
    for (index, placed) in parts.fragments.iter().enumerate() {
        let served = &data[placed.start..placed.end];
        let stored = &pieces[index].bytes[pieces[index].fragments_start..];
        let at = tfdt_field(served);
        let mut normalized = served.to_vec();
        normalized[at..at + 8].copy_from_slice(&stored[at..at + 8]);
        assert_eq!(normalized, stored, "fragment {index} matches stored bytes");
    }
    assert_eq!(
        first_tfdt(&data[parts.fragments[1].start..parts.fragments[1].end]),
        u64::from(TICKS_4S)
    );
    assert_eq!(
        first_tfdt(&data[parts.fragments[2].start..parts.fragments[2].end]),
        u64::from(TICKS_4S) * 2
    );
    env.finish();
}

#[tokio::test]
async fn a_gap_inside_the_run_downloads_as_one_collapsed_continuous_clip() {
    let env = TestEnv::start("gap").await;
    let piece = fixture_piece(&default_init_spec(), &[TICKS_4S]);
    env.seed_piece("front", 0, 1_000_000, 1_004_000, &piece.bytes);
    env.seed_piece("front", 1, 1_006_000, 1_010_000, &piece.bytes);

    let raw = env.download("front", "1000", "1010", &[]).await;
    let (headers, data) = downloaded(&raw);
    assert!(
        !headers.contains_key("x-corvette-next-start"),
        "a gap is not a parameter change"
    );
    let parts = rebase::split_piece(&data).expect("downloaded body parses");
    assert_eq!(
        parts.fragments.len(),
        2,
        "one file holds both sides of the gap"
    );
    assert_eq!(first_tfdt(&data), 0);
    let second = &data[parts.fragments[1].start..parts.fragments[1].end];
    assert_eq!(
        first_tfdt(second),
        u64::from(TICKS_4S),
        "the time base collapses the gap without a jump"
    );
    env.finish();
}

#[tokio::test]
async fn a_parameter_change_downloads_the_first_run_and_names_the_next_start() {
    let env = TestEnv::start("parameter-change").await;
    let first = fixture_piece(&default_init_spec(), &[TICKS_4S]);
    let second = fixture_piece(&init_with_width(640), &[TICKS_4S]);
    env.seed_piece("front", 0, 1_000_000, 1_004_000, &first.bytes);
    env.seed_piece("front", 1, 1_004_000, 1_008_000, &first.bytes);
    env.seed_piece("front", 2, 1_008_000, 1_012_000, &second.bytes);

    let raw = env.download("front", "1000", "1012", &[]).await;
    let (headers, data) = downloaded(&raw);
    assert_eq!(
        headers.get("x-corvette-next-start").map(String::as_str),
        Some("1008.000")
    );
    let parts = rebase::split_piece(&data).expect("first file parses");
    assert_eq!(
        parts.fragments.len(),
        2,
        "the file holds the first run only"
    );
    assert_eq!(parts.init_len, init_bytes(&default_init_spec()).len());

    let next = env.download("front", "1008.000", "1012", &[]).await;
    let (headers, next_data) = downloaded(&next);
    assert!(
        !headers.contains_key("x-corvette-next-start"),
        "the last part answers without a next-start header"
    );
    let next_parts = rebase::split_piece(&next_data).expect("second file parses");
    assert_eq!(next_parts.fragments.len(), 1, "the second run alone");
    assert_eq!(
        next_parts.init_len,
        init_bytes(&init_with_width(640)).len(),
        "the second file carries its run's own init"
    );
    assert_eq!(
        first_tfdt(&next_data),
        0,
        "each file starts its own time base"
    );
    env.finish();
}

#[tokio::test]
async fn an_overlapping_piece_end_is_not_reselected_by_the_next_part_request() {
    let env = TestEnv::start("overlap").await;
    let first = fixture_piece(&default_init_spec(), &[TICKS_4S]);
    let second = fixture_piece(&init_with_width(640), &[TICKS_4S]);
    env.seed_piece("front", 0, 1_000_000, 1_004_000, &first.bytes);
    env.seed_piece("front", 1, 1_004_000, 1_008_500, &first.bytes);
    env.seed_piece("front", 2, 1_008_000, 1_012_000, &second.bytes);

    let raw = env.download("front", "1000", "1012", &[]).await;
    let (headers, data) = downloaded(&raw);
    assert_eq!(
        headers.get("x-corvette-next-start").map(String::as_str),
        Some("1008.500"),
        "the next start is the later of the next start and the included end"
    );
    assert_eq!(
        rebase::split_piece(&data)
            .expect("first file parses")
            .fragments
            .len(),
        2
    );

    let next = env.download("front", "1008.500", "1012", &[]).await;
    let (_headers, next_data) = downloaded(&next);
    let parts = rebase::split_piece(&next_data).expect("second file parses");
    assert_eq!(
        parts.fragments.len(),
        1,
        "the last piece of the first run is not re-selected"
    );
    assert_eq!(parts.init_len, init_bytes(&init_with_width(640)).len());
    env.finish();
}

#[tokio::test]
async fn a_range_header_is_ignored_and_the_answer_is_the_whole_file() {
    let env = TestEnv::start("range").await;
    seed_three_contiguous(&env);
    let whole = env.download("front", "1000", "1012", &[]).await;
    let ranged = env
        .download("front", "1000", "1012", &["Range: bytes=0-9"])
        .await;
    let (status, headers, _body) = http_parts(&ranged);
    assert_eq!(status_code(&status), "200");
    assert!(!headers.contains_key("content-range"), "no partial answer");
    let (_headers, ranged_data) = downloaded(&ranged);
    let (_headers, whole_data) = downloaded(&whole);
    assert_eq!(ranged_data, whole_data, "the same bytes as without Range");
    env.finish();
}

#[tokio::test]
async fn the_download_error_matrix_matches_the_playlist_routes() {
    let env = TestEnv::start("error-matrix").await;
    seed_three_contiguous(&env);

    let empty = env.download("front", "5000", "5004", &[]).await;
    assert_error(&empty, "404", "not_found");

    let inverted = env.download("front", "2000", "1000", &[]).await;
    assert_error(&inverted, "404", "not_found");

    env.finish();
}

#[tokio::test]
async fn a_long_range_and_a_plan_over_the_piece_cap_answer_not_found() {
    let env = TestEnv::start("caps").await;
    env.open_index();
    env.seed_missing_piece("front", 0, 1_000_000, 1_004_000);
    let long_range = env.download("front", "1000", "2765800", &[]).await;
    assert_error(&long_range, "404", "not_found");

    let env2 = TestEnv::start("plan-cap").await;
    let mut index = env2.open_index();
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
    let capped = env2.download("front", "0", "100004", &[]).await;
    assert_error(&capped, "404", "not_found");
    env2.finish();
    env.finish();
}

#[tokio::test]
async fn an_absent_index_answers_recording_unavailable() {
    let env = TestEnv::start("absent-index").await;
    let raw = env.download("front", "1000", "1012", &[]).await;
    assert_error(&raw, "503", "recording_unavailable");
    assert!(
        !env.index_path.exists(),
        "the route must not create the index"
    );
    env.finish();
}

#[tokio::test]
async fn a_missing_first_piece_answers_a_proper_status_before_any_body() {
    let env = TestEnv::start("missing-first").await;
    env.open_index();
    env.seed_missing_piece("front", 0, 1_000_000, 1_004_000);
    let raw = env.download("front", "1000", "1012", &[]).await;
    assert_error(&raw, "503", "recording_unavailable");
    env.finish();
}

#[tokio::test]
async fn hostile_stored_paths_answer_recording_unavailable() {
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
        let raw = env.download("front", "1000", "1012", &[]).await;
        assert_error(&raw, "503", "recording_unavailable");
    }
    env.finish();
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
    let raw = env.download("front", "1000", "1012", &[]).await;
    assert_error(&raw, "503", "recording_unavailable");
    env.finish();
}

#[tokio::test]
async fn a_camera_outside_the_role_answers_like_the_recordings_route() {
    let env = TestEnv::start("role-filter").await;
    let piece = fixture_piece(&default_init_spec(), &[TICKS_4S]);
    env.seed_piece("back", 0, 1_000_000, 1_004_000, &piece.bytes);

    let refused = exchange(
        &env.socket,
        &get(
            "/corvette/api/v1/media/back/start/1000/end/1004/download.mp4",
            &["Remote-User: alice", "Remote-Role: frontonly"],
        ),
    )
    .await;
    let (status, _headers, body) = http_parts(&refused);
    assert_eq!(status_code(&status), "404");
    let error: ErrorBody = serde_json::from_slice(&body).expect("error body parses");
    assert_eq!(error.code, "not_found");
    assert_eq!(
        error.message,
        "No recorded footage is available for this camera and range."
    );
    env.finish();
}

#[tokio::test]
async fn downloads_never_modify_the_stored_bytes() {
    let env = TestEnv::start("byte-stability").await;
    seed_three_contiguous(&env);
    let stored: Vec<Vec<u8>> = (0..3_i64)
        .map(|seq| {
            let start = 1_000_000 + seq * 4_000;
            std::fs::read(env.root.join(piece_relative("front", seq, start))).expect("read piece")
        })
        .collect();
    let index_before = std::fs::read(&env.index_path).expect("read index");
    let raw = env.download("front", "1000", "1012", &[]).await;
    let (_headers, data) = downloaded(&raw);
    assert!(!data.is_empty(), "a body arrived to justify the comparison");
    let stored_after: Vec<Vec<u8>> = (0..3_i64)
        .map(|seq| {
            let start = 1_000_000 + seq * 4_000;
            std::fs::read(env.root.join(piece_relative("front", seq, start))).expect("read piece")
        })
        .collect();
    assert_eq!(stored_after, stored, "stored pieces are byte-stable");
    assert_eq!(
        std::fs::read(&env.index_path).expect("read index after"),
        index_before,
        "the index is byte-stable"
    );
    env.finish();
}

#[tokio::test]
async fn a_single_piece_download_equals_the_stored_piece() {
    let env = TestEnv::start("single-piece").await;
    let piece = fixture_piece(&default_init_spec(), &[TICKS_4S]);
    env.seed_piece("front", 0, 1_000_000, 1_004_000, &piece.bytes);
    let raw = env.download("front", "1000", "1004", &[]).await;
    let (_headers, data) = downloaded(&raw);
    assert_eq!(
        data, piece.bytes,
        "init plus the zero-based fragment as stored"
    );
    env.finish();
}

#[tokio::test]
async fn a_download_slower_than_the_request_deadline_completes_intact() {
    let env = TestEnv::start("slow-reader").await;
    let init = init_bytes(&default_init_spec());
    let mut stored: Vec<Vec<u8>> = Vec::new();
    for index in 0..8_i64 {
        let bytes = mdat_payload(&init, 1 << 20);
        let start = 1_000_000 + index * 4_000;
        env.seed_piece("front", index, start, start + 4_000, &bytes);
        stored.push(bytes);
    }
    // A client that reads 32 KiB per 25 ms for eight mebibytes cannot be done before
    // the request deadline: the socket buffers cannot hold the body, so the server blocks.
    let mut stream = UnixStream::connect(&env.socket).await.expect("connect");
    stream
        .write_all(&media_request("front", "1000", "1032", &[]))
        .await
        .expect("request");
    let mut raw: Vec<u8> = Vec::new();
    let mut buffer = vec![0_u8; 32 * 1024];
    let started = Instant::now();
    loop {
        let read = stream.read(&mut buffer).await.expect("slow read");
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..read]);
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed > Duration::from_secs(6),
        "the client read for {elapsed:?}, which does not outrun the request deadline"
    );
    let (_headers, data) = downloaded(&raw);
    let parts = rebase::split_piece(&data).expect("the streamed body parses");
    assert_eq!(parts.fragments.len(), 8, "every piece arrived in order");
    for (index, placed) in parts.fragments.iter().enumerate() {
        let served = &data[placed.start..placed.end];
        let stored_piece = &stored[index][init.len()..];
        let at = tfdt_field(served);
        let mut normalized = served.to_vec();
        normalized[at..at + 8].copy_from_slice(&stored_piece[at..at + 8]);
        assert_eq!(normalized, stored_piece, "piece {index} streamed intact");
    }
    env.finish();
}

#[tokio::test]
async fn a_camera_name_outside_the_plain_character_class_answers_not_found() {
    let env = TestEnv::start("odd-camera-name").await;
    env.open_index();
    let piece = fixture_piece(&default_init_spec(), &[TICKS_4S]);
    env.seed_piece("odd\"cam", 0, 1_000_000, 1_004_000, &piece.bytes);
    let raw = env.download("odd%22cam", "1000", "1004", &[]).await;
    assert_error(&raw, "404", "not_found");
    env.finish();
}

#[tokio::test]
async fn forty_one_mebibyte_pieces_stream_to_completion() {
    let env = TestEnv::start("forty-pieces").await;
    let init = init_bytes(&default_init_spec());
    let mut pieces: Vec<(Vec<u8>, usize)> = Vec::new();
    for index in 0..40_i64 {
        let bytes = mdat_payload(&init, 1 << 20);
        let start = 1_000_000 + index * 4_000;
        env.seed_piece("front", index, start, start + 4_000, &bytes);
        pieces.push((bytes, init.len()));
    }
    let raw = env.download("front", "1000", "10160", &[]).await;
    let (_headers, data) = downloaded(&raw);
    let parts = rebase::split_piece(&data).expect("the streamed body parses");
    assert_eq!(parts.fragments.len(), 40, "every piece arrived in order");
    for (index, placed) in parts.fragments.iter().enumerate() {
        let served = &data[placed.start..placed.end];
        let stored = &pieces[index].0[pieces[index].1..];
        let at = tfdt_field(served);
        let mut normalized = served.to_vec();
        normalized[at..at + 8].copy_from_slice(&stored[at..at + 8]);
        assert_eq!(normalized, stored, "piece {index} streamed intact");
    }
    env.finish();
}

/// init plus one fragment whose `mdat` payload reaches `payload_len` media bytes.
fn mdat_payload(init: &[u8], payload_len: usize) -> Vec<u8> {
    let mut out = init.to_vec();
    let mut mfhd_body = Vec::new();
    put_u32(&mut mfhd_body, 0);
    put_u32(&mut mfhd_body, 1);
    let mut traf_body = mp4_box(*b"tfhd", &[0, 2, 0, 0, 0, 0, 0, 1]);
    traf_body.extend_from_slice(&tfdt_box(0));
    traf_body.extend_from_slice(&trun_box(&[TICKS_4S]));
    let mut moof_body = mp4_box(*b"mfhd", &mfhd_body);
    moof_body.extend_from_slice(&mp4_box(*b"traf", &traf_body));
    out.extend_from_slice(&mp4_box(*b"moof", &moof_body));
    out.extend_from_slice(&mdat_box(payload_len));
    out
}

const LOG_CHILD_ENV: &str = "CORVETTE_DOWNLOAD_LOG_CHILD";

#[tokio::test]
async fn each_mid_stream_piece_failure_logs_exactly_one_fixed_word_line() {
    for event in ["recording-piece-unreadable", "recording-piece-malformed"] {
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args(["--exact", "download_piece_failure_log_child", "--nocapture"])
            .env(LOG_CHILD_ENV, event)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .expect("run the log child");
        assert!(output.status.success(), "event {event}:\n{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        let line = format!(
            "corvette-api-server unit=\"playlist\" event={event} detail=\"camera=front seq=1\""
        );
        assert_eq!(
            stderr.matches(line.as_str()).count(),
            1,
            "event {event}: {stderr}"
        );
    }
}

/// Child of the log-line test: one download whose second piece fails one way per mode.
#[tokio::test]
async fn download_piece_failure_log_child() {
    let Ok(event) = std::env::var(LOG_CHILD_ENV) else {
        return;
    };
    let env = TestEnv::start("log-child").await;
    env.open_index();
    let first = fixture_piece(&default_init_spec(), &[TICKS_4S]);
    env.seed_piece("front", 0, 1_000_000, 1_004_000, &first.bytes);
    match event.as_str() {
        "recording-piece-malformed" => {
            let mut bytes = init_bytes(&default_init_spec());
            let mut mfhd_body = Vec::new();
            put_u32(&mut mfhd_body, 0);
            put_u32(&mut mfhd_body, 1);
            let moof = mp4_box(*b"moof", &mp4_box(*b"mfhd", &mfhd_body));
            bytes.extend_from_slice(&moof);
            env.seed_piece("front", 1, 1_004_000, 1_008_000, &bytes);
        }
        "recording-piece-unreadable" => {
            let init = init_bytes(&default_init_spec());
            let frag = fragment(&[TICKS_4S], 0);
            let moof_len = u32::from_be_bytes(frag[..4].try_into().expect("moof size")) as usize;
            let mdat_size = 64 * 1024 * 1024 + 1024 - init.len() - moof_len;
            let total = init.len() + moof_len + mdat_size;
            let relative = piece_relative("front", 1, 1_004_000);
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
                1,
                1_004_000,
                1_008_000,
                &relative,
                i64::try_from(total).expect("size fits i64"),
            );
        }
        other => panic!("unknown log child mode {other}"),
    }
    let raw = env.download("front", "1000", "1012", &[]).await;
    let (status, headers, _body) = http_parts(&raw);
    assert_eq!(status_code(&status), "200", "the answer keeps its status");
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/mp4")
    );
    assert!(
        !raw.ends_with(b"0\r\n\r\n"),
        "the stream ends without the terminating chunk"
    );
    assert!(
        raw.windows(4).any(|w| w == b"ftyp"),
        "the first piece arrived"
    );
    env.finish();
}
