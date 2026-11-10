r"""Goldens for corvette-motion's difference, threshold and dilation: the bytes
cv2.absdiff, cv2.threshold (THRESH_BINARY, maximum 255) and cv2.dilate (default
3x3 kernel, one iteration) write for seeded, sparse and single-pixel grey
pictures.

Reference: the frigate repository, path frigate/motion/improved_motion.py, tag
v0.17.2, blob b081d37919541e6d1f95762ce084abde81774038, lines 124, 126 to 129
and 131 to 133, blank lines 125 and 130 omitted, copied verbatim:

        frameDelta = cv2.absdiff(resized_frame, cv2.convertScaleAbs(self.avg_frame))
        # compute the threshold image for the current frame
        thresh = cv2.threshold(
            frameDelta, self.config.threshold, 255, cv2.THRESH_BINARY
        )[1]
        # dilate the thresholded image to fill in holes, then find contours
        # on thresholded image
        thresh_dilated = cv2.dilate(thresh, None, iterations=1)

Reference: the frigate repository, path frigate/config/camera/motion.py, tag
v0.17.2, blob 65c03f73176cb2aab15a57e48f032065bc6cbae6, lines 12 to 17, copied
verbatim:

    threshold: int = Field(
        default=30,
        title="Motion detection threshold (1-255).",
        ge=1,
        le=255,
    )

Reference: the opencv repository, path modules/imgproc/src/thresh.cpp, tag
4.11.0, blob a8c89059a7807c9e507d64bab2389408b7b66061: line 125 (the signature
of thresh_8u), lines 199 and 206 to 207 and lines 271 and 275 to 279 (its two
THRESH_BINARY cases: maxval is written where the pixel is above the level, and
0 at or below it), and lines 1575 to 1577, 1580, 1582 and 1588 (the level is
floored to a whole number and, for THRESH_BINARY, a level of 255 writes 0
everywhere), copied verbatim:

thresh_8u( const Mat& _src, Mat& _dst, uchar thresh, uchar maxval, int type )
    case THRESH_BINARY:
                v0 = v_lt(thresh_u, v0);
                v0 = v_and(v0, maxval16);
        const int thresh_pivot = thresh + 1;
        case THRESH_BINARY:
            memset(tab, 0, thresh_pivot);
            if (thresh_pivot < 256) {
                memset(tab + thresh_pivot, maxval, 256 - thresh_pivot);
            }
        int ithresh = cvFloor(thresh);
        thresh = ithresh;
        int imaxval = cvRound(maxval);
        imaxval = saturate_cast<uchar>(imaxval);
        if( ithresh < 0 || ithresh >= 255 )
                int v = type == THRESH_BINARY ? (ithresh >= 255 ? 0 : imaxval) :

Reference: the opencv repository, path modules/imgproc/src/morph.dispatch.cpp,
tag 4.11.0, blob 0cb50ec368958842764449c9babdac3162a572cf, lines 960 to 964 (an
empty kernel means a rectangle of side 1 plus two per iteration, anchored at
its centre, at one iteration):

    if (kernel.empty())
    {
        kernel = getStructuringElement(MORPH_RECT, Size(1+iterations*2,1+iterations*2));
        anchor = Point(iterations, iterations);
        iterations = 1;
    }

Reference: the opencv repository, path
modules/imgproc/include/opencv2/imgproc.hpp, tag 4.11.0, blob
3daa6d1a0d838af940326a13b91857734c322e19, line 1483 (the morphology functions'
default border value):

static inline Scalar morphologyDefaultBorderValue() { return Scalar::all(DBL_MAX); }

Reference: the opencv repository, path modules/core/src/arithm.cpp, tag 4.11.0,
blob 01a1a02900acd1a404304c0762404d9dd1466910, lines 1151 and 1172:

void cv::absdiff( InputArray src1, InputArray src2, OutputArray dst )
    arithm_op(src1, src2, dst, noArray(), -1, getAbsDiffTab(), false, 0, OCL_OP_ABSDIFF,

The bytes are what cv2 4.13.0 of the goldens shell writes, the version printed
by versions.py; the deployed pin, OpenCV 4.11.0, was read at the lines quoted
above, not run.

Writes the fixture read by crates/corvette-motion/tests/delta.rs: one line per
record, the first field the kind, the first line "counts 120 90 27 36 71"
holding the numbers of absdiff, threshold, dilate, sparse and point records,
which follow in that order. The mode field is "hex" when the record's picture
has at most 400 pixels and "fnv" otherwise; the output field is the lowercase
hex of the result's bytes, row-major, when the mode is hex, and the 16
lowercase hex digits of the FNV-1a 64-bit hash of those bytes (offset basis
0xcbf29ce484222325, prime 0x100000001b3, per byte hash = (hash ^ byte) * prime
mod 2**64) when the mode is fnv. The picture P(seed, lo, span, w, h) comes from
a seeded linear congruential generator the Rust test reimplements: state =
seed, and for each byte state = (state * 1664525 + 1013904223) mod 2**32 and
the byte is state >> 24, row-major; the pixel is lo plus the byte modulo span;
the generator asserts lo + span - 1 <= 255; a picture of span 1 is flat.

"absdiff <mode> <w> <h> <seedA> <loA> <spanA> <seedB> <loB> <spanB> <output>":
cv2.absdiff of P(seedA, loA, spanA, w, h) with P(seedB, loB, spanB, w, h).
"threshold <mode> <w> <h> <seed> <lo> <span> <level> <output>": cv2.threshold
of P(seed, lo, span, w, h) at level with THRESH_BINARY and maximum 255.
"dilate <mode> <w> <h> <seed> <lo> <span> <output>": cv2.dilate of P(seed, lo,
span, w, h) with the default kernel and one iteration. "sparse <mode> <w> <h>
<seed> <modulus> <output>": the picture with 255 where the generator's byte is
a multiple of the modulus and 0 elsewhere, dilated as above. "point <mode> <w>
<h> <x> <y> <output>": the picture that is 0 everywhere and 255 at (x, y),
dilated as above.
"""

from pathlib import Path

import cv2
import numpy as np

OUT = Path(__file__).resolve().parents[2] / "crates/corvette-motion/tests/fixtures/delta_cases.txt"

ABSDIFF_SHAPES = [
    (1, 1),
    (1, 7),
    (7, 1),
    (2, 2),
    (3, 5),
    (8, 6),
    (21, 13),
    (177, 100),
    (100, 177),
    (64, 48),
]

THRESHOLD_SHAPES = [(1, 1), (3, 2), (8, 6), (21, 13), (177, 100)]

LEVELS = [0, 1, 29, 30, 31, 127, 128, 254, 255]

DILATE_SHAPES = [
    (1, 1),
    (1, 6),
    (6, 1),
    (2, 2),
    (3, 3),
    (5, 4),
    (9, 7),
    (177, 100),
    (100, 177),
]

MODULI = [2, 5, 11, 40]

POINT_SHAPES = [(1, 1), (1, 5), (5, 1), (2, 2), (3, 3), (4, 3), (5, 7)]


def lcg_bytes(seed: int, count: int) -> bytes:
    state = seed
    out = bytearray(count)
    for i in range(count):
        state = (state * 1664525 + 1013904223) % (1 << 32)
        out[i] = state >> 24
    return bytes(out)


def fnv1a_hex(data: bytes) -> str:
    value = 0xCBF29CE484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001B3) % (1 << 64)
    return f"{value:016x}"


def picture(seed: int, lo: int, span: int, w: int, h: int):
    assert lo + span - 1 <= 255, (seed, lo, span)
    raw = np.frombuffer(lcg_bytes(seed, w * h), dtype=np.uint8).reshape(h, w)
    return (raw.astype(np.int64) % span + lo).astype(np.uint8)


def sparse_picture(seed: int, modulus: int, w: int, h: int):
    raw = np.frombuffer(lcg_bytes(seed, w * h), dtype=np.uint8)
    return np.where(raw % modulus == 0, 255, 0).astype(np.uint8).reshape(h, w)


def output(result, w: int, h: int) -> tuple[str, str]:
    assert result.dtype == np.uint8 and result.shape == (h, w)
    if w * h <= 400:
        return "hex", result.tobytes().hex()
    return "fnv", fnv1a_hex(result.tobytes())


def absdiff_records() -> list[str]:
    lines = []
    for si, (w, h) in enumerate(ABSDIFF_SHAPES):
        for j in range(12):
            seed_a = 7000 + 100 * si + j
            seed_b = 8000 + 100 * si + j
            lo_a, span_a = (37 * j) % 100, 2 + (53 * j) % 150
            lo_b, span_b = (61 * j + 17) % 100, 2 + (29 * j + 5) % 150
            if j == 9:
                lo_a, span_a, lo_b, span_b = 255, 1, 0, 1
            elif j == 10:
                lo_a, span_a, lo_b, span_b = 0, 1, 255, 1
            elif j == 11:
                seed_b, lo_b, span_b = seed_a, lo_a, span_a
            a = picture(seed_a, lo_a, span_a, w, h)
            b = picture(seed_b, lo_b, span_b, w, h)
            result = cv2.absdiff(a, b)
            if j == 11:
                assert int(result.max()) == 0, (si, j)
            mode, field = output(result, w, h)
            lines.append(
                f"absdiff {mode} {w} {h} {seed_a} {lo_a} {span_a} {seed_b} {lo_b} {span_b} {field}"
            )
    return lines


def threshold_records() -> list[str]:
    lines = []
    for ti, (w, h) in enumerate(THRESHOLD_SHAPES):
        for li, level in enumerate(LEVELS):
            for k in (0, 1):
                seed = 9000 + 1000 * ti + 100 * li + k
                lo, span = (min(max(level - 2, 0), 251), 5) if k == 0 else (0, 256)
                result = cv2.threshold(picture(seed, lo, span, w, h), level, 255, cv2.THRESH_BINARY)[1]
                mode, field = output(result, w, h)
                lines.append(f"threshold {mode} {w} {h} {seed} {lo} {span} {level} {field}")
    return lines


def dilate_records() -> list[str]:
    lines = []
    for di, (w, h) in enumerate(DILATE_SHAPES):
        for j in range(3):
            seed = 10000 + 100 * di + j
            lo, span = 50 * j, 256 - 50 * j
            result = cv2.dilate(picture(seed, lo, span, w, h), None, iterations=1)
            mode, field = output(result, w, h)
            lines.append(f"dilate {mode} {w} {h} {seed} {lo} {span} {field}")
    return lines


def sparse_records() -> list[str]:
    lines = []
    for di, (w, h) in enumerate(DILATE_SHAPES):
        for mi, modulus in enumerate(MODULI):
            seed = 11000 + 100 * di + mi
            result = cv2.dilate(sparse_picture(seed, modulus, w, h), None, iterations=1)
            mode, field = output(result, w, h)
            lines.append(f"sparse {mode} {w} {h} {seed} {modulus} {field}")
    return lines


def point_records() -> list[str]:
    lines = []
    for (w, h) in POINT_SHAPES:
        for y in range(h):
            for x in range(w):
                dot = np.zeros((h, w), dtype=np.uint8)
                dot[y, x] = 255
                result = cv2.dilate(dot, None, iterations=1)
                mode, field = output(result, w, h)
                lines.append(f"point {mode} {w} {h} {x} {y} {field}")
    return lines


def main() -> None:
    absdiff_lines = absdiff_records()
    threshold_lines = threshold_records()
    dilate_lines = dilate_records()
    sparse_lines = sparse_records()
    point_lines = point_records()
    assert (len(absdiff_lines), len(threshold_lines), len(dilate_lines), len(sparse_lines), len(point_lines)) == (120, 90, 27, 36, 71)
    lines = ["counts 120 90 27 36 71"]
    lines.extend(absdiff_lines)
    lines.extend(threshold_lines)
    lines.extend(dilate_lines)
    lines.extend(sparse_lines)
    lines.extend(point_lines)
    assert len(lines) == 345
    OUT.write_text("\n".join(lines) + "\n", encoding="ascii")


main()
