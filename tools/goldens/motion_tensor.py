"""Goldens for corvette-motion's region tensor: the bytes Frigate's deployed
`create_tensor_input` returns for seeded frames, regions and model sizes, the exact
composition of the crop canvas, its I420-to-RGB conversion and the linear resize.

Run command (the wheel's native libraries need the two loader directories):

    cd /home/cvandesande/github/corvette && timeout 300 nix develop .#goldens --command \
        env LD_LIBRARY_PATH=/nix/store/fkcbg2c1w29jr5yp9awai9w3v1wvxdk9-zlib-1.3.2/lib:/nix/store/w2nk3f7prwzpm6h13rfxh8lh46yfdysj-gcc-15.3.0-lib/lib \
        python3 -I tools/goldens/motion_tensor.py <extracted wheel directory> <Frigate checkout directory>

Reference: the frigate repository, path frigate/util/image.py, tag v0.17.2, blob
ea9fb0a0a7f166d7b145c6c3af9941de673aebe3, lines 365 to 491 (`get_yuv_crop` and
`yuv_crop_and_resize`) and lines 680 to 688 (`yuv_region_2_rgb`), and path
frigate/util/object.py, blob b8f41e2c3214c22980f5227055aad348c9d6cd10, lines 287 to
304 (`create_tensor_input`), extracted by line range after the blob check and executed
unchanged in one namespace defining np, cv2, ModelConfig and PixelFormatEnum.

Reference: the opencv repository, path modules/imgproc/src/resize.cpp, tag 4.11.0,
lines 961 to 962 (coefficient bits), 1891 to 1927 and 1964 to 1987 (the horizontal and
vertical kernels), 4011 to 4041 (the exact 2-times area case), 4101 to 4141 (the source
mapping and coefficients), as pinned by the wheel
opencv_python_headless-4.11.0.86-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
(sha256 0e0a27c19dd1f40ddff94976cfe43066fbbe9dfbb2ec1907d66c19caef42a57b); the run refuses
any cv2 version other than 4.11.0.

Frame bytes come from a seeded linear congruential generator the Rust tests
reimplement: state = seed, and for each byte state = (state * 6364136223846793005 +
1442695040888963407) mod 2**64 and the byte is state >> 33, row-major. A frame is the
packed I420 bytes of a frame of the given width and height: height * 3 // 2 rows.

Writes crates/corvette-motion/tests/fixtures/tensor_cases.txt: one case per line,
"<class> <frame_w> <frame_h> <seed> <x1> <y1> <x2> <y2> <model_w> <model_h> <fnv>
<hex>", class exact (the Rust port must match byte for byte: no resize or the exact
2-times shrink) or tolerant (the wheel's SIMD vertical kernel may differ from its
scalar code by one level on a small part of the output); the output's 64-bit FNV-1a
hash in 16 lower-case hex digits and the whole output as hex when it has at most
12288 bytes (else "-").
"""

import hashlib
import subprocess
import sys
import types
from pathlib import Path

sys.path.insert(0, sys.argv[1])
import cv2
import numpy as np

assert cv2.__version__ == "4.11.0", cv2.__version__

FRIGATE_DIR = sys.argv[2]
HEX_LIMIT = 12288

REPO = Path(__file__).resolve().parents[2]
FIXTURE = REPO / "crates/corvette-motion/tests/fixtures/tensor_cases.txt"


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


def source_lines(path, blob):
    raw = git_show(FRIGATE_DIR, f"v0.17.2:{path}")
    assert blob_hash(raw) == blob, "the source does not hash to the quoted blob"
    return raw.decode("utf-8").splitlines()


def extract(lines, first, first_text, last, last_text, namespace):
    assert lines[first - 1].startswith(first_text), "unexpected line range"
    assert lines[last - 1].strip() == last_text, "unexpected line range"
    exec(compile("\n".join(lines[first - 1 : last]), "frigate", "exec"), namespace)


def frigate_namespace():
    class PixelFormatEnum:
        rgb = "rgb"
        bgr = "bgr"
        yuv = "yuv"

    namespace = {"np": np, "cv2": cv2, "ModelConfig": object, "PixelFormatEnum": PixelFormatEnum}
    image = source_lines("frigate/util/image.py", "ea9fb0a0a7f166d7b145c6c3af9941de673aebe3")
    extract(image, 365, "def get_yuv_crop", 491, "return yuv_cropped_frame", namespace)
    extract(image, 680, "def yuv_region_2_rgb", 688, "raise", namespace)
    obj = source_lines("frigate/util/object.py", "b8f41e2c3214c22980f5227055aad348c9d6cd10")
    extract(
        obj,
        287,
        "def create_tensor_input",
        304,
        "return np.expand_dims(cropped_frame, axis=0)",
        namespace,
    )
    return namespace


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


CASES = [
    ("exact", 64, 48, 3, 0, 0, 48, 48, 48, 48),
    ("exact", 64, 48, 3, 0, 0, 48, 48, 24, 24),
    ("exact", 64, 48, 3, 16, 0, 64, 48, 24, 24),
    ("exact", 64, 48, 3, -8, -8, 56, 56, 32, 32),
    ("exact", 64, 48, 3, 4, 4, 52, 52, 24, 24),
    ("exact", 32, 24, 1, 4, 4, 20, 20, 16, 16),
    ("exact", 32, 24, 1, 0, 0, 16, 16, 8, 8),
    ("exact", 32, 24, 1, -4, -4, 28, 28, 32, 32),
    ("exact", 32, 24, 1, -4, -4, 28, 28, 16, 16),
    ("exact", 32, 24, 1, 4, 4, 8, 8, 4, 4),
    ("exact", 32, 24, 1, 4, 4, 8, 8, 2, 2),
    ("exact", 160, 120, 5, 16, 8, 144, 136, 64, 64),
    ("tolerant", 64, 48, 3, 0, 0, 48, 48, 20, 12),
    ("tolerant", 64, 48, 3, 4, 4, 52, 52, 20, 12),
    ("tolerant", 64, 48, 3, -8, -8, 56, 56, 40, 40),
    ("tolerant", 64, 48, 3, 0, 0, 48, 48, 64, 64),
    ("tolerant", 64, 48, 3, 0, 0, 48, 48, 48, 24),
    ("tolerant", 32, 24, 1, 0, 0, 16, 16, 24, 24),
    ("tolerant", 32, 24, 1, -4, -4, 28, 28, 20, 20),
    ("tolerant", 32, 24, 1, 4, 4, 20, 20, 7, 5),
    ("tolerant", 48, 32, 2, 6, 3, 38, 35, 40, 24),
    ("tolerant", 48, 32, 2, 20, 0, 52, 32, 33, 33),
    ("tolerant", 160, 120, 5, 16, 8, 144, 136, 48, 48),
    ("tolerant", 48, 32, 2, 0, 0, 32, 32, 12, 40),
]


def main():
    namespace = frigate_namespace()
    lines = []
    for case_class, fw, fh, seed, x1, y1, x2, y2, mw, mh in CASES:
        frame = np.frombuffer(lcg_bytes(seed, fw * fh * 3 // 2), dtype=np.uint8)
        frame = frame.reshape(fh * 3 // 2, fw)
        config = types.SimpleNamespace(
            width=mw, height=mh, input_pixel_format=namespace["PixelFormatEnum"].rgb
        )
        try:
            result = namespace["create_tensor_input"](frame, config, (x1, y1, x2, y2))
        except Exception as error:
            sys.exit(f"Frigate raised on case {case_class} {fw} {fh} {seed} {x1} {y1} {x2} {y2}: {error}")
        assert result.shape == (1, mh, mw, 3), f"unexpected shape {result.shape}"
        payload = result.tobytes()
        hex_ = payload.hex() if len(payload) <= HEX_LIMIT else "-"
        lines.append(
            f"{case_class} {fw} {fh} {seed} {x1} {y1} {x2} {y2} {mw} {mh} {fnv_hex(payload)} {hex_}"
        )
    FIXTURE.write_text("\n".join(lines) + "\n")


main()
