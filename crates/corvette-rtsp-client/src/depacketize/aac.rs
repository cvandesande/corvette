//! AAC (RFC 3640, `mpeg4-generic`) RTP depacketization: extracts each raw
//! access unit an RTP payload's AU-header section declares, with no ADTS
//! header added (D-9).

use super::rtp_header::parse as parse_rtp_header;
use super::{Codec, DepacketizeError, Frame};

/// RFC 3640's own typical `aac-hbr` mode defaults, used for any parameter a
/// camera's SDP `a=fmtp` omits.
const DEFAULT_SIZE_LENGTH_BITS: u8 = 13;
const DEFAULT_INDEX_LENGTH_BITS: u8 = 3;
const DEFAULT_INDEX_DELTA_LENGTH_BITS: u8 = 3;

/// Extracts raw AAC access units from one track's `mpeg4-generic` RTP
/// payloads. Stateless between calls: each RTP payload's own AU-headers
/// section fully describes the access unit(s) it carries.
/// Bit widths of an RFC 3640 AU-header's fields, from that RFC's own
/// §4.1 `sizelength`/`indexlength`/`indexdeltalength` `a=fmtp` parameters.
#[derive(Debug, Clone, Copy)]
struct AuHeaderBitWidths {
    size: u8,
    index: u8,
    index_delta: u8,
}

#[derive(Debug)]
pub struct AacDepacketizer {
    bit_widths: AuHeaderBitWidths,
}

impl AacDepacketizer {
    /// Builds a depacketizer from the track's SDP `a=fmtp` parameters,
    /// reading `sizelength`/`indexlength`/`indexdeltalength` (RFC 3640
    /// §4.1) and falling back to that RFC's own `aac-hbr` defaults
    /// (13/3/3 bits) for any parameter a camera's SDP omits.
    #[must_use]
    pub fn new(format_params: &[(String, Option<String>)]) -> Self {
        Self {
            bit_widths: AuHeaderBitWidths {
                size: read_bit_length(format_params, "sizelength")
                    .unwrap_or(DEFAULT_SIZE_LENGTH_BITS),
                index: read_bit_length(format_params, "indexlength")
                    .unwrap_or(DEFAULT_INDEX_LENGTH_BITS),
                index_delta: read_bit_length(format_params, "indexdeltalength")
                    .unwrap_or(DEFAULT_INDEX_DELTA_LENGTH_BITS),
            },
        }
    }

    /// Depacketizes one RTP payload into its raw access unit(s), in the
    /// order their AU-headers declare them.
    ///
    /// # Errors
    ///
    /// Returns an error if the RTP header is malformed, the AU-headers
    /// section is truncated, or a declared access-unit size runs past the
    /// remaining payload.
    pub fn depacketize(&self, rtp_packet: &[u8]) -> Result<Vec<Frame>, DepacketizeError> {
        let header = parse_rtp_header(rtp_packet)?;
        let payload = header.payload;
        if payload.len() < 2 {
            return Err(DepacketizeError::TruncatedAccessUnit);
        }
        let au_headers_length_bits = usize::from(u16::from_be_bytes([payload[0], payload[1]]));
        let au_header_bytes = au_headers_length_bits.div_ceil(8);
        let headers_end = 2 + au_header_bytes;
        let header_bytes = payload
            .get(2..headers_end)
            .ok_or(DepacketizeError::TruncatedAccessUnit)?;

        let sizes = self.read_access_unit_sizes(header_bytes, au_headers_length_bits)?;

        let mut offset = headers_end;
        let mut frames = Vec::with_capacity(sizes.len());
        for size in sizes {
            let access_unit = payload
                .get(offset..offset + size)
                .ok_or(DepacketizeError::TruncatedAccessUnit)?;
            frames.push(Frame {
                codec: Codec::Aac,
                timestamp: header.timestamp,
                payload: bytes::Bytes::copy_from_slice(access_unit),
            });
            offset += size;
        }
        Ok(frames)
    }

    /// Reads each AU-header's size field out of `header_bytes` (the
    /// AU-headers section, per RFC 3640 §3.3.1): the first header's index
    /// field is `indexlength` bits wide, every subsequent header's is
    /// `indexdeltalength` bits wide.
    fn read_access_unit_sizes(
        &self,
        header_bytes: &[u8],
        au_headers_length_bits: usize,
    ) -> Result<Vec<usize>, DepacketizeError> {
        let mut reader = BitReader::new(header_bytes);
        let mut sizes = Vec::new();
        let mut consumed_bits = 0;
        while consumed_bits < au_headers_length_bits {
            let size = reader.read_bits(self.bit_widths.size)?;
            let index_bits = if sizes.is_empty() {
                self.bit_widths.index
            } else {
                self.bit_widths.index_delta
            };
            reader.read_bits(index_bits)?;
            consumed_bits += usize::from(self.bit_widths.size) + usize::from(index_bits);
            sizes.push(size);
        }
        Ok(sizes)
    }
}

fn read_bit_length(format_params: &[(String, Option<String>)], name: &str) -> Option<u8> {
    format_params
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.as_deref())
        .and_then(|value| value.parse().ok())
}

/// Reads big-endian, MSB-first bit fields out of a byte slice.
struct BitReader<'a> {
    bytes: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit_pos: 0 }
    }

    fn read_bits(&mut self, count: u8) -> Result<usize, DepacketizeError> {
        let mut value = 0usize;
        for _ in 0..count {
            let byte_index = self.bit_pos / 8;
            let bit_index = 7 - (self.bit_pos % 8);
            let byte = *self
                .bytes
                .get(byte_index)
                .ok_or(DepacketizeError::TruncatedAccessUnit)?;
            value = (value << 1) | usize::from((byte >> bit_index) & 1);
            self.bit_pos += 1;
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtp_packet(timestamp: u32, payload: &[u8]) -> Vec<u8> {
        let mut packet = vec![0x80, 0xE0, 0, 1];
        packet.extend_from_slice(&timestamp.to_be_bytes());
        packet.extend_from_slice(&[0, 0, 0, 0]); // SSRC
        packet.extend_from_slice(payload);
        packet
    }

    /// Builds an RFC 3640 AU-header section for a single access unit under
    /// the RFC's own `aac-hbr` defaults (sizelength=13, indexlength=3):
    /// AU-headers-length (2 bytes, in bits) then one 16-bit AU-header
    /// (13-bit size, 3-bit index -- together filling the 16-bit word
    /// exactly).
    fn single_au_payload(access_unit: &[u8]) -> Vec<u8> {
        let size = u16::try_from(access_unit.len()).expect("fits in 13 bits");
        let au_header: u16 = size << 3; // index = 0
        let mut payload = 16u16.to_be_bytes().to_vec();
        payload.extend_from_slice(&au_header.to_be_bytes());
        payload.extend_from_slice(access_unit);
        payload
    }

    #[test]
    fn a_single_access_unit_round_trips_with_no_adts_header() {
        let depacketizer = AacDepacketizer::new(&[]);
        let access_unit = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE];

        let frames = depacketizer
            .depacketize(&rtp_packet(1000, &single_au_payload(&access_unit)))
            .expect("depacketizes");

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].codec, Codec::Aac);
        assert_eq!(frames[0].timestamp, 1000);
        assert_eq!(frames[0].payload.as_ref(), &access_unit);
    }

    #[test]
    fn a_sequence_of_packets_each_produce_one_access_unit() {
        let depacketizer = AacDepacketizer::new(&[]);
        let access_units: [&[u8]; 2] = [&[0x01, 0x02], &[0x03, 0x04, 0x05]];

        for (index, access_unit) in access_units.into_iter().enumerate() {
            let timestamp = 1000 + u32::try_from(index).unwrap() * 1024;
            let frames = depacketizer
                .depacketize(&rtp_packet(timestamp, &single_au_payload(access_unit)))
                .expect("depacketizes");
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].payload.as_ref(), access_unit);
            assert_eq!(frames[0].timestamp, timestamp);
        }
    }

    #[test]
    fn two_access_units_aggregated_in_one_packet_each_produce_a_separate_frame() {
        let depacketizer = AacDepacketizer::new(&[]);
        let au_a = [0xAA, 0xBB];
        let au_b = [0xCC, 0xDD, 0xEE];
        let header_a: u16 = u16::try_from(au_a.len()).unwrap() << 3; // index = 0
        let header_b: u16 = u16::try_from(au_b.len()).unwrap() << 3; // index-delta = 0
        let mut payload = 32u16.to_be_bytes().to_vec(); // two 16-bit AU-headers
        payload.extend_from_slice(&header_a.to_be_bytes());
        payload.extend_from_slice(&header_b.to_be_bytes());
        payload.extend_from_slice(&au_a);
        payload.extend_from_slice(&au_b);

        let frames = depacketizer
            .depacketize(&rtp_packet(4000, &payload))
            .expect("depacketizes");

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].payload.as_ref(), &au_a);
        assert_eq!(frames[1].payload.as_ref(), &au_b);
    }

    #[test]
    fn respects_a_custom_sizelength_from_format_params() {
        let format_params = vec![("sizelength".to_string(), Some("8".to_string()))];
        let depacketizer = AacDepacketizer::new(&format_params);
        let access_unit = [0x11, 0x22, 0x33];
        // AU-headers-length = 8 (custom sizelength) + 3 (default
        // indexlength) = 11 bits, MSB-aligned in the 2-byte AU-header
        // field: size in bits 15..8, index (0) in bits 7..5, remaining bits
        // unused padding.
        let size = u16::try_from(access_unit.len()).unwrap();
        let au_header_bits: u16 = size << 8;
        let mut payload = 11u16.to_be_bytes().to_vec();
        payload.extend_from_slice(&au_header_bits.to_be_bytes());
        payload.extend_from_slice(&access_unit);

        let frames = depacketizer
            .depacketize(&rtp_packet(2000, &payload))
            .expect("depacketizes");

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].payload.as_ref(), &access_unit);
    }

    #[test]
    fn a_declared_size_past_the_payload_end_is_an_error() {
        let depacketizer = AacDepacketizer::new(&[]);
        let bogus_au_header: u16 = 100u16 << 3; // declares 100 bytes; payload has none
        let mut payload = 16u16.to_be_bytes().to_vec();
        payload.extend_from_slice(&bogus_au_header.to_be_bytes());

        let result = depacketizer.depacketize(&rtp_packet(3000, &payload));

        assert!(matches!(result, Err(DepacketizeError::TruncatedAccessUnit)));
    }
}
