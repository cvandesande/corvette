//! Splits an Annex-B byte stream -- one or more start-code-prefixed NAL
//! units, per Annex B of ITU-T H.264/H.265 -- into its individual NAL
//! units, each with its start code stripped. Shared by the H.264 and H.265
//! packetizers: both split their input the same way before deciding,
//! per-NAL, whether to emit it as a single packet or fragment it.

use super::PacketizeError;

/// One start code's position: where its own 3- or 4-byte marker begins,
/// and where the NAL unit it introduces begins.
struct StartCode {
    marker_begin: usize,
    data_begin: usize,
}

fn find_start_codes(annex_b: &[u8]) -> Vec<StartCode> {
    let mut start_codes = Vec::new();
    let mut i = 0;
    while i + 3 <= annex_b.len() {
        if annex_b[i] == 0 && annex_b[i + 1] == 0 {
            if annex_b[i + 2] == 1 {
                start_codes.push(StartCode {
                    marker_begin: i,
                    data_begin: i + 3,
                });
                i += 3;
                continue;
            }
            if i + 4 <= annex_b.len() && annex_b[i + 2] == 0 && annex_b[i + 3] == 1 {
                start_codes.push(StartCode {
                    marker_begin: i,
                    data_begin: i + 4,
                });
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    start_codes
}

/// Splits `annex_b` into its NAL units, in order, with each unit's leading
/// 3- or 4-byte start code removed.
///
/// # Errors
///
/// Returns [`PacketizeError::NoStartCode`] if `annex_b` contains no start
/// code at all, or [`PacketizeError::EmptyNalUnit`] if a start code is
/// immediately followed by another start code or by the end of the input --
/// a zero-length NAL unit, never valid Annex-B.
pub(super) fn split_nal_units(annex_b: &[u8]) -> Result<Vec<&[u8]>, PacketizeError> {
    let start_codes = find_start_codes(annex_b);
    if start_codes.is_empty() {
        return Err(PacketizeError::NoStartCode);
    }

    let mut units = Vec::with_capacity(start_codes.len());
    for (index, start_code) in start_codes.iter().enumerate() {
        let end = start_codes
            .get(index + 1)
            .map_or(annex_b.len(), |next| next.marker_begin);
        let unit = &annex_b[start_code.data_begin..end];
        if unit.is_empty() {
            return Err(PacketizeError::EmptyNalUnit);
        }
        units.push(unit);
    }
    Ok(units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_single_four_byte_start_coded_nal() {
        let annex_b = [0, 0, 0, 1, 0x67, 0xAA, 0xBB];
        let units = split_nal_units(&annex_b).expect("splits");
        assert_eq!(units, vec![&[0x67, 0xAA, 0xBB][..]]);
    }

    #[test]
    fn splits_a_single_three_byte_start_coded_nal() {
        let annex_b = [0, 0, 1, 0x68, 0xCC];
        let units = split_nal_units(&annex_b).expect("splits");
        assert_eq!(units, vec![&[0x68, 0xCC][..]]);
    }

    #[test]
    fn splits_multiple_nals_mixing_three_and_four_byte_start_codes() {
        let annex_b = [
            0, 0, 0, 1, 0x67, 0x01, // 4-byte start code, NAL "67 01"
            0, 0, 1, 0x68, 0x02, // 3-byte start code, NAL "68 02"
            0, 0, 0, 1, 0x65, 0x03, 0x04, // 4-byte start code, NAL "65 03 04"
        ];
        let units = split_nal_units(&annex_b).expect("splits");
        assert_eq!(
            units,
            vec![
                &[0x67, 0x01][..],
                &[0x68, 0x02][..],
                &[0x65, 0x03, 0x04][..],
            ]
        );
    }

    #[test]
    fn rejects_a_payload_with_no_start_code() {
        let annex_b = [0x67, 0xAA, 0xBB];
        assert!(matches!(
            split_nal_units(&annex_b),
            Err(PacketizeError::NoStartCode)
        ));
    }

    #[test]
    fn rejects_a_zero_length_nal_between_adjacent_start_codes() {
        let annex_b = [0, 0, 0, 1, 0, 0, 0, 1, 0x67, 0xAA];
        assert!(matches!(
            split_nal_units(&annex_b),
            Err(PacketizeError::EmptyNalUnit)
        ));
    }

    #[test]
    fn rejects_a_zero_length_nal_at_the_end_of_the_payload() {
        let annex_b = [0x67, 0xAA, 0, 0, 0, 1];
        assert!(matches!(
            split_nal_units(&annex_b),
            Err(PacketizeError::EmptyNalUnit)
        ));
    }
}
