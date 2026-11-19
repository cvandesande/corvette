//! Revision digest for saved configuration bytes, the `ETag` value of the configuration contract.

use std::fmt::Write;

// Round constants and initial state, per the FIPS Secure Hash Standard sections 4.2.2 and 5.3.3.
const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];
const INITIAL_STATE: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

fn compress(state: &mut [u32; 8], block: &[u8]) {
    let mut schedule = [0u32; 64];
    for (idx, word) in schedule.iter_mut().take(16).enumerate() {
        let base = idx * 4;
        *word = u32::from_be_bytes([
            block[base],
            block[base + 1],
            block[base + 2],
            block[base + 3],
        ]);
    }
    for idx in 16..64 {
        let old = schedule[idx - 15];
        let last = schedule[idx - 2];
        let spread = old.rotate_right(7) ^ old.rotate_right(18) ^ (old >> 3);
        let gather = last.rotate_right(17) ^ last.rotate_right(19) ^ (last >> 10);
        schedule[idx] = schedule[idx - 16]
            .wrapping_add(spread)
            .wrapping_add(schedule[idx - 7])
            .wrapping_add(gather);
    }
    let mut work = *state;
    for idx in 0..64 {
        let choose = (work[4] & work[5]) ^ (!work[4] & work[6]);
        let majority = (work[0] & work[1]) ^ (work[0] & work[2]) ^ (work[1] & work[2]);
        let carry = work[7]
            .wrapping_add(
                work[4].rotate_right(6) ^ work[4].rotate_right(11) ^ work[4].rotate_right(25),
            )
            .wrapping_add(choose)
            .wrapping_add(K[idx])
            .wrapping_add(schedule[idx]);
        let turn = (work[0].rotate_right(2) ^ work[0].rotate_right(13) ^ work[0].rotate_right(22))
            .wrapping_add(majority);
        work[7] = work[6];
        work[6] = work[5];
        work[5] = work[4];
        work[4] = work[3].wrapping_add(carry);
        work[3] = work[2];
        work[2] = work[1];
        work[1] = work[0];
        work[0] = carry.wrapping_add(turn);
    }
    for (slot, value) in state.iter_mut().zip(work) {
        *slot = slot.wrapping_add(value);
    }
}

/// Returns `sha256:` followed by the 64 lower-case hex digits of the SHA-256 of `bytes`, per the FIPS Secure Hash Standard.
#[must_use]
pub fn revision(bytes: &[u8]) -> String {
    let bit_length = (bytes.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(bytes.len() + 72);
    padded.extend_from_slice(bytes);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());
    let mut state = INITIAL_STATE;
    for block in padded.chunks_exact(64) {
        compress(&mut state, block);
    }
    let mut out = String::with_capacity(71);
    out.push_str("sha256:");
    for value in state {
        let _ = write!(out, "{value:08x}");
    }
    out
}
