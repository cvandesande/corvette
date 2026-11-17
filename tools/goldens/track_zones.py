"""Goldens for corvette_track::ZoneSet: the current zones and entered zones a
tracked object holds after every frame, produced by Frigate v0.17.2's own zone
section of TrackedObject.update, executed unchanged apart from one Corvette
decision. References, each blob-checked here from the checkout given as the
argument: frigate/track/tracked_object.py blob
a95221bbdff834edd8c0cb0d153b7ff92f3d06ca, lines 41, 45-434 (class body from
__init__ through is_stationary, holding the zone section at 183-298) and lines
633-661 (zone_filtered); frigate/config/camera/zone.py blob
7df1a1f25f4d27345d91077c5f075b659bcefff3, lines 107-171 (generate_contour, the
relative-to-pixel conversion). The one change, the absent-frame branch of the
zone loop (lines 289-291):

    -                # once an object has a zone inertia of 3+ it is not checked anymore
    -                if 0 < zone_score < zone.inertia:
    -                    self.zone_presence[name] = zone_score - 1
    +                self.zone_presence[name] = 0

so a zone counts consecutive inside frames and any absent frame resets the
counter instead of decaying. Pixel polygons come through the contour attribute
the real ZoneConfig __init__ accepts; relative coordinates go through the real
generate_contour. Labels, inertia, loitering_time 0 and empty filters and
distances are set exactly as the zone section reads them. An unpatched second
copy runs the same frames so disagreement frames are counted; the fixture
records only the patched result plus that flag. Two runs write byte-identical
files. cv2 comes from the pinned opencv_python_headless 4.11.0.86 wheel given
as the second argument; its version is asserted 4.11.0. Run command (the two
LD_LIBRARY_PATH entries name the directories holding the current libz.so.1 and
libgcc_s.so.1, needed by the wheel's native libraries; see motion_average.py's
docstring, and do not copy store paths: they do not survive a garbage
collection):

    cd /home/cvandesande/github/corvette && timeout 300 nix develop .#goldens --command env LD_LIBRARY_PATH=<libz.so.1 directory>:<libgcc_s.so.1 directory> python3 -I tools/goldens/track_zones.py <Frigate checkout directory> <wheel directory>

Fixture crates/corvette-track/tests/fixtures/zones_cases.txt: # header lines
then scenario/zone/frame/forget/end blocks as the Rust test parses: zone
lines `zone <n> inertia <i> labels <k> [label]... (poly <n> <x,y>... |
rel <n> <hexx:hexy>... pixel <m> <x,y>...)` where relative coordinates are 16
hex chars of the big-endian double, frame lines `frame <id> label <l> x <X>
y <Y> edge <0|1> excl <0|1> diff <0|1> cur <k> [name]... ent <k> [name]...`;
cur and ent are the patched object's current_zones and entered_zones, edge an
on-edge or on-vertex frame, excl a frame where a zone skipped the object by
label, diff a frame the unpatched rule would answer differently.
"""
import hashlib
import os
import statistics
import struct
import subprocess
import sys
from collections import defaultdict
from math import gcd
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Optional, cast

if len(sys.argv) != 3:
    sys.exit("usage: track_zones.py <Frigate checkout directory> <wheel directory>")
sys.path.insert(0, sys.argv[2])
assert os.path.isdir(
    os.path.join(sys.argv[2], "opencv_python_headless-4.11.0.86.dist-info")
), "the pinned opencv_python_headless 4.11.0.86 dist-info is missing from the wheel directory"

import numpy as np
import cv2

assert cv2.__version__ == "4.11.0", cv2.__version__

FRIGATE_DIR = sys.argv[1]
FIXTURE = Path(__file__).resolve().parents[2] / "crates/corvette-track/tests/fixtures/zones_cases.txt"
SEED = 20261131
LABELS = ["person", "car", "dog", "truck", "bicycle", "cat"]
FRAME_SIZES = [(64, 48), (40, 30), (96, 72), (120, 80), (21, 17), (32, 24)]
FRAMES = [10, 14, 18, 22, 27, 33, 40, 50, 60]
KINDS = ["rect", "flicker3", "L", "tri", "flicker5", "edge_rect", "rel", "flicker2",
         "labelblock", "leaver", "U", "mixed", "flicker1"]
FLOORS = {"scenarios": 120, "on_edge": 40, "disagreement": 40, "label_excluded": 40, "forgets": 40}

def blob_hash(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()

def blob(path, want):
    cmd = ["git", "-C", FRIGATE_DIR, "show", f"v0.17.2:{path}"]
    try:
        raw = subprocess.run(cmd, capture_output=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"cannot read v0.17.2:{path} from {FRIGATE_DIR}: {error}")
    assert blob_hash(raw) == want, f"{path} does not hash to the pinned blob"
    return raw.decode("utf-8").splitlines()

TRACKED = blob("frigate/track/tracked_object.py", "a95221bbdff834edd8c0cb0d153b7ff92f3d06ca")
ZONE = blob("frigate/config/camera/zone.py", "7df1a1f25f4d27345d91077c5f075b659bcefff3")
for probe, anchor, needle in [(44, TRACKED, "    def __init__("), (433, TRACKED, "return count >"),
                              (632, TRACKED, "def zone_filtered("), (660, TRACKED, "return False"),
                              (289, TRACKED, "if 0 < zone_score < zone.inertia:"),
                              (290, TRACKED, "self.zone_presence[name] = zone_score - 1"),
                              (40, TRACKED, "EXTENDED_LOITERING_OBJECTS"),
                              (106, ZONE, "    def generate_contour(")]:
    assert anchor[probe].strip().startswith(needle.strip()), anchor[probe]

def build(patched):
    lines = TRACKED[44:434]
    if patched:
        assert lines[244].strip().startswith("# once an object"), lines[244]
        lines = lines[:244] + ["                self.zone_presence[name] = 0"] + lines[247:]
    ns = {"np": np, "cv2": cv2, "median": statistics.median, "cast": cast, "Optional": Optional,
          "Any": Any, "defaultdict": defaultdict,
          "logger": SimpleNamespace(debug=lambda *args, **kwargs: None),
          "ModelConfig": None, "CameraConfig": None, "UIConfig": None, "sanitize_float": lambda v: v,
          "calculate_real_world_speed": lambda *a: (_ for _ in ()).throw(AssertionError("speed path"))}
    exec(compile("class Real:\n" + "\n".join(lines) + "\n", "tracked_object.py", "exec"), ns)
    ns["TrackedObject"] = ns["Real"]
    ns["FilterConfig"] = None
    exec(compile(TRACKED[40] + "\n" + "\n".join(TRACKED[632:661]), "helpers", "exec"), ns)
    return ns

NS_PATCH = build(True)
NS_BASE = build(False)
ZNS = {"np": np}
exec(compile(
    "class ZoneLike:\n    def __init__(self, coordinates):\n"
    "        self.coordinates = coordinates\n        self._contour = np.array([])\n\n"
    + "\n".join(ZONE[106:171]) + "\n", "zone.py", "exec"), ZNS)
ZoneLike = ZNS["ZoneLike"]
MODEL = SimpleNamespace(colormap=None, all_attribute_logos=[])
UI = SimpleNamespace(unit_system="metric")

class Lcg:
    def __init__(self, seed):
        self.state = seed
    def randint(self, low, high):
        self.state = (self.state * 6364136223846793005 + 1442695040888963407) & 0xFFFFFFFFFFFFFFFF
        return low + (self.state >> 33) % (high - low + 1)
    def choice(self, seq):
        return seq[self.randint(0, len(seq) - 1)]

rng = Lcg(SEED)

def make_camera(zones, width, height):
    quiet = SimpleNamespace(enabled=False, labels=[], required_zones=[])
    return SimpleNamespace(name="cam", zones=zones,
                           detect=SimpleNamespace(fps=5.0, width=width, height=height,
                                                  stationary=SimpleNamespace(threshold=None)),
                           objects=SimpleNamespace(filters=defaultdict(
                               lambda: SimpleNamespace(threshold=0.7))),
                           review=SimpleNamespace(alerts=quiet, detections=quiet))

def obj_data(label, x, y, frame):
    return {"id": "o", "label": label, "frame_time": float(frame), "score": 0.5,
            "score_history": [0.5], "box": [float(x - 10), float(y - 20), float(x + 10), float(y)],
            "centroid": [float(x), float(y - 10)], "area": 400, "ratio": 1.0,
            "region": [[0, 0, 64, 48]], "attributes": [], "start_time": 0.0,
            "motionless_count": 0, "position_changes": 0}

def new_object(ns, cam):
    return ns["Real"](MODEL, cam, UI, {}, obj_data("person", 0, 0, 0))

def hexf(value):
    return struct.pack(">d", value).hex()

def lattice(a, b):
    dx, dy = b[0] - a[0], b[1] - a[1]
    step = gcd(abs(dx), abs(dy)) or 1
    return [(a[0] + k * dx // step, a[1] + k * dy // step) for k in range(step + 1)]

def boundary_points(polygon):
    count = len(polygon)
    return [p for i in range(count) for p in lattice(polygon[i], polygon[(i + 1) % count])]

def near_points(polygon, want_inside):
    want = 1.0 if want_inside else -1.0
    offsets = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1), (1, -1), (-1, 1)]
    array = np.array(polygon)
    return [(x + dx, y + dy) for (x, y) in boundary_points(polygon) for (dx, dy) in offsets
            if cv2.pointPolygonTest(array, (x + dx, y + dy), False) == want]

def inside_point(polygon):
    xs = [p[0] for p in polygon]
    ys = [p[1] for p in polygon]
    start = (sum(xs) // len(xs), sum(ys) // len(ys))
    if cv2.pointPolygonTest(np.array(polygon), start, False) >= 0:
        return start
    near = near_points(polygon, True)
    grid = [(x, y) for y in range(min(ys), max(ys) + 1) for x in range(min(xs), max(xs) + 1)]
    point = next((p for p in near + grid
                  if cv2.pointPolygonTest(np.array(polygon), p, False) >= 0), None)
    assert point, "no inside point"
    return point

def outside_point(polygon, width, height):
    for _ in range(60):
        point = (rng.randint(-3, width + 2), rng.randint(-3, height + 2))
        if cv2.pointPolygonTest(np.array(polygon), point, False) < 0:
            return point
    return (-5, -5)


SHAPES = {
    "rect": lambda x0, y0, x1, y1: [[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
    "L": lambda x0, y0, x1, y1: [[x0, y0], [(x0 + x1) // 2, y0], [(x0 + x1) // 2, (y0 + y1) // 2],
                                 [x1, (y0 + y1) // 2], [x1, y1], [x0, y1]],
    "U": lambda x0, y0, x1, y1: [[x0, y0], [(x0 + x1) // 2, y0], [(x0 + x1) // 2, y1 - 2],
                                 [x1 - 2, y1 - 2], [x1 - 2, y0], [x1, y0], [x1, y1], [x0, y1]],
    "tri": lambda x0, y0, x1, y1: [[x0, y1], [x1, y1], [(x0 + x1) // 2, y0]],
}

def zone_stub(polygon, inertia, labels, contour=None):
    return SimpleNamespace(contour=contour if contour is not None else np.array(polygon),
                           objects=list(labels), inertia=inertia, loitering_time=0,
                           filters={}, distances=[], speed_threshold=None)

def build_zone(name, kind, width, height):
    edge_first = kind == "edge_rect" and rng.randint(0, 1)
    box = (0, 0, width, height - 1) if edge_first else (
        rng.randint(0, max(0, width // 2)), rng.randint(0, max(0, height // 2)),
        rng.randint(width // 2, width), rng.randint(height // 2, height))
    polygon = SHAPES["rect" if kind == "edge_rect" else kind](*box)
    inertia = rng.choice([1, 2, 3, 5])
    labels = [] if rng.randint(0, 1) else [rng.choice(LABELS) for _ in range(rng.randint(1, 3))]
    tokens = ["zone", name, "inertia", str(inertia), "labels", str(len(labels)), *labels,
              "poly", str(len(polygon)), *["%d,%d" % (p[0], p[1]) for p in polygon]]
    return zone_stub(polygon, inertia, labels), polygon, tokens

def build_rel_zone(name, width, height):
    count = rng.randint(3, 5)
    coords = [(rng.choice([0.0, 1.0, rng.randint(1, width) / width, rng.randint(1, 97) / 100.0]),
               rng.choice([0.0, 1.0, rng.randint(1, height) / height, rng.randint(1, 97) / 100.0]))
              for _ in range(count)]
    like = ZoneLike(["%r,%r" % c for c in coords])
    like.generate_contour((height, width))
    pixel = [list(p) for p in like._contour.tolist()]
    if len(pixel) < 3 or len(set(map(tuple, pixel))) < 3:
        return None
    inertia = rng.choice([1, 2, 3])
    labels = [] if rng.randint(0, 1) else [rng.choice(LABELS)]
    tokens = (["zone", name, "inertia", str(inertia), "labels", str(len(labels)), *labels,
               "rel", str(count)] + [hexf(rx) + ":" + hexf(ry) for rx, ry in coords] +
              ["pixel", str(len(pixel)), *["%d,%d" % (p[0], p[1]) for p in pixel]])
    return zone_stub(pixel, inertia, labels, contour=like._contour), pixel, tokens

def run_scenario(index, kind, out, stats):
    width, height = rng.choice(FRAME_SIZES)
    zones, polygons, zone_lines = {}, [], []
    kinds = {"rel": ["rel", "rect"], "mixed": ["rect", "L", "tri"]}.get(
        kind, [kind if kind in ("rect", "L", "U", "tri", "edge_rect") else "rect"])
    for zi, zkind in enumerate(kinds[: rng.randint(1, len(kinds))]):
        built = next((b for b in (build_rel_zone("z%d" % zi, width, height) for _ in range(8)) if b),
                     None) if zkind == "rel" else build_zone("z%d" % zi, zkind, width, height)
        if built is None:
            built = build_zone("z%d" % zi, "rect", width, height)
        stub, polygon, tokens = built
        zones["z%d" % zi] = stub
        polygons.append(polygon)
        zone_lines.append(" ".join(tokens))
    cam = make_camera(zones, width, height)
    frames = rng.choice(FRAMES)
    flicker = {"flicker1": 1, "flicker2": 2, "flicker3": 3, "flicker5": 5}.get(kind)
    objects = []
    for oi in range(1, rng.randint(1, 3) + 1):
        z0 = polygons[0]
        inside, outside = inside_point(z0), outside_point(z0, width, height)
        edge_pool = boundary_points(z0)
        in_pool, out_pool = near_points(z0, True), near_points(z0, False)
        plan = []
        if flicker:
            run = flicker * 2 + 1
            while len(plan) < frames:
                plan.extend([inside] * min(run, frames - len(plan)))
                if kind == "flicker3":
                    plan.extend([rng.choice(edge_pool)] * min(1, frames - len(plan)))
                if len(plan) < frames:
                    plan.extend([outside] * min(rng.randint(1, 3), frames - len(plan)))
        elif kind == "leaver":
            plan = ([inside] * 6 + [outside] * 4 + [inside] * 6)
            plan = (plan * 4)[:frames]
        elif kind == "labelblock":
            plan = [inside] * frames
        else:
            for step in range(frames):
                if step % 7 == 3:
                    pool = rng.choice([edge_pool, in_pool, out_pool,
                                       boundary_points(rng.choice(polygons))])
                    plan.append(rng.choice(pool) if pool else inside)
                elif step % 11 == 5:
                    plan.append((rng.randint(0, width), rng.randint(0, height)))
                else:
                    cx, cy = inside
                    plan.append((cx + rng.randint(-6, 6), cy + rng.randint(-6, 6)))
        forget_at = None
        if kind == "leaver" and frames > 2:
            forget_at = min(9, frames - 1)
        elif kind in ("labelblock", "mixed") and frames > 6:
            forget_at = 3
        objects.append({"id": oi, "label": rng.choice(LABELS), "plan": plan, "forget_at": forget_at,
                        "patch": new_object(NS_PATCH, cam), "base": new_object(NS_BASE, cam)})
    lines = ["scenario %d frame %d %d" % (index, width, height)] + zone_lines
    stats["scenarios"] += 1
    for frame in range(frames):
        for obj in objects:
            if obj["forget_at"] is not None and frame == obj["forget_at"]:
                obj["patch"] = new_object(NS_PATCH, cam)
                obj["base"] = new_object(NS_BASE, cam)
                lines.append("forget %d" % obj["id"])
                stats["forgets"] += 1
            label = obj["label"]
            (x, y) = obj["plan"][frame] if frame < len(obj["plan"]) else inside_point(polygons[0])
            edge = excl = 0
            for stub in zones.values():
                if stub.objects and label not in stub.objects:
                    excl = 1
                elif cv2.pointPolygonTest(stub.contour, (x, y), False) == 0:
                    edge = 1
            data = obj_data(label, x, y, frame)
            obj["patch"].update(float(frame), dict(data), False)
            obj["base"].update(float(frame), dict(data), False)
            cur = list(obj["patch"].current_zones)
            ent = list(obj["patch"].entered_zones)
            diff = 1 if (cur != list(obj["base"].current_zones)
                         or ent != list(obj["base"].entered_zones)) else 0
            if edge:
                stats["on_edge"] += 1
            if excl:
                stats["label_excluded"] += 1
            if diff:
                stats["disagreement"] += 1
            stats["frames"] += 1
            lines.append(" ".join(["frame", str(obj["id"]), "label", label, "x", str(x), "y", str(y),
                                   "edge", str(edge), "excl", str(excl), "diff", str(diff),
                                   "cur", str(len(cur)), *cur, "ent", str(len(ent)), *ent]))
    lines.append("end")
    out.extend(lines)

def main():
    out = []
    stats = {"scenarios": 0, "frames": 0, "on_edge": 0, "disagreement": 0, "label_excluded": 0,
             "forgets": 0}
    index = 0
    while index < 320 and not (index >= FLOORS["scenarios"]
                               and all(stats[n] >= f for n, f in FLOORS.items())):
        index += 1
        run_scenario(index, KINDS[(index - 1) % len(KINDS)], out, stats)
    for name, floor in FLOORS.items():
        assert stats[name] >= floor, f"floor {name} not met: {stats[name]} < {floor}"
    header = ["# zones cases: zone membership replay, corvette_track::ZoneSet",
              "# source frigate/track/tracked_object.py blob a95221bbdff834edd8c0cb0d153b7ff92f3d06ca",
              "# source frigate/config/camera/zone.py blob 7df1a1f25f4d27345d91077c5f075b659bcefff3 at v0.17.2",
              "# change self.zone_presence[name] = zone_score - 1 replaced by self.zone_presence[name] = 0",
              "# seed %d python %s numpy %s cv2 4.11.0 (opencv_python_headless 4.11.0.86 wheel)"
              % (SEED, sys.version.split()[0], np.__version__),
              "# counts scenarios %d frames %d forgets %d" % (stats["scenarios"], stats["frames"],
                                                              stats["forgets"]),
              "# floors %s" % " ".join("%s %d" % (n, stats[n]) for n in
                                        ("scenarios", "on_edge", "disagreement",
                                         "label_excluded", "forgets")),
              "# floats are 16 hex chars, big-endian IEEE-754 doubles, joined by a colon in rel coordinates"]
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text("\n".join(header + out) + "\n")
    print("scenarios %d frames %d edge %d diff %d excl %d forgets %d" %
          tuple(stats[n] for n in ("scenarios", "frames", "on_edge", "disagreement",
                                    "label_excluded", "forgets")))

main()
