//! Assembles one camera stream's `rtsp` address from its source table and a secret lookup.

use std::fmt;

use crate::config::StreamSource;

const HEX_DIGITS: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F',
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamKind {
    Main,
    Sub,
}

impl StreamKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Sub => "sub",
        }
    }
}

impl fmt::Display for StreamKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamAddressError {
    pub camera: String,
    pub kind: StreamKind,
    pub key: &'static str,
    pub rule: &'static str,
}

impl StreamAddressError {
    fn secret_refusal(camera: &str, kind: StreamKind, rule: &'static str) -> Self {
        Self {
            camera: camera.to_owned(),
            kind,
            key: "secret_env",
            rule,
        }
    }
}

impl fmt::Display for StreamAddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "camera {}: source {}: {}: {}",
            self.camera, self.kind, self.key, self.rule
        )
    }
}

impl std::error::Error for StreamAddressError {}

/// # Errors
/// `StreamAddressError` when `secret_env` names a variable the lookup cannot answer.
pub fn resolve_stream_url(
    camera: &str,
    kind: StreamKind,
    stream: &StreamSource,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<String, StreamAddressError> {
    let password = resolve_secret(stream, camera, kind, &lookup)?;
    let mut address = String::from("rtsp://");
    let has_username = !stream.username.is_empty();
    if has_username || password.is_some() {
        if has_username {
            address.push_str(&percent_encode(&stream.username));
        }
        if let Some(secret) = &password {
            address.push(':');
            address.push_str(&percent_encode(secret));
        }
        address.push('@');
    }
    address.push_str(&stream.host);
    address.push(':');
    address.push_str(&stream.port.to_string());
    address.push_str(&stream.path);
    Ok(address)
}

fn resolve_secret(
    stream: &StreamSource,
    camera: &str,
    kind: StreamKind,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<Option<String>, StreamAddressError> {
    let Some(variable) = &stream.secret_env else {
        return Ok(None);
    };
    let Some(secret) = lookup(variable) else {
        return Err(StreamAddressError::secret_refusal(
            camera,
            kind,
            "variable is not set",
        ));
    };
    if secret.is_empty() {
        return Err(StreamAddressError::secret_refusal(
            camera,
            kind,
            "variable is empty",
        ));
    }
    Ok(Some(secret))
}

fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(*byte));
            }
            _ => {
                encoded.push('%');
                encoded.push(HEX_DIGITS[usize::from(byte >> 4)]);
                encoded.push(HEX_DIGITS[usize::from(byte & 0x0F)]);
            }
        }
    }
    encoded
}
