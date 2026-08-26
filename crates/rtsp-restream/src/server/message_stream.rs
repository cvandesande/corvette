//! Reads whole RTSP messages -- text requests and `$`-prefixed binary
//! interleaved frames alike -- off one accepted connection's async byte
//! stream.
//!
//! `rtsp_types::Message::parse` already implements the RFC 2326 §10.12
//! grammar that distinguishes a `$`-prefixed binary frame from a plain-text
//! request by its leading byte, incrementally (it reports
//! [`rtsp_types::ParseError::Incomplete`] rather than failing when the
//! buffer holds a partial message). This type is the thin async-read loop
//! around that parser: accumulate bytes, retry the parse, hand back
//! whichever message completed.
//!
//! This is this crate's own server-side counterpart to issue #18's own RTSP
//! client crate's private client-side message-stream module (which reads
//! responses/data off a camera connection); the read-loop shape is the
//! same, but this crate is never in a position to reuse that module
//! directly -- it is private to a sibling crate this crate depends on only
//! as a dev-dependency (test-only correctness oracle, not a runtime
//! dependency; see this crate's own `Cargo.toml`) -- so this crate carries
//! its own copy.

use rtsp_types::{Message, ParseError};
use tokio::io::{AsyncRead, AsyncReadExt};

/// Bytes read per socket read call while accumulating a message. One RTSP
/// control message or one RTP frame are both far under this; it just bounds
/// how many read syscalls a large burst needs.
const READ_CHUNK: usize = 4096;

/// Wraps an [`AsyncRead`] half of an RTSP connection, buffering partial
/// reads until a complete [`Message`] is available.
pub(super) struct MessageStream<R> {
    reader: R,
    buffer: Vec<u8>,
}

/// Errors from reading the next message off the stream.
#[derive(Debug)]
pub(super) enum StreamError {
    /// The peer closed the connection (a clean EOF) before a full message
    /// arrived.
    ConnectionClosed,
    /// The bytes on the wire do not parse as any known RTSP message shape.
    Malformed,
    Io(std::io::Error),
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionClosed => {
                write!(f, "RTSP connection closed before a full message arrived")
            }
            Self::Malformed => write!(f, "malformed RTSP message"),
            Self::Io(err) => write!(f, "RTSP connection I/O error: {err}"),
        }
    }
}

impl std::error::Error for StreamError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::ConnectionClosed | Self::Malformed => None,
        }
    }
}

impl From<std::io::Error> for StreamError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl<R: AsyncRead + Unpin> MessageStream<R> {
    pub(super) const fn new(reader: R) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
        }
    }

    /// Reads and returns the next complete message, which may be a
    /// `Request`, a `Response`, or a binary `Data` frame -- reading more
    /// bytes off the underlying stream as needed.
    pub(super) async fn read_message(&mut self) -> Result<Message<Vec<u8>>, StreamError> {
        loop {
            match Message::<Vec<u8>>::parse(&self.buffer) {
                Ok((message, consumed)) => {
                    self.buffer.drain(0..consumed);
                    return Ok(message);
                }
                Err(ParseError::Incomplete(_)) => {
                    let mut chunk = [0_u8; READ_CHUNK];
                    let read = self.reader.read(&mut chunk).await?;
                    if read == 0 {
                        return Err(StreamError::ConnectionClosed);
                    }
                    self.buffer.extend_from_slice(&chunk[..read]);
                }
                Err(ParseError::Error) => return Err(StreamError::Malformed),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtsp_types::Message as RtspMessage;
    use std::io::Cursor;

    #[tokio::test]
    async fn reads_a_text_request_delivered_in_one_chunk() {
        let request = b"OPTIONS rtsp://127.0.0.1/cam RTSP/1.0\r\nCSeq: 1\r\n\r\n".to_vec();
        let mut stream = MessageStream::new(Cursor::new(request));

        let message = stream.read_message().await.expect("reads one message");
        assert!(matches!(message, RtspMessage::Request(_)));
    }

    #[tokio::test]
    async fn reads_a_binary_data_frame_delivered_in_one_chunk() {
        let mut framed = vec![b'$', 0, 0, 4];
        framed.extend_from_slice(&[1, 2, 3, 4]);
        let mut stream = MessageStream::new(Cursor::new(framed));

        let message = stream.read_message().await.expect("reads one message");
        match message {
            RtspMessage::Data(data) => assert_eq!(data.as_slice(), &[1, 2, 3, 4]),
            other => panic!("expected a Data frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn accumulates_a_message_split_across_many_small_reads() {
        struct OneByteAtATime(std::vec::IntoIter<u8>);

        impl AsyncRead for OneByteAtATime {
            fn poll_read(
                mut self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
                buf: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                if let Some(byte) = self.0.next() {
                    buf.put_slice(&[byte]);
                }
                std::task::Poll::Ready(Ok(()))
            }
        }

        let request = b"OPTIONS rtsp://127.0.0.1/cam RTSP/1.0\r\nCSeq: 1\r\n\r\n".to_vec();
        let mut stream = MessageStream::new(OneByteAtATime(request.into_iter()));

        let message = stream.read_message().await.expect("reads one message");
        assert!(matches!(message, RtspMessage::Request(_)));
    }

    #[tokio::test]
    async fn reports_connection_closed_on_a_clean_eof_before_any_message() {
        let mut stream = MessageStream::new(Cursor::new(Vec::new()));

        let err = stream.read_message().await.expect_err("no bytes at all");
        assert!(matches!(err, StreamError::ConnectionClosed));
    }
}
