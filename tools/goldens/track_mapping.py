"""Goldens for corvette-track's box mapping: the geometry Frigate's deployed
detect computes from a detector box, found by running the deployed text and
never by a restated rule.

Run command:

    cd /home/cvandesande/github/corvette && timeout 300 nix develop .#goldens --command python3 -I tools/goldens/track_mapping.py <Frigate checkout directory>

At tag v0.17.2, blob-checked and executed unchanged in one namespace holding
numpy, a create_tensor_input stub returning None, a stub detector returning
one row (person, np.float32 score, a box of np.float32 values) and a config
stub with width and height: frigate/video.py blob
38a39740410422ca3d9dae8ea8d42728f514cb87 lines 688-722 (detect) and
frigate/util/object.py blob b8f41e2c3214c22980f5227055aad348c9d6cd10 lines
216-267 (is_object_filtered).

Two expectations diverge from raw Frigate output by Corvette decision: a case
whose four sums box * size + origin are not finite in numpy float32 is named
nonfinite with no expected geometry (Frigate raises OverflowError on some and
clamps others); Frigate's edges are saturated at +-2**29 and area and ratio
recomputed from the saturated edges (with no edge saturated the recomputation
is asserted against Frigate's own area and ratio).

Fixture crates/corvette-track/tests/fixtures/mapping_cases.txt, # header
lines first (source blobs, strict filter settings, counts), one case per line:

    case <n> region <x_min> <y_min> <x_max> <y_max> frame <width> <height> box <hex> <hex> <hex> <hex> mask <rects|-> => skipped
    case ... => nonfinite
    case ... => box <x_min> <y_min> <x_max> <y_max> <area> <ratio hex> kept|dropped

Box hexes are the bits of the np.float32 values in the order y_min, x_min,
y_max, x_max; the ratio hex is the bits of the double. A mask is 0 to 3
rectangles x0,y0,x1,y1 (inclusive, ;-separated, - for none) of zero cells on
a frame-sized uint8 array of 255. skipped means Frigate's permissive call
(real infinities, no mask) returned nothing; kept or dropped is Frigate's
strict call (min_area 0, max_area 24000000, min_score 0.5, min_ratio 0.0,
max_ratio 24000000.0, the case's own mask, label person, score 0.9), an
IndexError there counts as dropped.
"""

import hashlib
import math
import struct
import subprocess
import sys
import types
import warnings
from pathlib import Path

warnings.simplefilter("ignore", RuntimeWarning)

import numpy as np

REPO = Path(__file__).resolve().parents[2]
FIXTURE = REPO / "crates/corvette-track/tests/fixtures/mapping_cases.txt"
VIDEO_BLOB = "38a39740410422ca3d9dae8ea8d42728f514cb87"
OBJECT_BLOB = "b8f41e2c3214c22980f5227055aad348c9d6cd10"
SEED = 20261122
TOTAL = 5000
SAT = 2**29
MASK_AREA = 4_000_000
STRICT = dict(min_area=0, max_area=24000000, min_score=0.5, min_ratio=0.0, max_ratio=24000000.0)
FRAMES = [(704, 576), (640, 480), (1280, 720), (1920, 1080), (2560, 1440), (1, 1), (2, 2), (33, 17), (500, 3)]
SMALL_MASK_FRAMES = [(704, 576), (640, 480), (1280, 720), (500, 3), (33, 17)]
SIZES = [4, 160, 240, 320, 480, 640, 960, 1280]
MAGNITUDES = [1e7, 3e7, 1e8, 3e8, 1e9, 3e9, 1e10, 1e11, 1e12]
NONSQ_SIZES = [80, 120, 240, 320, 480, 640, 800, 960, 1600]


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


def build_namespace(video, objects):
    assert video[687] == "def detect("
    assert video[721] == "    return detections"
    assert objects[215] == "def is_object_filtered(obj, objects_to_track, object_filters):"
    assert objects[266] == "    return False"
    ns = {"np": np, "create_tensor_input": lambda *args: None}
    exec(compile("\n".join(objects[215:267]), "object.py", "exec"), ns)
    exec(compile("\n".join(video[687:722]), "video.py", "exec"), ns)
    return ns


class Config:
    def __init__(self, width, height):
        self.width = width
        self.height = height


class Detector:
    def __init__(self, rows):
        self.rows = rows

    def detect(self, tensor):
        return self.rows


def sat(v):
    return max(-SAT, min(SAT, v))


def mask_of(rects, width, height):
    if not rects:
        return None
    arr = np.full((height, width), 255, dtype=np.uint8)
    for x0, y0, x1, y1 in rects:
        arr[y0 : y1 + 1, x0 : x1 + 1] = 0
    return arr


def f32bits(x):
    return f"{struct.unpack('>I', struct.pack('>f', x))[0]:08x}"


def f64bits(x):
    return f"{struct.unpack('>Q', struct.pack('>d', x))[0]:016x}"


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: track_mapping.py <Frigate checkout directory>")
    frigate = sys.argv[1]
    ns = build_namespace(
        source_lines(frigate, "frigate/video.py", VIDEO_BLOB),
        source_lines(frigate, "frigate/util/object.py", OBJECT_BLOB),
    )
    detect = ns["detect"]
    permissive = types.SimpleNamespace(min_area=float("-inf"), max_area=float("inf"), min_score=float("-inf"), min_ratio=float("-inf"), max_ratio=float("inf"), mask=None)
    rng = Lcg(SEED)
    lines = []
    stats = {
        "box": 0, "kept": 0, "dropped": 0, "index": 0,
        "skipped": 0, "nonfinite": 0, "divergent": 0,
        "neg_area": 0, "neg_x": 0, "saturated": 0, "nonsquare": 0,
    }

    def clamped_trunc(vals, width, height):
        limits = [(False, 0), (False, 0), (True, height - 1), (True, width - 1)]
        out = []
        for i, v in enumerate(vals):
            f = float(v)
            upper, limit = limits[i]
            if upper:
                f = f if f < limit else limit
            else:
                f = f if f > 0 else 0.0
            out.append(int(f))
        return out

    def classify(region, frame, fbox, rects):
        x0, y0, x1, y1 = region
        w, h = frame
        size = x1 - x0
        origins = [y0, x0, y0, x0]
        values32 = [np.float32(fbox[i]) * size + origins[i] for i in range(4)]
        if any(not math.isfinite(float(v)) for v in values32):
            stats["nonfinite"] += 1
            return "nonfinite"
        values64 = [np.float64(fbox[i]) * size + origins[i] for i in range(4)]
        if clamped_trunc(values32, w, h) != clamped_trunc(values64, w, h):
            stats["divergent"] += 1
        rows = [("person", np.float32(0.9), fbox)]
        try:
            out = detect(Config(w, h), Detector(rows), None, None, region, ["person"], {"person": permissive})
        except OverflowError as error:
            sys.exit(f"overflow on a finite case {region} {frame} {fbox}: {error}")
        if not out:
            stats["skipped"] += 1
            return "skipped"
        det = out[0]
        frigate_edges = det[2]
        saturated = any(e > SAT or e < -SAT for e in frigate_edges)
        if saturated:
            stats["saturated"] += 1
        edges = tuple(sat(e) for e in frigate_edges)
        width = edges[2] - edges[0]
        height = edges[3] - edges[1]
        area = width * height
        ratio = width / max(1, height)
        if not saturated:
            assert area == det[3] and ratio == det[4], (region, frame, fbox, det[3], det[4], area, ratio)
        stats["box"] += 1
        if area < 0:
            stats["neg_area"] += 1
        if edges[2] < 0:
            stats["neg_x"] += 1
        if (y1 - y0) != size:
            stats["nonsquare"] += 1
        spec = types.SimpleNamespace(**STRICT)
        spec.mask = mask_of(rects, w, h)
        try:
            filtered = ns["is_object_filtered"](det, ["person"], {"person": spec})
        except IndexError:
            stats["index"] += 1
            stats["dropped"] += 1
            return f"box {edges[0]} {edges[1]} {edges[2]} {edges[3]} {area} {f64bits(ratio)} dropped"
        if filtered:
            stats["dropped"] += 1
        else:
            stats["kept"] += 1
        return f"box {edges[0]} {edges[1]} {edges[2]} {edges[3]} {area} {f64bits(ratio)} {'kept' if not filtered else 'dropped'}"

    def add(region, frame, box, rects):
        fbox = tuple(np.float32(v) for v in box)
        bits = " ".join(f32bits(v) for v in fbox)
        rect_text = "-" if not rects else ";".join(f"{a},{b},{c},{d}" for a, b, c, d in rects)
        x0, y0, x1, y1 = region
        w, h = frame
        prefix = f"case {len(lines) + 1} region {x0} {y0} {x1} {y1} frame {w} {h} box {bits} mask {rect_text} =>"
        lines.append(f"{prefix} {classify(region, frame, fbox, rects)}")

    def ordinary():
        lo0 = rng.randint(0, 200) / 1000
        lo1 = rng.randint(0, 200) / 1000
        return (lo0, lo1, lo0 + rng.randint(1, 200) / 1000, lo1 + rng.randint(1, 200) / 1000)

    def plain_region(w, h):
        size = SIZES[rng.randint(0, len(SIZES) - 1)]
        ox = rng.randint(-size, w + size)
        oy = rng.randint(-size, h + size)
        return (ox, oy, ox + size, oy + size)

    def nonsquare_region(w, h):
        xs = SIZES[rng.randint(0, len(SIZES) - 1)]
        ys = NONSQ_SIZES[rng.randint(0, len(NONSQ_SIZES) - 1)]
        if xs == ys:
            ys += 3
        ox = rng.randint(-xs, w + xs)
        oy = rng.randint(-ys, h + ys)
        return (ox, oy, ox + xs, oy + ys)

    def frame_choice():
        return FRAMES[rng.randint(0, len(FRAMES) - 1)]

    def near(slot, k, size, x0, y0):
        origin = [y0, x0, y0, x0][slot]
        target = np.float32((k - origin) / size)
        return target + np.float32(np.spacing(target)) * rng.randint(-2, 2)

    def box_with(slot, value):
        a = rng.randint(0, 50) / 1000
        b = rng.randint(0, 50) / 1000
        box = [0.4 + a, 0.4 + b, 0.6 + a, 0.6 + b]
        box[slot] = float(value)
        return tuple(box)

    def hit_rect(y_max, mid, w, h):
        x0 = mid - rng.randint(0, 2)
        x1 = x0 + rng.randint(0, 2)
        y0 = y_max - rng.randint(0, 2)
        y1 = y0 + rng.randint(0, 2)
        if x1 < 0 or y1 < 0 or x0 > w - 1 or y0 > h - 1:
            return None
        return (max(0, x0), max(0, y0), min(x1, w - 1), min(y1, h - 1))

    def hit_rects(det, w, h):
        mid = (det[2][0] + det[2][2]) // 2
        rects = []
        if rng.randint(0, 4) != 0:
            got = hit_rect(det[2][3], mid, w, h)
            if got is not None:
                rects.append(got)
        for _ in range(rng.randint(0, 2)):
            x0 = rng.randint(0, w - 1)
            y0 = rng.randint(0, h - 1)
            rects.append((x0, y0, min(w - 1, x0 + rng.randint(0, 6)), min(h - 1, y0 + rng.randint(0, 6))))
        return rects[:3]

    for _ in range(1400):
        w, h = frame_choice()
        add(plain_region(w, h), (w, h), ordinary(), [])

    for _ in range(900):
        w, h = frame_choice()
        region = plain_region(w, h)
        slot = [0, 1, 1, 2, 2, 3, 3][rng.randint(0, 6)]
        limit = (w - 1 if slot in (1, 3) else h - 1) + rng.randint(-2, 2)
        k = limit if limit >= 0 else 0
        add(region, (w, h), box_with(slot, near(slot, k, region[2] - region[0], region[0], region[1])), [])

    for _ in range(500):
        w, h = frame_choice()
        region = plain_region(w, h)
        slot = [0, 1, 1, 2, 2, 3, 3][rng.randint(0, 6)]
        limit = w - 1 if slot in (1, 3) else h - 1
        k = limit + rng.randint(-1, 1)
        k = k if k >= 0 else 0
        add(region, (w, h), box_with(slot, near(slot, k, region[2] - region[0], region[0], region[1])), [])

    for _ in range(800):
        w, h = frame_choice()
        kind = rng.randint(0, 5)
        region = plain_region(w, h)
        if kind == 0:
            box = tuple(rng.randint(-30, 0) / 10 for _ in range(4))
        elif kind == 1:
            lo = rng.randint(0, 50) / 100
            box = (lo + rng.randint(1, 40) / 100, lo, lo, lo + rng.randint(1, 40) / 100)
        elif kind == 2:
            a = rng.randint(0, 100) / 100
            b = rng.randint(0, 100) / 100
            box = (a, b, a, b)
        elif kind == 3:
            size = region[2] - region[0]
            slot_x = rng.randint(1, 3)
            slot_y = rng.randint(0, 2)
            box = list(ordinary())
            box[slot_x] = float((rng.randint(-800, 800) - region[0]) / size)
            box[slot_y] = float((rng.randint(-800, 800) - region[1]) / size)
            box = tuple(box)
        elif kind == 4:
            size = SIZES[rng.randint(0, len(SIZES) - 1)]
            box = ordinary()
            region = (-size, rng.randint(-size, h), 0, rng.randint(0, h + size))
        else:
            box = ordinary()
            region = nonsquare_region(w, h)
        add(region, (w, h), box, [])

    for _ in range(300):
        if rng.randint(0, 3) != 3:
            w, h = 704, 576
            slot = rng.randint(2, 3)
            value = -MAGNITUDES[rng.randint(0, len(MAGNITUDES) - 1)]
        else:
            w, h = frame_choice()
            slot = rng.randint(0, 3)
            value = MAGNITUDES[rng.randint(0, len(MAGNITUDES) - 1)]
            if rng.randint(0, 1) == 0:
                value = -value
        region = plain_region(w, h)
        a = rng.randint(0, 50) / 1000
        b = rng.randint(0, 50) / 1000
        box = [0.2 + a, 0.2 + b, 0.6 + a, 0.6 + b]
        box[slot] = value
        add(region, (w, h), tuple(box), [])

    for _ in range(300):
        w, h = frame_choice()
        region = plain_region(w, h)
        a = rng.randint(0, 50) / 1000
        b = rng.randint(0, 50) / 1000
        box = [0.2 + a, 0.2 + b, 0.6 + a, 0.6 + b]
        for _ in range(rng.randint(1, 4)):
            box[rng.randint(0, 3)] = [float("nan"), float("inf"), float("-inf"), 3e38, -3e38][rng.randint(0, 4)]
        add(region, (w, h), tuple(box), [])

    masked = 0
    attempts = 0
    while masked < 800:
        attempts += 1
        if attempts > 50000:
            sys.exit(f"could not reach the masked cases: {masked}")
        if masked % 6 == 5:
            w, h = [(33, 17), (500, 3)][rng.randint(0, 1)]
            size = SIZES[rng.randint(4, 7)]
            ox, oy = -size, rng.randint(-size, h + size)
            region = (ox, oy, ox + size, oy + size)
            ky = -(h + 2 + rng.randint(0, 30))
            box = tuple(
                float(near(slot, k, size, ox, oy))
                for slot, k in ((0, 0), (1, 0), (2, ky), (3, 0))
            )
        else:
            w, h = SMALL_MASK_FRAMES[rng.randint(0, len(SMALL_MASK_FRAMES) - 1)]
            region = plain_region(w, h)
            box = ordinary()
        rows = [("person", np.float32(0.9), tuple(np.float32(v) for v in box))]
        try:
            out = detect(Config(w, h), Detector(rows), None, None, region, ["person"], {"person": permissive})
        except OverflowError:
            continue
        if not out:
            continue
        rects = hit_rects(out[0], w, h)
        if not rects:
            x0 = rng.randint(0, w - 1)
            y0 = rng.randint(0, h - 1)
            rects = [(x0, y0, min(w - 1, x0 + rng.randint(0, 4)), min(h - 1, y0 + rng.randint(0, 4)))]
        add(region, (w, h), box, rects)
        masked += 1

    assert stats["box"] >= 1500, stats
    assert stats["kept"] >= 400, stats
    assert stats["dropped"] >= 400, stats
    assert stats["skipped"] >= 150, stats
    assert stats["nonfinite"] >= 150, stats
    assert stats["divergent"] >= 300, stats
    assert stats["neg_area"] >= 200, stats
    assert stats["neg_x"] >= 200, stats
    assert stats["saturated"] >= 100, stats
    assert stats["nonsquare"] >= 100, stats
    assert stats["index"] >= 100, stats
    assert len(lines) == TOTAL, len(lines)

    header = [
        "# source v0.17.2 frigate/video.py blob 38a39740410422ca3d9dae8ea8d42728f514cb87 lines 688-722",
        "# source v0.17.2 frigate/util/object.py blob b8f41e2c3214c22980f5227055aad348c9d6cd10 lines 216-267",
        "# strict filter person min_area 0 max_area 24000000 min_score 0.5 min_ratio 0.0 max_ratio 24000000.0 label person score 0.9",
        f"# counts cases {TOTAL} box {stats['box']} kept {stats['kept']} dropped {stats['dropped']} skipped {stats['skipped']} nonfinite {stats['nonfinite']} divergent {stats['divergent']}",
        f"# counts neg_area {stats['neg_area']} neg_x_max {stats['neg_x']} saturated {stats['saturated']} nonsquare {stats['nonsquare']} index_error {stats['index']}",
    ]
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text("\n".join(header + lines) + "\n", encoding="utf-8")
    summary = " ".join(f"{name}={stats[name]}" for name in sorted(stats))
    print(f"track_mapping: {len(lines)} cases; {summary}")


if __name__ == "__main__":
    main()
