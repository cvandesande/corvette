use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use corvette_detect::{Decode, FrameClock, FrameReader, FrameSpec, ReadError};
use tokio::io::{AsyncRead, AsyncWriteExt, duplex};

fn spec() -> FrameSpec {
    FrameSpec::new(4, 2, 5, Decode::Cpu).expect("valid spec")
}

#[tokio::test]
async fn whole_frames_are_returned_with_their_index_and_replay_time() {
    let clock = FrameClock::replay(Duration::from_secs(1_000), spec());
    let data: Vec<u8> = (0u8..36).collect();
    let mut reader = FrameReader::new(data.as_slice(), spec(), clock);
    let first = reader
        .next_frame()
        .await
        .expect("read")
        .expect("frame present");
    assert_eq!(first.data, (0u8..12).collect::<Vec<u8>>());
    assert_eq!(first.index, 0);
    assert_eq!(first.time, Duration::from_secs(1_000));
    let second = reader
        .next_frame()
        .await
        .expect("read")
        .expect("frame present");
    assert_eq!(second.data, (12u8..24).collect::<Vec<u8>>());
    assert_eq!(second.index, 1);
    assert_eq!(second.time, Duration::from_millis(1_000_200));
    let third = reader
        .next_frame()
        .await
        .expect("read")
        .expect("frame present");
    assert_eq!(third.data, (24u8..36).collect::<Vec<u8>>());
    assert_eq!(third.index, 2);
    assert_eq!(third.time, Duration::from_millis(1_000_400));
    assert!(reader.next_frame().await.expect("read").is_none());
    assert!(reader.next_frame().await.expect("read").is_none());
}

#[tokio::test]
async fn an_empty_stream_ends_cleanly() {
    let data: &[u8] = &[];
    let mut reader = FrameReader::new(data, spec(), FrameClock::live());
    assert!(reader.next_frame().await.expect("read").is_none());
}

#[tokio::test]
async fn a_stream_that_ends_inside_a_frame_is_a_truncation_error() {
    let data: Vec<u8> = (0u8..30).collect();
    let mut reader = FrameReader::new(data.as_slice(), spec(), FrameClock::live());
    assert!(reader.next_frame().await.expect("read").is_some());
    assert!(reader.next_frame().await.expect("read").is_some());
    let result = reader.next_frame().await;
    assert!(matches!(
        result,
        Err(ReadError::Truncated { got: 6, want: 12 })
    ));
    assert_eq!(
        result.expect_err("truncated").to_string(),
        "stream ended inside a frame: got 6 of 12 bytes"
    );
}

#[tokio::test]
async fn a_stream_delivering_one_byte_at_a_time_still_yields_whole_frames() {
    let (mut writer, reader) = duplex(1);
    let writer_task = tokio::spawn(async move {
        let bytes: Vec<u8> = (0u8..24).collect();
        writer.write_all(&bytes).await.expect("write");
    });
    let mut reader = FrameReader::new(reader, spec(), FrameClock::live());
    let first = reader
        .next_frame()
        .await
        .expect("read")
        .expect("frame present");
    assert_eq!(first.data, (0u8..12).collect::<Vec<u8>>());
    let second = reader
        .next_frame()
        .await
        .expect("read")
        .expect("frame present");
    assert_eq!(second.data, (12u8..24).collect::<Vec<u8>>());
    assert!(reader.next_frame().await.expect("read").is_none());
    writer_task.await.expect("writer task");
}

#[tokio::test]
async fn a_cancelled_read_keeps_the_bytes_already_collected() {
    let (mut writer, reader) = duplex(64);
    writer
        .write_all(&(0u8..5).collect::<Vec<u8>>())
        .await
        .expect("write five");
    let mut reader = FrameReader::new(reader, spec(), FrameClock::live());
    let timed_out = tokio::time::timeout(Duration::from_millis(50), reader.next_frame()).await;
    assert!(timed_out.is_err());
    writer
        .write_all(&(5u8..12).collect::<Vec<u8>>())
        .await
        .expect("write seven");
    let frame = tokio::time::timeout(Duration::from_secs(5), reader.next_frame())
        .await
        .expect("the second read finished in time")
        .expect("read")
        .expect("frame");
    assert_eq!(frame.data, (0u8..12).collect::<Vec<u8>>());
    assert_eq!(frame.index, 0);
}

#[derive(Debug)]
struct FailingStream;

impl AsyncRead for FailingStream {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::Error::other("boom")))
    }
}

#[tokio::test]
async fn an_io_error_is_reported_as_an_io_error() {
    let mut reader = FrameReader::new(FailingStream, spec(), FrameClock::live());
    let result = reader.next_frame().await;
    assert!(matches!(result, Err(ReadError::Io(_))));
    assert_eq!(
        result.expect_err("io error").to_string(),
        "frame read failed: boom"
    );
}

#[tokio::test]
async fn a_live_clock_stamps_each_frame_when_it_completes() {
    let data: Vec<u8> = (0u8..24).collect();
    let mut reader = FrameReader::new(data.as_slice(), spec(), FrameClock::live());
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch");
    let first = reader
        .next_frame()
        .await
        .expect("read")
        .expect("frame present");
    let second = reader
        .next_frame()
        .await
        .expect("read")
        .expect("frame present");
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch");
    assert!(first.time >= before && first.time <= after);
    assert!(second.time >= before && second.time <= after);
    assert!(second.time >= first.time);
}
