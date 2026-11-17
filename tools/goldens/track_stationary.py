"""Goldens for corvette_track::Stationary: the motionless count, position
change count, position envelope, position lists and box history that Frigate
v0.17.2's own stationary lines leave after every step of 160 seeded motion
scenarios. The reference is four Frigate files at v0.17.2, each blob-checked
here: frigate/track/norfair_tracker.py blob
84a0f390a3742ab6dc5a339a1b54480570280110 lines 324-479 (update_position,
is_expired and update through the tracked_objects update line),
frigate/util/object.py blob b8f41e2c3214c22980f5227055aad348c9d6cd10 lines
339-358 (average_boxes, median_of_boxes), frigate/util/image.py blob
ea9fb0a0a7f166d7b145c6c3af9941de673aebe3 lines 701-749 (intersection, area,
intersection_over_union) and frigate/track/stationary_classifier.py blob
bea37f641beb7363cec3444bbcba456c29351736 lines 14-80. All lines run unchanged
except one function: in the primary namespace median_of_boxes is replaced by
the ordinary median (an even count takes the element-wise mean of the two
middle boxes), and a second subject runs the unchanged function so the steps
where the two medians differ can be counted. No frame is passed, so every
classifier branch in the executed lines is skipped, exactly as in Frigate
when no frame is given.

Run command:

    cd /home/cvandesande/github/corvette && timeout 300 nix develop .#goldens --command python3 -I tools/goldens/track_stationary.py /home/cvandesande/github/frigate

Fixture crates/corvette-track/tests/fixtures/stationary_cases.txt: # header
lines first (title, source, seed, python and numpy versions, counts, floors,
float note), then 160 blocks:

    scenario <n> label <label> after <n> max <n|none> size <w> <h> initial <k> <x0,y0,x1,y1>...
    step <x0,y0,x1,y1> verdict <alive|expired> motionless <n> changes <n> envelope <f> <f> <f> <f> lists <n> history <n> <x0,y0,x1,y1>...
    end

n counts scenarios from 1; initial lists the start boxes (always at least
one); a step line is the state after that step: the verdict, the motionless
count, the position change count, the position envelope as Python repr
floats, the length of the position lists and the whole box history in order
(history 0 - when empty). Floats are Python repr. Output is deterministic:
two runs write byte-identical files.
"""

import hashlib
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Optional, cast

import numpy as np

if len(sys.argv) != 2:
    sys.exit("usage: track_stationary.py <Frigate checkout directory>")

FRIGATE_DIR = sys.argv[1]
FIXTURE = Path(__file__).resolve().parents[2] / "crates/corvette-track/tests/fixtures/stationary_cases.txt"
SEED = 20261130
SCENARIOS = 160
LABELS = ["person", "person", "person", "dog", "car", "truck", "bicycle", "package", "waste_bin", "license_plate"]
FRAME_SIZES = [(1280, 720), (640, 480), (320, 240), (1920, 1080)]
AFTERS = [1, 1, 1, 2, 3, 5, 10, 50]
MAX_FRAMES_CHOICES = [None, None, None, 0, 1, 3, 10, 60]
INITIAL_LENGTHS = [1, 1, 1, 2, 3, 4]
PROFILES = ["still", "still", "jittery", "drifting", "drifting", "walker", "chaotic", "chaotic"]
FLOORS = {
    "scenarios": 160,
    "steps": 5500,
    "expired_scenarios": 25,
    "stationary_steps": 1600,
    "reset_steps": 900,
    "reset_while_stationary": 80,
    "changes_rose": 450,
    "reset_without_change": 100,
    "full_lists_steps": 1200,
    "full_history_steps": 600,
    "plate_full_history_steps": 100,
    "fractional_envelope_steps": 1800,
    "default_class_steps": 800,
    "dynamic_class_steps": 800,
    "object_class_steps": 800,
    "plate_class_steps": 800,
    "max_none_scenarios": 50,
    "max_zero_scenarios": 10,
    "initial_one_scenarios": 15,
    "initial_two_scenarios": 15,
    "initial_three_scenarios": 15,
    "initial_four_scenarios": 15,
    "median_path_steps": 800,
    "median_differs": 50,
}
LABEL_CLASS = {
    "person": "default_class_steps",
    "dog": "default_class_steps",
    "car": "dynamic_class_steps",
    "truck": "dynamic_class_steps",
    "bicycle": "dynamic_class_steps",
    "package": "object_class_steps",
    "waste_bin": "object_class_steps",
    "license_plate": "plate_class_steps",
}


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


TRACKER = blob("frigate/track/norfair_tracker.py", "84a0f390a3742ab6dc5a339a1b54480570280110")
OBJECT = blob("frigate/util/object.py", "b8f41e2c3214c22980f5227055aad348c9d6cd10")
IMAGE = blob("frigate/util/image.py", "ea9fb0a0a7f166d7b145c6c3af9941de673aebe3")
CLASSIFIER = blob("frigate/track/stationary_classifier.py", "bea37f641beb7363cec3444bbcba456c29351736")

assert TRACKER[323].startswith("    def update_position("), TRACKER[323]
assert TRACKER[426].startswith("    def is_expired("), TRACKER[426]
assert TRACKER[446].startswith("    def update("), TRACKER[446]
assert TRACKER[478].strip() == "self.tracked_objects[id].update(obj)", TRACKER[478]
assert OBJECT[338].startswith("def average_boxes("), OBJECT[338]
assert OBJECT[354].startswith("def median_of_boxes("), OBJECT[354]
assert IMAGE[700].startswith("def intersection("), IMAGE[700]
assert IMAGE[748].strip() == "return iou", IMAGE[748]
assert CLASSIFIER[13].startswith("@dataclass"), CLASSIFIER[13]
assert CLASSIFIER[66].startswith("def get_stationary_threshold("), CLASSIFIER[66]


def build(ordinary):
    ns = {
        "np": np,
        "cast": cast,
        "Optional": Optional,
        "Any": Any,
        "dataclass": dataclass,
        "field": field,
    }
    exec(compile("\n".join(IMAGE[700:749]), "image.py", "exec"), ns)
    exec(compile("\n".join(OBJECT[338:358]), "object.py", "exec"), ns)
    exec(compile("\n".join(CLASSIFIER[13:80]), "stationary_classifier.py", "exec"), ns)
    if ordinary:
        area = ns["area"]

        def ordinary_median(boxes):
            ordered = sorted(boxes, key=lambda x: area(x))
            count = len(ordered)
            if count % 2 == 1:
                return ordered[count // 2]
            lower, upper = ordered[count // 2 - 1], ordered[count // 2]
            return [(lower[i] + upper[i]) / 2.0 for i in range(4)]

        ns["median_of_boxes"] = ordinary_median
    exec(compile("class Real:\n" + "\n".join(TRACKER[323:479]) + "\n", "norfair_tracker.py", "exec"), ns)
    return ns


NS_ORD = build(True)
NS_REAL = build(False)
IOU_CALLS = {"count": 0}
BASE_IOU = NS_ORD["intersection_over_union"]


def counted_iou(box_a, box_b):
    IOU_CALLS["count"] += 1
    return BASE_IOU(box_a, box_b)


NS_ORD["intersection_over_union"] = counted_iou


class NoClassifier:
    def on_active(self, id):
        pass

    def __getattr__(self, name):
        raise AssertionError(f"classifier.{name} touched")


def make_subject(ns):
    class Subject(ns["Real"]):
        def __init__(self, label, after, max_frames, width, height, initial):
            self.detect_config = SimpleNamespace(
                width=width,
                height=height,
                stationary=SimpleNamespace(
                    threshold=after, max_frames=SimpleNamespace(objects={}, default=max_frames)
                ),
            )
            self.stationary_classifier = NoClassifier()
            self.tracked_objects = {"o": {"label": label, "motionless_count": 0, "position_changes": 0}}
            self.disappeared = {"o": 0}
            self.track_id_map = {"t": "o"}
            boxes = [list(b) for b in initial]
            xmins, ymins, xmaxs, ymaxs = zip(*boxes)
            self.positions = {
                "o": {
                    "xmins": list(xmins),
                    "ymins": list(ymins),
                    "xmaxs": list(xmaxs),
                    "ymaxs": list(ymaxs),
                    "xmin": 0,
                    "ymin": 0,
                    "xmax": width,
                    "ymax": height,
                }
            }
            self.stationary_box_history = {"o": boxes}
            self.deregistered = False

        def deregister(self, id, track_id):
            self.deregistered = True

    return Subject


SubjectOrd = make_subject(NS_ORD)
SubjectReal = make_subject(NS_REAL)


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


def pick(items):
    return items[rng.randint(0, len(items) - 1)]


def signed(low, high):
    return (1 if rng.uniform() < 0.5 else -1) * rint(low, high)


def clamp_box(box, frame):
    width, height = frame
    x0, y0, x1, y1 = box
    x0 = max(0, min(x0, width))
    x1 = max(0, min(x1, width))
    y0 = max(0, min(y0, height))
    y1 = max(0, min(y1, height))
    if x0 >= x1:
        x1 = x0 + 1
        if x1 > width:
            x0, x1 = width - 1, width
    if y0 >= y1:
        y1 = y0 + 1
        if y1 > height:
            y0, y1 = height - 1, height
    return [x0, y0, x1, y1]


def shift_box(box, dx, dy, frame):
    return clamp_box([box[0] + dx, box[1] + dy, box[2] + dx, box[3] + dy], frame)


def nudge_box(box, amount, frame):
    return clamp_box([box[0] + rint(-amount, amount), box[1] + rint(-amount, amount),
                      box[2] + rint(-amount, amount), box[3] + rint(-amount, amount)], frame)


def resize_box(box, frame):
    return clamp_box([box[0] + signed(2, 15), box[1] + signed(2, 15),
                      box[2] + signed(2, 15), box[3] + signed(2, 15)], frame)


def new_box(frame, max_width, max_height):
    width = rint(6, max_width)
    height = rint(6, max_height)
    x0 = rint(0, frame[0] - width)
    y0 = rint(0, frame[1] - height)
    return [x0, y0, x0 + width, y0 + height]


def draw_step_box(profile, current, frame, max_width, max_height, velocity):
    roll = rng.uniform()
    if profile == "still":
        return current if roll < 0.85 else nudge_box(current, 3, frame)
    if profile == "jittery":
        return current if roll < 0.40 else nudge_box(current, 4, frame)
    if profile == "drifting":
        if roll < 0.15:
            return current
        if roll < 0.30:
            return nudge_box(current, 4, frame)
        if roll < 0.80:
            return shift_box(current, signed(2, 30), signed(2, 30), frame)
        return resize_box(current, frame)
    if profile == "walker":
        return nudge_box(shift_box(current, velocity[0], velocity[1], frame), 1, frame)
    if roll < 0.30:
        return new_box(frame, max_width, max_height)
    if roll < 0.60:
        return shift_box(current, signed(2, 30), signed(2, 30), frame)
    return nudge_box(current, 4, frame)


def flat(boxes):
    return " ".join(",".join(str(v) for v in b) for b in boxes) or "-"


def run_scenario(number, stats):
    label = pick(LABELS)
    frame = pick(FRAME_SIZES)
    after = pick(AFTERS)
    max_frames = pick(MAX_FRAMES_CHOICES)
    initial_length = pick(INITIAL_LENGTHS)
    profile = pick(PROFILES)
    step_count = rint(25, 80)
    max_width = min(300, frame[0] // 2)
    max_height = min(300, frame[1] // 2)
    base = new_box(frame, max_width, max_height)
    initial = [base]
    for _ in range(initial_length - 1):
        initial.append(clamp_box([v + rint(-3, 3) for v in base], frame))
    velocity = (signed(3, 20), signed(3, 20))

    bump(stats, "scenarios")
    bump(stats, f"initial_{['zero', 'one', 'two', 'three', 'four'][initial_length]}_scenarios")
    if max_frames is None:
        bump(stats, "max_none_scenarios")
    if max_frames == 0:
        bump(stats, "max_zero_scenarios")

    subject = SubjectOrd(label, after, max_frames, frame[0], frame[1], initial)
    other = SubjectReal(label, after, max_frames, frame[0], frame[1], initial)
    thresholds = NS_ORD["get_stationary_threshold"](label)
    other_thresholds = NS_REAL["get_stationary_threshold"](label)

    shown_max = "none" if max_frames is None else str(max_frames)
    lines = [
        f"scenario {number} label {label} after {after} max {shown_max} size {frame[0]} {frame[1]} "
        f"initial {initial_length} {flat(initial)}"
    ]
    current = list(base)
    previous_motionless = 0
    previous_changes = 0
    expired = False
    for _ in range(step_count):
        current = draw_step_box(profile, current, frame, max_width, max_height, velocity)
        IOU_CALLS["count"] = 0
        subject.update("t", {"box": list(current)}, thresholds, None)
        other.update("t", {"box": list(current)}, other_thresholds, None)
        if (other.positions["o"], other.tracked_objects["o"]) != (
            subject.positions["o"],
            subject.tracked_objects["o"],
        ):
            bump(stats, "median_differs")
        if IOU_CALLS["count"] == 2:
            bump(stats, "median_path_steps")
        position = subject.positions["o"]
        obj = subject.tracked_objects["o"]
        history = subject.stationary_box_history["o"]
        envelope = [float(position[key]) for key in ("xmin", "ymin", "xmax", "ymax")]
        verdict = "expired" if subject.deregistered else "alive"

        bump(stats, "steps")
        bump(stats, LABEL_CLASS[label])
        if previous_motionless >= after:
            bump(stats, "stationary_steps")
        if obj["motionless_count"] == 0 and verdict == "alive":
            bump(stats, "reset_steps")
            if previous_motionless >= after:
                bump(stats, "reset_while_stationary")
            if obj["position_changes"] == previous_changes:
                bump(stats, "reset_without_change")
        if obj["position_changes"] > previous_changes:
            bump(stats, "changes_rose")
        if len(position["xmins"]) == 10:
            bump(stats, "full_lists_steps")
        if len(history) == 10:
            bump(stats, "full_history_steps")
        if label == "license_plate" and len(history) == 4:
            bump(stats, "plate_full_history_steps")
        if any(not value.is_integer() for value in envelope):
            bump(stats, "fractional_envelope_steps")

        lines.append(
            f"step {','.join(str(v) for v in current)} verdict {verdict} "
            f"motionless {obj['motionless_count']} changes {obj['position_changes']} "
            f"envelope {' '.join(repr(value) for value in envelope)} "
            f"lists {len(position['xmins'])} history {len(history)} {flat(history)}"
        )
        previous_motionless = obj["motionless_count"]
        previous_changes = obj["position_changes"]
        if subject.deregistered:
            expired = True
            break
    if expired:
        bump(stats, "expired_scenarios")
    lines.append("end")
    return lines


def bump(stats, name):
    stats[name] += 1


def main():
    stats = {name: 0 for name in FLOORS}
    blocks = []
    for number in range(1, SCENARIOS + 1):
        blocks.extend(run_scenario(number, stats))
    failing = [f"floor {name}: {stats[name]} < {floor}" for name, floor in FLOORS.items() if stats[name] < floor]
    if failing:
        for name, floor in FLOORS.items():
            print(f"{name}={stats[name]} floor={floor}", file=sys.stderr)
        raise AssertionError("floors not reached: " + "; ".join(failing))
    header = [
        "# stationary goldens: the motionless count, position change count, position envelope, position lists and box history Frigate v0.17.2 leaves after every step of 160 seeded motion scenarios, its own lines run unchanged except median_of_boxes",
        "# source frigate/track/norfair_tracker.py at v0.17.2 blob 84a0f390a3742ab6dc5a339a1b54480570280110, lines 324-479; frigate/util/object.py blob b8f41e2c3214c22980f5227055aad348c9d6cd10, lines 339-358; frigate/util/image.py blob ea9fb0a0a7f166d7b145c6c3af9941de673aebe3, lines 701-749; frigate/track/stationary_classifier.py blob bea37f641beb7363cec3444bbcba456c29351736, lines 14-80; executed unchanged except median_of_boxes, which the ordinary median replaces",
        f"# seed {SEED}",
        f"# python {sys.version.split()[0]} numpy {np.__version__}",
        "# counts " + " ".join(f"{name} {stats[name]}" for name in FLOORS),
        "# floors " + " ".join(f"{name} {FLOORS[name]}" for name in FLOORS),
        "# floats use Python repr (shortest round trip); the pixel classifier is skipped because no frame is given, so every classifier branch is the false branch",
    ]
    content = "\n".join(header + blocks).rstrip("\n") + "\n"
    data = content.encode("utf-8")
    assert len(data) <= 2_500_000, f"fixture is {len(data)} bytes, over 2.5 megabytes"
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_bytes(data)
    summary = " ".join(f"{name}={stats[name]}" for name in FLOORS)
    print(f"track_stationary: {summary} bytes={len(data)}")


main()
