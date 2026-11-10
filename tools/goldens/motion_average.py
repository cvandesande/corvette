r"""Goldens for corvette-motion's running average: the float32 average that
cv2.accumulateWeighted leaves after every frame of a seeded sequence, and the
bytes cv2.convertScaleAbs returns for it.

The numbers are what the pinned wheel
opencv_python_headless-4.11.0.86-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
(sha256
0e0a27c19dd1f40ddff94976cfe43066fbbe9dfbb2ec1907d66c19caef42a57b, built with
Intel IPP 2021.12.0) writes on the machine that ran the generator; IPP chooses
its code path by CPU, so other CPUs are not verified. The script prints the
version and the CPU feature line of the machine it ran on. Its only argument is
the directory the wheel was extracted into, and the wheel's native libraries
need two loader directories, so the full run command is:

    nix develop .#goldens --command env \
        LD_LIBRARY_PATH=/nix/store/6s4wm3zl2y0v5g998n2g88k1zh3vf2pr-zlib-1.3.2/lib:/nix/store/0ba5kj6r0zhfwd84l5vzccqvx7amlq2c-gcc-15.3.0-lib/lib \
        python3 -I tools/goldens/motion_average.py <extracted wheel directory>

accumulateWeighted of an 8-bit picture into a float32 plane runs a closed IPP
routine, and the arithmetic it performs is a measurement: every pixel update is
new = fma(alpha32, d, old), where alpha32 is alpha rounded to the nearest
float32 (ties to the even significand), d is the float32 difference of the
picture byte and the old average, and fma is one fused rounding of
alpha32 * d + old. The measurement covers 9,689,592 pixel updates: 17 alphas
(including 0, 1, two exact halfway cases between float32 neighbours, a
float32-subnormal alpha, the smallest float32-normal alpha and the smallest
float64 subnormal), 16 shapes of 1 to 16,000 pixels including lengths that are
and are not multiples of 8, 16, 32 and 64, random, flat-255 and flat-0
pictures, 12 frames each, with zero differences from that formula. This
fixture is its permanent record.

With IPP switched off the same wheel runs OpenCV's own portable loop from
accum.simd.hpp instead, which differs from IPP in 10 to 35 percent of the
pixels by one unit in the last place; that loop is not what is deployed and is
not what these numbers record. convertScaleAbs has no IPP path: every build
runs the same portable code, the absolute value rounded to the nearest whole
number with ties to the even number, capped at 255.

Reference: the frigate repository, path frigate/motion/improved_motion.py, tag
v0.17.2, blob b081d37919541e6d1f95762ce084abde81774038, line 35, line 124,
lines 218 to 222, lines 225 to 229 and line 244, copied verbatim:

        self.avg_frame = np.zeros(self.motion_frame_size, np.float32)
        frameDelta = cv2.absdiff(resized_frame, cv2.convertScaleAbs(self.avg_frame))
                cv2.accumulateWeighted(
                    resized_frame,
                    self.avg_frame,
                    0.2 if self.calibrating else self.config.frame_alpha,
                )
            cv2.accumulateWeighted(
                resized_frame,
                self.avg_frame,
                0.2 if self.calibrating else self.config.frame_alpha,
            )
        self.avg_frame = np.zeros(self.motion_frame_size, np.float32)

Reference: the frigate repository, path frigate/config/camera/motion.py, tag
v0.17.2, blob 65c03f73176cb2aab15a57e48f032065bc6cbae6, lines 23 and 24,
copied verbatim:

    delta_alpha: float = Field(default=0.2, title="Delta Alpha")
    frame_alpha: float = Field(default=0.01, title="Frame Alpha")

Reference: the frigate repository, path docker/main/requirements-wheels.txt,
tag v0.17.2, lines 40 to 42, copied verbatim:

numpy == 1.26.*
opencv-python-headless == 4.11.0.*
opencv-contrib-python == 4.11.0.*

Reference: the opencv repository, path modules/imgproc/src/accum.cpp, tag
4.11.0, blob d1e347babc934ed4d05004557271420ae23a4533, line 569 (the 8-bit
source with float32 destination picks ippiAddWeighted_8u32f_C1IR), line 597
(the call, with (Ipp32f)alpha), and line 625 (CV_IPP_RUN tries IPP first),
copied verbatim:

            ippiAddWeighted_I = sdepth == CV_8U && ddepth == CV_32F ? (ippiAddWeighted)ippiAddWeighted_8u32f_C1IR :
                status = CV_INSTRUMENT_FUN_IPP(ippiAddWeighted_I, src.ptr(), srcstep, dst.ptr<Ipp32f>(), dststep, ippiSize(size.width, size.height), (Ipp32f)alpha);
    CV_IPP_RUN((_src.dims() <= 2 || (_src.isContinuous() && _dst.isContinuous() && _mask.isContinuous())), ipp_accumulate_weighted(_src, _dst, alpha, _mask));

Reference: the opencv repository, path modules/imgproc/src/accum.simd.hpp,
tag 4.11.0, blob 13363026135d99ecab2e6b61a485968913ed4cb2, line 2622 (the
first v_fma of the 8-bit to float32 kernel) and line 268 (the scalar statement
of the portable loop that runs only when IPP is off), copied verbatim:

            v_dst00 = v_fma(v_dst00, v_beta, v_mul(v_cvt_f32(v_reinterpret_as_s32(v_src00)), v_alpha));
            dst[i] = src[i]*a + dst[i]*b;

Reference: the opencv repository, path modules/core/src/convert_scale.simd.hpp,
tag 4.11.0, blob b5322ed888340d3df3a797671d261182276a6e40, line 52 (the scalar
statement of the conversion) and line 225 (the 32f-to-8u instantiation),
copied verbatim:

            dst[j] = saturate_cast<_Td>(std::abs(src[j]*a + b));
DEF_CVT_SCALE_ABS_FUNC(32f8u, cvtabs_32f, float,  uchar, float)

Reference: the opencv repository, path modules/core/src/convert_scale.dispatch.cpp,
tag 4.11.0, blob bb9df344d3c0d412ef85d742ed39a85a8d05493e, line 87 (the
convertScaleAbs definition) and line 105 (the call of the picked function),
copied verbatim:

void convertScaleAbs(InputArray _src, OutputArray _dst, double alpha, double beta)
        func( src.ptr(), src.step, 0, 0, dst.ptr(), dst.step, sz, scale );

Reference: the opencv repository, path modules/imgproc/CMakeLists.txt, tag
4.11.0, blob 7bf6087a9a89e9c86f55d5506c712a5f89645310, line 2, copied verbatim:

ocv_add_dispatched_file(accum SSE4_1 AVX AVX2)

Writes the fixture read by crates/corvette-motion/tests/average.rs: one line
per record, the first field average, and the first line "counts 216". Nine
shapes, each with four seeded sequences that start from a new all-zero average
and run six frames; the alpha of a sequence and the seed, low and span of a
frame follow the formulas in the code below, and alphas include the two exact
halfway cases between float32 neighbours, a float32-subnormal alpha, the
smallest float32-normal alpha and a float64 subnormal. The mode field is "hex"
when the picture has at most 100 pixels and "fnv" otherwise. average_output
is, in hex mode, the lowercase hex of each pixel's float32 bit pattern as 8
digits, row-major, concatenated, and in fnv mode the 16 lowercase hex digits
of the FNV-1a 64-bit hash (offset basis 0xcbf29ce484222325, prime
0x100000001b3, per byte hash = (hash ^ byte) * prime mod 2**64) of those bit
patterns written as 4 little-endian bytes each, row-major. image_output is
the bytes of the converted picture, lowercase hex row-major in hex mode and
their FNV-1a hash in fnv mode. The record, in sequence order and frame order:

"average <mode> <w> <h> <sequence> <frame> <alpha_bits> <seed> <lo> <span>
<average_output> <image_output>", where alpha_bits is the 16 lowercase hex
digits of the big-endian IEEE double of alpha (struct.pack(">d", alpha).hex()).

The picture P(seed, lo, span, w, h) comes from a seeded linear congruential
generator the Rust test reimplements: state = seed, and for each byte state =
(state * 1664525 + 1013904223) mod 2**32 and the byte is state >> 24,
row-major; the pixel is lo plus the byte modulo span; the generator asserts
lo + span - 1 <= 255; a picture of span 1 is flat.
"""

import struct
import sys
from pathlib import Path

if len(sys.argv) != 2:
    sys.exit("usage: motion_average.py <directory the opencv-python-headless wheel was extracted into>")

sys.path.insert(0, sys.argv[1])

import cv2
import numpy as np

assert cv2.__version__ == "4.11.0", cv2.__version__
assert cv2.ipp.useIPP(), "the wheel must run with IPP enabled"
IPP_LINES = [line for line in cv2.getBuildInformation().splitlines() if "Intel IPP:" in line]
assert IPP_LINES and "2021.12.0" in IPP_LINES[0], IPP_LINES

OUT = Path(__file__).resolve().parents[2] / "crates/corvette-motion/tests/fixtures/average_cases.txt"

SHAPES = [
    (1, 1),
    (1, 5),
    (1, 33),
    (1, 64),
    (1, 65),
    (7, 9),
    (8, 5),
    (177, 100),
    (133, 100),
]

ALPHAS = [0.2, 0.01, 0.5, 0.99, 1 / 3, 0.37, 0.001, 0.0, 1.0, 0.5 + 2**-25, 0.5 + 3 * 2**-25, 1e-40, 2**-126, 2**-1074]

SEQUENCE_ALPHAS = [0, 1, 3, 4]


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


def average_field(avg, mode: str) -> str:
    if mode == "hex":
        words = avg.view(np.uint32).ravel()
        return "".join(f"{int(word):08x}" for word in words)
    return fnv1a_hex(avg.tobytes())


def image_field(image, mode: str) -> str:
    if mode == "hex":
        return image.tobytes().hex()
    return fnv1a_hex(image.tobytes())


def records() -> list[str]:
    lines = []
    for si, (w, h) in enumerate(SHAPES):
        for qi in range(4):
            sequence = 4 * si + qi
            alpha_index = (si + 4 * qi) % 14 if si < 7 else SEQUENCE_ALPHAS[qi]
            alpha = ALPHAS[alpha_index]
            alpha_bits = struct.pack(">d", alpha).hex()
            avg = np.zeros((h, w), np.float32)
            for f in range(6):
                seed = 20000 + 1000 * si + 100 * qi + f
                lo = (41 * f + 7 * qi) % 100
                span = 2 + (67 * f + 13 * qi + si) % 150
                if qi == 0 and f >= 3:
                    lo, span = 255, 1
                if qi == 1 and f >= 4:
                    lo, span = 0, 1
                cv2.accumulateWeighted(picture(seed, lo, span, w, h), avg, alpha)
                assert avg.dtype == np.float32 and avg.shape == (h, w)
                image = cv2.convertScaleAbs(avg)
                assert image.dtype == np.uint8 and image.shape == (h, w)
                mode = "hex" if w * h <= 100 else "fnv"
                lines.append(
                    f"average {mode} {w} {h} {sequence} {f} {alpha_bits} {seed} {lo} {span} "
                    f"{average_field(avg, mode)} {image_field(image, mode)}"
                )
    return lines


def main() -> None:
    print(cv2.__version__, cv2.getCPUFeaturesLine())
    body = records()
    assert len(body) == 216, len(body)
    lines = ["counts 216"]
    lines.extend(body)
    assert len(lines) == 217
    OUT.write_text("\n".join(lines) + "\n", encoding="ascii")


main()
