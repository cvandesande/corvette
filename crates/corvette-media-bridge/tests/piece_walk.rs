use bytes::Bytes;
use corvette_media_bridge::fmp4::{Fragmenter, InitSegmentTracker};
use corvette_media_bridge::piece_walk::{self, WalkError};
use corvette_rtsp_client::depacketize::{Codec, Frame};

const SPS: &[u8] = &[
    0x67, 0x42, 0xc0, 0x1f, 0xda, 0x01, 0x40, 0x16, 0xe9, 0xb8, 0x08, 0x08, 0x0a, 0x00, 0x00, 0x07,
    0xd0, 0x00, 0x01, 0xd4, 0xc0, 0x80,
];
const PPS: &[u8] = &[0x68, 0xce, 0x3c, 0x80];
const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x21];
const NON_IDR: &[u8] = &[0x41, 0x9a, 0x24, 0x6c];

fn annex_b(nal: &[u8]) -> bytes::Bytes {
    let mut payload = Vec::with_capacity(nal.len() + 4);
    payload.extend_from_slice(&[0, 0, 0, 1]);
    payload.extend_from_slice(nal);
    Bytes::from(payload)
}

fn frame(timestamp: u32, nal: &[u8]) -> Frame {
    Frame {
        codec: Codec::H264,
        timestamp,
        payload: annex_b(nal),
    }
}

fn init_bytes() -> Vec<u8> {
    let mut tracker = InitSegmentTracker::default();
    tracker.observe(&frame(0, SPS));
    tracker
        .observe(&frame(0, PPS))
        .expect("SPS and PPS complete the init segment")
        .to_vec()
}

fn fragment_bytes(timestamps: &[u32]) -> Vec<u8> {
    let mut fragmenter = Fragmenter::default();
    let mut out = Vec::new();
    for (index, timestamp) in timestamps.iter().enumerate() {
        let nal = if index == 0 { IDR } else { NON_IDR };
        let fragment = fragmenter
            .next(&frame(*timestamp, nal))
            .expect("each VCL slice yields one fragment");
        out.extend_from_slice(&fragment.bytes);
    }
    out
}

fn valid_piece() -> Vec<u8> {
    let mut piece = init_bytes();
    piece.extend_from_slice(&fragment_bytes(&[0, 3000]));
    piece
}

fn top_boxes(bytes: &[u8]) -> Vec<([u8; 4], usize, usize)> {
    let mut boxes = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let mut kind = [0u8; 4];
        kind.copy_from_slice(&bytes[at + 4..at + 8]);
        boxes.push((kind, at, at + size));
        at += size;
    }
    boxes
}

fn child_of(bytes: &[u8], parent: (usize, usize), wanted: [u8; 4]) -> (usize, usize) {
    let mut at = parent.0 + 8;
    while at < parent.1 {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let mut kind = [0u8; 4];
        kind.copy_from_slice(&bytes[at + 4..at + 8]);
        if kind == wanted {
            return (at, at + size);
        }
        at += size;
    }
    panic!(
        "fixture box {:?} has no {wanted:?} child",
        &bytes[parent.0 + 4..parent.0 + 8]
    );
}

#[test]
fn two_fragment_piece_returns_the_fragment_summed_span_and_h264() {
    // Fragments at base 0 (3000 ticks default) and base 3000 (3000-tick delta):
    // the span is 6000 ticks at 90 kHz, 66.67 ms rounded down.
    let facts = piece_walk::walk(&valid_piece()).expect("a muxer-built piece walks");
    assert_eq!(facts.duration_ms, 66);
    assert_eq!(facts.codec, "h264");
}

#[test]
fn buffer_cut_inside_a_box_is_refused() {
    let piece = valid_piece();
    let error = piece_walk::walk(&piece[..piece.len() / 2])
        .expect_err("a buffer ending inside a box is refused");
    assert!(matches!(error, WalkError::Truncated), "{error}");
}

#[test]
fn trailing_partial_box_is_refused() {
    let mut piece = valid_piece();
    piece.extend_from_slice(&16u32.to_be_bytes());
    piece.extend_from_slice(b"junk");
    piece.extend_from_slice(&[7u8; 4]);
    let error = piece_walk::walk(&piece).expect_err("a final box short of its declared size fails");
    assert!(matches!(error, WalkError::Truncated), "{error}");
}

#[test]
fn piece_without_moov_is_refused() {
    let init = init_bytes();
    let ftyp_len = u32::from_be_bytes(init[..4].try_into().unwrap()) as usize;
    let mut piece = init[..ftyp_len].to_vec();
    piece.extend_from_slice(&fragment_bytes(&[0, 3000]));
    let error = piece_walk::walk(&piece).expect_err("moov is required after ftyp");
    assert!(matches!(error, WalkError::MissingMoov), "{error}");
}

#[test]
fn piece_without_any_fragment_is_refused() {
    let error = piece_walk::walk(&init_bytes()).expect_err("an init segment alone fails");
    assert!(matches!(error, WalkError::NoFragments), "{error}");
}

#[test]
fn zero_sized_box_is_refused() {
    let mut piece = valid_piece();
    let moof = top_boxes(&piece)
        .into_iter()
        .find(|(kind, ..)| kind == b"moof")
        .expect("fixture has a moof");
    piece[moof.1..moof.1 + 4].copy_from_slice(&0u32.to_be_bytes());
    let error = piece_walk::walk(&piece).expect_err("a declared size of zero is refused");
    assert!(matches!(error, WalkError::BoxSizeInvalid), "{error}");
}

#[test]
fn zero_duration_is_refused() {
    let mut piece = valid_piece();
    let moofs: Vec<(usize, usize)> = top_boxes(&piece)
        .into_iter()
        .filter(|(kind, ..)| kind == b"moof")
        .map(|(_, start, end)| (start, end))
        .collect();
    let last = *moofs.last().expect("fixture has a fragment");
    let traf = child_of(&piece, last, *b"traf");
    let tfdt = child_of(&piece, traf, *b"tfdt");
    piece[tfdt.0 + 12..tfdt.0 + 20].copy_from_slice(&0u64.to_be_bytes());
    let trun = child_of(&piece, traf, *b"trun");
    piece[trun.0 + 20..trun.0 + 24].copy_from_slice(&0u32.to_be_bytes());
    let error = piece_walk::walk(&piece).expect_err("a zero span is refused");
    assert!(matches!(error, WalkError::SpanInvalid), "{error}");
}

#[test]
fn mdhd_declared_smaller_than_its_fields_is_refused() {
    let mut piece = valid_piece();
    let moov = top_boxes(&piece)
        .into_iter()
        .find(|(kind, ..)| kind == b"moov")
        .expect("fixture has a moov");
    let trak = child_of(&piece, (moov.1, moov.2), *b"trak");
    let mdia = child_of(&piece, trak, *b"mdia");
    let mdhd = child_of(&piece, mdia, *b"mdhd");
    piece[mdhd.0..mdhd.0 + 4].copy_from_slice(&12u32.to_be_bytes());
    let error = piece_walk::walk(&piece)
        .expect_err("an mdhd smaller than its fields cannot state a timescale");
    assert!(matches!(error, WalkError::MissingMdhd), "{error}");
}

#[test]
fn empty_buffer_is_refused_without_panic() {
    let error = piece_walk::walk(&[]).expect_err("no boxes at all");
    assert!(matches!(error, WalkError::MissingFtyp), "{error}");
}
