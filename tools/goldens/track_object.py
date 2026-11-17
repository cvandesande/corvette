"""Goldens for corvette_track::Track: the full state the real norfair 2.3.0
TrackedObject and the id counting of _TrackedObjectFactory show after every
new, step and hit of 300 seeded scenarios of one to three objects sharing one
id source, so the Rust port can be checked field for field and operation for
operation.

Run command:

    cd /home/cvandesande/github/corvette && timeout 500 nix develop .#goldens --command python3 -I tools/goldens/track_object.py <extracted norfair directory>

The reference is the norfair 2.3.0 sdist norfair-2.3.0.tar.gz, sha256
8331c7fb2290fe92e9cd73877d43b0a5134fafd8846082002a9b545845d2a79c (PyPI);
extract it with `tar -xzf norfair-2.3.0.tar.gz` and pass the directory that
holds the norfair package. The script asserts the package tree hash
e8626859af72d20d25705e3563f2a6d3f7ae410476e4adaae106c0672bb0d5be — the recipe
`cd <dir>/norfair && find . -name '*.py' | LC_ALL=C sort | xargs sha256sum |
sha256sum`, recomputed here in Python with a plain byte-order sort — and the
version norfair reports.

norfair imports rich and filterpy, which the tracked-object path never touches
and the shell does not carry, so a meta-path import hook gives any module named
rich or filterpy (and their submodules) a stub module whose attribute lookup
returns a dummy class (print for rich.print), and importlib.metadata.version
('norfair') is patched to '2.3.0' — both only so that `import norfair` succeeds.

The shell's numpy (2.x, version recorded in the fixture header) is newer than
the numpy 1.x pin norfair declares; every recorded field comes from the real
norfair object running on this numpy.

Frigate builds its trackers with initialization_delay = detect.min_initialized
(2 at the default frame rate) and hit_counter_max = detect.max_disappeared
(25), never passes period (so it stays 1), and hands the filter per-tracker
(R, Q): (3.4, 0.03) cars, (2.5, 0.05) license plates, (3.4, 0.1) default,
(4.0, 0.2) default PTZ (frigate/track/norfair_tracker.py lines 126, 131, 159,
165 at v0.17.2). Each scenario drives TrackedObject(factory, detection,
hit_counter_max, initialization_delay, 4, 0, 1,
OptimizedKalmanFilterFactory(R=r, Q=q), past_detections_length, None, None)
directly; the extra arguments (point-counter maximum, threshold, period) keep
their defaults or fixed values and touch no recorded field.

Fixture crates/corvette-track/tests/fixtures/object_cases.txt, # header lines
first (sources, seed, numpy, the Frigate note, counts, floors), then 300
blocks:

    scenario <n> max <hit_counter_max> delay <delay> past <past> r <h16> q <h16>
    new <obj> <tag> <label> <x1> <y1> <x2> <y2> => <state>
    step <obj> => <state>
    hit <obj> <tag> <label> <x1> <y1> <x2> <y2> => <state>
    end

<obj> counts objects in creation order from 0; r and q are the big-endian hex
of the float64 bits; <state> is the object's label, hit_counter, age,
is_initializing (0 or 1), initializing_id, id (- when None),
hit_counter_is_positive (0 or 1), active (0 or 1), the last detection's tag,
the past list as tag@age joined with / (- when empty), then estimate (4 values)
and estimate_velocity (4 values) each as the big-endian hex of the float64
bits, all separated by single spaces. n counts scenarios from 1. Output is
deterministic.
"""

import hashlib
import importlib.abc
import importlib.machinery
import importlib.metadata
import struct
import sys
import types
import warnings
from pathlib import Path

if len(sys.argv) != 2:
    sys.exit("usage: track_object.py <extracted norfair directory (the one holding norfair/)>")

SRC = Path(sys.argv[1]).resolve()
NORFAIR_PACKAGE = SRC / "norfair"
FIXTURE = Path(__file__).resolve().parents[2] / "crates/corvette-track/tests/fixtures/object_cases.txt"
SDIST_SHA256 = "8331c7fb2290fe92e9cd73877d43b0a5134fafd8846082002a9b545845d2a79c"
TREE_SHA256 = "e8626859af72d20d25705e3563f2a6d3f7ae410476e4adaae106c0672bb0d5be"
SEED = 20261127
MAX_CHOICES = [1, 2, 3, 4, 5, 8, 10, 15, 25, 25, 25, 50]
PAST_CHOICES = [0, 0, 1, 1, 2, 3, 4, 4, 5, 5, 8]
FRIGATE_RQ = [(3.4, 0.03), (2.5, 0.05), (3.4, 0.1), (4.0, 0.2)]
LABELS = ["car", "person", "dog", "license_plate"]


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

assert norfair.__version__ == "2.3.0", norfair.__version__
from norfair.tracker import Detection, TrackedObject, _TrackedObjectFactory
from norfair.filter import OptimizedKalmanFilterFactory


class Lcg:
    def __init__(self, seed):
        self.state = seed

    def randint(self, low, high):
        self.state = (self.state * 6364136223846793005 + 1442695040888963407) & 0xFFFFFFFFFFFFFFFF
        return low + (self.state >> 33) % (high - low + 1)


rng = Lcg(SEED)
FLOORS = {
    "scenarios": 300,
    "operations": 6000,
    "obj1": 40, "obj2": 40, "obj3": 40,
    "delay0": 25, "delay1": 25, "delay2": 25, "delay3": 25,
    "past0": 25, "past1": 25,
    "uninit": 300, "capfull": 300, "negops": 300,
    "hitneg": 100, "hitneg1": 50,
    "pastreplace": 200, "pastsame": 200,
    "idoutoforder": 20, "labeldiff": 100,
}


def rint(low, high):
    return rng.randint(low, high)


def hx64(value):
    return struct.pack(">d", float(value)).hex()


def uniform_unit():
    bits = rint(0, 2**31 - 1) * 2**31 + rint(0, 2**31 - 1)
    return bits / 2**62


def detection(edges, tag, label):
    points = np.array([[edges[0], edges[1]], [edges[2], edges[3]]])
    return Detection(points=points, label=label, data={"tag": tag})


def new_object(factory, edges, tag, label, hit_max, delay, past_len, r, q):
    return TrackedObject(
        factory,
        detection(edges, tag, label),
        hit_max,
        delay,
        4,
        0,
        1,
        OptimizedKalmanFilterFactory(R=r, Q=q),
        past_len,
        None,
        None,
    )


def state(o):
    id_text = "-" if o.id is None else str(o.id)
    positive = int(o.hit_counter_is_positive)
    active = int(not o.is_initializing and o.hit_counter_is_positive)
    past = "/".join(f"{d.data['tag']}@{d.age}" for d in o.past_detections) or "-"
    fields = [
        o.label,
        str(o.hit_counter),
        str(o.age),
        str(int(o.is_initializing)),
        str(o.initializing_id),
        id_text,
        str(positive),
        str(active),
        str(o.last_detection.data["tag"]),
        past,
    ]
    fields += [hx64(v) for v in o.estimate.flatten()]
    fields += [hx64(v) for v in o.estimate_velocity.flatten()]
    return " ".join(fields)


def one_scenario(number, stats):
    stats["scenarios"] += 1
    factory = _TrackedObjectFactory()
    hit_max = MAX_CHOICES[rint(0, len(MAX_CHOICES) - 1)]
    delay_choices = list(range(0, min(hit_max - 1, 5) + 1))
    if 2 in delay_choices:
        delay_choices.append(2)
    delay = delay_choices[rint(0, len(delay_choices) - 1)]
    slow_start_fast_latecomer = 150 < number <= 250 and number % 3 == 1
    if slow_start_fast_latecomer:
        delay = rint(2, 3)
        hit_max = 25
    past_len = PAST_CHOICES[rint(0, len(PAST_CHOICES) - 1)]
    if rint(0, 1) == 0:
        r, q = FRIGATE_RQ[rint(0, len(FRIGATE_RQ) - 1)]
    else:
        r = 0.5 + uniform_unit() * 9.5
        q = 0.001 + uniform_unit() * 0.999
    if number <= 150:
        object_count = 1
    elif number <= 250:
        object_count = 2
    else:
        object_count = 3
    stats[f"obj{object_count}"] += 1
    if delay <= 3:
        stats[f"delay{delay}"] += 1
    if past_len == 0:
        stats["past0"] += 1
    if past_len == 1:
        stats["past1"] += 1
    target = rint(20, 60)
    if slow_start_fast_latecomer:
        target = rint(45, 75)
    arbitrary = rint(0, 19) == 0
    if slow_start_fast_latecomer:
        births = [0, rint(2, 8)]
    else:
        births = [0]
        for _ in range(1, object_count):
            if rint(0, 1) == 0:
                births.append(rint(1, 4))
            else:
                births.append(rint(1, max(2, target // 2)))

    def draw_prob(index):
        if slow_start_fast_latecomer:
            if index == 0:
                return rint(56, 64)
            return rint(92, 100)
        if rint(0, 11) == 0:
            return rint(1, 10)
        return rint(60, 100)

    probs = [draw_prob(index) for index in range(object_count)]
    label_indices = [rint(0, len(LABELS) - 1) for _ in range(object_count)]
    lines = [f"scenario {number} max {hit_max} delay {delay} past {past_len} r {hx64(r)} q {hx64(q)}"]
    boxes = []
    objects = []
    tag = [0]
    ops = [0]
    created = [0]

    def draw_box():
        if arbitrary:
            return [rint(-2000, 4000) for _ in range(4)]
        x1 = rint(0, 1799)
        y1 = rint(0, 999)
        width = rint(10, 299)
        height = rint(10, 299)
        return [x1, y1, x1 + width, y1 + height]

    def move_box(edges):
        if rint(0, 9) == 0:
            return [value + rint(-200, 200) for value in edges]
        return [value + rint(-12, 12) for value in edges]

    def note_counter(o):
        if o.hit_counter < 0:
            stats["negops"] += 1

    def do_new(index):
        edges = draw_box()
        boxes.append(edges)
        label = LABELS[label_indices[index]]
        o = new_object(factory, edges, tag[0], label, hit_max, delay, past_len, r, q)
        objects.append(o)
        lines.append(f"new {index} {tag[0]} {label} {edges[0]} {edges[1]} {edges[2]} {edges[3]} => {state(o)}")
        note_counter(o)
        tag[0] += 1
        ops[0] += 1
        created[0] += 1

    def do_step(index):
        objects[index].tracker_step()
        lines.append(f"step {index} => {state(objects[index])}")
        note_counter(objects[index])
        ops[0] += 1

    def do_hit(index):
        o = objects[index]
        own = label_indices[index]
        if rint(0, 9) < 8:
            label = LABELS[own]
        else:
            label = LABELS[(own + 1 + rint(0, len(LABELS) - 2)) % len(LABELS)]
        before_counter = o.hit_counter
        before_initializing = o.is_initializing
        full_before = past_len > 0 and len(o.past_detections) == past_len
        boxes[index] = move_box(boxes[index])
        edges = boxes[index]
        o.hit(detection(edges, tag[0], label))
        lines.append(f"hit {index} {tag[0]} {label} {edges[0]} {edges[1]} {edges[2]} {edges[3]} => {state(o)}")
        note_counter(o)
        if before_initializing and not o.is_initializing:
            stats["uninit"] += 1
        if before_counter + 2 > hit_max and o.hit_counter == hit_max:
            stats["capfull"] += 1
        if before_counter < 0:
            stats["hitneg"] += 1
            if before_counter == -1:
                stats["hitneg1"] += 1
        if full_before:
            kept = bool(o.past_detections) and o.past_detections[-1].data["tag"] == tag[0]
            stats["pastreplace" if kept else "pastsame"] += 1
        if label != o.label:
            stats["labeldiff"] += 1
        tag[0] += 1
        ops[0] += 1

    while ops[0] < target or created[0] < object_count:
        while created[0] < object_count and ops[0] >= births[created[0]]:
            do_new(created[0])
        if ops[0] >= target:
            break
        alive = created[0]
        for index in range(alive):
            if ops[0] >= target:
                break
            do_step(index)
        for index in range(alive):
            if ops[0] >= target:
                break
            if rint(0, 99) < probs[index]:
                do_hit(index)
                if rint(0, 9) == 0 and ops[0] < target:
                    do_hit(index)
        if rint(0, 19) == 0 and ops[0] < target:
            index = rint(0, created[0] - 1)
            if rint(0, 1) == 0:
                do_step(index)
            else:
                do_hit(index)
    ids = [o.id for o in objects]
    out_of_order = any(
        ids[late] is not None and ids[early] is not None and ids[late] < ids[early]
        for early in range(len(ids))
        for late in range(early + 1, len(ids))
    )
    if out_of_order:
        stats["idoutoforder"] += 1
    stats["operations"] += ops[0]
    lines.append("end")
    return lines


def main():
    warnings.simplefilter("ignore")
    stats = {name: 0 for name in FLOORS}
    scenario_lines = []
    for number in range(1, 301):
        scenario_lines.extend(one_scenario(number, stats))
    failing = [f"{name} {stats[name]} < {floor}" for name, floor in FLOORS.items() if stats[name] < floor]
    assert not failing, "floors not reached: " + "; ".join(failing)
    header = [
        "# track-object goldens: the full state norfair 2.3.0 TrackedObject and the _TrackedObjectFactory id counters show after every new, step and hit of 300 seeded scenarios of one to three objects sharing one id source",
        f"# source norfair-2.3.0 sdist sha256 {SDIST_SHA256} (PyPI), package tree hash {TREE_SHA256} (find . -name '*.py' | LC_ALL=C sort | xargs sha256sum | sha256sum under norfair/), classes TrackedObject (tracker.py 433-740) and _TrackedObjectFactory (388-431)",
        f"# seed {SEED}",
        f"# numpy {np.__version__} (goldens shell; norfair declares numpy 1.x; every recorded field comes from the real norfair object running on this numpy)",
        "# Frigate's trackers use initialization_delay 2 and hit_counter_max 25 at the default frame rate, never pass period so it stays 1, and use the (R, Q) pairs (3.4, 0.03), (2.5, 0.05), (3.4, 0.1), (4.0, 0.2) (norfair_tracker.py lines 126, 131, 159, 165 at v0.17.2)",
        "# counts " + " ".join(f"{name} {stats[name]}" for name in FLOORS),
        "# floors " + " ".join(f"{name} {FLOORS[name]}" for name in FLOORS),
    ]
    content = "\n".join(header + scenario_lines) + "\n"
    assert len(content.encode("utf-8")) <= 3_000_000, "fixture over three megabytes"
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text(content, encoding="utf-8")
    summary = " ".join(f"{name}={stats[name]}" for name in sorted(stats))
    print(f"track_object: scenarios 300 operations {stats['operations']}; {summary}")


with np.errstate(all="ignore"):
    main()
