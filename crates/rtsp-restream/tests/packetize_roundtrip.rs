//! Issue #12 item X2's own round-trip verification: feed this crate's
//! packetizers' RTP output back through an independent depacketizer and
//! assert the reassembled access unit matches the original byte-for-byte.
//!
//! For H.264/H.265, the oracle is issue #18's own already-shipped,
//! already-reviewed `H264Depacketizer`/`H265Depacketizer`
//! (`corvette-rtsp-client`, a dev-dependency of this crate only -- see
//! `Cargo.toml`). For AAC, issue #18's own `AacDepacketizer` exists but
//! (confirmed by reading `crates/corvette-rtsp-client/src/client/task.rs`'s
//! `Depacketizer` enum, which has only `H264`/`H265` variants) is not wired
//! into any shipped consumer, so this file uses a small, independent,
//! test-only decoder instead (`depacketize_aac_access_units` below) rather
//! than treating an unexercised code path as a trustworthy oracle.

use corvette_rtsp_client::depacketize::{H264Depacketizer, H265Depacketizer, SpropParameterSets};
use rtsp_restream::provider::Frame;
use rtsp_restream::{AacPacketizer, H264Packetizer, H265Packetizer};
use std::time::Duration;

fn annex_b(nals: &[&[u8]]) -> Vec<u8> {
    let mut payload = Vec::new();
    for nal in nals {
        payload.extend_from_slice(&[0, 0, 0, 1]);
        payload.extend_from_slice(nal);
    }
    payload
}

const fn frame(payload: Vec<u8>) -> Frame {
    Frame {
        track_index: 0,
        timestamp: Duration::from_millis(1500),
        payload,
    }
}

/// Feeds `packets` through a fresh [`H264Depacketizer`], dropping the one
/// injected empty-parameter-set frame every fresh depacketizer emits before
/// its first real NAL (this test constructs it with an empty
/// `sprop-parameter-sets`, since no parameter-set NAL is under test here).
fn depacketize_h264(packets: &[Vec<u8>]) -> Vec<u8> {
    let mut depacketizer = H264Depacketizer::new("").expect("decodes an empty sprop");
    let mut reassembled = Vec::new();
    for (packet_index, packet) in packets.iter().enumerate() {
        for frame in depacketizer.depacketize(packet).expect("depacketizes") {
            if packet_index == 0 && frame.payload.as_ref() == [0, 0, 0, 1] {
                continue; // the injected empty parameter set, not real NAL data
            }
            reassembled.extend_from_slice(frame.payload.as_ref());
        }
    }
    reassembled
}

/// Same as [`depacketize_h264`], for H.265's three (VPS/SPS/PPS) injected
/// empty parameter sets.
fn depacketize_h265(packets: &[Vec<u8>]) -> Vec<u8> {
    let sprop = SpropParameterSets {
        vps: "",
        sps: "",
        pps: "",
    };
    let mut depacketizer = H265Depacketizer::new(sprop).expect("decodes an empty sprop");
    let mut reassembled = Vec::new();
    for (packet_index, packet) in packets.iter().enumerate() {
        for frame in depacketizer.depacketize(packet).expect("depacketizes") {
            if packet_index == 0 && frame.payload.as_ref() == [0, 0, 0, 1] {
                continue;
            }
            reassembled.extend_from_slice(frame.payload.as_ref());
        }
    }
    reassembled
}

// --- H.264 --------------------------------------------------------------

#[test]
fn h264_single_nal_round_trips_byte_for_byte() {
    let original = annex_b(&[&[0x65, 0xAA, 0xBB, 0xCC]]);
    let mut packetizer = H264Packetizer::new(96, 0x1234_5678);

    let packets = packetizer
        .packetize(&frame(original.clone()))
        .expect("packetizes");
    assert_eq!(packets.len(), 1, "small NAL stays a single packet");

    let reassembled = depacketize_h264(&packets);
    assert_eq!(reassembled, original);
}

#[test]
fn h264_fu_a_fragmented_nal_round_trips_byte_for_byte() {
    let mut nal = vec![0x65u8];
    nal.extend((0..3000u32).map(|n| u8::try_from(n % 256).unwrap()));
    let original = annex_b(&[&nal]);
    let mut packetizer = H264Packetizer::new(96, 1);

    let packets = packetizer
        .packetize(&frame(original.clone()))
        .expect("packetizes");
    assert!(packets.len() > 1, "a 3001-byte NAL must fragment");

    let reassembled = depacketize_h264(&packets);
    assert_eq!(reassembled, original);
}

/// Multiple small NAL units in one access unit -- small enough that a
/// STAP-A aggregation packet would also have been a valid choice for them,
/// but this packetizer emits each as its own single-NAL packet instead
/// (issue #12 item X2's own `Do` step names only single-NAL and FU-A, not
/// STAP-A aggregation). Round-tripping must still recover every NAL, in
/// order, byte-identical.
#[test]
fn h264_multiple_small_nals_stap_a_eligible_round_trip_in_order() {
    let sps = [0x67u8, 0x42, 0x00, 0x1E];
    let pps = [0x68u8, 0xCE, 0x3C, 0x80];
    let idr = [0x65u8, 0x11, 0x22, 0x33];
    let original = annex_b(&[&sps, &pps, &idr]);
    let mut packetizer = H264Packetizer::new(96, 1);

    let packets = packetizer
        .packetize(&frame(original.clone()))
        .expect("packetizes");
    assert_eq!(packets.len(), 3, "each small NAL becomes its own packet");

    let reassembled = depacketize_h264(&packets);
    assert_eq!(reassembled, original);
}

// --- H.265 ----------------------------------------------------------------

#[test]
fn h265_single_nal_round_trips_byte_for_byte() {
    let original = annex_b(&[&[0x26, 0x01, 0xDE, 0xAD, 0xBE, 0xEF]]);
    let mut packetizer = H265Packetizer::new(96, 0x1234_5678);

    let packets = packetizer
        .packetize(&frame(original.clone()))
        .expect("packetizes");
    assert_eq!(packets.len(), 1);

    let reassembled = depacketize_h265(&packets);
    assert_eq!(reassembled, original);
}

#[test]
fn h265_fu_fragmented_nal_round_trips_byte_for_byte() {
    let mut nal = vec![0x26u8, 0x01];
    nal.extend((0..3000u32).map(|n| u8::try_from(n % 256).unwrap()));
    let original = annex_b(&[&nal]);
    let mut packetizer = H265Packetizer::new(96, 1);

    let packets = packetizer
        .packetize(&frame(original.clone()))
        .expect("packetizes");
    assert!(packets.len() > 1, "a 3002-byte NAL must fragment");

    let reassembled = depacketize_h265(&packets);
    assert_eq!(reassembled, original);
}

/// Multiple small NAL units in one access unit -- small enough that an AP
/// (H.265's aggregation packet) would also have been a valid choice, but
/// this packetizer emits each as its own single-NAL packet instead, same
/// as H.264's STAP-A-eligible case above.
#[test]
fn h265_multiple_small_nals_ap_eligible_round_trip_in_order() {
    let vps = [0x40u8, 0x01, 0x0C];
    let sps = [0x42u8, 0x01, 0x01];
    let pps = [0x44u8, 0x01, 0xC1];
    let original = annex_b(&[&vps, &sps, &pps]);
    let mut packetizer = H265Packetizer::new(96, 1);

    let packets = packetizer
        .packetize(&frame(original.clone()))
        .expect("packetizes");
    assert_eq!(packets.len(), 3);

    let reassembled = depacketize_h265(&packets);
    assert_eq!(reassembled, original);
}

// --- AAC --------------------------------------------------------------

/// A small, test-only decoder for RFC 3640 `aac-hbr` AU-header framing
/// (13-bit size, 3-bit index/index-delta fields) -- written independently
/// of both `AacPacketizer` and issue #18's own (unwired) `AacDepacketizer`,
/// so this test's oracle is not simply this crate's own encoding step run
/// in reverse. Generic over the number of access units a packet declares,
/// even though `AacPacketizer` itself only ever emits one.
fn depacketize_aac_access_units(rtp_packet: &[u8]) -> Vec<Vec<u8>> {
    let payload = &rtp_packet[12..]; // fixed 12-byte RTP header; no CSRC/extension/padding here
    let au_headers_length_bits = usize::from(u16::from_be_bytes([payload[0], payload[1]]));
    let au_header_bytes = au_headers_length_bits.div_ceil(8);
    let header_bytes = &payload[2..2 + au_header_bytes];

    let mut reader = BitReader {
        bytes: header_bytes,
        bit_pos: 0,
    };
    let mut sizes = Vec::new();
    let mut consumed_bits = 0;
    while consumed_bits < au_headers_length_bits {
        const SIZE_BITS: usize = 13;
        const INDEX_BITS: usize = 3;
        let size = reader.read_bits(SIZE_BITS);
        reader.read_bits(INDEX_BITS);
        consumed_bits += SIZE_BITS + INDEX_BITS;
        sizes.push(size);
    }

    let mut offset = 2 + au_header_bytes;
    let mut access_units = Vec::with_capacity(sizes.len());
    for size in sizes {
        access_units.push(payload[offset..offset + size].to_vec());
        offset += size;
    }
    access_units
}

struct BitReader<'a> {
    bytes: &'a [u8],
    bit_pos: usize,
}

impl BitReader<'_> {
    fn read_bits(&mut self, count: usize) -> usize {
        let mut value = 0usize;
        for _ in 0..count {
            let byte_index = self.bit_pos / 8;
            let bit_index = 7 - (self.bit_pos % 8);
            let bit = (self.bytes[byte_index] >> bit_index) & 1;
            value = (value << 1) | usize::from(bit);
            self.bit_pos += 1;
        }
        value
    }
}

#[test]
fn aac_access_unit_round_trips_byte_for_byte() {
    let original = vec![0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77];
    let mut packetizer = AacPacketizer::new(97, 1, 48_000);

    let packet = packetizer
        .packetize(&frame(original.clone()))
        .expect("packetizes");

    let access_units = depacketize_aac_access_units(&packet);
    assert_eq!(access_units, vec![original]);
}
