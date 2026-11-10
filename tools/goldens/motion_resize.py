"""Goldens for corvette-motion's nearest-neighbour resize: the destination a
cv2.INTER_NEAREST resize produces for seeded random grey pictures.

Reference: the frigate repository, path frigate/motion/improved_motion.py, tag
v0.17.2, blob b081d37919541e6d1f95762ce084abde81774038, lines 24 and 42 and 76
to 80, copied verbatim:

        interpolation=cv2.INTER_NEAREST,
        self.interpolation = interpolation
        resized_frame = cv2.resize(
            gray,
            dsize=(self.motion_frame_size[1], self.motion_frame_size[0]),
            interpolation=self.interpolation,
        )

Reference: the opencv repository, path modules/imgproc/src/resize.cpp, tag
4.11.0, blob 4b317c6a5a3f3067d2b170aec8bd5deb2bc3e1a4, lines 3836 to 3837,
3993 to 3995, 1128, 1133 to 1134 and 1044, copied verbatim:

        inv_scale_x = static_cast<double>(dst_width) / src_width;
        inv_scale_y = static_cast<double>(dst_height) / src_height;
    if( interpolation == INTER_NEAREST )
    {
        resizeNN( src, dst, inv_scale_x, inv_scale_y );
    double ifx = 1./fx, ify = 1./fy;
        int sx = cvFloor(x*ifx);
        x_ofs[x] = std::min(sx, ssize.width-1)*pix_size;
            int sy = std::min(cvFloor(y*ify), ssize.height-1);

The bytes are what cv2.resize of the goldens shell's cv2 writes; versions.py
prints that cv2's version. The source bytes are pseudo-random, so a wrong
source index changes the output byte except by a coincidence of about one in
256. The picture bytes come from a seeded linear
congruential generator the Rust test reimplements: state = seed, and for each
byte state = (state * 1664525 + 1013904223) mod 2**32 and the byte is
state >> 24, row-major.

Writes the fixture read by crates/corvette-motion/tests/resize.rs: one line
per case, "<sw> <sh> <dw> <dh> <seed> <hex>", the destination's dw * dh bytes
as lower-case hex, rows top to bottom.
"""

from pathlib import Path

import cv2
import numpy

OUT = Path(__file__).resolve().parents[2] / "crates/corvette-motion/tests/fixtures/resize_cases.txt"

CASES = [
    (704, 576, 122, 100),
    (640, 480, 133, 100),
    (2688, 1520, 176, 100),
    (576, 704, 81, 100),
    (64, 48, 64, 48),
    (704, 576, 50, 50),
    (1, 1, 1, 1),
    (7, 5, 3, 2),
    (5, 7, 2, 3),
    (3, 3, 5, 5),
    (13, 1, 4, 1),
    (1, 13, 1, 4),
    (100, 100, 33, 100),
    (100, 100, 100, 33),
    (6, 4, 34, 3),
]


def lcg_bytes(seed: int, count: int) -> bytes:
    state = seed
    out = bytearray(count)
    for i in range(count):
        state = (state * 1664525 + 1013904223) % (1 << 32)
        out[i] = state >> 24
    return bytes(out)


def main() -> None:
    lines = []
    for case, seed in zip(CASES, range(101, 116), strict=True):
        sw, sh, dw, dh = case
        picture = lcg_bytes(seed, sw * sh)
        src = numpy.frombuffer(picture, dtype=numpy.uint8).reshape(sh, sw)
        out = cv2.resize(src, dsize=(dw, dh), interpolation=cv2.INTER_NEAREST)
        assert out.shape == (dh, dw)
        lines.append(f"{sw} {sh} {dw} {dh} {seed} {out.tobytes().hex()}")
    OUT.write_text("\n".join(lines) + "\n", encoding="ascii")


main()
