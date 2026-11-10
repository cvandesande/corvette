//! Reads whole frames of a fixed size from an asynchronous byte stream, numbering and stamping them.

use std::io;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};

use crate::clock::FrameClock;
use crate::ffmpeg::FrameSpec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub index: u64,
    pub time: Duration,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    Truncated { got: usize, want: usize },
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "frame read failed: {error}"),
            Self::Truncated { got, want } => {
                write!(f, "stream ended inside a frame: got {got} of {want} bytes")
            }
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Truncated { .. } => None,
        }
    }
}

impl From<io::Error> for ReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug)]
pub struct FrameReader<R> {
    reader: R,
    clock: FrameClock,
    frame_bytes: usize,
    buffer: Vec<u8>,
    filled: usize,
    next_index: u64,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    #[must_use]
    pub fn new(reader: R, spec: FrameSpec, clock: FrameClock) -> Self {
        let frame_bytes = spec.frame_bytes();
        Self {
            reader,
            clock,
            frame_bytes,
            buffer: vec![0; frame_bytes],
            filled: 0,
            next_index: 0,
        }
    }

    /// # Errors
    /// Returns [`ReadError::Io`] when the stream fails, [`ReadError::Truncated`] when it ends inside a frame.
    pub async fn next_frame(&mut self) -> Result<Option<Frame>, ReadError> {
        while self.filled < self.buffer.len() {
            let read = self.reader.read(&mut self.buffer[self.filled..]).await?;
            if read == 0 {
                if self.filled == 0 {
                    return Ok(None);
                }
                return Err(ReadError::Truncated {
                    got: self.filled,
                    want: self.buffer.len(),
                });
            }
            self.filled += read;
        }
        let data = std::mem::replace(&mut self.buffer, vec![0; self.frame_bytes]);
        self.filled = 0;
        let index = self.next_index;
        self.next_index = index.saturating_add(1);
        let frame = Frame {
            index,
            time: self.clock.stamp(index),
            data,
        };
        Ok(Some(frame))
    }
}
