//! Parses the fixed RTP header (RFC 3550 §5.1) common to every
//! packetization mode this crate depacketizes -- the codec-specific logic in
//! sibling modules only ever sees the payload past this header.

/// One RTP packet's header fields relevant to depacketization, plus the
/// payload bytes that follow (any CSRC list, extension header, and padding
/// already excluded).
#[derive(Debug)]
pub(super) struct RtpHeader<'a> {
    pub(super) timestamp: u32,
    pub(super) payload: &'a [u8],
}

/// Errors parsing an RTP packet's fixed header.
#[derive(Debug)]
pub(super) enum RtpHeaderError {
    /// The packet is shorter than the 12-byte fixed RTP header, or shorter
    /// than the header plus its own declared CSRC list, extension, or
    /// padding.
    TooShort,
    /// The packet's RTP version field is not 2, the only version this crate
    /// or any camera it targets speaks.
    UnsupportedVersion(u8),
}

impl std::fmt::Display for RtpHeaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "RTP packet is shorter than its own declared header"),
            Self::UnsupportedVersion(version) => {
                write!(f, "RTP packet declared version {version}, expected 2")
            }
        }
    }
}

impl std::error::Error for RtpHeaderError {}

const FIXED_HEADER_LEN: usize = 12;

/// Parses `packet`'s fixed RTP header and returns its timestamp and payload
/// slice, with any CSRC list, extension header, and padding already
/// excluded.
pub(super) fn parse(packet: &[u8]) -> Result<RtpHeader<'_>, RtpHeaderError> {
    if packet.len() < FIXED_HEADER_LEN {
        return Err(RtpHeaderError::TooShort);
    }
    let version = packet[0] >> 6;
    if version != 2 {
        return Err(RtpHeaderError::UnsupportedVersion(version));
    }
    let has_padding = packet[0] & 0x20 != 0;
    let has_extension = packet[0] & 0x10 != 0;
    let csrc_count = usize::from(packet[0] & 0x0F);
    let timestamp = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);

    let mut offset = FIXED_HEADER_LEN + 4 * csrc_count;
    if packet.len() < offset {
        return Err(RtpHeaderError::TooShort);
    }
    if has_extension {
        if packet.len() < offset + 4 {
            return Err(RtpHeaderError::TooShort);
        }
        let extension_words =
            usize::from(u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]));
        offset += 4 + extension_words * 4;
        if packet.len() < offset {
            return Err(RtpHeaderError::TooShort);
        }
    }

    let mut end = packet.len();
    if has_padding {
        let padding_len = usize::from(*packet.last().ok_or(RtpHeaderError::TooShort)?);
        end = end
            .checked_sub(padding_len)
            .ok_or(RtpHeaderError::TooShort)?;
    }
    if end < offset {
        return Err(RtpHeaderError::TooShort);
    }

    Ok(RtpHeader {
        timestamp,
        payload: &packet[offset..end],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_header(timestamp: u32) -> Vec<u8> {
        let mut packet = vec![0x80, 0xE0, 0, 1];
        packet.extend_from_slice(&timestamp.to_be_bytes());
        packet.extend_from_slice(&[0, 0, 0, 0]); // SSRC
        packet
    }

    #[test]
    fn parses_timestamp_and_payload_with_no_csrc_or_extension() {
        let mut packet = fixed_header(90_000);
        packet.extend_from_slice(&[0x65, 0xAA]);
        let header = parse(&packet).expect("parses");
        assert_eq!(header.timestamp, 90_000);
        assert_eq!(header.payload, &[0x65, 0xAA]);
    }

    #[test]
    fn skips_a_declared_csrc_list() {
        let mut packet = vec![0x81, 0xE0, 0, 1]; // CC=1
        packet.extend_from_slice(&500u32.to_be_bytes());
        packet.extend_from_slice(&[0, 0, 0, 0]); // SSRC
        packet.extend_from_slice(&[1, 2, 3, 4]); // one CSRC
        packet.extend_from_slice(&[0x65]);
        let header = parse(&packet).expect("parses");
        assert_eq!(header.payload, &[0x65]);
    }

    #[test]
    fn skips_a_declared_extension_header() {
        let mut packet = vec![0x90, 0xE0, 0, 1]; // extension bit set
        packet.extend_from_slice(&500u32.to_be_bytes());
        packet.extend_from_slice(&[0, 0, 0, 0]); // SSRC
        packet.extend_from_slice(&[0xBE, 0xDE, 0, 1]); // ext header id + 1-word length
        packet.extend_from_slice(&[9, 9, 9, 9]); // the one extension word
        packet.extend_from_slice(&[0x65]);
        let header = parse(&packet).expect("parses");
        assert_eq!(header.payload, &[0x65]);
    }

    #[test]
    fn strips_declared_padding_from_the_end() {
        let mut packet = fixed_header(1);
        packet[0] |= 0x20; // padding bit
        packet.extend_from_slice(&[0x65, 0, 0, 3]); // 1 payload byte + 3 padding bytes (last = count)
        let header = parse(&packet).expect("parses");
        assert_eq!(header.payload, &[0x65]);
    }

    #[test]
    fn rejects_a_packet_shorter_than_the_fixed_header() {
        assert!(matches!(
            parse(&[0x80, 0xE0, 0, 1]),
            Err(RtpHeaderError::TooShort)
        ));
    }

    #[test]
    fn rejects_an_unsupported_rtp_version() {
        let mut packet = fixed_header(1);
        packet[0] = 0x40; // version 1
        assert!(matches!(
            parse(&packet),
            Err(RtpHeaderError::UnsupportedVersion(1))
        ));
    }
}
