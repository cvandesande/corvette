# Issue #12 item X2 -- fragmentation-size threshold

**Chosen value:** 1400 bytes, as `MAX_SINGLE_NAL_SIZE` in
`crates/rtsp-restream/src/packetize/mod.rs`. Applies uniformly to both the
H.264 and H.265 packetizers.

## What it governs

An H.264/H.265 NAL unit whose length (including its own 1-byte/2-byte NAL
header) is at most 1400 bytes is emitted as a single-NAL RTP packet
carrying the whole NAL unverbatim. A larger NAL unit is split into FU-A
(H.264, RFC 6184 §5.8) or FU (H.265, RFC 7798 §4.4.3) fragments, each sized
so the *total* RTP payload of every fragment -- including that mode's own
2-byte (H.264) or 3-byte (H.265) fragmentation-header overhead -- also
never exceeds 1400 bytes.

## Why 1400

This crate's own `SETUP` handling accepts only
`Transport: RTP/AVP/TCP;interleaved=N-M` (D-7; see
`crates/rtsp-restream/src/session.rs`), so there is no real network MTU
constraint on an RTP payload carried over this crate's own interleaved
binary-data frames -- a TCP stream has no per-packet size ceiling the way a
UDP datagram does.

The threshold is still chosen conservatively rather than left unbounded,
for two reasons:

1. **Uniformity with what a UDP-capable RTP receiver would also need to
   tolerate.** RFC 6184/RFC 7798 packetizers are conventionally written
   against a real path MTU because the same wire format is routinely also
   carried over UDP. Emitting arbitrarily large single-NAL packets here
   would make this crate's own output shape depend on which transport
   happens to be negotiated, which is exactly the kind of transport-
   dependent behavior a packetizer should not have.
2. **A concrete, standard, and easily-justified number.** A standard
   1500-byte Ethernet frame, minus a 20-byte IPv4 header, an 8-byte UDP
   header, and a 12-byte RTP header, leaves 1460 bytes of usable RTP
   payload. 1400 keeps roughly 60 bytes of headroom under that figure for
   whatever additional encapsulation a real network path might add -- an
   802.1Q VLAN tag (4 bytes), IPv6 in place of IPv4 (20 bytes larger than
   the 1460 figure already assumes), a tunnel encapsulation -- without this
   crate having to enumerate or special-case which one applies. It is also
   the same order of magnitude several widely deployed RTP implementations
   use as a default fragmentation threshold for exactly this reason.

No alternative fixed size was seriously considered beyond checking that
1400 leaves comfortable headroom under 1460: a materially smaller value
would fragment more NAL units than necessary with no corresponding benefit
(this crate never ships over UDP today), and a materially larger value
(close to or above 1460) would leave no margin for the encapsulation
overhead named above.

## Where it is enforced

`crates/rtsp-restream/src/packetize/mod.rs`'s `MAX_SINGLE_NAL_SIZE`
constant is the single source of truth; both `h264.rs` and `h265.rs`
import it rather than declaring their own copies. Unit tests in both
modules (`a_nal_larger_than_the_threshold_is_fu_a_fragmented` /
`..._is_fu_fragmented`) assert every emitted packet's RTP payload actually
stays at or under this bound, including every FU-A/FU fragment.
