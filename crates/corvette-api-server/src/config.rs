//! Frigate's configuration as an HTTP source with a bounded in-memory cache
//! (issue #4 D3).
//!
//! The fetch is a hand-written `HTTP/1.0` GET; this service adds no HTTP
//! client dependency. The accepted response is deliberately narrow — a plain
//! `200` whose body matches its `Content-Length` — because a chunked, encoded,
//! or truncated response cannot be a trustworthy configuration.

use crate::{find_header_end, log_event, split_at_byte, split_header_lines, trim_spaces, utf8};
use corvette_api::FrigateConfig;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Environment variable naming the `host:port` Frigate answers `/api/config`
/// on.
pub const CONFIG_ADDR_ENV: &str = "CORVETTE_API_CONFIG_ADDR";

/// The configuration source used when `CORVETTE_API_CONFIG_ADDR` is unset:
/// Frigate's own nginx, in the same pod (issue #4 D3).
pub const DEFAULT_CONFIG_ADDR: &str = "127.0.0.1:5000";

/// How young a fetched configuration must be to serve from it (issue #4 D3).
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(10);

/// Overall budget for one fetch, covering connect, request, and response.
const FETCH_TIMEOUT: Duration = Duration::from_secs(2);

/// The largest configuration response accepted; anything larger is a broken
/// upstream, not a configuration worth parsing.
const MAX_CONFIG_BYTES: usize = 4 * 1024 * 1024;

/// Why one configuration fetch failed. The text names the operation and the
/// source, so it serves both the log line and the caller's error body.
#[derive(Debug)]
pub(crate) struct ConfigFetchError(String);

impl std::fmt::Display for ConfigFetchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A parsed configuration and the moment it arrived.
#[derive(Debug)]
struct CachedConfig {
    config: FrigateConfig,
    fetched_at: Instant,
}

/// Frigate's configuration, fetched on demand and kept in memory for a
/// bounded time (issue #4 D3). Share through the server with `Arc`; one
/// instance is meant for the whole process.
#[derive(Debug)]
pub struct ConfigSource {
    addr: String,
    ttl: Duration,
    /// The lock is held across a fetch, so concurrent requests that find the
    /// copy stale wait and share one fetch instead of starting one each
    /// (issue #4 D3).
    cache: tokio::sync::Mutex<Option<CachedConfig>>,
}

impl ConfigSource {
    /// A source fetching from `addr` and serving any copy younger than `ttl`.
    #[must_use]
    pub fn new(addr: String, ttl: Duration) -> Self {
        Self {
            addr,
            ttl,
            cache: tokio::sync::Mutex::new(None),
        }
    }

    /// The serialized `{"cameras":[...]}` list for `role`, or the fetch
    /// failure when no copy younger than the TTL is available.
    pub(crate) async fn camera_list_json(&self, role: &str) -> Result<Vec<u8>, ConfigFetchError> {
        let list = corvette_api::CameraList {
            cameras: self.config().await?.camera_entries_for_role(role),
        };
        // `CameraList` holds only strings and integers; its derived
        // serializer cannot fail.
        Ok(serde_json::to_vec(&list).expect("camera list serializes"))
    }

    /// The newest configuration, refetched when the cached copy is older than
    /// the TTL. A copy older than the TTL is never returned: a failed fetch
    /// propagates even when a stale copy is held.
    async fn config(&self) -> Result<FrigateConfig, ConfigFetchError> {
        let mut cache = self.cache.lock().await;
        if let Some(cached) = cache.as_ref()
            && cached.fetched_at.elapsed() < self.ttl
        {
            return Ok(cached.config.clone());
        }
        let config = match fetch(&self.addr).await {
            Ok(config) => config,
            Err(err) => {
                log_event("config", "fetch-error", &err);
                return Err(err);
            }
        };
        *cache = Some(CachedConfig {
            config: config.clone(),
            fetched_at: Instant::now(),
        });
        drop(cache);
        Ok(config)
    }
}

/// Fetches and parses one configuration, end to end within [`FETCH_TIMEOUT`]
/// so a stalled upstream cannot hold a request open longer.
async fn fetch(addr: &str) -> Result<FrigateConfig, ConfigFetchError> {
    tokio::time::timeout(FETCH_TIMEOUT, fetch_from(addr))
        .await
        .map_err(|_elapsed| {
            ConfigFetchError(format!(
                "no complete response from {addr} within {FETCH_TIMEOUT:?}"
            ))
        })?
}

async fn fetch_from(addr: &str) -> Result<FrigateConfig, ConfigFetchError> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|err| ConfigFetchError(format!("could not connect to {addr}: {err}")))?;
    // Frigate's nginx caches `/api/` responses unless the request sends
    // `X-Cache-Bypass`, so a fetched copy could otherwise be arbitrarily old.
    let request = format!(
        "GET /api/config HTTP/1.0\r\nHost: {addr}\r\nX-Cache-Bypass: 1\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.map_err(|err| {
        ConfigFetchError(format!(
            "could not send the config request to {addr}: {err}"
        ))
    })?;
    // `Connection: close` makes end of stream the body end, so nothing needs
    // transfer-encoding decoding; the cap bounds one upstream answer.
    let mut response = Vec::new();
    stream
        .take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut response)
        .await
        .map_err(|err| {
            ConfigFetchError(format!(
                "could not read the config response from {addr}: {err}"
            ))
        })?;
    if response.len() > MAX_CONFIG_BYTES {
        return Err(ConfigFetchError(format!(
            "config response from {addr} exceeded the {} MiB limit",
            MAX_CONFIG_BYTES / (1024 * 1024)
        )));
    }
    parse_response(&response)
}

/// Parses one narrowly accepted `HTTP/1.0` or `HTTP/1.1` `200` response into
/// the configuration. Every other framing is an error rather than something to
/// decode.
fn parse_response(response: &[u8]) -> Result<FrigateConfig, ConfigFetchError> {
    let Some(head_end) = find_header_end(response) else {
        return Err(ConfigFetchError(
            "response ended before its header block did".to_owned(),
        ));
    };
    let mut lines = split_header_lines(&response[..head_end]).into_iter();
    let status_line = lines
        .next()
        .expect("split_header_lines always yields the first line");
    let status_line = utf8(status_line)
        .ok_or_else(|| ConfigFetchError("status line is not valid UTF-8".to_owned()))?;
    let mut fields = status_line.split(' ');
    let version = fields.next().unwrap_or_default();
    if (version != "HTTP/1.0" && version != "HTTP/1.1") || fields.next() != Some("200") {
        return Err(ConfigFetchError(format!(
            "status line {status_line:?} is not an HTTP/1.0 or HTTP/1.1 200"
        )));
    }

    let mut content_length: Option<usize> = None;
    for line in lines {
        let Some((name, value)) = split_at_byte(line, b':') else {
            return Err(ConfigFetchError(
                "response header line without a colon".to_owned(),
            ));
        };
        match utf8(name).map(str::to_ascii_lowercase).as_deref() {
            Some("transfer-encoding") => {
                return Err(ConfigFetchError(
                    "response uses a transfer encoding this reader does not decode".to_owned(),
                ));
            }
            Some("content-encoding") => {
                return Err(ConfigFetchError(
                    "response body carries a content encoding".to_owned(),
                ));
            }
            Some("content-length") => {
                let declared = std::str::from_utf8(trim_spaces(value))
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                    .ok_or_else(|| ConfigFetchError("Content-Length is not a number".to_owned()))?;
                content_length = Some(declared);
            }
            _ => {}
        }
    }

    let body = &response[head_end + 4..];
    if let Some(length) = content_length
        && body.len() != length
    {
        return Err(ConfigFetchError(format!(
            "response body is {} bytes but Content-Length said {length}",
            body.len()
        )));
    }
    serde_json::from_slice(body)
        .map_err(|err| ConfigFetchError(format!("response body is not a config: {err}")))
}
