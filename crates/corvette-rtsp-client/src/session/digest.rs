//! Client-side RFC 2617 Digest authentication.
//!
//! Every camera this project has tested (real Reolink hardware, and the
//! `mock_camera` test double built to match it) issues an unqualified
//! challenge: a `realm` and a `nonce`, no `qop`. This module implements only
//! that form. A camera whose challenge carries `qop` is rejected with a clear
//! error rather than silently computing a response against the wrong
//! algorithm -- nothing in this project's own testing has exercised `qop`,
//! so there is no way to validate an implementation of it.

use md5::{Digest as _, Md5};
use std::fmt::Write as _;

/// A parsed `WWW-Authenticate: Digest ...` challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Challenge {
    pub realm: String,
    pub nonce: String,
}

/// Errors from parsing a `WWW-Authenticate` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChallengeError {
    /// The header did not use the `Digest` scheme at all (e.g. `Basic`).
    NotDigest,
    /// The challenge is missing `realm` or `nonce`.
    MissingField(&'static str),
    /// The challenge declares `qop`, which this implementation does not
    /// support (see module docs).
    UnsupportedQop,
}

impl std::fmt::Display for ChallengeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotDigest => write!(f, "WWW-Authenticate challenge is not a Digest challenge"),
            Self::MissingField(field) => {
                write!(f, "Digest challenge is missing required field '{field}'")
            }
            Self::UnsupportedQop => {
                write!(f, "Digest challenge declares qop, which is not supported")
            }
        }
    }
}

impl std::error::Error for ChallengeError {}

/// Parses a `WWW-Authenticate` header value into a [`Challenge`].
pub(super) fn parse_challenge(header_value: &str) -> Result<Challenge, ChallengeError> {
    let rest = header_value
        .trim()
        .strip_prefix("Digest ")
        .ok_or(ChallengeError::NotDigest)?;

    let mut realm = None;
    let mut nonce = None;
    let mut has_qop = false;

    for field in rest.split(',') {
        let Some((key, value)) = field.trim().split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "realm" => realm = Some(value.to_string()),
            "nonce" => nonce = Some(value.to_string()),
            "qop" => has_qop = true,
            _ => {}
        }
    }

    if has_qop {
        return Err(ChallengeError::UnsupportedQop);
    }

    Ok(Challenge {
        realm: realm.ok_or(ChallengeError::MissingField("realm"))?,
        nonce: nonce.ok_or(ChallengeError::MissingField("nonce"))?,
    })
}

/// Credentials for a camera, plus the challenge most recently accepted on
/// this connection. Recomputes a fresh `response=` value (HA2 depends on the
/// method and URI) for every request; the `nonce` and `realm` are reused
/// as-is until the camera issues a new challenge.
///
/// `Debug` deliberately redacts `password`.
#[derive(Clone)]
pub(super) struct DigestAuth {
    username: String,
    password: String,
    challenge: Challenge,
}

impl std::fmt::Debug for DigestAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DigestAuth")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .field("challenge", &self.challenge)
            .finish()
    }
}

impl DigestAuth {
    pub(super) const fn new(username: String, password: String, challenge: Challenge) -> Self {
        Self {
            username,
            password,
            challenge,
        }
    }

    /// Replaces the accepted challenge, e.g. after the camera issues a fresh
    /// nonce.
    pub(super) fn set_challenge(&mut self, challenge: Challenge) {
        self.challenge = challenge;
    }

    /// Builds an `Authorization: Digest ...` header value for `method` and
    /// `uri`.
    pub(super) fn authorization_header(&self, method: &str, uri: &str) -> String {
        let ha1 = hex_md5(format!(
            "{}:{}:{}",
            self.username, self.challenge.realm, self.password
        ));
        let ha2 = hex_md5(format!("{method}:{uri}"));
        let response = hex_md5(format!("{ha1}:{}:{ha2}", self.challenge.nonce));

        format!(
            r#"Digest username="{}", realm="{}", nonce="{}", uri="{}", response="{}""#,
            self.username, self.challenge.realm, self.challenge.nonce, uri, response
        )
    }
}

fn hex_md5(input: impl AsRef<[u8]>) -> String {
    let digest = Md5::digest(input);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _: std::fmt::Result = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_realm_and_nonce_challenge() {
        let challenge =
            parse_challenge(r#"Digest realm="mock-camera", nonce="abc123""#).expect("parses");
        assert_eq!(challenge.realm, "mock-camera");
        assert_eq!(challenge.nonce, "abc123");
    }

    #[test]
    fn rejects_a_non_digest_scheme() {
        assert_eq!(
            parse_challenge("Basic realm=\"mock-camera\""),
            Err(ChallengeError::NotDigest)
        );
    }

    #[test]
    fn rejects_a_qop_challenge_rather_than_silently_mis_authenticating() {
        assert_eq!(
            parse_challenge(r#"Digest realm="r", nonce="n", qop="auth""#),
            Err(ChallengeError::UnsupportedQop)
        );
    }

    #[test]
    fn computes_the_documented_rfc_2617_response() {
        // Cross-checked against the independent implementation in
        // `mock_camera::digest` (server side): both compute
        // MD5(MD5(user:realm:pass):nonce:MD5(method:uri)).
        let auth = DigestAuth::new(
            "admin".to_string(),
            "test-password-not-real".to_string(),
            Challenge {
                realm: "mock-camera".to_string(),
                nonce: "noncevalue".to_string(),
            },
        );
        let header = auth.authorization_header("DESCRIBE", "rtsp://127.0.0.1/stream/");

        let ha1 = hex_md5("admin:mock-camera:test-password-not-real");
        let ha2 = hex_md5("DESCRIBE:rtsp://127.0.0.1/stream/");
        let expected_response = hex_md5(format!("{ha1}:noncevalue:{ha2}"));

        assert!(header.contains(&format!(r#"response="{expected_response}""#)));
    }

    #[test]
    fn digest_auth_debug_never_prints_the_plaintext_password() {
        let auth = DigestAuth::new(
            "admin".to_string(),
            "test-password-not-real".to_string(),
            Challenge {
                realm: "mock-camera".to_string(),
                nonce: "noncevalue".to_string(),
            },
        );
        let debug = format!("{auth:?}");
        assert!(debug.contains("admin"));
        assert!(
            !debug.contains("test-password-not-real"),
            "Debug output must redact the password: {debug}"
        );
    }
}
