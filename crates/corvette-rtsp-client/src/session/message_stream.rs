//! Reads whole RTSP messages -- text requests/responses and `$`-prefixed
//! binary interleaved frames alike -- off one async byte stream.
//!
//! `rtsp_types::Message::parse` already implements the RFC 2326 §10.12
//! grammar that distinguishes a `$`-prefixed binary frame from a plain-text
//! response by its leading byte, incrementally (it reports
//! [`rtsp_types::ParseError::Incomplete`] rather than failing when the
//! buffer holds a partial message). This type is the thin async-read loop
//! around that parser: accumulate bytes, retry the parse, hand back
//! whichever message completed. Because both shapes go through the same
//! parse call on the same buffer, arrival order between a keep-alive's text
//! reply and the RTP frames flowing around it cannot desynchronize the
//! parser (INV-6).

use rtsp_types::{Message, ParseError};
use tokio::io::{AsyncRead, AsyncReadExt};

/// Bytes read per socket read call while accumulating a message. RTSP
/// control messages and one RTP frame are both far under this; it just
/// bounds how many read syscalls a large burst needs.
const READ_CHUNK: usize = 4096;

/// Wraps an [`AsyncRead`] half of an RTSP connection, buffering partial
/// reads until a complete [`Message`] is available.
#[derive(Debug)]
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
    async fn reads_a_text_response_delivered_in_one_chunk() {
        let response = b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n".to_vec();
        let mut stream = MessageStream::new(Cursor::new(response));

        let message = stream.read_message().await.expect("reads one message");
        assert!(matches!(message, RtspMessage::Response(_)));
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
    async fn demuxes_a_binary_frame_followed_by_a_text_response_regardless_of_order() {
        // INV-6: a keep-alive's text reply and RTP frames share one stream.
        // Build one buffer with a binary frame, then a text response, and
        // confirm each is read out correctly and in order -- neither
        // conflates with the other.
        let mut buffer = vec![b'$', 0, 0, 3];
        buffer.extend_from_slice(&[9, 9, 9]);
        buffer.extend_from_slice(b"RTSP/1.0 200 OK\r\nCSeq: 7\r\n\r\n");
        let mut stream = MessageStream::new(Cursor::new(buffer));

        match stream.read_message().await.expect("first message") {
            RtspMessage::Data(data) => assert_eq!(data.as_slice(), &[9, 9, 9]),
            other => panic!("expected the binary frame first, got {other:?}"),
        }
        match stream.read_message().await.expect("second message") {
            RtspMessage::Response(response) => {
                assert_eq!(response.status(), rtsp_types::StatusCode::Ok);
            }
            other => panic!("expected the text response second, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn demuxes_a_text_response_followed_by_a_binary_frame() {
        // Same property, reversed arrival order.
        let mut buffer = b"RTSP/1.0 200 OK\r\nCSeq: 7\r\n\r\n".to_vec();
        buffer.extend_from_slice(&[b'$', 1, 0, 2, 5, 6]);
        let mut stream = MessageStream::new(Cursor::new(buffer));

        match stream.read_message().await.expect("first message") {
            RtspMessage::Response(_) => {}
            other => panic!("expected the text response first, got {other:?}"),
        }
        match stream.read_message().await.expect("second message") {
            RtspMessage::Data(data) => {
                assert_eq!(data.channel_id(), 1);
                assert_eq!(data.as_slice(), &[5, 6]);
            }
            other => panic!("expected the binary frame second, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn accumulates_a_message_split_across_many_small_reads() {
        // Exercises the ParseError::Incomplete retry path directly, rather
        // than relying on a real socket happening to deliver bytes in
        // dribbles.
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

        let mut framed = vec![b'$', 2, 0, 3];
        framed.extend_from_slice(&[7, 8, 9]);
        let mut stream = MessageStream::new(OneByteAtATime(framed.into_iter()));

        let message = stream.read_message().await.expect("reads one message");
        match message {
            RtspMessage::Data(data) => {
                assert_eq!(data.channel_id(), 2);
                assert_eq!(data.as_slice(), &[7, 8, 9]);
            }
            other => panic!("expected a Data frame, got {other:?}"),
        }
    }
}
