r"""Goldens for corvette-motion's relative-polygon fill: for a frame size and a
list of relative polygon strings, the frame-size picture the deployed
create_mask text writes — 255 everywhere a polygon does not cover, 0 where one
covers it.

The numbers are what the pinned wheel
opencv_python_headless-4.11.0.86-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
(sha256
0e0a27c19dd1f40ddff94976cfe43066fbbe9dfbb2ec1907d66c19caef42a57b) writes: the
script takes the Frigate checkout directory as its second argument, reads
frigate/util/image.py at tag v0.17.2 from it with git show, refuses the bytes
unless they hash to the blob quoted below, and executes the quoted lines in a
namespace holding only numpy as np and cv2 as cv2; no Frigate import runs. The
wheel directory is the first argument; the wheel's native libraries need two
loader directories, so the full run command is:

    nix develop .#goldens --command env \
        LD_LIBRARY_PATH=/nix/store/6s4wm3zl2y0v5g998n2g88k1zh3vf2pr-zlib-1.3.2/lib:/nix/store/0ba5kj6r0zhfwd84l5vzccqvx7amlq2c-gcc-15.3.0-lib/lib \
        python3 -I tools/goldens/motion_polygon.py <extracted wheel directory> <Frigate checkout directory>

Measured with the wheel: the picture create_mask writes for every case, under
the versions the run prints: Python 3.14.6, numpy 2.5.1, cv2 4.11.0 with
version-control line "4.11.0" and cv2.ipp.useIPP() True; the script prints them
again on every run and refuses to write the fixture unless cv2 is 4.11.0. Read,
not measured: the OpenCV build of the deployed image (this fixture is evidence
about the version printed above and nothing else); the CPU-specific code paths
of the wheel (the earlier source reading of the fill found none, and none were
exercised across processors here); the class of polygon whose edge clipping
divides by zero, which is undefined in the wheel and is refused instead of
measured.

Reference: the frigate repository, path frigate/util/image.py, tag v0.17.2,
blob ea9fb0a0a7f166d7b145c6c3af9941de673aebe3, lines 911 to 943 (create_mask
and add_mask), copied verbatim:

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

Writes the fixture crates/corvette-motion/tests/polygon.rs reads: a first line
"counts camera 3 edge N2 overlap N3 random N4" with the number of cases of each
kind, then one line per case, the kinds in the header's order:

"case <kind> <index> <width> <height> <polygons> = <hex>": <polygons> is the
polygon strings joined by ";", each string the comma-separated fraction list
exactly as passed to create_mask (a coordinate is k / 1000 written as
str(round(k / 1000, 3)), and for the values 0 and 1 a random choice of 0, 0.0
or 1, 1.0; the empty list is "-"); <hex> is the picture bit-packed, one bit per
pixel row-major, 1 where the byte is 0, the first pixel in the high bit of the
first digit, lowercase, zero-padded at the end to a whole digit, so its length
is ceil(width * height / 4). One space between fields, no trailing space.

Kinds: camera is the three deployed masks (front 704x576, back 704x576, living
640x480), each one polygon as the deployment writes it; the script asserts the
zero-byte counts 4928, 7521 and 5980. edge is 25 cases at alternating sizes
12x9 and 37x23 — a triangle, a thin sliver, a concave L, a self-intersecting
bow-tie, repeated points, collinear points, all-equal points, a polygon
touching each of the four borders and each of the four corners, the whole
frame, a horizontal edge on each horizontal border, a vertical edge on each
vertical border — followed by five worked pictures at their own sizes, the
last with no polygon at all. overlap is 6 cases at alternating sizes 33x24 and
48x64 of two or three overlapping polygons; the script asserts that for at
least 4 of them the picture of one cv2.fillPoly call holding all the polygons
differs from the deployed per-polygon picture in at least one pixel. random is
240 cases at the sizes 33x24, 48x64, 90x127, 128x96, 77x33 cycling by index:
1 to 3 polygons of 3 to 12 points, each coordinate k / 1000 with k from 0 to
1000 and, with probability 0.25, snapped to 0 or 1; a polygon whose create_mask
call raises is redrawn. Each random case draws from
random.Random(20261115 + 1000 * kind_number + index), the kind number being the
kind's position in the header, from 0; every random number the other kinds
format draws from one random.Random(20261115) restarted fresh on each build.
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

OUT = Path(__file__).resolve().parents[2] / "crates/corvette-motion/tests/fixtures/polygon_cases.txt"

IMAGE_PATH = "frigate/util/image.py"
IMAGE_BLOB = "ea9fb0a0a7f166d7b145c6c3af9941de673aebe3"
SEED = 20261115
KINDS = ["camera", "edge", "overlap", "random"]
LIMIT = 1_000_000
RANDOM_COUNT = 240
CAMERAS = [
    (704, 576, ["0.75,0.033,0.968,0.036,0.968,0.089,0.751,0.087"], 4928),
    (704, 576, ["0.66,0.035,0.66,0.087,0.981,0.089,0.984,0.031"], 7521),
    (640, 480, ["0.714,0.068,0.714,0,1,0,1,0.065"], 5980),
]
RANDOM_SIZES = [(33, 24), (48, 64), (90, 127), (128, 96), (77, 33)]
OVERLAP_SIZES = [(33, 24), (48, 64)]
EDGE_SIZES = [(12, 9), (37, 23)]
WORKED = [
    (8, 6, ["0.25,0.25,0.75,0.25,0.75,0.75,0.25,0.75"]),
    (10, 8, ["0.1,0.1,0.9,0.1,0.5,0.9"]),
    (10, 8, ["0.5,0,0.5,0.5,1,0.5,1,0"]),
    (10, 8, ["0.2,0.2,0.8,0.2,0.8,0.8,0.2,0.8", "0.5,0.5,1,0.5,1,1,0.5,1"]),
    (6, 5, []),
]
EDGE_POLYS = [
    [[(100, 100), (900, 100), (500, 900)]],
    [[(20, 20), (980, 60), (30, 100)]],
    [[(0, 0), (400, 0), (400, 600), (1000, 600), (1000, 1000), (0, 1000)]],
    [[(0, 0), (1000, 0), (0, 1000), (1000, 1000)]],
    [[(100, 100), (100, 100), (900, 500), (900, 500), (100, 900)]],
    [[(0, 0), (500, 0), (1000, 0), (1000, 1000), (0, 1000)]],
    [[(250, 250), (250, 250), (250, 250)]],
    [[(0, 200), (300, 200), (300, 800), (0, 800)]],
    [[(200, 0), (800, 0), (800, 300), (200, 300)]],
    [[(700, 200), (1000, 200), (1000, 800), (700, 800)]],
    [[(200, 700), (800, 700), (800, 1000), (200, 1000)]],
    [[(0, 0), (300, 0), (300, 300), (0, 300)]],
    [[(700, 0), (1000, 0), (1000, 300), (700, 300)]],
    [[(0, 700), (300, 700), (300, 1000), (0, 1000)]],
    [[(700, 700), (1000, 700), (1000, 1000), (700, 1000)]],
    [[(0, 0), (1000, 0), (1000, 1000), (0, 1000)]],
    [[(0, 0), (1000, 0), (1000, 600), (0, 600)]],
    [[(0, 400), (1000, 400), (1000, 1000), (0, 1000)]],
    [[(0, 0), (400, 0), (400, 1000), (0, 1000)]],
    [[(600, 0), (1000, 0), (1000, 1000), (600, 1000)]],
]
OVERLAP_POLYS = [
    [[(100, 100), (600, 100), (600, 600), (100, 600)], [(400, 300), (900, 300), (900, 800), (400, 800)]],
    [[(0, 0), (1000, 0), (1000, 1000), (0, 1000)], [(200, 200), (800, 200), (800, 800), (200, 800)]],
    [[(100, 100), (500, 100), (500, 500), (100, 500)], [(300, 300), (700, 300), (700, 700), (300, 700)], [(600, 500), (900, 500), (900, 900), (600, 900)]],
    [[(0, 200), (400, 0), (400, 400), (0, 400)], [(300, 300), (1000, 300), (1000, 700), (300, 700)]],
    [[(200, 100), (800, 100), (800, 900), (200, 900)], [(0, 500), (1000, 500), (1000, 1000), (0, 500)], [(100, 100), (300, 100), (300, 300), (100, 300)]],
    [[(0, 0), (500, 0), (500, 500), (0, 500)], [(250, 250), (750, 250), (750, 750), (250, 750)], [(500, 500), (1000, 500), (1000, 1000), (500, 1000)]],
]


def token(rng, k):
    if k == 0:
        return rng.choice(("0", "0.0"))
    if k == 1000:
        return rng.choice(("1", "1.0"))
    return str(round(k / 1000, 3))


def points_string(rng, points):
    return ",".join(token(rng, k) for point in points for k in point)


def pixel_points(string, width, height):
    fields = string.split(",")
    return np.array(
        [[int(float(fields[i]) * width), int(float(fields[i + 1]) * height)] for i in range(0, len(fields), 2)]
    )


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


def case_line(create_mask, kind, index, width, height, strings):
    picture = create_mask((height, width), list(strings))
    hex_bits = pack(picture)
    assert len(hex_bits) == -(-width * height // 4), (kind, index)
    polygons = ";".join(strings) if strings else "-"
    line = " ".join(["case", kind, str(index), str(width), str(height), polygons, "=", hex_bits])
    return line, int((picture == 0).sum())


def camera_cases(create_mask):
    lines = []
    for index, (width, height, strings, zeros) in enumerate(CAMERAS):
        line, got = case_line(create_mask, "camera", index, width, height, strings)
        assert got == zeros, (index, got, zeros)
        lines.append(line)
    return lines


def edge_cases(create_mask, fmt):
    lines = []
    for index, polys in enumerate(EDGE_POLYS):
        width, height = EDGE_SIZES[index % 2]
        strings = [points_string(fmt, points) for points in polys]
        lines.append(case_line(create_mask, "edge", index, width, height, strings)[0])
    for offset, (width, height, strings) in enumerate(WORKED):
        line = case_line(create_mask, "edge", len(EDGE_POLYS) + offset, width, height, strings)[0]
        lines.append(line)
    return lines


def overlap_cases(create_mask, fmt):
    lines = []
    differing = 0
    for index, polys in enumerate(OVERLAP_POLYS):
        width, height = OVERLAP_SIZES[index % 2]
        strings = [points_string(fmt, points) for points in polys]
        line, _zeros = case_line(create_mask, "overlap", index, width, height, strings)
        one_call = np.full((height, width), 255, np.uint8)
        cv2.fillPoly(one_call, pts=[pixel_points(string, width, height) for string in strings], color=0)
        if (one_call != create_mask((height, width), list(strings))).any():
            differing += 1
        lines.append(line)
    assert differing >= 4, differing
    return lines


def random_cases(create_mask):
    lines = []
    for index in range(RANDOM_COUNT):
        rng = random.Random(SEED + 1000 * 3 + index)
        width, height = RANDOM_SIZES[index % 5]
        strings = []
        for _ in range(rng.randint(1, 3)):
            while True:
                points = []
                for _ in range(rng.randint(3, 12)):
                    coords = []
                    for _ in range(2):
                        k = rng.randrange(1001)
                        if rng.random() < 0.25:
                            k = rng.choice((0, 1000))
                        coords.append(k)
                    points.append(coords)
                string = points_string(rng, points)
                try:
                    create_mask((height, width), [string])
                except Exception:
                    continue
                break
            strings.append(string)
        lines.append(case_line(create_mask, "random", index, width, height, strings)[0])
    return lines


def text_of(blocks):
    counts = "counts " + " ".join(f"{kind} {len(blocks[kind])}" for kind in KINDS)
    return "\n".join([counts] + [line for kind in KINDS for line in blocks[kind]]) + "\n"


def build(create_mask, fmt):
    blocks = {
        "camera": camera_cases(create_mask),
        "edge": edge_cases(create_mask, fmt),
        "overlap": overlap_cases(create_mask, fmt),
        "random": random_cases(create_mask),
    }
    while len(text_of(blocks).encode("ascii")) > LIMIT:
        assert blocks["random"], "the random cases alone exceed the fixture limit"
        blocks["random"].pop()
    return text_of(blocks)


def deployed_create_mask(frigate_dir):
    raw = subprocess.run(
        ["git", "-C", frigate_dir, "show", "v0.17.2:" + IMAGE_PATH],
        capture_output=True,
        check=True,
    ).stdout
    assert hashlib.sha1(b"blob %d\0" % len(raw) + raw).hexdigest() == IMAGE_BLOB, "the source does not hash to the quoted blob"
    namespace = {"np": np, "cv2": cv2}
    exec(compile("\n".join(raw.decode("utf-8").splitlines()[910:943]) + "\n", IMAGE_PATH, "exec"), namespace)
    return namespace["create_mask"]


def main():
    print("python", platform.python_version())
    print("numpy", np.__version__)
    print("cv2", cv2.__version__)
    for line in cv2.getBuildInformation().splitlines():
        if "Version control" in line:
            print(line.strip())
    print("cv2.ipp.useIPP()", cv2.ipp.useIPP())
    assert cv2.__version__ == "4.11.0", cv2.__version__
    create_mask = deployed_create_mask(sys.argv[2])
    first = build(create_mask, random.Random(SEED))
    assert first == build(create_mask, random.Random(SEED)), "the fixture is not reproducible"
    header = first.splitlines()[0].split()
    for kind, count in zip(header[1::2], header[2::2]):
        written = sum(1 for line in first.splitlines()[1:] if line.startswith(f"case {kind} "))
        assert written == int(count), (kind, count, written)
    OUT.write_text(first, encoding="ascii")


main()
