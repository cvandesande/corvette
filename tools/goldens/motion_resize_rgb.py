"""Goldens for corvette-motion's linear resize of an interleaved RGB picture: the bytes
`cv2.resize(src, (w, h), interpolation=cv2.INTER_LINEAR)` returns for seeded and flat
three-byte-per-pixel pictures at sample source and destination sizes.

Run command (the wheel's native libraries need the two loader directories):

    cd /home/cvandesande/github/corvette && timeout 300 nix develop .#goldens --command \
        env LD_LIBRARY_PATH=/nix/store/fkcbg2c1w29jr5yp9awai9w3v1wvxdk9-zlib-1.3.2/lib:/nix/store/w2nk3f7prwzpm6h13rfxh8lh46yfdysj-gcc-15.3.0-lib/lib \
        python3 -I tools/goldens/motion_resize_rgb.py <extracted wheel directory>

Reference: the opencv repository, path modules/imgproc/src/resize.cpp, tag 4.11.0,
lines 961 to 962 (coefficient bits), 1891 to 1927 and 1964 to 1987 (the horizontal and
vertical kernels), 4011 to 4041 (the exact 2-times area case), 4101 to 4141 (the source
mapping and coefficients), as pinned by the wheel
opencv_python_headless-4.11.0.86-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
(sha256 0e0a27c19dd1f40ddff94976cfe43066fbbe9dfbb2ec1907d66c19caef42a57b); the run refuses
any cv2 version other than 4.11.0.

Source bytes come from a seeded linear congruential generator the Rust tests
reimplement: state = seed, and for each byte state = (state * 6364136223846793005 +
1442695040888963407) mod 2**64 and the byte is state >> 33, row-major.

Writes crates/corvette-motion/tests/fixtures/resize_rgb_cases.txt: one case per line,
"<class> <src_w> <src_h> <dst_w> <dst_h> <kind> <seed> <fnv> <hex>", class exact (the
Rust port must match byte for byte) or tolerant (the wheel's SIMD vertical kernel may
differ from its scalar code by one level on a small part of the output); kind seed
(seeded source) or flat (every byte equal to seed); the 64-bit FNV-1a of the output in
16 lower-case hex digits, and the whole output as hex when it has at most 12288 bytes
(else "-").
"""

import sys
from pathlib import Path

sys.path.insert(0, sys.argv[1])
import cv2
import numpy as np

assert cv2.__version__ == "4.11.0", cv2.__version__

REPO = Path(__file__).resolve().parents[2]
FIXTURE = REPO / "crates/corvette-motion/tests/fixtures/resize_rgb_cases.txt"
HEX_LIMIT = 12288


def lcg_bytes(seed, count):
    state = seed
    out = bytearray(count)
    for i in range(count):
        state = (state * 6364136223846793005 + 1442695040888963407) & 0xFFFFFFFFFFFFFFFF
        out[i] = (state >> 33) & 0xFF
    return bytes(out)


def fnv_hex(payload):
    hash_ = 14695981039346656037
    for byte in payload:
        hash_ = ((hash_ ^ byte) * 1099511628211) & 0xFFFFFFFFFFFFFFFF
    return f"{hash_:016x}"


def source(kind, seed, width, height):
    if kind == "flat":
        return np.full((height, width, 3), seed, dtype=np.uint8)
    raw = lcg_bytes(seed, width * height * 3)
    return np.frombuffer(raw, dtype=np.uint8).reshape(height, width, 3)


# (class, src_w, src_h, dst_w, dst_h, kind, seed). Flat pictures, same-size copies and
# the exact 2-times shrinks (OpenCV's area path) are exact; seeded fractional ratios
# carry the tolerant class, where the wheel's SIMD kernel can differ by one level.
CASES = [
    ("exact", 4, 4, 8, 8, "flat", 7),
    ("exact", 8, 6, 4, 3, "flat", 200),
    ("exact", 5, 5, 5, 5, "flat", 128),
    ("exact", 3, 7, 7, 3, "flat", 90),
    ("exact", 70, 60, 70, 60, "flat", 17),
    ("exact", 17, 17, 23, 23, "flat", 64),
    ("exact", 9, 5, 5, 9, "flat", 30),
    ("exact", 2, 2, 2, 2, "flat", 64),
    ("exact", 4, 4, 4, 4, "seed", 3),
    ("exact", 8, 8, 4, 4, "seed", 11),
    ("exact", 16, 16, 8, 8, "seed", 12),
    ("exact", 40, 30, 20, 15, "seed", 13),
    ("exact", 128, 128, 64, 64, "seed", 14),
    ("exact", 1280, 1280, 640, 640, "seed", 15),
    ("exact", 16, 8, 8, 8, "seed", 16),
    ("exact", 5, 5, 3, 3, "seed", 42),
    ("exact", 11, 11, 7, 7, "seed", 43),
    ("exact", 10, 7, 6, 4, "seed", 44),
    ("exact", 6, 4, 10, 7, "seed", 236),
    # Wide sources at the rounding seams: a side of 16383 (the float-exact limit of
    # the mapping) or 2049 over a side of 2048 puts every weight exactly half way.
    ("exact", 16383, 2, 2048, 2, "seed", 7),
    ("exact", 2049, 2, 2048, 2, "seed", 7),
    ("exact", 2, 16383, 2, 2048, "seed", 7),
    ("exact", 2, 2049, 2, 2048, "seed", 7),
    ("exact", 16381, 2, 1024, 2, "seed", 7),
    ("exact", 4097, 2, 4096, 2, "seed", 7),
    ("tolerant", 17, 17, 23, 23, "seed", 21),
    ("tolerant", 40, 40, 64, 64, "seed", 22),
    ("tolerant", 64, 64, 40, 40, "seed", 23),
    ("tolerant", 33, 33, 64, 64, "seed", 24),
    ("tolerant", 64, 64, 33, 33, "seed", 25),
    ("tolerant", 100, 100, 64, 64, "seed", 26),
    ("tolerant", 80, 80, 64, 64, "seed", 27),
    ("tolerant", 96, 96, 64, 64, "seed", 28),
    ("tolerant", 50, 50, 64, 64, "seed", 29),
    ("tolerant", 20, 20, 64, 64, "seed", 30),
    ("tolerant", 40, 30, 64, 48, "seed", 31),
    ("tolerant", 64, 48, 40, 30, "seed", 32),
    ("tolerant", 30, 40, 20, 50, "seed", 33),
    ("tolerant", 1, 1, 8, 8, "seed", 34),
    ("tolerant", 1, 1, 1, 1, "seed", 35),
    ("tolerant", 2, 1, 4, 1, "seed", 36),
    ("tolerant", 3, 3, 7, 5, "seed", 37),
    ("tolerant", 23, 17, 7, 29, "seed", 38),
]


def main():
    lines = []
    for case_class, sw, sh, dw, dh, kind, seed in CASES:
        src = source(kind, seed, sw, sh)
        out = cv2.resize(src, (dw, dh), interpolation=cv2.INTER_LINEAR)
        payload = out.tobytes()
        hex_ = payload.hex() if len(payload) <= HEX_LIMIT else "-"
        lines.append(
            f"{case_class} {sw} {sh} {dw} {dh} {kind} {seed} {fnv_hex(payload)} {hex_}"
        )
    FIXTURE.write_text("\n".join(lines) + "\n")


main()
