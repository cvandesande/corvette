"""Goldens for corvette-motion's size grid: the grid-adjusted region and the
startup regions that Frigate's deployed get_region_from_grid,
get_cluster_region_from_grid and get_startup_regions return.

Run command:

    cd /home/cvandesande/github/corvette && timeout 300 nix develop .#goldens --command python3 -I tools/goldens/motion_grid.py <Frigate checkout directory>

Reference: the frigate repository, path frigate/util/image.py, tag v0.17.2, blob
ea9fb0a0a7f166d7b145c6c3af9941de673aebe3, lines 291 to 314 (calculate_region); path
frigate/util/object.py, tag v0.17.2, blob b8f41e2c3214c22980f5227055aad348c9d6cd10,
line 35 (GRID_SIZE = 8), lines 151 to 163 (get_cluster_region_from_grid), lines 166
to 213 (get_region_from_grid) and lines 461 to 492 (get_startup_regions); each range
asserted at its first and last text and executed unchanged in one namespace defining
np and Any.

Fixture crates/corvette-motion/tests/fixtures/grid_cases.txt, one case per line
(floats written with Python's repr, which Rust's f64 parse reads back exactly; no
tabs):

    grid <id> <cell;cell;...|->
    fromgrid <id> <frame_width> <frame_height> <floor> <x1> <y1> <x2> <y2> <rx1> <ry1> <rx2> <ry2>
    clustergrid <id> <frame_width> <frame_height> <floor> <x1,y1,x2,y2;...> <i;j;...|-> <rx1> <ry1> <rx2> <ry2>
    startup <id> <frame_width> <frame_height> <floor> <x1,y1,x2,y2;...|->

with a grid line preceding every line that uses its id, each grid cell written
x,y,samples,mean,std_dev where mean and std_dev are np.mean and np.std of the cell's
sizes (relative region widths), and the expected values what the extracted functions
return for the line's frame, floor, box, cluster and grid. Draws come from one
Lcg(SEED5) in this order: 30 grids (id 0 empty, id 1 the single cell (3, 4), id 2
all 64 cells with sample counts cycling 1 to 3, ids 3 to 29 holding 2 to 14 distinct
random cells each with 1 to 6 sizes of randint(50, 1200) / 1000); 600 fromgrid
lines, 20 per grid in id order, every second one centred inside a populated cell's
pixel rectangle with half-width and half-height randint(0, 200) and the others
drawing a box the draw_box way; 150 clustergrid lines cycling the grid ids, each
with 1 to 5 drawn boxes and a random non-empty ascending index subset, every
thirtieth with the empty cluster; 50 startup lines, one per grid for ids 0 to 29
then 20 more for ids 2 to 21, each with its own drawn frame and floor.
"""

import hashlib
import subprocess
import sys
from pathlib import Path

import numpy as np
from typing import Any

REPO = Path(__file__).resolve().parents[2]
FIXTURE = REPO / "crates/corvette-motion/tests/fixtures/grid_cases.txt"
IMAGE_BLOB = "ea9fb0a0a7f166d7b145c6c3af9941de673aebe3"
OBJECT_BLOB = "b8f41e2c3214c22980f5227055aad348c9d6cd10"
SEED5 = 20261120
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


def frigate_namespace(frigate_dir):
    namespace = {"np": np, "Any": Any}
    image = source_lines(frigate_dir, "frigate/util/image.py", IMAGE_BLOB)
    extract(image, 291, "def calculate_region", 314,
            "return (x_offset, y_offset, x_offset + size, y_offset + size)", namespace)
    obj = source_lines(frigate_dir, "frigate/util/object.py", OBJECT_BLOB)
    extract(obj, 35, "GRID_SIZE", 35, "GRID_SIZE = 8", namespace)
    extract(obj, 151, "def get_cluster_region_from_grid", 163, ")", namespace)
    extract(obj, 166, "def get_region_from_grid", 213, ")", namespace)
    extract(obj, 461, "def get_startup_regions", 492, "return regions", namespace)
    return namespace


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: motion_grid.py <Frigate checkout directory>")
    namespace = frigate_namespace(sys.argv[1])
    grid_region = namespace["get_region_from_grid"]
    cluster_grid_region = namespace["get_cluster_region_from_grid"]
    startup = namespace["get_startup_regions"]
    rng = Lcg(SEED5)
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
    for _ in range(27):
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

    for grid_id, entries in enumerate(entries_by_id):
        populated = sorted(entries)
        for j in range(20):
            width, height = FRAMES[rng.randint(0, len(FRAMES) - 1)]
            floor = FLOORS[rng.randint(0, len(FLOORS) - 1)]
            if j % 2 == 1 and populated:
                x, y = populated[rng.randint(0, len(populated) - 1)]
                cx = rng.randint(x * width // GRID_SIZE, (x + 1) * width // GRID_SIZE - 1)
                cy = rng.randint(y * height // GRID_SIZE, (y + 1) * height // GRID_SIZE - 1)
                half_w = rng.randint(0, 200)
                half_h = rng.randint(0, 200)
                box = (cx - half_w, cy - half_h, cx + half_w, cy + half_h)
            else:
                box = draw_box(rng, (width, height))
            r = grid_region((height, width), list(box), floor, frigate_grids[grid_id])
            lines.append(
                f"fromgrid {grid_id} {width} {height} {floor} "
                f"{box[0]} {box[1]} {box[2]} {box[3]} {r[0]} {r[1]} {r[2]} {r[3]}"
            )

    for k in range(150):
        grid_id = k % 30
        width, height = FRAMES[rng.randint(0, len(FRAMES) - 1)]
        floor = FLOORS[rng.randint(0, len(FLOORS) - 1)]
        boxes = [draw_box(rng, (width, height)) for _ in range(rng.randint(1, 5))]
        if k % 30 == 0:
            cluster = []
        else:
            wanted = rng.randint(1, len(boxes))
            picks = set()
            while len(picks) < wanted:
                picks.add(rng.randint(0, len(boxes) - 1))
            cluster = sorted(picks)
        r = cluster_grid_region((height, width), floor, cluster, boxes, frigate_grids[grid_id])
        box_text = ";".join(",".join(str(v) for v in b) for b in boxes)
        cluster_text = ";".join(str(i) for i in cluster) if cluster else "-"
        lines.append(
            f"clustergrid {grid_id} {width} {height} {floor} {box_text} {cluster_text} "
            f"{r[0]} {r[1]} {r[2]} {r[3]}"
        )

    def emit_startup(grid_id):
        width, height = FRAMES[rng.randint(0, len(FRAMES) - 1)]
        floor = FLOORS[rng.randint(0, len(FLOORS) - 1)]
        regions = startup((height, width), floor, frigate_grids[grid_id])
        text = ";".join(",".join(str(v) for v in region) for region in regions) if regions else "-"
        lines.append(f"startup {grid_id} {width} {height} {floor} {text}")

    for grid_id in range(30):
        emit_startup(grid_id)
    for grid_id in range(2, 22):
        emit_startup(grid_id)

    kinds = [line.split(" ", 1)[0] for line in lines]
    assert kinds.count("grid") == 30, kinds
    assert kinds.count("fromgrid") == 600, kinds
    assert kinds.count("clustergrid") == 150, kinds
    assert kinds.count("startup") == 50, kinds
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text("\n".join(lines) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
