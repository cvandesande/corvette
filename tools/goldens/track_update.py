"""Goldens for corvette_track::Tracker: the exact state after every frame of 120
seeded scenarios driven through the real norfair 2.3.0 Tracker.update with
Frigate v0.17.2's own frigate_distance — after each update, whether it raised,
then every track in list order (label, hit counter, age, initializing flag,
initializing id, id, active flag, last observation tag, the past list as
tag@age pairs, and the Kalman estimate as float64 bits).

Run command:

    cd /home/cvandesande/github/corvette && timeout 500 nix develop .#goldens --command python3 -I tools/goldens/track_update.py /home/cvandesande/github/frigate <extracted norfair directory>

The reference is the norfair 2.3.0 sdist norfair-2.3.0.tar.gz, sha256
8331c7fb2290fe92e9cd73877d43b0a5134fafd8846082002a9b545845d2a79c (PyPI);
extract it with `tar -xzf norfair-2.3.0.tar.gz` and pass the directory that
holds the norfair package. The script asserts the package tree hash
e8626859af72d20d25705e3563f2a6d3f7ae410476e4adaae106c0672bb0d5be — the recipe
`cd <dir>/norfair && find . -name '*.py' | LC_ALL=C sort | xargs sha256sum |
sha256sum`, recomputed here in Python with a plain byte-order sort — and the
version norfair reports. The Frigate text is blob-checked: sha1 of the git blob
header plus the file bytes must equal 84a0f390a3742ab6dc5a339a1b54480570280110,
and the executed slice lines[40:75] gives frigate_distance.

norfair imports rich and filterpy, which the tracker path never touches and the
shell does not carry, so a meta-path import hook gives any module named rich or
filterpy (and their submodules) a stub module whose attribute lookup returns a
dummy class (print for rich.print), and importlib.metadata.version('norfair')
is patched to '2.3.0' — both only so that `import norfair` succeeds. The shell's
numpy (2.x, version recorded in the fixture header) is newer than the numpy 1.x
pin norfair declares; every recorded field comes from the real norfair objects
running on this numpy. division-by-zero warnings from the distance function are
expected and suppressed, as are the constructor's scalar-distance warnings.

Frigate builds its trackers with initialization_delay = detect.min_initialized
(2 at the default frame rate) and hit_counter_max = detect.max_disappeared (25),
never passes past_detections_length (the library default 4), and hands the
filter and the threshold per tracker: the (threshold, R, Q) settings (2.5, 3.4,
0.03) for cars, (3.75, 2.5, 0.05) for license plates, (2.5, 3.4, 0.1) for the
default static tracker and (3.0, 4.0, 0.2) for the default PTZ tracker
(frigate/track/norfair_tracker.py lines 126, 131, 159, 165 and 199 to 209 at
v0.17.2). Half of the scenarios use one of those four settings; the other half
draw the settings and the filter parameters at random.

Fixture crates/corvette-track/tests/fixtures/tracker_cases.txt, # header lines
first (sources, seed, numpy, the Frigate note, counts, floors), then 120
blocks:

    scenario <n> max <hit_counter_max> delay <delay> past <past> threshold <h16> r <h16> q <h16>
    frame <k> dets <d>
    det <tag> <label> <x1> <y1> <x2> <y2>            (d lines)
    result <ok|nan> tracks <t>
    track <label> <hit_counter> <age> <init 0|1> <initializing_id> <id|-> <active 0|1> <last tag> <past|-> <e1> <e2> <e3> <e4>
    end

n counts scenarios from 1 and k frames from 1; threshold, r, q and e1..e4 are
the big-endian hex of the float64 bits; result is nan exactly when the real
update raised ValueError; the track lines are tracker.tracked_objects after the
update (also after a raise), in list order; the past list is tag@age joined with
/. Integers are decimal, fields separated by single spaces. Output is
deterministic.
"""

import hashlib
import importlib.abc
import importlib.machinery
import importlib.metadata
import logging
import struct
import subprocess
import sys
import types
import warnings
from pathlib import Path

if len(sys.argv) != 3:
    sys.exit("usage: track_update.py <Frigate checkout directory> <extracted norfair directory (the one holding norfair/)>")

FRIGATE_DIR = sys.argv[1]
SRC = Path(sys.argv[2]).resolve()
NORFAIR_PACKAGE = SRC / "norfair"
FIXTURE = Path(__file__).resolve().parents[2] / "crates/corvette-track/tests/fixtures/tracker_cases.txt"
FRIGATE_FILE = "frigate/track/norfair_tracker.py"
FRIGATE_BLOB = "84a0f390a3742ab6dc5a339a1b54480570280110"
SDIST_SHA256 = "8331c7fb2290fe92e9cd73877d43b0a5134fafd8846082002a9b545845d2a79c"
TREE_SHA256 = "e8626859af72d20d25705e3563f2a6d3f7ae410476e4adaae106c0672bb0d5be"
SEED = 20261128
LABELS = ["car", "person", "dog", "license_plate"]
FRIGATE_SETTINGS = [
    (2.5, 3.4, 0.03),
    (3.75, 2.5, 0.05),
    (2.5, 3.4, 0.1),
    (3.0, 4.0, 0.2),
]
MAX_CHOICES = [1, 1, 2, 5, 10, 25, 50]
PAST_CHOICES = [0, 1, 2, 4, 4, 8]
THRESHOLDS = [0.5, 1.3, 1.3, 2.0, 2.5, 2.5, 2.7, 2.7, 3.0, 3.75, 6.0, 6.0, 12.0]
OBJ_COUNTS = [4, 5, 5, 6, 6, 7, 7, 8, 9]
DETECT_PROBS = [1.0, 1.0, 1.0, 0.9, 0.8]
FLOORS = {
    "scenarios": 120,
    "frames": 5000,
    "empty_frames": 400,
    "triple_frames": 1000,
    "created": 1500,
    "removed": 600,
    "first_pass_hits": 3000,
    "second_pass_hits": 1000,
    "left_initializing": 500,
    "two_left_frames": 30,
    "pick_order_frames": 10,
    "same_label_created_frames": 600,
    "two_hit_frames": 1000,
    "negative_track_frames": 250,
    "nan_frames": 40,
    "nan_with_hits": 15,
    "nan_without_hits": 15,
    "delay0": 12,
    "delay1": 12,
    "delay2": 12,
    "delay3": 12,
    "max1": 6,
    "past0": 8,
    "f32_threshold": 15,
}


class _Dummy:
    def __init__(self, *args, **kwargs):
        pass


class _StubLoader(importlib.abc.Loader):
    def create_module(self, spec):
        module = types.ModuleType(spec.name)
        module.__path__ = []
        module.__getattr__ = lambda attr: print if attr == "print" else _Dummy
        return module

    def exec_module(self, module):
        pass


class _StubFinder(importlib.abc.MetaPathFinder):
    def find_spec(self, name, path, target=None):
        if name.split(".")[0] in ("rich", "filterpy"):
            return importlib.machinery.ModuleSpec(name, _StubLoader(), is_package=True)
        return None


def tree_hash():
    names = sorted(f"./{path.relative_to(NORFAIR_PACKAGE)}" for path in NORFAIR_PACKAGE.rglob("*.py"))
    manifest = b""
    for name in names:
        digest = hashlib.sha256((NORFAIR_PACKAGE / name[2:]).read_bytes()).hexdigest()
        manifest += f"{digest}  {name}\n".encode()
    return hashlib.sha256(manifest).hexdigest()


assert tree_hash() == TREE_SHA256, "the norfair package does not hash to the pinned tree hash"
sys.meta_path.insert(0, _StubFinder())
_original_version = importlib.metadata.version
importlib.metadata.version = lambda name: "2.3.0" if name == "norfair" else _original_version(name)
sys.path.insert(0, str(SRC))
import numpy as np
import norfair

warnings.simplefilter("ignore")
np.seterr(all="ignore")
logging.disable(logging.CRITICAL)
from norfair.filter import OptimizedKalmanFilterFactory
from norfair.tracker import Detection, Tracker, _TrackedObjectFactory

assert norfair.__version__ == "2.3.0", norfair.__version__


def blob_hash(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


try:
    cmd = ["git", "-C", FRIGATE_DIR, "show", f"v0.17.2:{FRIGATE_FILE}"]
    RAW = subprocess.run(cmd, capture_output=True, check=True).stdout
except (OSError, subprocess.SubprocessError) as error:
    sys.exit(f"cannot read v0.17.2:{FRIGATE_FILE} from {FRIGATE_DIR}: {error}")
assert blob_hash(RAW) == FRIGATE_BLOB, "the Frigate source does not hash to the pinned blob"
LINES = RAW.decode("utf-8").splitlines()
assert LINES[40].startswith("def distance("), LINES[40]
assert LINES[73].startswith("def frigate_distance("), LINES[73]
NAMESPACE = {"np": np, "Detection": Detection}
exec(compile("\n".join(LINES[40:75]), FRIGATE_FILE, "exec"), NAMESPACE)
frigate_distance = NAMESPACE["frigate_distance"]


class Lcg:
    def __init__(self, seed):
        self.state = seed

    def randint(self, low, high):
        self.state = (self.state * 6364136223846793005 + 1442695040888963407) & 0xFFFFFFFFFFFFFFFF
        return low + (self.state >> 33) % (high - low + 1)

    def uniform(self):
        bits = self.randint(0, 2**31 - 1) * 2**31 + self.randint(0, 2**31 - 1)
        return bits / 2**62


rng = Lcg(SEED)


def rint(low, high):
    return rng.randint(low, high)


def hx64(value):
    return struct.pack(">d", float(value)).hex()


def rounds_to_f32(value):
    return struct.unpack(">f", struct.pack(">f", float(value)))[0] == float(value)


def tag_of(detection):
    if isinstance(detection.data, dict):
        return detection.data["tag"]
    return detection.data


def make_object(frame_count):
    appear = rint(0, frame_count // 2 - 1)
    first_vanish = appear + 5
    vanish = first_vanish + rint(0, max(0, frame_count - first_vanish))
    x1 = float(rint(0, 1499))
    y1 = float(rint(0, 799))
    width = rint(20, 249)
    height = rint(20, 249)
    vx = rint(-12, 12)
    vy = rint(-12, 12)
    drift = [rint(-10, 10) / 10.0 for _ in range(4)]
    corners = [x1, y1, x1 + float(width), y1 + float(height)]
    corner_vel = [vx + drift[0], vy + drift[1], vx + drift[2], vy + drift[3]]
    prob = DETECT_PROBS[rint(0, len(DETECT_PROBS) - 1)]
    label = None
    return {
        "appear": appear,
        "vanish": vanish,
        "corners": corners,
        "corner_vel": corner_vel,
        "prob": prob,
        "label": label,
    }


def trajectory_edges(obj, frame):
    shift = frame - obj["appear"]
    return [obj["corners"][slot] + obj["corner_vel"][slot] * shift for slot in range(4)]


def arrange_crossing(objects):
    chosen = rng.randint(0, len(objects) - 2)
    left, right = objects[chosen], objects[chosen + 1]
    appear = max(left["appear"], right["appear"])
    left_edges = trajectory_edges(left, appear)
    right_edges = trajectory_edges(right, appear)
    if right_edges[0] < left_edges[0]:
        left, right = right, left
    gap = float(rint(60, 150))
    speed = float(rint(6, 12))
    right["label"] = left["label"]
    delta_x = left_edges[0] + gap - right_edges[0]
    delta_y = left_edges[1] - right_edges[1]
    right["corners"] = [
        value + (delta_x if slot in (0, 2) else delta_y)
        for slot, value in enumerate(right["corners"])
    ]
    right["corner_vel"] = [-speed, 0.0, -speed, 0.0]
    left["corner_vel"] = [speed, 0.0, speed, 0.0]


def make_stretch(obj, frame_count):
    start = obj["appear"] + rint(0, max(0, obj["vanish"] - obj["appear"] - 5))
    upper = min(25, frame_count - start, obj["vanish"] - start)
    if upper < 5:
        return None
    length = rint(5, upper)
    edges = [int(value + 0.5) for value in trajectory_edges(obj, start)]
    if rint(0, 1) == 0:
        center = (edges[0] + edges[2]) // 2
        box = [center, edges[1], center, edges[3]]
    else:
        center = (edges[1] + edges[3]) // 2
        box = [edges[0], center, edges[2], center]
    return {"obj": obj, "start": start, "length": length, "box": box}


def state_of(obj):
    id_text = "-" if obj.id is None else str(obj.id)
    active = int(not obj.is_initializing and obj.hit_counter_is_positive)
    past = "/".join(f"{tag_of(d)}@{d.age}" for d in obj.past_detections) or "-"
    fields = [
        obj.label,
        str(obj.hit_counter),
        str(obj.age),
        str(int(obj.is_initializing)),
        str(obj.initializing_id),
        id_text,
        str(active),
        str(tag_of(obj.last_detection)),
        past,
    ]
    fields += [hx64(value) for value in obj.estimate.flatten()]
    return " ".join(fields)


def make_config(number):
    if number <= 60:
        threshold, r_value, q_value = FRIGATE_SETTINGS[rint(0, len(FRIGATE_SETTINGS) - 1)]
        return 25, 2, 4, threshold, r_value, q_value
    hit_max = MAX_CHOICES[rint(0, len(MAX_CHOICES) - 1)]
    valid_top = min(hit_max - 1, 4)
    if valid_top <= 0:
        delay_choices = [0]
    elif valid_top == 1:
        delay_choices = [0, 0, 1]
    elif valid_top == 2:
        delay_choices = [0, 1, 1, 2, 2]
    else:
        template = [0, 1, 1, 2, 2, 3, 3, 3, 3]
        delay_choices = [choice for choice in template if choice <= valid_top]
    delay = delay_choices[rint(0, len(delay_choices) - 1)]
    past = PAST_CHOICES[rint(0, len(PAST_CHOICES) - 1)]
    threshold = THRESHOLDS[rint(0, len(THRESHOLDS) - 1)]
    r_value = 0.5 + rng.uniform() * 9.5
    q_value = 0.001 + rng.uniform() * 0.999
    return hit_max, delay, past, threshold, r_value, q_value


def run_scenario(number, stats):
    hit_max, delay, past, threshold, r_value, q_value = make_config(number)
    frame_count = rint(30, 70)
    label_pool = LABELS[: 2 + rint(0, 1)]
    object_count = OBJ_COUNTS[rint(0, len(OBJ_COUNTS) - 1)]
    objects = []
    for _ in range(object_count):
        obj = make_object(frame_count)
        obj["label"] = label_pool[rint(0, len(label_pool) - 1)]
        objects.append(obj)
    if object_count >= 2 and rint(0, 2) == 0:
        arrange_crossing(objects)
    stretch = None
    stretch_chance = 2 if delay == 0 else 9
    if rint(0, stretch_chance) == 0:
        candidate = objects[rint(0, object_count - 1)]
        stretch = make_stretch(candidate, frame_count)
    gap = None
    if rint(0, 11) == 0:
        gap_length = rint(10, 30)
        if gap_length < frame_count:
            gap = (rint(0, frame_count - gap_length), gap_length)
    jump = None
    if rint(0, 9) == 0:
        jumper = objects[rint(0, object_count - 1)]
        span = jumper["vanish"] - jumper["appear"] - 1
        if span > 0:
            magnitude = float(rint(100, 400)) * (1.0 if rint(0, 1) == 0 else -1.0)
            jump = (jumper, jumper["appear"] + 1 + rint(0, span - 1), magnitude, rint(0, 1))

    tracker = Tracker(
        distance_function=frigate_distance,
        distance_threshold=threshold,
        hit_counter_max=hit_max,
        initialization_delay=delay,
        filter_factory=OptimizedKalmanFilterFactory(R=r_value, Q=q_value),
        past_detections_length=past,
    )
    base = _TrackedObjectFactory.global_count
    stats["scenarios"] += 1
    if delay <= 3:
        stats[f"delay{delay}"] += 1
    if hit_max == 1:
        stats["max1"] += 1
    if past == 0:
        stats["past0"] += 1
    if not rounds_to_f32(threshold):
        stats["f32_threshold"] += 1

    lines = [
        f"scenario {number} max {hit_max} delay {delay} past {past} "
        f"threshold {hx64(threshold)} r {hx64(r_value)} q {hx64(q_value)}"
    ]
    tag = [0]
    previous = []
    for frame in range(frame_count):
        draws = []
        if gap is None or not gap[0] <= frame < gap[0] + gap[1]:
            for obj in objects:
                if not obj["appear"] <= frame < obj["vanish"]:
                    continue
                forced = (
                    stretch is not None
                    and stretch["obj"] is obj
                    and stretch["start"] <= frame < stretch["start"] + stretch["length"]
                )
                if forced or rng.uniform() < obj["prob"]:
                    if forced:
                        box = list(stretch["box"])
                    else:
                        edges = trajectory_edges(obj, frame)
                        if jump is not None and jump[0] is obj and frame >= jump[1]:
                            edges[jump[3]] += jump[2]
                            edges[jump[3] + 2] += jump[2]
                        box = [int(value + 0.5) + rint(-4, 4) for value in edges]
                    draws.append((box, obj["label"]))
                    if rint(0, 99) < 12:
                        jittered = [value + rint(-4, 4) for value in box]
                        draws.append((jittered, obj["label"]))
            if rint(0, 99) < 12:
                extra_x = rint(0, 1200)
                extra_y = rint(0, 600)
                extra_box = [extra_x, extra_y, extra_x + rint(20, 240), extra_y + rint(20, 240)]
                draws.append((extra_box, label_pool[rint(0, len(label_pool) - 1)]))
        for slot in range(len(draws) - 1, 0, -1):
            swap = rint(0, slot)
            draws[slot], draws[swap] = draws[swap], draws[slot]
        frame_dets = []
        lines.append(f"frame {frame + 1} dets {len(draws)}")
        for box, label in draws:
            lines.append(f"det {tag[0]} {label} {box[0]} {box[1]} {box[2]} {box[3]}")
            frame_dets.append(
                Detection(
                    points=np.array([[box[0], box[1]], [box[2], box[3]]]),
                    label=label,
                    data={"tag": tag[0]},
                )
            )
            tag[0] += 1
        raised = False
        returned = None
        try:
            returned = tracker.update(detections=frame_dets)
        except ValueError:
            raised = True
        listed = list(tracker.tracked_objects)
        if not raised:
            expected_active = [o for o in listed if not o.is_initializing and o.hit_counter_is_positive]
            assert [id(o) for o in returned] == [id(o) for o in expected_active]
        stats["frames"] += 1
        stats["empty_frames"] += int(len(draws) == 0)
        stats["triple_frames"] += int(len(draws) >= 3)
        lines.append(f"result {'nan' if raised else 'ok'} tracks {len(listed)}")
        current = []
        for obj in listed:
            lines.append(f"track {state_of(obj)}")
            if obj.id is not None:
                assert obj.global_id - base == obj.id, (number, frame, obj.initializing_id)
            current.append((obj.initializing_id, obj.is_initializing, tag_of(obj.last_detection), obj.label, obj.id))
            stats["negative_track_frames"] += int(obj.hit_counter < 0)
        previous_map = {row[0]: (row[1], row[2]) for row in previous}
        current_set = {row[0] for row in current}
        created = [row for row in current if row[0] not in previous_map]
        removed = [row for row in previous if row[0] not in current_set]
        hits = 0
        for row in current:
            old = previous_map.get(row[0])
            if old is None or row[2] == old[1]:
                continue
            hits += 1
            if old[0]:
                stats["second_pass_hits"] += 1
            else:
                stats["first_pass_hits"] += 1
        left = [row for row in current if previous_map.get(row[0], (False,))[0] and not row[1]]
        stats["created"] += len(created)
        stats["removed"] += len(removed)
        stats["left_initializing"] += len(left)
        stats["two_left_frames"] += int(len(left) >= 2)
        stats["two_hit_frames"] += int(hits >= 2)
        left_ids = [row[4] for row in left if row[4] is not None]
        if any(first > second for first, second in zip(left_ids, left_ids[1:])):
            stats["pick_order_frames"] += 1
        created_labels = {row[3] for row in created}
        previous_labels = {row[3] for row in previous}
        stats["same_label_created_frames"] += int(len(created) > 0 and len(created_labels & previous_labels) > 0)
        stats["nan_frames"] += int(raised)
        stats["nan_with_hits"] += int(raised and hits >= 1)
        stats["nan_without_hits"] += int(raised and hits == 0)
        previous = current
    lines.append("end")
    lines.append("")
    return lines


def main():
    stats = {name: 0 for name in FLOORS}
    blocks = []
    for number in range(1, 121):
        blocks.extend(run_scenario(number, stats))
    failing = [f"floor {name}: {stats[name]} < {floor}" for name, floor in FLOORS.items() if stats[name] < floor]
    assert not failing, "floors not reached: " + "; ".join(failing)
    header = [
        "# tracker-update goldens: the exact per-frame state norfair 2.3.0 Tracker.update (norfair/tracker.py 144-260) with Frigate's frigate_distance leaves after every frame of 120 seeded scenarios: the raise flag, then every track's label, hit counter, age, initializing flag, initializing id, id, active flag, last tag, past list and Kalman estimate",
        f"# source norfair-2.3.0 sdist sha256 {SDIST_SHA256} (PyPI), package tree hash {TREE_SHA256} (find . -name '*.py' | LC_ALL=C sort | xargs sha256sum | sha256sum under norfair/), classes Tracker (tracker.py 29-386) and TrackedObject, distance frigate_distance (frigate/track/norfair_tracker.py lines 41 to 75 at v0.17.2, blob {FRIGATE_BLOB})",
        f"# seed {SEED}",
        f"# numpy {np.__version__} (goldens shell; norfair declares numpy 1.x; every recorded field comes from the real norfair object running on this numpy)",
        "# Frigate's trackers use initialization_delay 2, hit_counter_max 25 and never pass past_detections_length (the library default 4); the four (threshold, R, Q) settings are (2.5, 3.4, 0.03), (3.75, 2.5, 0.05), (2.5, 3.4, 0.1) and (3.0, 4.0, 0.2) (norfair_tracker.py lines 126, 131, 159, 165 and 199 to 209 at v0.17.2); half the scenarios use one of them, the other half draws max, delay, past, threshold and the filter parameters at random",
        "# counts " + " ".join(f"{name} {stats[name]}" for name in FLOORS),
        "# floors " + " ".join(f"{name} {FLOORS[name]}" for name in FLOORS),
    ]
    content = "\n".join(header + blocks).rstrip("\n") + "\n"
    assert len(content.encode("utf-8")) <= 4_000_000, "fixture over four megabytes"
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text(content, encoding="utf-8")
    summary = " ".join(f"{name}={stats[name]}" for name in FLOORS)
    print(f"track_update: {summary}")


main()
