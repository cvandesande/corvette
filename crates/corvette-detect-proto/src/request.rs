use crate::header::{DecodeError, Header, Kind};
use crate::{DEADLINE_LEN, HEADER_LEN, ID_LEN, TENSOR_BYTES};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request<'a> {
    InferRequest {
        id: u64,
        deadline_ms: u32,
        tensor: &'a [u8; TENSOR_BYTES],
    },
    InferRawRequest {
        id: u64,
        deadline_ms: u32,
        tensor: &'a [u8; TENSOR_BYTES],
    },
    HealthRequest {
        id: u64,
        deadline_ms: u32,
    },
}

impl<'a> Request<'a> {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let (kind, id, deadline_ms, tensor) = match self {
            Self::InferRequest {
                id,
                deadline_ms,
                tensor,
            } => (Kind::InferRequest, *id, *deadline_ms, Some(*tensor)),
            Self::InferRawRequest {
                id,
                deadline_ms,
                tensor,
            } => (Kind::InferRawRequest, *id, *deadline_ms, Some(*tensor)),
            Self::HealthRequest { id, deadline_ms } => {
                (Kind::HealthRequest, *id, *deadline_ms, None)
            }
        };
        let body_len = ID_LEN + DEADLINE_LEN + if tensor.is_some() { TENSOR_BYTES } else { 0 };
        let mut out = Vec::with_capacity(HEADER_LEN + body_len);
        kind.push_header(body_len, &mut out);
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&deadline_ms.to_le_bytes());
        if let Some(tensor) = tensor {
            out.extend_from_slice(tensor);
        }
        out
    }

    /// # Errors
    /// Returns a `DecodeError` when the frame is not a well-formed request frame.
    pub fn decode(frame: &'a [u8]) -> Result<Self, DecodeError> {
        let (header, body) = Header::from_frame(frame)?;
        if matches!(
            header.kind,
            Kind::RowsReply | Kind::RawReply | Kind::HealthReply | Kind::ErrorReply
        ) {
            return Err(DecodeError::UnexpectedKind(header.kind));
        }
        header.check_body(body)?;
        match header.kind {
            Kind::InferRequest => {
                let (id, deadline_ms, tensor) = decode_tensor_body(body)?;
                Ok(Self::InferRequest {
                    id,
                    deadline_ms,
                    tensor,
                })
            }
            Kind::InferRawRequest => {
                let (id, deadline_ms, tensor) = decode_tensor_body(body)?;
                Ok(Self::InferRawRequest {
                    id,
                    deadline_ms,
                    tensor,
                })
            }
            Kind::HealthRequest => {
                let required = ID_LEN + DEADLINE_LEN;
                if body.len() != required {
                    return Err(DecodeError::BodyLength {
                        expected: required,
                        found: body.len(),
                    });
                }
                let (id_bytes, deadline_bytes) = body
                    .split_at_checked(ID_LEN)
                    .ok_or(DecodeError::Truncated)?;
                Ok(Self::HealthRequest {
                    id: Header::read_id(id_bytes).ok_or(DecodeError::Truncated)?,
                    deadline_ms: Header::read_deadline(deadline_bytes)
                        .ok_or(DecodeError::Truncated)?,
                })
            }
            Kind::RowsReply | Kind::RawReply | Kind::HealthReply | Kind::ErrorReply => {
                Err(DecodeError::UnexpectedKind(header.kind))
            }
        }
    }
}

fn decode_tensor_body(body: &[u8]) -> Result<(u64, u32, &[u8; TENSOR_BYTES]), DecodeError> {
    let required = ID_LEN + DEADLINE_LEN + TENSOR_BYTES;
    if body.len() != required {
        return Err(DecodeError::BodyLength {
            expected: required,
            found: body.len(),
        });
    }
    let (head, tensor) = body
        .split_at_checked(ID_LEN + DEADLINE_LEN)
        .ok_or(DecodeError::Truncated)?;
    let deadline_bytes = head
        .split_at_checked(ID_LEN)
        .ok_or(DecodeError::Truncated)?
        .1;
    let tensor = <&[u8; TENSOR_BYTES]>::try_from(tensor).map_err(|_| DecodeError::BodyLength {
        expected: TENSOR_BYTES,
        found: tensor.len(),
    })?;
    Ok((
        Header::read_id(head).ok_or(DecodeError::Truncated)?,
        Header::read_deadline(deadline_bytes).ok_or(DecodeError::Truncated)?,
        tensor,
    ))
}
