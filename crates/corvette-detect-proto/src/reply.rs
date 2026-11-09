use std::fmt;

use crate::header::{DecodeError, Header, Kind};
use crate::{HEADER_LEN, ID_LEN, MAX_ROWS, RAW_VALUES, ROW_LEN};

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub class_id: u32,
    pub score: f32,
    pub y_min: f32,
    pub x_min: f32,
    pub y_max: f32,
    pub x_max: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    DeviceLost,
    DeadlineOverrun,
    BadRequest,
}

impl ErrorCode {
    #[must_use]
    pub const fn byte(self) -> u8 {
        match self {
            Self::DeviceLost => 1,
            Self::DeadlineOverrun => 2,
            Self::BadRequest => 3,
        }
    }

    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::DeviceLost),
            2 => Some(Self::DeadlineOverrun),
            3 => Some(Self::BadRequest),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    RowCount(usize),
    RawLength(usize),
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RowCount(count) => write!(f, "row count {count} exceeds {MAX_ROWS}"),
            Self::RawLength(len) => write!(f, "raw value count {len} differs from {RAW_VALUES}"),
        }
    }
}

impl std::error::Error for EncodeError {}

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Rows { id: u64, rows: Vec<Row> },
    Raw { id: u64, values: Vec<f32> },
    Health { id: u64 },
    Error { id: u64, code: ErrorCode },
}

impl Reply {
    /// # Errors
    /// Returns an `EncodeError` when the payload cannot be described within the wire limits.
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        match self {
            Self::Rows { id, rows } => {
                if rows.len() > MAX_ROWS {
                    return Err(EncodeError::RowCount(rows.len()));
                }
                let count = u8::try_from(rows.len()).unwrap_or(0);
                let body_len = ID_LEN + 1 + usize::from(count) * ROW_LEN;
                let mut out = Vec::with_capacity(HEADER_LEN + body_len);
                Kind::RowsReply.push_header(body_len, &mut out);
                out.extend_from_slice(&id.to_le_bytes());
                out.push(count);
                for row in rows {
                    out.extend_from_slice(&row.class_id.to_le_bytes());
                    for value in [row.score, row.y_min, row.x_min, row.y_max, row.x_max] {
                        out.extend_from_slice(&value.to_le_bytes());
                    }
                }
                Ok(out)
            }
            Self::Raw { id, values } => {
                if values.len() != RAW_VALUES {
                    return Err(EncodeError::RawLength(values.len()));
                }
                let body_len = ID_LEN + RAW_VALUES * 4;
                let mut out = Vec::with_capacity(HEADER_LEN + body_len);
                Kind::RawReply.push_header(body_len, &mut out);
                out.extend_from_slice(&id.to_le_bytes());
                for value in values {
                    out.extend_from_slice(&value.to_le_bytes());
                }
                Ok(out)
            }
            Self::Health { id } => {
                let mut out = Vec::with_capacity(HEADER_LEN + ID_LEN);
                Kind::HealthReply.push_header(ID_LEN, &mut out);
                out.extend_from_slice(&id.to_le_bytes());
                Ok(out)
            }
            Self::Error { id, code } => {
                let body_len = ID_LEN + 1;
                let mut out = Vec::with_capacity(HEADER_LEN + body_len);
                Kind::ErrorReply.push_header(body_len, &mut out);
                out.extend_from_slice(&id.to_le_bytes());
                out.push(code.byte());
                Ok(out)
            }
        }
    }

    /// # Errors
    /// Returns a `DecodeError` when the frame is not a well-formed reply frame.
    pub fn decode(frame: &[u8]) -> Result<Self, DecodeError> {
        let (header, body) = Header::from_frame(frame)?;
        if matches!(
            header.kind,
            Kind::InferRequest | Kind::InferRawRequest | Kind::HealthRequest
        ) {
            return Err(DecodeError::UnexpectedKind(header.kind));
        }
        header.check_body(body)?;
        match header.kind {
            Kind::RowsReply => decode_rows(body),
            Kind::RawReply => decode_raw(body),
            Kind::HealthReply => {
                if body.len() != ID_LEN {
                    return Err(DecodeError::BodyLength {
                        expected: ID_LEN,
                        found: body.len(),
                    });
                }
                Ok(Self::Health {
                    id: Header::read_id(body).ok_or(DecodeError::Truncated)?,
                })
            }
            Kind::ErrorReply => {
                let required = ID_LEN + 1;
                if body.len() != required {
                    return Err(DecodeError::BodyLength {
                        expected: required,
                        found: body.len(),
                    });
                }
                let code_byte = body.get(ID_LEN).copied().ok_or(DecodeError::Truncated)?;
                Ok(Self::Error {
                    id: Header::read_id(body).ok_or(DecodeError::Truncated)?,
                    code: ErrorCode::from_byte(code_byte)
                        .ok_or(DecodeError::ErrorCode(code_byte))?,
                })
            }
            Kind::InferRequest | Kind::InferRawRequest | Kind::HealthRequest => {
                Err(DecodeError::UnexpectedKind(header.kind))
            }
        }
    }
}

fn decode_rows(body: &[u8]) -> Result<Reply, DecodeError> {
    if body.len() < ID_LEN + 1 {
        return Err(DecodeError::BodyLength {
            expected: ID_LEN + 1,
            found: body.len(),
        });
    }
    let count = usize::from(body.get(ID_LEN).copied().ok_or(DecodeError::Truncated)?);
    if count > MAX_ROWS {
        return Err(DecodeError::RowCount(count));
    }
    let required = ID_LEN + 1 + count * ROW_LEN;
    if body.len() != required {
        return Err(DecodeError::BodyLength {
            expected: required,
            found: body.len(),
        });
    }
    let row_bytes = body
        .split_at_checked(ID_LEN + 1)
        .ok_or(DecodeError::Truncated)?
        .1;
    let mut rows = Vec::with_capacity(count);
    for chunk in row_bytes.chunks_exact(ROW_LEN) {
        rows.push(row_from_bytes(chunk).ok_or(DecodeError::Truncated)?);
    }
    Ok(Reply::Rows {
        id: Header::read_id(body).ok_or(DecodeError::Truncated)?,
        rows,
    })
}

fn row_from_bytes(chunk: &[u8]) -> Option<Row> {
    let (class_bytes, rest) = chunk.split_at_checked(4)?;
    let class_id = u32::from_le_bytes(*class_bytes.first_chunk::<4>()?);
    let (score_bytes, rest) = rest.split_at_checked(4)?;
    let (y_min_bytes, rest) = rest.split_at_checked(4)?;
    let (x_min_bytes, rest) = rest.split_at_checked(4)?;
    let (y_max_bytes, x_max_bytes) = rest.split_at_checked(4)?;
    let [score, y_min, x_min, y_max, x_max] = [
        f32::from_le_bytes(*score_bytes.first_chunk::<4>()?),
        f32::from_le_bytes(*y_min_bytes.first_chunk::<4>()?),
        f32::from_le_bytes(*x_min_bytes.first_chunk::<4>()?),
        f32::from_le_bytes(*y_max_bytes.first_chunk::<4>()?),
        f32::from_le_bytes(*x_max_bytes.first_chunk::<4>()?),
    ];
    Some(Row {
        class_id,
        score,
        y_min,
        x_min,
        y_max,
        x_max,
    })
}

fn decode_raw(body: &[u8]) -> Result<Reply, DecodeError> {
    let required = ID_LEN + RAW_VALUES * 4;
    if body.len() != required {
        return Err(DecodeError::BodyLength {
            expected: required,
            found: body.len(),
        });
    }
    let value_bytes = body
        .split_at_checked(ID_LEN)
        .ok_or(DecodeError::Truncated)?
        .1;
    let (words, leftover) = value_bytes.as_chunks::<4>();
    if !leftover.is_empty() {
        return Err(DecodeError::BodyLength {
            expected: required,
            found: body.len(),
        });
    }
    let values: Vec<f32> = words
        .iter()
        .copied()
        .map(u32::from_le_bytes)
        .map(f32::from_bits)
        .collect();
    Ok(Reply::Raw {
        id: Header::read_id(body).ok_or(DecodeError::Truncated)?,
        values,
    })
}
