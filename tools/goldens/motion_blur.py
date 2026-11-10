r"""Goldens for corvette-motion's Gaussian blur: the bytes
scipy.ndimage.gaussian_filter writes for a uint8 grey picture at sigma 1 and
radius 1.

Reference: the frigate repository, path frigate/motion/improved_motion.py, tag
v0.17.2, blob b081d37919541e6d1f95762ce084abde81774038, lines 5, 23, 41 and 116,
copied verbatim:

from scipy.ndimage import gaussian_filter
        blur_radius=1,
        self.blur_radius = blur_radius
        resized_frame = gaussian_filter(resized_frame, sigma=1, radius=self.blur_radius)

Reference: the scipy repository, path scipy/ndimage/_filters.py, tag v1.16.0,
blob 8079d32993392aaa99f9e0ba3df5a23b3d4b9b32, lines 662 to 664, 745 to 746,
750 to 751 and 852 to 855, copied verbatim:

    x = np.arange(-radius, radius+1)
    phi_x = np.exp(-0.5 / sigma2 * x ** 2)
    phi_x = phi_x / phi_x.sum()
    if radius is not None:
        lw = radius
    weights = _gaussian_kernel1d(sigma, order, lw)[::-1]
    return correlate1d(input, weights, axis, output, mode, cval, 0)
        for axis, sigma, order, mode, radius in axes:
            gaussian_filter1d(input, sigma, axis, order, output,
                              mode, cval, truncate, radius=radius)
            input = output

Reference: the scipy repository, path scipy/ndimage/src/ni_filters.c, tag
v1.16.0, blob 8d063dd2a3c55fefd0871185c5fb4b6d3fe39c5e, lines 104 to 106,
copied verbatim:

                    oline[ll] = iline[0] * fw[0];
                    for(jj = -size1 ; jj < 0; jj++)
                        oline[ll] += (iline[jj] + iline[-jj]) * fw[jj];

Reference: the scipy repository, path scipy/ndimage/src/ni_support.c, tag
v1.16.0, blob a6e3ff06a33954c5a09db7bc0a0ee5da42e3fb96, lines 266 to 276,
442 to 443 and 419, copied verbatim:

        case NI_EXTEND_REFLECT:
            src = first;
            dst = first - 1;
            while (size_before && src < last) {
                *dst-- = *src++;
                --size_before;
            }
            src = last - 1;
            while (size_before--) {
                *dst-- = *src--;
            }
            CASE_COPY_LINE_TO_DATA(NPY_UBYTE, npy_ubyte,
                                   pb, pa, length, buffer->line_stride);
        *(_type *)_po = (_type)_pi[_ii];                                 \

The bytes are what gaussian_filter of the goldens shell's scipy writes for a
uint8 input at the default mode; versions.py prints that shell's Python, numpy
and scipy versions, and scipy 1.18.0's code above was read at the v1.16.0
lines. The deployed pins numpy 1.26 and scipy 1.16 were not run. The picture
bytes come from a seeded linear congruential generator the Rust test
reimplements: state = seed, and for each byte state = (state * 1664525 +
1013904223) mod 2**32 and the byte is state >> 24, row-major.

Writes the fixture read by crates/corvette-motion/tests/blur.rs: one line per
record, first field the kind. "weights <outer_hex> <centre_hex>" first: the
kernel weights' float64 bit patterns as 16-digit lower-case hex. "seeded <sw>
<sh> <seed> <out_hex>": a seeded sw by sh picture, the blurred bytes as hex.
"explicit <sw> <sh> <src_hex> <out_hex>": a hand-built picture and its blur.
"level <v> <out>": a flat 4 by 3 picture of level v blurred, the one output
byte in decimal.
"""

from pathlib import Path
import struct

import numpy
import scipy.ndimage
import scipy.ndimage._filters

OUT = Path(__file__).resolve().parents[2] / "crates/corvette-motion/tests/fixtures/blur_cases.txt"

SEEDED = [
    (122, 100),
    (133, 100),
    (81, 100),
    (100, 122),
    (1, 1),
    (1, 5),
    (5, 1),
    (2, 2),
    (3, 3),
    (7, 5),
    (5, 7),
    (13, 1),
    (1, 13),
    (64, 48),
]


def lcg_bytes(seed: int, count: int) -> bytes:
    state = seed
    out = bytearray(count)
    for i in range(count):
        state = (state * 1664525 + 1013904223) % (1 << 32)
        out[i] = state >> 24
    return bytes(out)


def bits(value: float) -> str:
    return format(struct.unpack("<Q", struct.pack("<d", value))[0], "016x")


def blurred(picture):
    out = scipy.ndimage.gaussian_filter(picture, sigma=1, radius=1)
    assert out.dtype == numpy.uint8
    assert out.shape == picture.shape
    return out


def main() -> None:
    lines = []

    x = numpy.arange(-1, 2)
    phi = numpy.exp(-0.5 / 1.0 * x ** 2)
    phi = phi / phi.sum()
    assert numpy.array_equal(phi, scipy.ndimage._filters._gaussian_kernel1d(1.0, 0, 1))
    lines.append(f"weights {bits(phi[0])} {bits(phi[1])}")

    for (sw, sh), seed in zip(SEEDED, range(201, 215), strict=True):
        picture = numpy.frombuffer(lcg_bytes(seed, sw * sh), dtype=numpy.uint8).reshape(sh, sw)
        lines.append(f"seeded {sw} {sh} {seed} {blurred(picture).tobytes().hex()}")

    rows = numpy.tile(numpy.arange(256, dtype=numpy.uint8), (4, 1))
    columns = numpy.tile(numpy.arange(256, dtype=numpy.uint8).reshape(256, 1), (1, 4))
    xs, ys = numpy.meshgrid(numpy.arange(8), numpy.arange(8))
    checkerboard = numpy.where((xs + ys) % 2 == 0, 255, 0).astype(numpy.uint8)
    spot = numpy.full((5, 5), 255, dtype=numpy.uint8)
    spot[2, 2] = 0
    for picture in (rows, columns, checkerboard, spot):
        sh, sw = picture.shape
        lines.append(
            f"explicit {sw} {sh} {picture.tobytes().hex()} {blurred(picture).tobytes().hex()}"
        )

    for v in range(256):
        flat = numpy.full((3, 4), v, dtype=numpy.uint8)
        out = blurred(flat)
        assert (out == out[0, 0]).all()
        lines.append(f"level {v} {out[0, 0].item()}")

    OUT.write_text("\n".join(lines) + "\n", encoding="ascii")


main()
