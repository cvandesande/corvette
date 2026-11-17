"""Goldens for corvette_track::Scores: the score history, median score, top
score and confirmed flag of a followed object that Frigate v0.17.2's own score
lines leave after every step of 300 seeded scenarios of seen and predicted
frames. The reference is frigate/track/tracked_object.py at v0.17.2, git blob
a95221bbdff834edd8c0cb0d153b7ff92f3d06ca, blob-checked here; the executed text
is lines 115-125 (_is_false_positive and compute_score) plus lines 134-148 (the
first half of update) assembled into a class and run unchanged with
statistics.median in its namespace, exactly as the hand-row probe does. Start
values follow the constructor (false_positive True, top_score = computed_score
= 0.0) and norfair_tracker.py line 276: the history starts as the list of the
track's past detection scores, untrimmed.

Run command:

    cd /home/cvandesande/github/corvette && timeout 300 nix develop .#goldens --command python3 -I tools/goldens/track_score.py /home/cvandesande/github/frigate

Fixture crates/corvette-track/tests/fixtures/score_cases.txt: # header lines
first (title, source, seed, counts, floors, the float note), then 300 blocks:

    scenario <n> threshold <repr> initial <k> <repr>...
    step <seen|predicted> <repr or -> computed <repr> top <repr> confirmed <0|1> history <len> <repr>...
    end

n counts scenarios from 1; initial lists the starting history (nothing after
the count when it is empty); a step line is the object's state after that step:
the drawn score (a dash for a predicted frame), computed_score, top_score,
confirmed as 0 or 1, and the whole score_history in order. Floats are Python
repr, the shortest round trip; NaN, infinity and negative zero are never drawn.
Output is deterministic: two runs write byte-identical files.
"""

import hashlib
import math
import subprocess
import sys
from pathlib import Path
from statistics import median
from types import SimpleNamespace

if len(sys.argv) != 2:
    sys.exit("usage: track_score.py <Frigate checkout directory>")

FRIGATE_DIR = sys.argv[1]
FIXTURE = Path(__file__).resolve().parents[2] / "crates/corvette-track/tests/fixtures/score_cases.txt"
FRIGATE_FILE = "frigate/track/tracked_object.py"
FRIGATE_BLOB = "a95221bbdff834edd8c0cb0d153b7ff92f3d06ca"
SEED = 20261129
SCENARIOS = 300
THRESHOLD_CHOICES = [0.0, 0.3, 0.5, 0.6, 0.7, 0.7, 0.7, 0.75, 0.8, 0.9, 1.0, 1.5]
INITIAL_LENGTHS = [0, 1, 1, 2, 3, 4, 4, 5, 10, 12]
SEEN_PROBS = [1.0, 0.9, 0.7, 0.5, 0.3]
FLOORS = {
    "scenarios": 300,
    "steps": 6000,
    "predicted_steps": 1500,
    "full_history": 3000,
    "even_history": 2000,
    "odd_history": 500,
    "equal_threshold": 100,
    "ulp_below": 10,
    "flips": 120,
    "never_confirm": 30,
    "first_step_confirm": 20,
    "sticky_steps": 300,
    "top_above_computed": 1000,
    "initial_zero": 25,
    "initial_one": 25,
    "initial_four": 25,
    "initial_ten": 10,
    "initial_twelve": 10,
    "threshold_zero": 10,
    "threshold_above_one": 10,
}


def blob_hash(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


try:
    cmd = ["git", "-C", FRIGATE_DIR, "show", f"v0.17.2:{FRIGATE_FILE}"]
    RAW = subprocess.run(cmd, capture_output=True, check=True).stdout
except (OSError, subprocess.SubprocessError) as error:
    sys.exit(f"cannot read v0.17.2:{FRIGATE_FILE} from {FRIGATE_DIR}: {error}")
assert blob_hash(RAW) == FRIGATE_BLOB, "the Frigate source does not hash to the pinned blob"
LINES = RAW.decode("utf-8").splitlines()
assert LINES[114].startswith("    def _is_false_positive("), LINES[114]
assert LINES[122].startswith("    def compute_score("), LINES[122]
assert LINES[133].lstrip().startswith("# if the object is not in the current frame"), LINES[133]
assert LINES[147].strip() == "self.false_positive = self._is_false_positive()", LINES[147]

SOURCE = (
    "class Real:\n"
    + "\n".join(LINES[114:125])
    + "\n    def apply(self, current_frame_time, obj_data):\n"
    + "\n".join(LINES[133:148])
    + "\n"
)
NAMESPACE = {"median": median}
exec(compile(SOURCE, FRIGATE_FILE, "exec"), NAMESPACE)
Real = NAMESPACE["Real"]


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


def score_draw(threshold):
    roll = rng.uniform()
    if roll < 0.40:
        return rint(0, 100) / 100
    if roll < 0.55:
        if 0.0 <= threshold <= 1.0:
            return threshold
        return rint(0, 100) / 100
    if roll < 0.60:
        return math.nextafter(threshold, 0.0)
    if roll < 0.65:
        return math.nextafter(threshold, 1.0)
    if roll < 0.70:
        return 0.0
    if roll < 0.75:
        return 1.0
    return rng.uniform()


def fmt(values):
    return " ".join(repr(v) for v in values)


def make_state(threshold, initial):
    real = Real()
    real.score_history = list(initial)
    real.false_positive = True
    real.top_score = real.computed_score = 0.0
    real.obj_data = {"label": "person"}
    real.camera_config = SimpleNamespace(
        objects=SimpleNamespace(filters={"person": SimpleNamespace(threshold=threshold)})
    )
    return real


def bump(stats, name):
    stats[name] += 1


def step_stats(stats, real, threshold, ulp, kind, state):
    bump(stats, "steps")
    if kind == "predicted":
        bump(stats, "predicted_steps")
    length = len(real.score_history)
    if length == 10:
        bump(stats, "full_history")
    if length % 2 == 0:
        bump(stats, "even_history")
    else:
        bump(stats, "odd_history")
    if real.computed_score == threshold:
        bump(stats, "equal_threshold")
    if real.computed_score == ulp:
        bump(stats, "ulp_below")
    confirmed = not real.false_positive
    if confirmed and not state["confirmed"]:
        bump(stats, "flips")
    state["confirmed"] = confirmed
    if confirmed:
        state["ever"] = True
        if real.computed_score < threshold:
            bump(stats, "sticky_steps")
    if real.top_score > real.computed_score:
        bump(stats, "top_above_computed")
    return confirmed


def run_scenario(number, stats):
    if rng.randint(0, 5) == 0:
        threshold = rng.uniform()
    else:
        threshold = pick(THRESHOLD_CHOICES)
    initial_length = pick(INITIAL_LENGTHS)
    initial = [score_draw(threshold) for _ in range(initial_length)]
    seen_prob = pick(SEEN_PROBS)
    step_count = rint(10, 60)
    ulp = math.nextafter(threshold, 0.0)

    bump(stats, "scenarios")
    if threshold == 0.0:
        bump(stats, "threshold_zero")
    if threshold > 1.0:
        bump(stats, "threshold_above_one")
    bucket = {0: "initial_zero", 1: "initial_one", 4: "initial_four", 10: "initial_ten", 12: "initial_twelve"}
    if initial_length in bucket:
        bump(stats, bucket[initial_length])

    real = make_state(threshold, initial)
    state = {"confirmed": False, "ever": False}
    header = f"scenario {number} threshold {threshold!r} initial {initial_length}"
    if initial:
        header += " " + fmt(initial)
    lines = [header]
    for index in range(step_count):
        if rng.uniform() < seen_prob:
            score = score_draw(threshold)
            obj_data = {"frame_time": 100.0, "score": score}
            kind, shown = "seen", repr(score)
        else:
            obj_data = {"frame_time": 99.0, "score": 0.0}
            kind, shown = "predicted", "-"
        real.apply(100.0, obj_data)
        confirmed = step_stats(stats, real, threshold, ulp, kind, state)
        if index == 0 and confirmed:
            bump(stats, "first_step_confirm")
        lines.append(
            f"step {kind} {shown} computed {real.computed_score!r} top {real.top_score!r} "
            f"confirmed {int(confirmed)} history {len(real.score_history)} {fmt(real.score_history)}"
        )
    if not state["ever"]:
        bump(stats, "never_confirm")
    lines.append("end")
    return lines


def main():
    stats = {name: 0 for name in FLOORS}
    blocks = []
    for number in range(1, SCENARIOS + 1):
        blocks.extend(run_scenario(number, stats))
    failing = [f"floor {name}: {stats[name]} < {floor}" for name, floor in FLOORS.items() if stats[name] < floor]
    assert not failing, "floors not reached: " + "; ".join(failing)
    header = [
        "# score goldens: the score history, median score, top score and confirmed flag Frigate v0.17.2 leaves after every step of 300 seeded scenarios of seen and predicted frames, its own lines run unchanged",
        f"# source frigate/track/tracked_object.py at v0.17.2, blob {FRIGATE_BLOB}, lines 115-125 and 134-148 executed unchanged",
        f"# seed {SEED}",
        "# counts " + " ".join(f"{name} {stats[name]}" for name in FLOORS),
        "# floors " + " ".join(f"{name} {FLOORS[name]}" for name in FLOORS),
        "# floats use Python repr (shortest round trip); NaN, infinity and negative zero are not drawn",
    ]
    content = "\n".join(header + blocks).rstrip("\n") + "\n"
    data = content.encode("utf-8")
    assert len(data) <= 2_000_000, f"fixture is {len(data)} bytes, over two megabytes"
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_bytes(data)
    summary = " ".join(f"{name}={stats[name]}" for name in FLOORS)
    print(f"track_score: {summary} bytes={len(data)}")


main()
