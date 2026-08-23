//! RFC 2617 Digest authentication, server side, for [`super::MockCamera`].
//!
//! The mock never caches a single nonce across challenges: every time a
//! request fails to authenticate, a brand new nonce is issued and the
//! previous one stops being accepted. This reproduces directly-tested
//! IP camera RTSP servers' own observed behavior of re-challenging rather
//! than tolerating a stale-nonce retry.

use md5::{Digest as _, Md5};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const REALM: &str = "mock-camera";

static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generates an opaque token unpredictable enough for a test double's nonce
/// and session-id needs (not a cryptographic-strength requirement: this
/// server only ever talks to a test's own loopback client).
pub(super) fn fresh_token() -> String {
    let counter = TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let mut hasher = Md5::new();
    hasher.update(counter.to_be_bytes());
    hasher.update(now_nanos.to_be_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _: std::fmt::Result = write!(out, "{byte:02x}");
    }
    out
}

fn ha1(username: &str, password: &str) -> String {
    hex_encode(&Md5::digest(format!("{username}:{REALM}:{password}")))
}

fn ha2(method: &str, uri: &str) -> String {
    hex_encode(&Md5::digest(format!("{method}:{uri}")))
}

fn expected_response(
    username: &str,
    password: &str,
    nonce: &str,
    method: &str,
    uri: &str,
) -> String {
    let ha1 = ha1(username, password);
    let ha2 = ha2(method, uri);
    hex_encode(&Md5::digest(format!("{ha1}:{nonce}:{ha2}")))
}

/// The fields of a client's `Authorization: Digest ...` header.
pub(super) struct Credentials {
    username: String,
    nonce: String,
    uri: String,
    response: String,
}

/// Parses an `Authorization` header value, returning `None` for anything
/// that is not a well-formed `Digest` challenge response (including a
/// `Basic` scheme, which this mock never accepts).
pub(super) fn parse_authorization(value: &str) -> Option<Credentials> {
    let rest = value.strip_prefix("Digest ")?;
    let mut username = None;
    let mut nonce = None;
    let mut uri = None;
    let mut response = None;
    for field in rest.split(',') {
        let (key, raw_value) = field.trim().split_once('=')?;
        let value = raw_value.trim().trim_matches('"').to_string();
        match key.trim() {
            "username" => username = Some(value),
            "nonce" => nonce = Some(value),
            "uri" => uri = Some(value),
            "response" => response = Some(value),
            _ => {}
        }
    }
    Some(Credentials {
        username: username?,
        nonce: nonce?,
        uri: uri?,
        response: response?,
    })
}

/// Checks a parsed set of credentials against the last nonce this server
/// issued and the configured username/password. A nonce that does not match
/// exactly the most recently issued one is rejected, even if it was valid
/// against an earlier challenge.
pub(super) fn verify(
    credentials: &Credentials,
    last_nonce: Option<&str>,
    method: &str,
    request_uri: &str,
    username: &str,
    password: &str,
) -> bool {
    let Some(last_nonce) = last_nonce else {
        return false;
    };
    if credentials.nonce != last_nonce
        || credentials.username != username
        || credentials.uri != request_uri
    {
        return false;
    }
    let expected = expected_response(username, password, last_nonce, method, &credentials.uri);
    expected == credentials.response
}
