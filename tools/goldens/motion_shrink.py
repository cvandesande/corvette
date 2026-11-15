r"""Goldens for corvette-motion's keep-mask shrink: for a frame-size 0/255 mask
picture and a motion frame size, the zero set of the deployed shrink — the
deployed software shrinks the mask with cv2.resize(..., INTER_AREA) and treats
every pixel of the result that is 0 as masked, so a line holds one case's mask
picture and the result's zero set (1 bits mark shrunk pixels whose byte is 0).

The numbers are what the pinned wheel
opencv_python_headless-4.11.0.86-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
(sha256
0e0a27c19dd1f40ddff94976cfe43066fbbe9dfbb2ec1907d66c19caef42a57b) writes. The
first argument is the extracted wheel directory, the second the Frigate checkout
directory; the script reads the two reference files at tag v0.17.2 with git
show and refuses the bytes unless they hash to the blobs quoted below, executes
the image.py lines in a namespace holding only numpy as np and cv2 as cv2, and
quotes the improved_motion lines without executing them: it writes their two
statements itself with the case's picture and size. The wheel's native
libraries need two loader directories, so the full run command is:

    nix develop .#goldens --command env \
        LD_LIBRARY_PATH=/nix/store/6s4wm3zl2y0v5g998n2g88k1zh3vf2pr-zlib-1.3.2/lib:/nix/store/0ba5kj6r0zhfwd84l5vzccqvx7amlq2c-gcc-15.3.0-lib/lib \
        python3 -I tools/goldens/motion_shrink.py <extracted wheel directory> <Frigate checkout directory>

Measured with the wheel: cv2.resize(picture, dsize=(to_width, to_height),
interpolation=cv2.INTER_AREA) followed by np.where(resized == [0]), under the
versions the run prints: Python 3.14.6, numpy 2.5.1, cv2 4.11.0 with
version-control line "4.11.0" and cv2.ipp.useIPP() True; every run prints them
again and refuses to write unless cv2 is 4.11.0. For every case a float32
replica of OpenCV's area-table accumulation must give accumulated < 0.5 exactly
where the wheel gives 0, and the script prints the smallest abs(accumulated -
0.5) over every cell of every case. Read, not measured: the OpenCV build of the
deployed image (this fixture is evidence about the version printed above and
nothing else); the CPU code path of the wheel, where the research found the
generic and the vendor paths byte-identical on the nine cases it ran; the
f32-against-f64 difference between the wheel's accumulation and an f64 port,
which shows only in the printed margin.

Reference: the frigate repository, path frigate/motion/improved_motion.py, tag
v0.17.2, blob b081d37919541e6d1f95762ce084abde81774038, lines 234 to 240,
copied verbatim:

        def update_mask(self) -> None:
            resized_mask = cv2.resize(
                self.config.mask,
                dsize=(self.motion_frame_size[1], self.motion_frame_size[0]),
                interpolation=cv2.INTER_AREA,
            )
            self.mask = np.where(resized_mask == [0])

and path frigate/util/image.py, blob ea9fb0a0a7f166d7b145c6c3af9941de673aebe3,
lines 911 to 943, copied verbatim (create_mask makes the camera pictures):

    def create_mask(frame_shape, mask):
        mask_img = np.zeros(frame_shape, np.uint8)
        mask_img[:] = 255

        if isinstance(mask, list):
            for m in mask:
                add_mask(m, mask_img)

        elif isinstance(mask, str):
            add_mask(mask, mask_img)

        return mask_img


    def add_mask(mask: str, mask_img: np.ndarray):
        points = mask.split(",")

        # masks and zones are saved as relative coordinates
        # we know if any points are > 1 then it is using the
        # old native resolution coordinates
        if any(x > "1.0" for x in points):
            raise Exception("add mask expects relative coordinates only")

        contour = np.array(
            [
                [
                    int(float(points[i]) * mask_img.shape[1]),
                    int(float(points[i + 1]) * mask_img.shape[0]),
                ]
                for i in range(0, len(points), 2)
            ]
        )
        cv2.fillPoly(mask_img, pts=[contour], color=(0))

Writes the fixture crates/corvette-motion/tests/shrink.rs reads: a first line
"counts camera 3 edge 6 pixel 1167 random 100 wide 12 large 2", then one line
per case, the kinds in the header's order:

"case <kind> <index> <width> <height> <to_width> <to_height> <input> = <hex>":
<input> is, for camera, the polygon strings joined by ";" exactly as passed to
create_mask; for every other kind the mask picture bit-packed: one bit per
pixel, row-major, 1 where the byte is 0, first pixel in the high bit of the
first digit, lowercase, zero-padded, length ceil(width * height / 4). <hex> is
the deployed result bit-packed the same way, 1 where resized == 0, length
ceil(to_width * to_height / 4). One space between fields, no trailing space.

Kinds: camera is the three deployed masks (704x576 -> 122x100 twice, 640x480 ->
133x100), asserted shrunk zero counts 117, 176 and 228. edge is, per pixel
geometry below, an all-255 picture (result has no zero, asserted) and an all-0
picture (result all zero, asserted). pixel is one case per source pixel of the
three geometries 23x17 -> 9x7 (both ratios fractional), 24x17 -> 12x7 (width
ratio whole) and 23x16 -> 9x8 (height ratio whole), each picture with exactly
that pixel at 255 and every other byte 0, ordered by geometry then row-major
(391, 408 and 368 cases). random is 100 cases: geometry (i // 3) % 3 from
127x90 -> 33x24, 64x48 -> 30x23, 77x33 -> 56x24; share of pixels at 255 i % 3
from 0.25, 0.1, 0.02; every pixel drawn independently with that probability.
wide is 12 cases at 233x177 -> 13x11 (cells about 18 pixels on a side) with
shares i % 3 from 0.01, 0.003, 0.001. large is 704x576 -> 122x100 with share
0.006 and 640x480 -> 133x100 with share 0.005. Each random, wide and large case
draws from random.Random(20261115 + 1000 * kind_number + index), kind numbers
from 0 in header order; a case whose deployed result is all zero or all
non-zero is redrawn with the seed advanced by 100000 per retry, at most 20
retries (these shares make one retry the observed worst; more than 20 is an
error), and the run prints every retry count that was needed. The script also
asserts the fixture is reproducible (built twice in memory from fresh seeds and
compared), is at most 1,000,000 bytes, and has header counts equal to the cases
written.
"""

import hashlib
import platform
import random
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, sys.argv[1])
import cv2
import numpy as np

OUT = Path(__file__).resolve().parents[2] / "crates/corvette-motion/tests/fixtures/shrink_cases.txt"
MOTION_PATH = "frigate/motion/improved_motion.py"
MOTION_BLOB = "b081d37919541e6d1f95762ce084abde81774038"
IMAGE_PATH = "frigate/util/image.py"
IMAGE_BLOB = "ea9fb0a0a7f166d7b145c6c3af9941de673aebe3"
SEED = 20261115
KINDS = ["camera", "edge", "pixel", "random", "wide", "large"]
LIMIT = 1_000_000
CAMERAS = [
    (704, 576, 122, 100, ["0.75,0.033,0.968,0.036,0.968,0.089,0.751,0.087"], 117),
    (704, 576, 122, 100, ["0.66,0.035,0.66,0.087,0.981,0.089,0.984,0.031"], 176),
    (640, 480, 133, 100, ["0.714,0.068,0.714,0,1,0,1,0.065"], 228),
]
GEOMETRIES = [(23, 17, 9, 7), (24, 17, 12, 7), (23, 16, 9, 8)]
RANDOM_GEOMS = [(127, 90, 33, 24), (64, 48, 30, 23), (77, 33, 56, 24)]
RANDOM_SHARES = [0.25, 0.1, 0.02]
WIDE_GEOM = (233, 177, 13, 11)
WIDE_SHARES = [0.01, 0.003, 0.001]
LARGE_CASES = [(704, 576, 122, 100, 0.006), (640, 480, 133, 100, 0.005)]


def area_tab(ssize, dsize, cn, scale):
    """Mirrors computeResizeAreaTab (resize.cpp:3334) -> list of (di, si, alpha_float)."""
    import math
    tab = []
    for dx in range(dsize):
        fsx1 = dx * scale
        fsx2 = fsx1 + scale
        cellWidth = min(scale, ssize - fsx1)
        sx1 = math.ceil(fsx1)
        sx2 = math.floor(fsx2)
        sx2 = min(sx2, ssize - 1)
        sx1 = min(sx1, sx2)
        if sx1 - fsx1 > 1e-3:
            tab.append((dx, sx1 - 1, np.float32((sx1 - fsx1) / cellWidth)))
        for sx in range(sx1, sx2):
            tab.append((dx, sx, np.float32(1.0 / cellWidth)))
        if fsx2 - sx2 > 1e-3:
            tab.append((dx, sx2, np.float32(min(min(fsx2 - sx2, 1.0), cellWidth) / cellWidth)))
    return tab


def resize_area_repro(src, dsize_w, dsize_h):
    h, w = src.shape
    sx, sy = w / dsize_w, h / dsize_h
    if not (sx >= 1 and sy >= 1):
        return None
    fast = abs(sx - round(sx)) < np.finfo(float).eps and abs(sy - round(sy)) < np.finfo(float).eps
    if fast:
        return None
    xtab = area_tab(w, dsize_w, 1, sx)
    ytab = area_tab(h, dsize_h, 1, sy)
    xs = {}
    for di, si, a in xtab:
        xs.setdefault(di, []).append((si, a))
    S = src.astype(np.float32)
    buf = np.zeros((h, dsize_w), np.float32)
    for dx, entries in xs.items():
        col = np.zeros(h, np.float32)
        for si, a in entries:
            col = (col + S[:, si] * a).astype(np.float32)
        buf[:, dx] = col
    ys = {}
    for di, si, a in ytab:
        ys.setdefault(di, []).append((si, a))
    out = np.zeros((dsize_h, dsize_w), np.float32)
    for dy, entries in ys.items():
        acc = np.zeros(dsize_w, np.float32)
        for si, a in entries:
            acc = (acc + buf[si] * a).astype(np.float32)
        out[dy] = acc
    return out


def pack(mask):
    bits = [1 if byte == 0 else 0 for byte in mask.reshape(-1)]
    digits = []
    for start in range(0, len(bits), 4):
        group = bits[start : start + 4]
        value = 0
        for bit in group:
            value = value * 2 + bit
        digits.append(format(value << (4 - len(group)), "x"))
    return "".join(digits)


def draw(w, h, share, seed):
    rng = random.Random(seed)
    return np.array([(255 if rng.random() < share else 0) for _ in range(w * h)], np.uint8).reshape(h, w)


class Writer:
    def __init__(self):
        self.blocks = {kind: [] for kind in KINDS}
        self.worst = float("inf")
        self.redraws = []

    def record(self, kind, index, w, h, tw, th, input_field, picture):
        resized = cv2.resize(picture, dsize=(tw, th), interpolation=cv2.INTER_AREA)
        where = np.where(resized == [0])
        fl = resize_area_repro(picture, tw, th)
        assert fl is not None, (kind, index, "the replica refuses the pair")
        assert np.array_equal(where, np.where(fl < np.float32(0.5))), (kind, index, "replica disagrees with the wheel")
        self.worst = min(self.worst, float(np.abs(fl - np.float32(0.5)).min()))
        hex_out = pack(resized)
        assert len(hex_out) == -(-tw * th // 4), (kind, index)
        assert kind == "camera" or len(input_field) == -(-w * h // 4), (kind, index)
        self.blocks[kind].append(
            " ".join(["case", kind, str(index), str(w), str(h), str(tw), str(th), input_field, "=", hex_out])
        )
        return int((resized == 0).sum())

    def seeded(self, kind, kind_number, index, w, h, tw, th, share):
        seed = SEED + 1000 * kind_number + index
        tried = []
        for attempt in range(21):
            picture = draw(w, h, share, seed)
            resized = cv2.resize(picture, dsize=(tw, th), interpolation=cv2.INTER_AREA)
            zeros = int((resized == 0).sum())
            if 0 < zeros < tw * th:
                if attempt:
                    self.redraws.append((kind, index, attempt))
                self.record(kind, index, w, h, tw, th, pack(picture), picture)
                return
            tried.append((seed, zeros))
            seed += 100000
        raise AssertionError(("more than 20 redraws", kind, index, tried))


def build(create_mask):
    out = Writer()
    for index, (w, h, tw, th, strings, zeros) in enumerate(CAMERAS):
        assert tw == 100 * w // h and th == 100, index
        picture = create_mask((h, w), list(strings))
        assert out.record("camera", index, w, h, tw, th, ";".join(strings), picture) == zeros, index
    for index, (w, h, tw, th) in enumerate(GEOMETRIES):
        assert out.record("edge", 2 * index, w, h, tw, th, pack(np.full((h, w), 255, np.uint8)),
                          np.full((h, w), 255, np.uint8)) == 0
        assert out.record("edge", 2 * index + 1, w, h, tw, th, pack(np.zeros((h, w), np.uint8)),
                          np.zeros((h, w), np.uint8)) == tw * th
    index = 0
    for w, h, tw, th in GEOMETRIES:
        for y in range(h):
            for x in range(w):
                picture = np.zeros((h, w), np.uint8)
                picture[y, x] = 255
                out.record("pixel", index, w, h, tw, th, pack(picture), picture)
                index += 1
    assert index == 391 + 408 + 368, index
    for i in range(100):
        w, h, tw, th = RANDOM_GEOMS[(i // 3) % 3]
        out.seeded("random", 3, i, w, h, tw, th, RANDOM_SHARES[i % 3])
    w, h, tw, th = WIDE_GEOM
    for i in range(12):
        out.seeded("wide", 4, i, w, h, tw, th, WIDE_SHARES[i % 3])
    for i, (w, h, tw, th, share) in enumerate(LARGE_CASES):
        out.seeded("large", 5, i, w, h, tw, th, share)
    return out


def text_of(out):
    counts = "counts " + " ".join(f"{kind} {len(out.blocks[kind])}" for kind in KINDS)
    return "\n".join([counts] + [line for kind in KINDS for line in out.blocks[kind]]) + "\n"


def frigate_lines(frigate_dir, path, blob):
    raw = subprocess.run(
        ["git", "-C", frigate_dir, "show", "v0.17.2:" + path],
        capture_output=True,
        check=True,
    ).stdout
    assert hashlib.sha1(b"blob %d\0" % len(raw) + raw).hexdigest() == blob, f"{path} does not hash to the quoted blob"
    return raw.decode("utf-8").splitlines()


def main():
    print("python", platform.python_version())
    print("numpy", np.__version__)
    print("cv2", cv2.__version__)
    for line in cv2.getBuildInformation().splitlines():
        if "Version control" in line:
            print(line.strip())
    print("cv2.ipp.useIPP()", cv2.ipp.useIPP())
    assert cv2.__version__ == "4.11.0", cv2.__version__
    frigate_lines(sys.argv[2], MOTION_PATH, MOTION_BLOB)
    image_lines = frigate_lines(sys.argv[2], IMAGE_PATH, IMAGE_BLOB)
    namespace = {"np": np, "cv2": cv2}
    exec(compile("\n".join(image_lines[910:943]) + "\n", IMAGE_PATH, "exec"), namespace)
    first = build(namespace["create_mask"])
    assert text_of(first) == text_of(build(namespace["create_mask"])), "the fixture is not reproducible"
    lines = text_of(first).splitlines()
    header = lines[0].split()
    for kind, count in zip(header[1::2], header[2::2]):
        written = sum(1 for line in lines[1:] if line.startswith(f"case {kind} "))
        assert written == int(count), (kind, count, written)
    text = text_of(first)
    assert len(text.encode("ascii")) <= LIMIT, len(text.encode("ascii"))
    print("smallest abs(accumulated - 0.5):", first.worst)
    for kind, index, attempt in first.redraws:
        print(f"redraws: {kind} {index}: {attempt}")
    OUT.write_text(text, encoding="ascii")
    print("wrote", OUT, len(text.encode("ascii")), "bytes")


main()
