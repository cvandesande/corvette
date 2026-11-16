"""Goldens for corvette-motion's crop-and-pad and its I420-to-RGB conversion: the square
canvas Frigate's deployed `yuv_crop_and_resize` returns for seeded frames and regions, and
the bytes `cv2.cvtColor(..., cv2.COLOR_YUV2RGB_I420)` returns for seeded and flat canvases.

Run command (the wheel's native libraries need the two loader directories):

    cd /home/cvandesande/github/corvette && timeout 300 nix develop .#goldens --command \
        env LD_LIBRARY_PATH=/nix/store/fkcbg2c1w29jr5yp9awai9w3v1wvxdk9-zlib-1.3.2/lib:/nix/store/w2nk3f7prwzpm6h13rfxh8lh46yfdysj-gcc-15.3.0-lib/lib \
        python3 -I tools/goldens/motion_crop.py <extracted wheel directory> <Frigate checkout directory>

Reference: the frigate repository, path frigate/util/image.py, tag v0.17.2, blob
ea9fb0a0a7f166d7b145c6c3af9941de673aebe3, lines 365 to 491 (`get_yuv_crop` and
`yuv_crop_and_resize`), extracted by line range after the blob check and executed
unchanged in a namespace defining np and cv2.

Reference: the opencv repository, path modules/imgproc/src/color_yuv.simd.hpp, tag
4.11.0, lines 1018 to 1099 (the fixed-point conversion), as pinned by the wheel
opencv_python_headless-4.11.0.86-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
(sha256 0e0a27c19dd1f40ddff94976cfe43066fbbe9dfbb2ec1907d66c19caef42a57b); the run refuses
any cv2 version other than 4.11.0.

Frame and canvas bytes come from a seeded linear congruential generator the Rust tests
reimplement: state = seed, and for each byte state = (state * 6364136223846793005 +
1442695040888963407) mod 2**64 and the byte is state >> 33, row-major. A frame is the
packed I420 bytes of a frame of the given width and height: height * 3 // 2 rows.

Writes crates/corvette-motion/tests/fixtures/crop_pad_cases.txt: one case per line,
"<width> <height> <seed> <x1> <y1> <x2> <y2> <size> <fnv> <hex>", the canvas's 64-bit
FNV-1a hash in 16 lower-case hex digits and the whole canvas as hex when size <= 16
(else "-"); the last lines are "<raises> <width> <height> <x1> <y1> <x2> <y2>" for the
probed geometries Frigate raised on, observed against the seed-1 frame. Writes
crates/corvette-motion/tests/fixtures/i420_rgb_cases.txt: "<side> <kind> <a> <b> <c>
<fnv> <hex>", kind seed (canvas from seed a) or flat (Y byte a, U byte b, V byte c), the
conversion's FNV-1a and its hex when side <= 8.
"""

import hashlib
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, sys.argv[1])
import cv2
import numpy as np

assert cv2.__version__ == "4.11.0", cv2.__version__

FRIGATE_DIR = sys.argv[2]
IMAGE_PATH = "frigate/util/image.py"
IMAGE_BLOB = "ea9fb0a0a7f166d7b145c6c3af9941de673aebe3"
IMAGE_FIRST = 365
IMAGE_LAST = 491

REPO = Path(__file__).resolve().parents[2]
FIXTURES = REPO / "crates/corvette-motion/tests/fixtures"


def blob_hash(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def git_show(frigate_dir, ref):
    try:
        return subprocess.run(
            ["git", "-C", frigate_dir, "show", ref],
            capture_output=True,
            check=True,
        ).stdout
    except (OSError, subprocess.SubprocessError) as error:
        sys.exit(f"cannot read {ref} from {frigate_dir}: {error}")


def frigate_functions():
    raw = git_show(FRIGATE_DIR, f"v0.17.2:{IMAGE_PATH}")
    assert blob_hash(raw) == IMAGE_BLOB, "the source does not hash to the quoted blob"
    lines = raw.decode("utf-8").splitlines()
    assert lines[IMAGE_FIRST - 1].startswith("def get_yuv_crop"), "unexpected line range"
    assert lines[IMAGE_LAST - 1].strip() == "return yuv_cropped_frame", "unexpected line range"
    namespace = {"np": np, "cv2": cv2}
    exec(
        compile("\n".join(lines[IMAGE_FIRST - 1 : IMAGE_LAST]), IMAGE_PATH, "exec"),
        namespace,
    )
    return namespace["yuv_crop_and_resize"]


def lcg_bytes(seed, count):
    state = seed
    out = bytearray(count)
    for i in range(count):
        state = (state * 6364136223846793005 + 1442695040888963407) & 0xFFFFFFFFFFFFFFFF
        out[i] = (state >> 33) & 0xFF
    return bytes(out)


def frame_bytes(width, height, seed):
    return lcg_bytes(seed, width * height * 3 // 2)


def fnv_hex(payload):
    hash_ = 14695981039346656037
    for byte in payload:
        hash_ = ((hash_ ^ byte) * 1099511628211) & 0xFFFFFFFFFFFFFFFF
    return f"{hash_:016x}"


def crop_case(crop_and_resize, width, height, seed, region):
    frame = np.frombuffer(frame_bytes(width, height, seed), dtype=np.uint8)
    frame = frame.reshape(height * 3 // 2, width)
    try:
        canvas = crop_and_resize(frame, region)
    except Exception as error:
        sys.exit(f"Frigate raised on case {width} {height} {seed} {region}: {error}")
    assert canvas.shape[1] == (region[3] - region[1]) // 4 * 4
    assert canvas.shape[0] == canvas.shape[1] + canvas.shape[1] // 2
    payload = canvas.tobytes()
    size = canvas.shape[1]
    hex_ = payload.hex() if size <= 16 else "-"
    return (
        f"{width} {height} {seed} {region[0]} {region[1]} {region[2]} {region[3]} "
        f"{size} {fnv_hex(payload)} {hex_}"
    )


CROP_CASES = [
    (32, 24, 1, (4, 4, 20, 20)),
    (32, 24, 1, (0, 0, 16, 16)),
    (32, 24, 1, (2, 2, 22, 26)),
    (32, 24, 1, (6, 4, 22, 20)),
    (32, 24, 1, (0, 0, 32, 32)),
    (32, 24, 1, (24, 0, 32, 24)),
    (32, 24, 1, (0, 8, 16, 24)),
    (32, 24, 1, (2, 0, 18, 16)),
    (32, 24, 1, (12, 8, 20, 16)),
    (32, 24, 1, (16, 0, 36, 16)),
    (32, 24, 1, (12, 4, 40, 24)),
    (32, 24, 1, (8, 12, 24, 40)),
    (32, 24, 1, (-4, -4, 28, 28)),
    (32, 24, 1, (-4, 0, 12, 16)),
    (32, 24, 1, (-8, -4, 24, 36)),
    (32, 24, 1, (0, -8, 16, 24)),
    (32, 24, 1, (4, 4, 8, 8)),
    (32, 24, 1, (0, 0, 4, 4)),
    (32, 24, 1, (0, 0, 12, 12)),
    (32, 24, 1, (8, 4, 16, 12)),
    (32, 24, 1, (10, 6, 18, 14)),
    (32, 24, 1, (24, 0, 36, 12)),
    (32, 24, 1, (-4, -4, 8, 8)),
    (48, 32, 2, (0, 0, 32, 32)),
    (48, 32, 2, (16, 0, 48, 32)),
    (48, 32, 2, (0, 16, 16, 32)),
    (48, 32, 2, (20, 0, 52, 32)),
    (48, 32, 2, (6, 3, 38, 35)),
    (48, 32, 2, (10, 10, 42, 42)),
    (48, 32, 2, (2, 6, 34, 38)),
    (64, 48, 3, (0, 0, 48, 48)),
    (64, 48, 3, (16, 0, 64, 48)),
    (64, 48, 3, (16, 16, 64, 64)),
    (64, 48, 3, (16, 0, 68, 48)),
    (64, 48, 3, (0, 24, 48, 72)),
    (64, 48, 3, (4, 4, 52, 52)),
    (64, 48, 3, (-8, -8, 56, 56)),
    (64, 48, 3, (6, 3, 54, 51)),
    (32, 32, 4, (0, 0, 32, 32)),
    (32, 32, 4, (16, 16, 32, 32)),
    (32, 32, 4, (-4, -4, 28, 28)),
    (32, 32, 4, (4, 4, 36, 36)),
    (32, 32, 4, (-8, -8, 24, 24)),
    (32, 32, 4, (-4, -4, 36, 36)),
    (50, 26, 5, (0, 0, 24, 24)),
    (50, 26, 5, (12, 2, 36, 26)),
    (50, 26, 5, (26, 0, 50, 24)),
    (50, 26, 5, (26, 0, 54, 24)),
    (50, 26, 5, (0, 2, 24, 26)),
    (60, 44, 6, (4, 4, 36, 36)),
    (60, 44, 6, (12, 8, 44, 40)),
    (60, 44, 6, (30, 4, 60, 38)),
    (60, 44, 6, (20, 0, 60, 40)),
    (60, 44, 6, (0, 4, 40, 44)),
    (60, 44, 6, (24, 2, 64, 42)),
    (36, 40, 7, (0, 0, 36, 40)),
    (36, 40, 7, (2, 2, 34, 38)),
    (36, 40, 7, (-4, -4, 32, 36)),
    (36, 40, 7, (-4, -4, 40, 44)),
    (64, 24, 8, (48, 8, 64, 24)),
    (64, 24, 8, (0, 0, 64, 64)),
    (32, 48, 9, (0, 0, 32, 32)),
    (32, 48, 9, (8, 16, 24, 32)),
    (32, 48, 9, (0, 8, 24, 32)),
    (32, 48, 9, (0, 8, 32, 40)),
    (704, 576, 11, (64, 64, 384, 384)),
    (704, 576, 11, (388, 64, 708, 384)),
    (704, 576, 11, (64, 256, 384, 576)),
    (704, 576, 11, (64, 300, 384, 620)),
    (704, 576, 11, (184, 64, 512, 392)),
    (704, 576, 11, (380, 100, 708, 428)),
    (704, 576, 11, (0, 0, 336, 336)),
    (704, 576, 11, (372, 60, 708, 396)),
    (704, 576, 11, (100, 240, 436, 576)),
    (704, 576, 11, (0, 80, 400, 480)),
    (704, 576, 11, (308, 100, 708, 500)),
    (704, 576, 11, (260, 176, 660, 596)),
    (704, 576, 11, (0, 0, 640, 640)),
    (704, 576, 11, (60, 0, 700, 640)),
    (704, 576, 11, (64, 0, 704, 640)),
    (704, 576, 11, (4, 4, 704, 704)),
    (704, 576, 11, (2, 2, 702, 702)),
    (704, 576, 11, (5, 5, 705, 705)),
    (640, 480, 12, (160, 80, 480, 400)),
    (640, 480, 12, (324, 80, 652, 408)),
    (640, 480, 12, (0, 16, 336, 352)),
    (640, 480, 12, (308, 64, 644, 400)),
    (640, 480, 12, (196, 148, 532, 484)),
    (640, 480, 12, (100, 40, 500, 440)),
    (640, 480, 12, (244, 40, 644, 440)),
    (640, 480, 12, (0, 0, 640, 640)),
    (640, 480, 12, (0, 0, 644, 644)),
    (640, 480, 12, (0, 0, 700, 700)),
    (704, 576, 11, (151, 155, 480, 484)),
    (704, 576, 11, (369, 509, 374, 514)),
    (64, 48, 13, (55, 31, 655, 39)),
    (64, 48, 13, (0, 0, 5, 4)),
]

CROP_RAISE_PROBES = [
    (32, 24, 1, (0, 0, 20, 16)),
    (32, 24, 1, (0, 0, 16, 1)),
    (32, 24, 1, (32, 0, 48, 16)),
    (32, 24, 1, (-16, -16, -4, -4)),
    (32, 24, 1, (-1, -1, 4, 4)),
    (64, 48, 1, (0, 0, 6, 4)),
    (64, 48, 1, (-1, 0, 4, 4)),
]

COLOUR_SEED_SIDES = [(4, 21), (4, 22), (4, 23), (8, 21), (8, 22), (8, 23), (12, 21), (12, 22),
                     (12, 23), (16, 21), (16, 22), (16, 23), (32, 21), (32, 22), (32, 23),
                     (64, 21), (64, 22), (64, 23), (320, 24), (320, 26), (640, 25), (640, 27)]
COLOUR_FLAT_Y = [0, 16, 17, 128, 235, 255]
COLOUR_FLAT_UV = [(128, 128), (0, 0), (255, 255), (0, 255), (255, 0), (64, 192)]


def rgb_case(canvas_bytes, side):
    canvas = np.frombuffer(canvas_bytes, dtype=np.uint8).reshape(side + side // 2, side)
    rgb = cv2.cvtColor(canvas, cv2.COLOR_YUV2RGB_I420)
    assert rgb.shape == (side, side, 3)
    payload = rgb.tobytes()
    hex_ = payload.hex() if side <= 8 else "-"
    return fnv_hex(payload), hex_


def main():
    crop_and_resize = frigate_functions()

    crop_lines = [
        crop_case(crop_and_resize, width, height, seed, region)
        for width, height, seed, region in CROP_CASES
    ]
    for width, height, seed, region in CROP_RAISE_PROBES:
        frame = np.frombuffer(frame_bytes(width, height, seed), dtype=np.uint8)
        frame = frame.reshape(height * 3 // 2, width)
        try:
            crop_and_resize(frame, region)
        except Exception:
            crop_lines.append(
                f"raises {width} {height} {region[0]} {region[1]} {region[2]} {region[3]}"
            )
    assert any(line.startswith("raises") for line in crop_lines), "no probe raised at all"
    (FIXTURES / "crop_pad_cases.txt").write_text("\n".join(crop_lines) + "\n", encoding="ascii")

    colour_lines = []
    for side, seed in COLOUR_SEED_SIDES:
        canvas_bytes = lcg_bytes(seed, side * (side + side // 2))
        fnv, hex_ = rgb_case(canvas_bytes, side)
        colour_lines.append(f"{side} seed {seed} 0 0 {fnv} {hex_}")
    for y in COLOUR_FLAT_Y:
        for (u, v) in COLOUR_FLAT_UV:
            side = 8
            chroma_plane = side * side // 4
            canvas_bytes = bytes([y]) * (side * side) + bytes([u]) * chroma_plane + bytes([v]) * chroma_plane
            fnv, hex_ = rgb_case(canvas_bytes, side)
            colour_lines.append(f"{side} flat {y} {u} {v} {fnv} {hex_}")
    (FIXTURES / "i420_rgb_cases.txt").write_text("\n".join(colour_lines) + "\n", encoding="ascii")

    print(f"crop cases: {sum(1 for line in crop_lines if not line.startswith('raises'))}")
    print(f"crop raises: {sum(1 for line in crop_lines if line.startswith('raises'))}")
    print(f"colour cases: {len(colour_lines)}")


main()
