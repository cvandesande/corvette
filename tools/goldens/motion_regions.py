"""Goldens for corvette-motion's square-region arithmetic: the minimum region size,
the square region around a box and the square region around a cluster that Frigate's
deployed get_min_region_size, calculate_region and get_cluster_region return.

Run command:

    cd /home/cvandesande/github/corvette && timeout 120 python3 -I tools/goldens/motion_regions.py <Frigate checkout directory>

Reference: the frigate repository, path frigate/util/image.py, tag v0.17.2, blob
ea9fb0a0a7f166d7b145c6c3af9941de673aebe3, lines 291 to 314 (calculate_region); path
frigate/util/object.py, tag v0.17.2, blob b8f41e2c3214c22980f5227055aad348c9d6cd10,
lines 270 to 284 (get_min_region_size) and lines 446 to 458 (get_cluster_region);
each range asserted at its first and last text and executed unchanged in one
namespace defining math and ModelConfig.

Fixture crates/corvette-motion/tests/fixtures/region_cases.txt, one case per line:

    min <model_width> <model_height> <result>
    region <frame_width> <frame_height> <x1> <y1> <x2> <y2> <floor> <margin> <rx1> <ry1> <rx2> <ry2>
    cluster <frame_width> <frame_height> <floor> <i0,i1|-> <x1,y1,x2,y2;...|-> <rx1> <ry1> <rx2> <ry2>

with margin the word cluster (multiplier 1.35) or motion (multiplier 2). Random
draws use a linear congruential rule: state = seed; each draw state = (state *
6364136223846793005 + 1442695040888963407) mod 2**64, value = state >> 33, and
randint(lo, hi) = lo + value % (hi - lo + 1).

Fixture crates/corvette-motion/tests/fixtures/cluster_cases.txt, one case per line
(box is x1 y1 x2 y2):

    inside <box1> <box2> <0|1>
    area <box> <result>
    boundary <box> <floor> <rx1> <ry1> <rx2> <ry2>
    candidates <frame_width> <frame_height> <floor> <x1,y1,x2,y2;...|-> <i,j;k;...|->

with box1 the outer box and the candidate clusters sorted by their first member. The
functions box_inside, area, get_cluster_boundary and get_cluster_candidates are
extracted from object.py 313-317 and 376-443 and from image.py 719-720 of the same
revision and run unchanged in the same namespace.
"""

import hashlib
import math
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
FIXTURE = REPO / "crates/corvette-motion/tests/fixtures/region_cases.txt"
FIXTURE2 = REPO / "crates/corvette-motion/tests/fixtures/cluster_cases.txt"
IMAGE_BLOB = "ea9fb0a0a7f166d7b145c6c3af9941de673aebe3"
OBJECT_BLOB = "b8f41e2c3214c22980f5227055aad348c9d6cd10"
SEED = 20261116
SEED2 = 20261117
SEED3 = 20261118
FRAMES = [(64, 48), (160, 120), (640, 480), (704, 576), (1280, 720), (1920, 1080), (100, 300)]
FLOORS = [4, 32, 224, 252, 320, 640]


class Lcg:
    def __init__(self, seed):
        self.state = seed

    def randint(self, low, high):
        self.state = (self.state * 6364136223846793005 + 1442695040888963407) & 0xFFFFFFFFFFFFFFFF
        return low + (self.state >> 33) % (high - low + 1)


class Model:
    def __init__(self, width, height):
        self.width = width
        self.height = height


def blob_hash(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def source_lines(frigate_dir, path, blob):
    try:
        cmd = ["git", "-C", frigate_dir, "show", f"v0.17.2:{path}"]
        raw = subprocess.run(cmd, capture_output=True, check=True).stdout
    except (OSError, subprocess.SubprocessError) as error:
        sys.exit(f"cannot read v0.17.2:{path} from {frigate_dir}: {error}")
    assert blob_hash(raw) == blob, "the source does not hash to the quoted blob"
    return raw.decode("utf-8").splitlines()


def extract(lines, first, first_text, last, last_text, namespace):
    assert lines[first - 1].startswith(first_text), "unexpected line range"
    assert lines[last - 1].strip() == last_text, "unexpected line range"
    exec(compile("\n".join(lines[first - 1 : last]), "frigate", "exec"), namespace)


def frigate_namespace(frigate_dir):
    namespace = {"math": math, "ModelConfig": object}
    image = source_lines(frigate_dir, "frigate/util/image.py", IMAGE_BLOB)
    extract(image, 291, "def calculate_region", 314,
            "return (x_offset, y_offset, x_offset + size, y_offset + size)", namespace)
    extract(image, 719, "def area(box):", 720,
            "return (box[2] - box[0] + 1) * (box[3] - box[1] + 1)", namespace)
    obj = source_lines(frigate_dir, "frigate/util/object.py", OBJECT_BLOB)
    extract(obj, 270, "def get_min_region_size", 284, "return 320", namespace)
    extract(obj, 313, "def box_inside(b1, b2):", 317, "return False", namespace)
    extract(obj, 376, "def get_cluster_boundary(box, min_region):", 393, "]", namespace)
    extract(obj, 396, "def get_cluster_candidates(frame_shape, min_region, boxes):", 443,
            "return [list(tup) for tup in unique]", namespace)
    extract(obj, 446, "def get_cluster_region", 458, ")", namespace)
    return namespace


def draw_inside_box(rng, frame):
    width, height = frame
    x1 = rng.randint(-20, width + 19)
    y1 = rng.randint(-20, height + 19)
    return (x1, y1, x1 + rng.randint(1, 700), y1 + rng.randint(1, 700))


def draw_box(rng, frame):
    width, height = frame
    if rng.randint(0, 3) != 0:
        return draw_inside_box(rng, frame)
    box = draw_inside_box(rng, frame)
    if rng.randint(0, 3) == 0:
        x1 = rng.randint(width + 1, width + 100)
        return (x1, box[1], x1 + 700 - box[3] + box[1], box[3])
    edge = rng.randint(0, 2)
    if edge == 0:
        x2 = rng.randint(-100, -1)
        return (x2 - 700 + box[3] - box[1], box[1], x2, box[3])
    if edge == 1:
        y1 = rng.randint(height + 1, height + 100)
        return (box[0], y1, box[2], y1 + 700 - box[3] + box[1])
    y2 = rng.randint(-100, -1)
    return (box[0], y2 - 700 + box[3] - box[1], box[2], y2)


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: motion_regions.py <Frigate checkout directory>")
    namespace = frigate_namespace(sys.argv[1])
    min_size = namespace["get_min_region_size"]
    calculate = namespace["calculate_region"]
    cluster_region = namespace["get_cluster_region"]
    rng = Lcg(SEED)
    lines = []

    def emit_min(width, height):
        lines.append(f"min {width} {height} {min_size(Model(width, height))}")

    def emit_region(frame, box, floor, multiplier):
        width, height = frame
        r = calculate((height, width), box[0], box[1], box[2], box[3], floor, multiplier)
        margin = "cluster" if multiplier == 1.35 else "motion"
        box_text = " ".join(str(v) for v in box)
        result_text = " ".join(str(v) for v in r)
        lines.append(f"region {width} {height} {box_text} {floor} {margin} {result_text}")

    def emit_cluster(frame, floor, indices, boxes):
        width, height = frame
        r = cluster_region((height, width), floor, indices, boxes)
        index_text = ",".join(str(i) for i in indices) if indices else "-"
        box_text = ";".join(",".join(str(v) for v in box) for box in boxes) if boxes else "-"
        result_text = " ".join(str(v) for v in r)
        lines.append(f"cluster {width} {height} {floor} {index_text} {box_text} {result_text}")

    for model_max in list(range(1, 331)) + [331, 400, 640, 1000, 4096]:
        other = rng.randint(1, model_max)
        emit_min(*( (model_max, other) if model_max % 2 == 1 else (other, model_max) ))

    for width, height in [(640, 640), (320, 320), (224, 224), (250, 250), (300, 416), (1, 1), (3, 2), (319, 10), (321, 5)]:
        emit_min(width, height)

    reference_examples = [
        ((640, 480), (0, 0, 50, 40), 320, 1.35),
        ((1920, 1080), (50, 400, 250, 600), 320, 2),
        ((1920, 1080), (1650, 800, 1850, 1000), 320, 2),
        ((1920, 1080), (900, 340, 1140, 400), 320, 1.35),
        ((640, 480), (0, 380, 500, 460), 320, 1.35),
    ]
    for frame, box, floor, multiplier in reference_examples:
        emit_region(frame, box, floor, multiplier)

    for frame in FRAMES:
        width, height = frame
        bigger = max(320, max(width, height) + 4)
        edge_boxes = [(0, 0, width, height), (0, 0, 1, 1), (width - 1, 0, width, 1),
                      (0, height - 1, 1, height), (width - 1, height - 1, width, height)]
        for multiplier in (1.35, 2):
            for box in edge_boxes:
                for floor in (width, bigger):
                    emit_region(frame, box, floor, multiplier)

    for edge in (100, 200, 300, 400):
        emit_region((1920, 1080), (0, 0, edge, edge), 4, 1.35)

    for counter in range(460):
        frame = FRAMES[rng.randint(0, len(FRAMES) - 1)]
        multiplier = 1.35 if counter % 2 == 0 else 2
        emit_region(frame, draw_inside_box(rng, frame), FLOORS[rng.randint(0, len(FLOORS) - 1)], multiplier)

    emit_cluster((704, 576), 320, [], [])
    emit_cluster((704, 576), 320, [0, 1], [(100, 100, 180, 200), (300, 50, 340, 120)])
    emit_cluster((640, 480), 320, [], [(100, 100, 200, 200), (300, 50, 340, 120), (500, 300, 600, 450)])

    for _ in range(200):
        frame = FRAMES[rng.randint(0, len(FRAMES) - 1)]
        boxes = [draw_box(rng, frame) for _ in range(rng.randint(0, 6))]
        remaining = list(range(len(boxes)))
        indices = []
        for _ in range(rng.randint(0, len(boxes))):
            indices.append(remaining.pop(rng.randint(0, len(remaining) - 1)))
        emit_cluster(frame, FLOORS[rng.randint(0, len(FLOORS) - 1)], indices, boxes)

    rng2 = Lcg(SEED2)

    for counter in range(30):
        width, height = FRAMES[rng2.randint(0, len(FRAMES) - 1)]
        x1, y1 = rng2.randint(-20, width + 19), rng2.randint(-20, height + 19)
        box = (x1, y1, x1 - rng2.randint(1, 700), y1 - rng2.randint(1, 700))
        emit_region((width, height), box, FLOORS[rng2.randint(0, len(FLOORS) - 1)], 1.35 if counter % 2 == 0 else 2)

    for counter in range(20):
        width, height = FRAMES[rng2.randint(0, len(FRAMES) - 1)]
        x1, y1 = rng2.randint(-20, width + 19), rng2.randint(-20, height + 19)
        dx, dy = rng2.randint(1, 700), rng2.randint(1, 700)
        box = (x1, y1, x1 - dx, y1 + dy) if counter % 2 == 0 else (x1, y1, x1 + dx, y1 - dy)
        emit_region((width, height), box, FLOORS[rng2.randint(0, len(FLOORS) - 1)], 1.35 if counter % 2 == 0 else 2)

    for counter in range(10):
        width, height = FRAMES[rng2.randint(0, len(FRAMES) - 1)]
        x1, y1 = rng2.randint(-20, width + 19), rng2.randint(-20, height + 19)
        emit_region((width, height), (x1, y1, x1, y1), FLOORS[rng2.randint(0, len(FLOORS) - 1)], 1.35 if counter % 2 == 0 else 2)

    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text("\n".join(lines) + "\n", encoding="utf-8")

    rng3 = Lcg(SEED3)
    box_inside = namespace["box_inside"]
    area = namespace["area"]
    boundary = namespace["get_cluster_boundary"]
    candidates = namespace["get_cluster_candidates"]
    lines2 = []

    def fmt(box):
        return " ".join(str(v) for v in box)

    def emit_inside(first, second):
        verdict = 1 if box_inside(first, second) else 0
        lines2.append(f"inside {fmt(first)} {fmt(second)} {verdict}")

    for counter in range(300):
        frame = FRAMES[rng3.randint(0, len(FRAMES) - 1)]
        first = draw_box(rng3, frame)
        lo_x, hi_x = min(first[0], first[2]), max(first[0], first[2])
        lo_y, hi_y = min(first[1], first[3]), max(first[1], first[3])
        part = counter % 5
        if part == 0:
            second = first
        elif part == 1:
            x1, y1 = rng3.randint(lo_x, hi_x), rng3.randint(lo_y, hi_y)
            second = (x1, y1, rng3.randint(x1, hi_x), rng3.randint(y1, hi_y))
        elif part == 2:
            x1 = first[2] + rng3.randint(0, 1)
            second = (x1, first[1], x1 + rng3.randint(1, 50), first[3])
        elif part == 3:
            x1 = hi_x + rng3.randint(100, 500)
            second = (x1, first[1], x1 + 10, first[3] + 10)
        else:
            second = draw_box(rng3, frame)
        emit_inside(first, second)

    for counter in range(200):
        frame = FRAMES[rng3.randint(0, len(FRAMES) - 1)]
        part = counter % 4
        if part == 0:
            x, y = rng3.randint(-50, 2000), rng3.randint(-50, 2000)
            box = (x, y, x, y)
        elif part == 1:
            x, y = rng3.randint(-2000, -1), rng3.randint(-2000, -1)
            box = (x, y, x - rng3.randint(1, 500), y - rng3.randint(1, 500))
        elif part == 2:
            box = draw_inside_box(rng3, frame)
        else:
            x, y = rng3.randint(0, 1000), rng3.randint(0, 1000)
            box = (x, y, x - rng3.randint(0, 300), y + rng3.randint(0, 300))
        lines2.append(f"area {fmt(box)} {area(box)}")

    for counter in range(500):
        if counter % 4 == 0:
            wide, high = rng3.randint(0, 1500), rng3.randint(0, 1500)
        else:
            wide, high = rng3.randint(0, 200), rng3.randint(0, 200)
        dx = -wide if counter % 8 == 3 else wide
        dy = -high if counter % 9 == 4 else high
        x1, y1 = rng3.randint(-50, 2000), rng3.randint(-50, 2000)
        box = (x1, y1, x1 + dx, y1 + dy)
        floor = FLOORS[rng3.randint(0, len(FLOORS) - 1)]
        result = " ".join(str(v) for v in boundary(box, floor))
        lines2.append(f"boundary {fmt(box)} {floor} {result}")

    for counter in range(400):
        frame = FRAMES[rng3.randint(0, len(FRAMES) - 1)]
        wide, high = frame
        floor = FLOORS[rng3.randint(0, len(FLOORS) - 1)]
        boxes = []
        if counter % 3 < 2:
            for _ in range(rng3.randint(2, 3)):
                anchor_x = rng3.randint(0, wide - 1)
                anchor_y = rng3.randint(0, high - 1)
                size = rng3.randint(20, 250)
                for _ in range(rng3.randint(1, 3)):
                    x1 = anchor_x + rng3.randint(-size, size)
                    y1 = anchor_y + rng3.randint(-size, size)
                    boxes.append((x1, y1, x1 + rng3.randint(0, size), y1 + rng3.randint(0, size)))
        else:
            for _ in range(rng3.randint(0, 10)):
                x1 = rng3.randint(0, wide - 1)
                y1 = rng3.randint(0, high - 1)
                boxes.append((x1, y1, x1 + rng3.randint(0, wide - 1 - x1), y1 + rng3.randint(0, high - 1 - y1)))
        result = sorted(sorted(cluster) for cluster in candidates((high, wide), floor, boxes))
        box_text = ";".join(",".join(str(v) for v in b) for b in boxes) if boxes else "-"
        cluster_text = ";".join(",".join(str(i) for i in c) for c in result) if result else "-"
        lines2.append(f"candidates {wide} {high} {floor} {box_text} {cluster_text}")

    FIXTURE2.write_text("\n".join(lines2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
