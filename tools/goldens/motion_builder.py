"""Goldens for corvette-motion's region builder: the region list that Frigate's
deployed frame loop composes for one frame from object cluster regions,
grid-adjusted regions for the motion boxes outside them, and startup regions.

Run command:

    cd /home/cvandesande/github/corvette && timeout 300 nix develop .#goldens --command python3 -I tools/goldens/motion_builder.py <Frigate checkout directory>

Reference: the frigate repository, path frigate/util/image.py, tag v0.17.2, blob
ea9fb0a0a7f166d7b145c6c3af9941de673aebe3, lines 291 to 314 (calculate_region) and
lines 719 to 720 (area); path frigate/util/object.py, tag v0.17.2, blob
b8f41e2c3214c22980f5227055aad348c9d6cd10, line 35 (GRID_SIZE = 8), lines 313 to 317
(box_inside), lines 368 to 373 (inside_any), lines 376 to 393 (get_cluster_boundary),
lines 396 to 443 (get_cluster_candidates), lines 446 to 458 (get_cluster_region),
lines 151 to 163 (get_cluster_region_from_grid), lines 166 to 213
(get_region_from_grid) and lines 461 to 492 (get_startup_regions); path
frigate/video.py, tag v0.17.2, blob 38a39740410422ca3d9dae8ea8d42728f514cb87,
lines 899 to 945 (the region block of process_frames); each range asserted at its
first and last text and executed unchanged in one namespace defining np, Any and
math. The video block is dedented and wrapped, its text not edited, as the body of a
function taking the block's free names (frame_shape, region_min_size, object_boxes,
motion_boxes, region_grid, startup_scan, motion_detector,
ptz_moving_at_frame_time, frame_time, ptz_metrics) and returning regions and
startup_scan.

Fixture crates/corvette-motion/tests/fixtures/builder_cases.txt, one case per line
(floats written with Python's repr, which Rust's f64 parse reads back exactly; no
tabs):

    grid <id> <cell;cell;...|->
    build <id> <frame_width> <frame_height> <floor> <startup 0|1> <enabled 0|1> <object boxes|-> <motion boxes|-> <n_object_regions> <n_motion_regions> <regions|->

with a grid line preceding every build line that uses its id, each grid cell written
x,y,samples,mean,std_dev where mean and std_dev are np.mean and np.std of the cell's
sizes (relative region widths), boxes and regions written x1,y1,x2,y2, and <regions>
the whole list the block returned for the line's frame, floor, boxes, grid and
startup flag with the motion gate open when <enabled> is 1 and closed otherwise;
n_object_regions is the object candidate count and n_motion_regions the regions that
are neither object nor startup regions. Draws come from one Lcg(SEED6) in this
order: 12 grids (id 0 empty, id 1 the single cell (3, 4), id 2 all 64 cells with
sample counts cycling 1 to 3, ids 3 to 11 holding 2 to 14 distinct random cells
each with 1 to 6 sizes of randint(50, 1200) / 1000); 400 build lines, grid id i % 12,
each drawing a frame and a floor, the startup flag randint(0, 1), the gate (one in
four closed), 0 to 5 drawn object boxes and 0 to 8 motion boxes, each motion box one
of: a draw_box box (40 percent); a random sub-box inside the object region of the
candidate holding a chosen object box, edges drawn x1 <= a <= b <= x2 on both axes
(25 percent); a box equal to an object region (15 percent); that region with its
right edge one pixel wider (20 percent) — on a line without object boxes the last
three kinds draw a draw_box box instead.
"""

import hashlib
import subprocess
import sys
import textwrap
from pathlib import Path

import numpy as np
from typing import Any

REPO = Path(__file__).resolve().parents[2]
FIXTURE = REPO / "crates/corvette-motion/tests/fixtures/builder_cases.txt"
IMAGE_BLOB = "ea9fb0a0a7f166d7b145c6c3af9941de673aebe3"
OBJECT_BLOB = "b8f41e2c3214c22980f5227055aad348c9d6cd10"
VIDEO_BLOB = "38a39740410422ca3d9dae8ea8d42728f514cb87"
SEED6 = 20261121
FRAMES = [(64, 48), (160, 120), (640, 480), (704, 576), (1280, 720), (1920, 1080), (100, 300)]
FLOORS = [4, 32, 224, 252, 320, 640]
GRID_SIZE = 8


class Lcg:
    def __init__(self, seed):
        self.state = seed

    def randint(self, low, high):
        self.state = (self.state * 6364136223846793005 + 1442695040888963407) & 0xFFFFFFFFFFFFFFFF
        return low + (self.state >> 33) % (high - low + 1)


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


class CalibratingDetector:
    def __init__(self, calibrating):
        self.calibrating = calibrating

    def is_calibrating(self):
        return self.calibrating


class Stamp:
    value = 0.0


class PtzMetrics:
    start_time = Stamp()
    stop_time = Stamp()


def frigate_namespace(frigate_dir):
    namespace = {"np": np, "Any": Any, "math": __import__("math")}
    image = source_lines(frigate_dir, "frigate/util/image.py", IMAGE_BLOB)
    extract(image, 291, "def calculate_region", 314,
            "return (x_offset, y_offset, x_offset + size, y_offset + size)", namespace)
    extract(image, 719, "def area(box)", 720,
            "return (box[2] - box[0] + 1) * (box[3] - box[1] + 1)", namespace)
    obj = source_lines(frigate_dir, "frigate/util/object.py", OBJECT_BLOB)
    extract(obj, 35, "GRID_SIZE", 35, "GRID_SIZE = 8", namespace)
    extract(obj, 313, "def box_inside(b1, b2)", 317, "return False", namespace)
    extract(obj, 368, "def inside_any(box_a, boxes)", 373, "return False", namespace)
    extract(obj, 376, "def get_cluster_boundary(box, min_region)", 393, "]", namespace)
    extract(obj, 396, "def get_cluster_candidates(frame_shape, min_region, boxes)", 443,
            "return [list(tup) for tup in unique]", namespace)
    extract(obj, 446, "def get_cluster_region(frame_shape, min_region, cluster, boxes)", 458,
            ")", namespace)
    extract(obj, 151, "def get_cluster_region_from_grid", 163, ")", namespace)
    extract(obj, 166, "def get_region_from_grid", 213, ")", namespace)
    extract(obj, 461, "def get_startup_regions", 492, "return regions", namespace)
    video = source_lines(frigate_dir, "frigate/video.py", VIDEO_BLOB)
    extract_block(video, namespace)
    return namespace


def extract_block(lines, namespace):
    assert lines[899 - 1].strip().startswith("# get consolidated regions for tracked objects"), \
        "unexpected line range"
    assert lines[945 - 1].strip() == "startup_scan = False", "unexpected line range"
    body = textwrap.dedent("\n".join(lines[899 - 1 : 945]))
    names = ("frame_shape, region_min_size, object_boxes, motion_boxes, region_grid, "
             "startup_scan, motion_detector, ptz_moving_at_frame_time, frame_time, ptz_metrics")
    source = ("def block(" + names + "):\n" + textwrap.indent(body, "    ")
              + "\n    return regions, startup_scan\n")
    exec(compile(source, "block", "exec"), namespace)


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: motion_builder.py <Frigate checkout directory>")
    namespace = frigate_namespace(sys.argv[1])
    block = namespace["block"]
    cluster_candidates = namespace["get_cluster_candidates"]
    cluster_region = namespace["get_cluster_region"]
    startup_regions = namespace["get_startup_regions"]
    rng = Lcg(SEED6)
    lines = []

    def random_sizes():
        return [rng.randint(50, 1200) / 1000 for _ in range(rng.randint(1, 6))]

    def frigate_grid(entries):
        grid = [[{"sizes": []} for _ in range(GRID_SIZE)] for _ in range(GRID_SIZE)]
        for (x, y), sizes in entries.items():
            grid[x][y] = {"sizes": sizes, "x": x, "y": y,
                          "std_dev": float(np.std(sizes)), "mean": float(np.mean(sizes))}
        return grid

    every_cell = {}
    for x in range(GRID_SIZE):
        for y in range(GRID_SIZE):
            count = 1 + (x * GRID_SIZE + y) % 3
            every_cell[(x, y)] = [rng.randint(50, 1200) / 1000 for _ in range(count)]
    entries_by_id = [{}, {(3, 4): random_sizes()}, every_cell]
    for _ in range(9):
        wanted = rng.randint(2, 14)
        cells = {}
        while len(cells) < wanted:
            spot = (rng.randint(0, GRID_SIZE - 1), rng.randint(0, GRID_SIZE - 1))
            if spot not in cells:
                cells[spot] = random_sizes()
        entries_by_id.append(cells)
    frigate_grids = [frigate_grid(entries) for entries in entries_by_id]

    for grid_id, entries in enumerate(entries_by_id):
        if entries:
            texts = []
            for (x, y) in sorted(entries):
                sizes = entries[(x, y)]
                texts.append(
                    f"{x},{y},{len(sizes)},{repr(float(np.mean(sizes)))},{repr(float(np.std(sizes)))}"
                )
            lines.append(f"grid {grid_id} " + ";".join(texts))
        else:
            lines.append(f"grid {grid_id} -")

    def box_text(boxes):
        if not boxes:
            return "-"
        return ";".join(",".join(str(value) for value in box) for box in boxes)

    def sub_box(region):
        x1 = rng.randint(region[0], region[2])
        x2 = rng.randint(x1, region[2])
        y1 = rng.randint(region[1], region[3])
        y2 = rng.randint(y1, region[3])
        return (x1, y1, x2, y2)

    for i in range(400):
        grid_id = i % 12
        width, height = FRAMES[rng.randint(0, len(FRAMES) - 1)]
        floor = FLOORS[rng.randint(0, len(FLOORS) - 1)]
        frame_shape = (height, width)
        grid = frigate_grids[grid_id]
        startup_flag = rng.randint(0, 1)
        enabled = 0 if rng.randint(0, 3) == 0 else 1
        object_boxes = [draw_box(rng, (width, height)) for _ in range(rng.randint(0, 5))]
        clusters = cluster_candidates(frame_shape, floor, object_boxes)
        object_regions = [cluster_region(frame_shape, floor, cluster, object_boxes)
                          for cluster in clusters]
        region_of_box = {}
        for cluster_index, cluster in enumerate(clusters):
            for box_index in cluster:
                region_of_box[box_index] = cluster_index
        motion_boxes = []
        for _ in range(rng.randint(0, 8)):
            kind = rng.randint(0, 99)
            if kind < 40 or not object_regions:
                motion_boxes.append(draw_box(rng, (width, height)))
            elif kind < 65:
                region = object_regions[region_of_box[rng.randint(0, len(object_boxes) - 1)]]
                motion_boxes.append(sub_box(region))
            else:
                region = object_regions[rng.randint(0, len(object_regions) - 1)]
                if kind < 80:
                    motion_boxes.append(tuple(region))
                else:
                    motion_boxes.append((region[0], region[1], region[2] + 1, region[3]))
        regions, startup_scan = block(
            frame_shape, floor, object_boxes, motion_boxes, grid, startup_flag == 1,
            CalibratingDetector(enabled == 0), lambda *ignored: False, 0.0, PtzMetrics())
        assert startup_scan is False, f"line {i}: the block left the startup flag set"
        n_object = len(clusters)
        n_startup = len(startup_regions(frame_shape, floor, grid)) if startup_flag == 1 else 0
        n_motion = len(regions) - n_object - n_startup
        assert n_motion >= 0, f"line {i}: negative motion group"
        assert n_motion == 0 or enabled == 1, f"line {i}: motion regions with a closed gate"
        assert regions[:n_object] == list(object_regions), f"line {i}: the object group differs"
        lines.append(
            f"build {grid_id} {width} {height} {floor} {startup_flag} {enabled} "
            f"{box_text(object_boxes)} {box_text(motion_boxes)} "
            f"{n_object} {n_motion} {box_text(regions)}"
        )

    kinds = [line.split(" ", 1)[0] for line in lines]
    assert kinds.count("grid") == 12, kinds
    assert kinds.count("build") == 400, kinds
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text("\n".join(lines) + "\n", encoding="utf-8")
    assert FIXTURE.stat().st_size < 100 * 1024, "the fixture must stay under 100 KB"


if __name__ == "__main__":
    main()
