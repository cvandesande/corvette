"""Goldens for corvette_track::ObjectTracker: the per-frame object state that
Frigate v0.17.2's own NorfairTracker.match_and_update leaves when run on a
static camera, driven through the real norfair 2.3.0 Tracker over 150 seeded
scenarios.

The reference is Frigate at v0.17.2, each file blob-checked here:
frigate/track/norfair_tracker.py blob
84a0f390a3742ab6dc5a339a1b54480570280110, and
frigate/util/object.py blob b8f41e2c3214c22980f5227055aad348c9d6cd10 lines
339-358, frigate/util/image.py blob ea9fb0a0a7f166d7b145c6c3af9941de673aebe3
lines 701-749, frigate/track/stationary_classifier.py blob
bea37f641beb7363cec3444bbcba456c29351736 lines 14-80. Executed unchanged as
methods of a shell class: distance, frigate_distance, _create_tracker,
get_tracker, register, deregister, update_position, is_expired, update,
update_frame_times and match_and_update. Two deliberate changes and nothing
else: median_of_boxes is replaced by the ordinary median (an even count takes
the element-wise mean of the two middle boxes), which is what the Corvette
design specifies, and the one list rewrite in deregister,
`tracker.tracked_objects = [...]`, removes only the track whose
str(global_id) equals the removed track_id instead of also dropping every
other live track of that tracker (the Corvette deregister contract). The
shell's __init__ sets the attributes those methods read and builds the
trackers the way __init__ lines 126-195 and _create_tracker do, from the real
norfair Tracker and OptimizedKalmanFilterFactory: static mode only, car and
license_plate get their own trackers when objects.track names them, reid
parameters are added only when that label's max_frames is None and the static
object_type_configs carry no reid keys, so that branch changes nothing. The
stationary classifier is a stub whose on_active does nothing and whose
evaluate and anchor path are never reached (no frame, classifier off, no
autotracking). random.choices in register is seeded so ids are deterministic
strings; the ids' text is never compared, objects are named by registration
order. norfair's global id counter is a class attribute shared by every
tracker in the process, so the global ids recorded on the n lines continue
across every scenario of one run and restart only when the script runs again;
the Rust replay therefore threads one shared id source through all scenarios
it replays in order.

Run command:

    cd /home/cvandesande/github/corvette && timeout 600 nix develop .#goldens --command python3 -I tools/goldens/track_object_tracker.py /home/cvandesande/github/frigate <extracted norfair directory>

The norfair reference is the norfair 2.3.0 sdist norfair-2.3.0.tar.gz,
sha256 8331c7fb2290fe92e9cd73877d43b0a5134fafd8846082002a9b545845d2a79c
(PyPI); extract it and pass the directory that holds the norfair package.
The script asserts the package tree hash
e8626859af72d20d25705e3563f2a6d3f7ae410476e4adaae106c0672bb0d5be — the
recipe `cd <dir>/norfair && find . -name '*.py' | LC_ALL=C sort | xargs
sha256sum | sha256sum`, recomputed here — and the version norfair reports.
rich and filterpy are stubbed import hooks and importlib.metadata.version is
patched so `import norfair` succeeds, exactly as in track_update.py; the
shell's numpy 2.x is newer than norfair's declared numpy 1.x pin and every
recorded field comes from the real norfair objects running on this numpy.

Fixture crates/corvette-track/tests/fixtures/object_tracker_cases.txt: #
header lines first (title, source, seed, versions, counts, floors, notes),
then 150 scenario blocks:

    s <n> size <w> <h> init <i> gone <g> after <a> defmax <n|none> max <none|lab=n,lab=n> own <none|lab,lab>
    d <time> <ndets>            detections frame, followed by ndets lines
    D <label> <score> <x0>,<y0>,<x1>,<y1> <area> <ratio> <region>
    t <time>                    update_frame_times frame
    o <idx> <label> <score> <box> <area> <ratio> <region> <frame_time> <cx>,<cy> <est4> <start> <motionless> <changes> <scores> <hk> <hboxes> <disappeared> <bits>
    m <n>                       entries in track_id_map
    u <n>                       untracked object boxes, next line their boxes or -
    n <name> <count> <triples>  one line per tracker (own labels then default)
    end

Boxes are x0,y0,x1,y1 joined with commas; est4 is the clamped estimate; bits
is the four estimate_velocity floats followed by the four position envelope
floats, each as big-endian float64 hex, concatenated; scores is - or the
score history joined with commas; triples is - or
global_id:hit_counter:initializing joined with spaces; region is - or boxes
joined with /. Integers are decimal and floats are Python repr; two runs
write byte-identical files.
"""

import hashlib
import importlib.abc
import importlib.machinery
import importlib.metadata
import logging
import random
import string
import struct
import subprocess
import sys
import types
import warnings
from dataclasses import dataclass, field
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Optional, cast

if len(sys.argv) != 3:
    sys.exit(
        "usage: track_object_tracker.py <Frigate checkout directory> "
        "<extracted norfair directory (the one holding norfair/)>"
    )

FRIGATE_DIR = sys.argv[1]
SRC = Path(sys.argv[2]).resolve()
NORFAIR_PACKAGE = SRC / "norfair"
FIXTURE = Path(__file__).resolve().parents[2] / "crates/corvette-track/tests/fixtures/object_tracker_cases.txt"
SDIST_SHA256 = "8331c7fb2290fe92e9cd73877d43b0a5134fafd8846082002a9b545845d2a79c"
TREE_SHA256 = "e8626859af72d20d25705e3563f2a6d3f7ae410476e4adaae106c0672bb0d5be"
SEED = 20261161
SCENARIOS = 150
LABELS = ["person", "car", "license_plate", "dog", "bicycle", "package"]
FLOORS = {
    "scenarios": 150,
    "frames": 5000,
    "registered": 400,
    "retire_max_frames": 40,
    "retire_expiry": 40,
    "frames_disappeared": 40,
    "frames_untracked": 40,
    "threshold_crossings": 40,
    "retire_with_sibling": 40,
    "time_only_frames": 40,
}
DEFAULT_MAX_CHOICES = [None, None, None, None, 0, 1, 2, 3, 5, 10, 30, 60, 200]
OBJECT_COUNTS = [1, 1, 1, 1, 1, 1, 2, 2, 2, 3, 4, 5]
PROFILES = ["still", "still", "still", "walker", "walker", "drift"]
DETECT_PROBS = [0.8, 0.85, 0.9, 0.95, 1.0, 1.0]
TIME_STEPS = [0.04, 0.1, 0.5, 1.0]


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
from norfair.tracker import Detection, Tracker

assert norfair.__version__ == "2.3.0", norfair.__version__


def blob_hash(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def blob(path, want):
    cmd = ["git", "-C", FRIGATE_DIR, "show", f"v0.17.2:{path}"]
    try:
        raw = subprocess.run(cmd, capture_output=True, check=True).stdout
    except (OSError, subprocess.SubprocessError) as error:
        sys.exit(f"cannot read v0.17.2:{path} from {FRIGATE_DIR}: {error}")
    assert blob_hash(raw) == want, f"{path} does not hash to the pinned blob"
    return raw.decode("utf-8").splitlines()


TRACKER_FILE = "frigate/track/norfair_tracker.py"
TRACKER = blob(TRACKER_FILE, "84a0f390a3742ab6dc5a339a1b54480570280110")
OBJECT = blob("frigate/util/object.py", "b8f41e2c3214c22980f5227055aad348c9d6cd10")
IMAGE = blob("frigate/util/image.py", "ea9fb0a0a7f166d7b145c6c3af9941de673aebe3")
CLASSIFIER = blob("frigate/track/stationary_classifier.py", "bea37f641beb7363cec3444bbcba456c29351736")

assert TRACKER[40].startswith("def distance("), TRACKER[40]
assert TRACKER[73].startswith("def frigate_distance("), TRACKER[73]
assert TRACKER[218].startswith("    def _create_tracker("), TRACKER[218]
assert TRACKER[245].strip() == "return Tracker(**tracker_params)", TRACKER[245]
assert TRACKER[247].startswith("    def get_tracker("), TRACKER[247]
assert TRACKER[260].startswith("    def register("), TRACKER[260]
assert TRACKER[299].startswith("    def deregister("), TRACKER[299]
assert TRACKER[323].startswith("    def update_position("), TRACKER[323]
assert TRACKER[426].startswith("    def is_expired("), TRACKER[426]
assert TRACKER[446].startswith("    def update("), TRACKER[446]
assert TRACKER[478].strip() == "self.tracked_objects[id].update(obj)", TRACKER[478]
assert TRACKER[480].startswith("    def update_frame_times("), TRACKER[480]
assert TRACKER[496].startswith("    def match_and_update("), TRACKER[496]
assert TRACKER[635].strip() == "]", TRACKER[635]
assert OBJECT[338].startswith("def average_boxes("), OBJECT[338]
assert OBJECT[354].startswith("def median_of_boxes("), OBJECT[354]
assert IMAGE[700].startswith("def intersection("), IMAGE[700]
assert IMAGE[748].strip() == "return iou", IMAGE[748]
assert CLASSIFIER[13].startswith("@dataclass"), CLASSIFIER[13]
assert CLASSIFIER[66].startswith("def get_stationary_threshold("), CLASSIFIER[66]

DEREGISTER_ORIGINAL = "\n".join(TRACKER[299:320])
DEREGISTER_LIVE_KEEP = "if str(o.global_id) != track_id and o.hit_counter < 0"
DEREGISTER_ONE_TRACK = "if str(o.global_id) != track_id"
assert DEREGISTER_ORIGINAL.count(DEREGISTER_LIVE_KEEP) == 1
DEREGISTER_MODIFIED = DEREGISTER_ORIGINAL.replace(DEREGISTER_LIVE_KEEP, DEREGISTER_ONE_TRACK)


def build_namespace():
    ns = {
        "np": np,
        "cast": cast,
        "Optional": Optional,
        "Any": Any,
        "dataclass": dataclass,
        "field": field,
        "random": random,
        "string": string,
        "Tracker": Tracker,
        "Detection": Detection,
        "OptimizedKalmanFilterFactory": OptimizedKalmanFilterFactory,
    }
    exec(compile("\n".join(TRACKER[40:75]), TRACKER_FILE, "exec"), ns)
    exec(compile("\n".join(IMAGE[700:749]), "image.py", "exec"), ns)
    exec(compile("\n".join(OBJECT[338:358]), "object.py", "exec"), ns)
    exec(compile("\n".join(CLASSIFIER[13:80]), "stationary_classifier.py", "exec"), ns)
    area = ns["area"]

    def ordinary_median(boxes):
        ordered = sorted(boxes, key=lambda x: area(x))
        count = len(ordered)
        if count % 2 == 1:
            return ordered[count // 2]
        lower, upper = ordered[count // 2 - 1], ordered[count // 2]
        return [(lower[i] + upper[i]) / 2.0 for i in range(4)]

    ns["median_of_boxes"] = ordinary_median
    body = "\n".join(
        [
            "\n".join(TRACKER[218:246]),
            "\n".join(TRACKER[247:259]),
            "\n".join(TRACKER[260:298]),
            DEREGISTER_MODIFIED,
            "\n".join(TRACKER[323:425]),
            "\n".join(TRACKER[426:445]),
            "\n".join(TRACKER[446:479]),
            "\n".join(TRACKER[480:495]),
            "\n".join(TRACKER[496:636]),
            "",
        ]
    )
    exec(compile("class Real:\n" + body, TRACKER_FILE, "exec"), ns)
    return ns


NS = build_namespace()
frigate_distance = NS["frigate_distance"]


class NoClassifier:
    def on_active(self, id):
        pass

    def __getattr__(self, name):
        raise AssertionError(f"classifier.{name} touched")


class Subject(NS["Real"]):
    def __init__(self, width, height, min_initialized, max_disappeared, threshold,
                 default_max, max_overrides, own_labels, stats):
        self.stats = stats
        self.tracked_objects = {}
        self.untracked_object_boxes = []
        self.disappeared = {}
        self.positions = {}
        self.stationary_box_history = {}
        self.track_id_map = {}
        self.camera_config = SimpleNamespace(
            objects=SimpleNamespace(track=list(own_labels)),
            onvif=SimpleNamespace(autotracking=SimpleNamespace(enabled_in_config=False, track=[])),
        )
        self.detect_config = SimpleNamespace(
            width=width,
            height=height,
            min_initialized=min_initialized,
            max_disappeared=max_disappeared,
            stationary=SimpleNamespace(
                threshold=threshold,
                classifier=False,
                max_frames=SimpleNamespace(objects=dict(max_overrides), default=default_max),
            ),
        )
        self.ptz_metrics = SimpleNamespace(autotracker_enabled=SimpleNamespace(value=False))
        self.stationary_classifier = NoClassifier()
        self.frame_manager = None
        self.object_type_configs = {
            "car": {
                "filter_factory": OptimizedKalmanFilterFactory(R=3.4, Q=0.03),
                "distance_function": frigate_distance,
                "distance_threshold": 2.5,
            },
            "license_plate": {
                "filter_factory": OptimizedKalmanFilterFactory(R=2.5, Q=0.05),
                "distance_function": frigate_distance,
                "distance_threshold": 3.75,
            },
        }
        self.default_tracker_config = {
            "filter_factory": OptimizedKalmanFilterFactory(R=3.4),
            "distance_function": frigate_distance,
            "distance_threshold": 2.5,
        }
        self.trackers = {}
        for obj_type, tracker_config in self.object_type_configs.items():
            if obj_type in self.camera_config.objects.track:
                self.trackers[obj_type] = {"static": self._create_tracker(obj_type, tracker_config)}
        self.default_tracker = {
            "static": Tracker(
                distance_function=frigate_distance,
                distance_threshold=self.default_tracker_config["distance_threshold"],
                initialization_delay=self.detect_config.min_initialized,
                hit_counter_max=self.detect_config.max_disappeared,
                filter_factory=self.default_tracker_config["filter_factory"],
            )
        }

    def max_frames_for(self, label):
        return self.detect_config.stationary.max_frames.objects.get(
            label, self.detect_config.stationary.max_frames.default
        )

    def deregister(self, id, track_id):
        obj = self.tracked_objects[id]
        max_frames = self.max_frames_for(obj["label"])
        capped = (
            max_frames is not None
            and obj["motionless_count"] - self.detect_config.stationary.threshold > max_frames
        )
        if capped:
            self.stats["retire_max_frames"] += 1
        else:
            self.stats["retire_expiry"] += 1
        if max_frames is not None:
            tracker = self.get_tracker(obj["label"])
            if any(
                str(o.global_id) != track_id and o.label == obj["label"] and o.hit_counter >= 0
                for o in tracker.tracked_objects
            ):
                self.stats["retire_with_sibling"] += 1
        super().deregister(id, track_id)


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
random.seed(SEED)


def rint(low, high):
    return rng.randint(low, high)


def pick(items):
    return items[rng.randint(0, len(items) - 1)]


def signed(low, high):
    return (1 if rng.uniform() < 0.5 else -1) * rint(low, high)


def hx64(value):
    return struct.pack(">d", float(value)).hex()


def clamp_box(box, frame):
    width, height = frame
    x0, y0, x1, y1 = box
    x0 = max(0, min(x0, width - 1))
    x1 = max(0, min(x1, width - 1))
    y0 = max(0, min(y0, height - 1))
    y1 = max(0, min(y1, height - 1))
    if x0 >= x1:
        x1 = x0 + 1
        if x1 > width - 1:
            x0, x1 = width - 2, width - 1
    if y0 >= y1:
        y1 = y0 + 1
        if y1 > height - 1:
            y0, y1 = height - 2, height - 1
    return [x0, y0, x1, y1]


def new_box(frame, max_side):
    box_w = rint(12, max_side)
    box_h = rint(12, max_side)
    x0 = rint(0, frame[0] - box_w - 1)
    y0 = rint(0, frame[1] - box_h - 1)
    return [x0, y0, x0 + box_w, y0 + box_h]


def encode_regions(regions):
    return "/".join(regions) if regions else "-"


def encode_box(box):
    return ",".join(str(v) for v in box)


def make_object(frame, frames_total, label):
    max_side = max(12, min(200, frame[0] // 3, frame[1] // 3))
    appear = rint(0, max(0, frames_total - 6))
    vanish = appear + rint(5, max(5, frames_total - appear))
    return {
        "label": label,
        "appear": appear,
        "vanish": vanish,
        "box": new_box(frame, max_side),
        "profile": pick(PROFILES),
        "vx": signed(1, 14),
        "vy": signed(1, 14),
        "prob": pick(DETECT_PROBS),
    }


def step_box(obj, frame):
    box = list(obj["box"])
    if obj["profile"] == "still":
        if rng.uniform() < 0.25:
            box = [v + signed(1, 2) for v in box]
    elif obj["profile"] == "walker":
        box = [box[0] + obj["vx"], box[1] + obj["vy"], box[2] + obj["vx"], box[3] + obj["vy"]]
        box = [v + signed(1, 2) for v in box]
    else:
        box = [v + signed(1, 3) for v in box]
    obj["box"] = clamp_box(box, frame)
    return list(obj["box"])


def arrange_crossing(objects):
    chosen = rng.randint(0, len(objects) - 2)
    left, right = objects[chosen], objects[chosen + 1]
    right["label"] = left["label"]
    speed = rint(6, 12)
    if right["box"][0] < left["box"][0]:
        left, right = right, left
    gap = rint(60, 150)
    delta_x = left["box"][0] + gap - right["box"][0]
    right["box"] = [v + delta_x if slot in (0, 2) else v for slot, v in enumerate(right["box"])]
    right["vx"] = -speed
    right["vy"] = 0
    left["vx"] = speed
    left["vy"] = 0


def frame_line(kind, frame_time, count):
    if kind == "t":
        return f"t {repr(frame_time)}"
    return f"d {repr(frame_time)} {count}"


def object_line(index, obj, position, history, disappeared):
    estimate = obj["estimate"]
    velocity = np.asarray(obj["estimate_velocity"]).flatten()
    envelope = [float(position[key]) for key in ("xmin", "ymin", "xmax", "ymax")]
    scores = ",".join(repr(float(value)) for value in obj["score_history"]) or "-"
    history_text = " ".join(encode_box(box) for box in history) or "-"
    bits = "".join(hx64(v) for v in velocity) + "".join(hx64(v) for v in envelope)
    return (
        f"o {index} {obj['label']} {repr(obj['score'])} {encode_box(obj['box'])} "
        f"{obj['area']} {repr(obj['ratio'])} {encode_regions(obj['region'])} "
        f"{repr(obj['frame_time'])} {obj['centroid'][0]},{obj['centroid'][1]} "
        f"{encode_box(estimate)} {repr(obj['start_time'])} {obj['motionless_count']} "
        f"{obj['position_changes']} {scores} {len(history)} {history_text} {disappeared} {bits}"
    )


def draw_scenario(number, stats):
    width = rint(100, 4000)
    height = rint(100, 3000)
    frame = (width, height)
    min_initialized = rint(1, 4)
    max_disappeared = rint(5, 40)
    threshold = rint(1, 60)
    default_max = pick(DEFAULT_MAX_CHOICES)
    max_overrides = {}
    for label in LABELS:
        if rng.uniform() < 0.25:
            max_overrides[label] = rint(0, 50)
    own_labels = [label for label in ("car", "license_plate") if rng.uniform() < 0.75]
    if not own_labels:
        own_labels = ["car"]
    frame_count = rint(40, 44)
    roll = rng.uniform()
    if roll < 0.05:
        frame_count = rint(45, 90)
    elif roll < 0.08:
        frame_count = rint(91, 400)
    object_count = pick(OBJECT_COUNTS)
    objects = [make_object(frame, frame_count, pick(LABELS)) for _ in range(object_count)]
    if object_count >= 2 and rng.uniform() < 0.25:
        arrange_crossing(objects)
    if number % 4 == 0:
        threshold = min(threshold, rint(1, 10))
        if default_max is None:
            default_max = rint(0, 3)
        still = objects[0]
        still["profile"] = "still"
        still["appear"] = 0
        still["vanish"] = frame_count
        still["prob"] = 1.0
        max_overrides.pop(still["label"], None)
    time_step = pick(TIME_STEPS)
    time_start = float(rint(1, 500) * 100)
    time_only = rng.uniform() < 0.45
    subject = Subject(width, height, min_initialized, max_disappeared, threshold,
                      default_max, max_overrides, own_labels, stats)

    max_text = "none" if default_max is None else str(default_max)
    overrides_text = (
        ",".join(f"{label}={value}" for label, value in sorted(max_overrides.items())) or "none"
    )
    own_text = ",".join(own_labels)
    lines = [
        f"s {number} size {width} {height} init {min_initialized} gone {max_disappeared} "
        f"after {threshold} defmax {max_text} max {overrides_text} own {own_text}"
    ]

    id_index = {}
    order = []
    previous_motion = {}

    for frame_number in range(frame_count):
        frame_time = round(time_start + frame_number * time_step, 3)
        if time_only and frame_number > 2 and rng.uniform() < 0.15:
            lines.append(frame_line("t", frame_time, 0))
            subject.update_frame_times("f", frame_time)
            stats["time_only_frames"] += 1
        else:
            dets = []
            for obj in objects:
                if not obj["appear"] <= frame_number < obj["vanish"]:
                    continue
                if rng.uniform() >= obj["prob"]:
                    continue
                box = step_box(obj, frame)
                score = round(0.25 + rng.uniform() * 0.74, 3)
                ratio = round(0.3 + rng.uniform() * 3.2, 3)
                area = int((box[2] - box[0]) * (box[3] - box[1]))
                dets.append((obj["label"], score, box, area, ratio, regions_of_box(box)))
            if rng.uniform() < 0.08:
                junk = new_box(frame, max(12, min(120, frame[0] // 6, frame[1] // 6)))
                score = round(0.25 + rng.uniform() * 0.74, 3)
                ratio = round(0.3 + rng.uniform() * 3.2, 3)
                area = int((junk[2] - junk[0]) * (junk[3] - junk[1]))
                dets.append((pick(LABELS), score, junk, area, ratio, regions_of_box(junk)))
            lines.append(frame_line("d", frame_time, len(dets)))
            for label, score, box, area, ratio, regions in dets:
                lines.append(
                    f"D {label} {repr(score)} {encode_box(box)} {area} {repr(ratio)} "
                    f"{encode_regions(regions)}"
                )
            subject.match_and_update("f", frame_time, detections=dets)
        lines.extend(record_state(subject, id_index, order, previous_motion, threshold, stats))
    lines.append("end")
    return lines


def regions_of_box(box):
    if rng.uniform() < 0.1:
        x = max(0, box[0] - 2)
        y = max(0, box[1] - 2)
        return [f"{x},{y},{x + 4},{y + 4}"]
    return []


def record_state(subject, id_index, order, previous_motion, threshold, stats):
    lines = []
    for id_value, obj in subject.tracked_objects.items():
        if id_value not in id_index:
            id_index[id_value] = len(order)
            order.append(id_value)
            stats["registered"] += 1
    any_disappeared = False
    for id_value, obj in subject.tracked_objects.items():
        index = id_index[id_value]
        position = subject.positions[id_value]
        history = subject.stationary_box_history[id_value]
        motion = obj["motionless_count"]
        if previous_motion.get(id_value, 0) < threshold <= motion:
            stats["threshold_crossings"] += 1
        previous_motion[id_value] = motion
        disappeared = subject.disappeared[id_value]
        any_disappeared = any_disappeared or disappeared > 0
        lines.append(object_line(index, obj, position, history, disappeared))
    if any_disappeared:
        stats["frames_disappeared"] += 1
    lines.append(f"m {len(subject.track_id_map)}")
    untracked = subject.untracked_object_boxes
    stats["frames_untracked"] += int(len(untracked) > 0)
    lines.append(f"u {len(untracked)}")
    lines.append(" ".join(encode_box(box) for box in untracked) or "-")
    for name, tracker in tracker_rows(subject):
        triples = " ".join(
            f"{('-' if o.global_id is None else o.global_id)}:{o.hit_counter}:{int(o.is_initializing)}"
            for o in tracker.tracked_objects
        ) or "-"
        lines.append(f"n {name} {len(tracker.tracked_objects)} {triples}")
    return lines


def tracker_rows(subject):
    rows = [(label, subject.trackers[label]["static"]) for label in subject.trackers]
    rows.append(("default", subject.default_tracker["static"]))
    return rows


def main():
    stats = {name: 0 for name in FLOORS}
    stats["scenarios"] = SCENARIOS
    blocks = []
    for number in range(1, SCENARIOS + 1):
        blocks.extend(draw_scenario(number, stats))
    stats["frames"] = sum(1 for line in blocks if line.startswith(("d ", "t ")))
    failing = [f"{name}: {stats[name]} < {floor}" for name, floor in FLOORS.items() if stats[name] < floor]
    if failing:
        for name, floor in FLOORS.items():
            print(f"{name}={stats[name]} floor={floor}", file=sys.stderr)
        raise AssertionError("floors not reached: " + "; ".join(failing))
    header = [
        "# object-tracker goldens: the tracked-object records, disappearance counts, untracked boxes and per-tracker track state that Frigate v0.17.2's own NorfairTracker.match_and_update leaves after every frame of 150 seeded scenarios on a static camera, run through the real norfair 2.3.0 Tracker with two deliberate changes",
        f"# source {TRACKER_FILE} at v0.17.2 blob 84a0f390a3742ab6dc5a339a1b54480570280110 lines 41-75, 218-636 executed unchanged as methods of a shell class; frigate/util/object.py blob b8f41e2c3214c22980f5227055aad348c9d6cd10 lines 339-358; frigate/util/image.py blob ea9fb0a0a7f166d7b145c6c3af9941de673aebe3 lines 701-749; frigate/track/stationary_classifier.py blob bea37f641beb7363cec3444bbcba456c29351736 lines 14-80; norfair-2.3.0 sdist sha256 {SDIST_SHA256}, package tree hash {TREE_SHA256}",
        "# the executed text differs from Frigate's original in exactly two places: median_of_boxes is replaced by the ordinary median (an even count takes the element-wise mean of the two middle boxes), and the list rewrite in deregister removes only the track whose str(global_id) equals the removed track_id instead of also dropping every other live track of that tracker",
        f"# seed {SEED}",
        f"# python {sys.version.split()[0]} numpy {np.__version__} norfair {norfair.__version__}",
        "# trackers: car (R 3.4, Q 0.03, threshold 2.5), license_plate (R 2.5, Q 0.05, threshold 3.75) own trackers when objects.track names them, default (R 3.4, Q 0.1, threshold 2.5), all with hit_counter_max = max_disappeared, initialization_delay = min_initialized and the library default past list of 4",
        "# counts " + " ".join(f"{name} {stats[name]}" for name in FLOORS),
        "# floors " + " ".join(f"{name} {FLOORS[name]}" for name in FLOORS),
        "# objects are numbered by first registration within a scenario; estimate_velocity and the position envelope are big-endian float64 hex, other floats are Python repr; static camera, no ptz, no frame, so every classifier branch takes the false branch",
        "# the global ids on the n lines come from one counter shared by every tracker of the generator process, so they increase across the scenarios of this file and restart only when the script runs again; a replay of one run's scenarios in order uses one shared id source",
    ]
    content = "\n".join(header + blocks).rstrip("\n") + "\n"
    data = content.encode("utf-8")
    assert len(data) <= 3_000_000, f"fixture is {len(data)} bytes, over 3 megabytes"
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_bytes(data)
    summary = " ".join(f"{name}={stats[name]}" for name in FLOORS)
    print(f"track_object_tracker: {summary} bytes={len(data)}")


main()
