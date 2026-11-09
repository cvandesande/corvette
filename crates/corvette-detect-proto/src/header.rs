use std::fmt;

use crate::{DEADLINE_LEN, HEADER_LEN, ID_LEN, MAX_BODY, MAX_ROWS, VERSION};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    InferRequest,
    InferRawRequest,
    HealthRequest,
    RowsReply,
    RawReply,
    HealthReply,
    ErrorReply,
}

impl Kind {
    #[must_use]
    pub const fn byte(self) -> u8 {
        match self {
            Self::InferRequest => 0x01,
            Self::InferRawRequest => 0x02,
            Self::HealthRequest => 0x03,
            Self::RowsReply => 0x81,
            Self::RawReply => 0x82,
            Self::HealthReply => 0x83,
            Self::ErrorReply => 0x8F,
        }
    }

    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(Self::InferRequest),
            0x02 => Some(Self::InferRawRequest),
            0x03 => Some(Self::HealthRequest),
            0x81 => Some(Self::RowsReply),
            0x82 => Some(Self::RawReply),
            0x83 => Some(Self::HealthReply),
            0x8F => Some(Self::ErrorReply),
            _ => None,
        }
    }

    pub(crate) fn push_header(self, body_len: usize, out: &mut Vec<u8>) {
        out.extend_from_slice(&[VERSION, self.byte()]);
        out.extend_from_slice(&u32::try_from(body_len).unwrap_or(u32::MAX).to_le_bytes());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub kind: Kind,
    pub body_len: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeaderError {
    Version(u8),
    Kind(u8),
    BodyTooLarge(u32),
}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Version(byte) => write!(f, "unknown protocol version {byte}"),
            Self::Kind(byte) => write!(f, "unknown message kind byte {byte}"),
            Self::BodyTooLarge(declared) => {
                write!(f, "declared body length {declared} exceeds {MAX_BODY}")
            }
        }
    }
}

impl std::error::Error for HeaderError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    Truncated,
    Header(HeaderError),
    UnexpectedKind(Kind),
    BodyLength { expected: usize, found: usize },
    RowCount(usize),
    ErrorCode(u8),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "frame is shorter than the header"),
            Self::Header(error) => write!(f, "{error}"),
            Self::UnexpectedKind(kind) => {
                write!(f, "frame carries the other direction's kind {kind:?}")
            }
            Self::BodyLength { expected, found } => {
                write!(
                    f,
                    "body length mismatch: expected {expected}, found {found}"
                )
            }
            Self::RowCount(count) => write!(f, "row count {count} exceeds {MAX_ROWS}"),
            Self::ErrorCode(byte) => write!(f, "unknown error code byte {byte}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// # Errors
/// Returns the first header fault found, checked in the order version, kind, declared length.
pub fn parse_header(bytes: &[u8; HEADER_LEN]) -> Result<Header, HeaderError> {
    let &[version, kind_byte, len0, len1, len2, len3] = bytes;
    if version != VERSION {
        return Err(HeaderError::Version(version));
    }
    let kind = Kind::from_byte(kind_byte).ok_or(HeaderError::Kind(kind_byte))?;
    let declared = u32::from_le_bytes([len0, len1, len2, len3]);
    let max_body = u32::try_from(MAX_BODY).unwrap_or(u32::MAX);
    if declared > max_body {
        return Err(HeaderError::BodyTooLarge(declared));
    }
    Ok(Header {
        kind,
        body_len: usize::try_from(declared).unwrap_or(usize::MAX),
    })
}

impl Header {
    pub(crate) fn from_frame(frame: &[u8]) -> Result<(Self, &[u8]), DecodeError> {
        let (head, body) = frame
            .split_at_checked(HEADER_LEN)
            .ok_or(DecodeError::Truncated)?;
        let header = parse_header(head.first_chunk().ok_or(DecodeError::Truncated)?)
            .map_err(DecodeError::Header)?;
        Ok((header, body))
    }

    pub(crate) const fn check_body(&self, body: &[u8]) -> Result<(), DecodeError> {
        if body.len() != self.body_len {
            return Err(DecodeError::BodyLength {
                expected: self.body_len,
                found: body.len(),
            });
        }
        Ok(())
    }

    pub(crate) fn read_id(body: &[u8]) -> Option<u64> {
        Some(u64::from_le_bytes(*body.first_chunk::<ID_LEN>()?))
    }

    pub(crate) fn read_deadline(body: &[u8]) -> Option<u32> {
        Some(u32::from_le_bytes(*body.first_chunk::<DEADLINE_LEN>()?))
    }
}
