"""Goldens for corvette_track::kalman::Kalman: the position, velocity and
covariance bits the real norfair 2.3.0 OptimizedKalmanFilter (norfair/filter.py
lines 148 to 227) produces under Frigate's tracker usage — predict, then update
with every coordinate measured — so a Rust port can be checked bit for bit.

Run command:

    cd /home/cvandesande/github/corvette && timeout 500 nix develop .#goldens --command python3 -I tools/goldens/track_kalman.py <extracted norfair directory>

The reference is the norfair 2.3.0 sdist norfair-2.3.0.tar.gz, sha256
8331c7fb2290fe92e9cd73877d43b0a5134fafd8846082002a9b545845d2a79c (PyPI);
extract it with `tar -xzf norfair-2.3.0.tar.gz` and pass the directory that
holds the norfair package. The script asserts the package tree hash
e8626859af72d20d25705e3563f2a6d3f7ae410476e4adaae106c0672bb0d5be — the recipe
`cd <dir>/norfair && find . -name '*.py' | LC_ALL=C sort | xargs sha256sum |
sha256sum`, recomputed here in Python with a plain byte-order sort — and the
version norfair reports.

norfair imports rich and filterpy, which the Kalman path never touches and the
shell does not carry, so a meta-path import hook gives any module named rich or
filterpy (and their submodules) a stub module whose attribute lookup returns a
dummy class (print for rich.print), and importlib.metadata.version('norfair')
is patched to '2.3.0' — both only so that `import norfair` succeeds.

The shell's numpy (2.x, version recorded in the fixture header) is newer than
the numpy 1.x pin norfair declares; every operation on this path is an
elementwise IEEE double add, multiply or divide, so the bits do not depend on
it. Sequences whose reference state or final covariance becomes non-finite are
excluded and counted as nonfinite_excluded (expected 0).

Fixture crates/corvette-track/tests/fixtures/kalman_cases.txt, # header lines
first (sources, seed, counts, floors), one case per line after them:

    kalman <n> r <hex16> q <hex16> init <h> <h> <h> <h> ops <op> <op> ... => <state> <state> ... cov <h>,<h>...(12)

<hex16>/<h> is a double's big-endian bits as 16 hex digits. An <op> is P or
U:<h>,<h>,<h>,<h> (the measurement's four doubles, order x1 y1 x2 y2); a
<state> is eight hex16 comma-joined (position x1 y1 x2 y2 then velocity) and
is recorded after every op; the cov token carries pos_variance four,
pos_vel_covariance four and vel_variance four after the last op. init holds
the four creation doubles. Output is deterministic.
"""

import hashlib
import importlib.abc
import importlib.machinery
import importlib.metadata
import math
import struct
import sys
import types
from pathlib import Path

if len(sys.argv) != 2:
    sys.exit("usage: track_kalman.py <extracted norfair directory (the one holding norfair/)>")

SRC = Path(sys.argv[1]).resolve()
NORFAIR_PACKAGE = SRC / "norfair"
FIXTURE = Path(__file__).resolve().parents[2] / "crates/corvette-track/tests/fixtures/kalman_cases.txt"
SDIST_SHA256 = "8331c7fb2290fe92e9cd73877d43b0a5134fafd8846082002a9b545845d2a79c"
TREE_SHA256 = "e8626859af72d20d25705e3563f2a6d3f7ae410476e4adaae106c0672bb0d5be"
SEED = 20261118
FRAME_W = 704
FRAME_H = 576
NAMED_PAIRS = [(3.4, 0.1), (3.4, 0.03), (2.5, 0.05), (4.5, 0.25), (4.0, 0.2), (4.0, 0.1)]
NAMED_NAMES = ["named_a", "named_b", "named_c", "named_d", "named_e", "named_f"]


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
from norfair.filter import OptimizedKalmanFilterFactory

assert norfair.__version__ == "2.3.0", norfair.__version__
H = np.hstack([np.identity(4), np.zeros((4, 4))])


class Lcg:
    def __init__(self, seed):
        self.state = seed

    def randint(self, low, high):
        self.state = (self.state * 6364136223846793005 + 1442695040888963407) & 0xFFFFFFFFFFFFFFFF
        return low + (self.state >> 33) % (high - low + 1)


rng = Lcg(SEED)


def rint(low, high):
    return rng.randint(low, high)


def hx(value):
    return struct.pack(">d", float(value)).hex()


def is_int_value(value):
    number = float(value)
    return math.isfinite(number) and number == math.trunc(number)


def box_array(values, as_int):
    dtype = np.int64 if as_int else np.float64
    return np.array([[values[0], values[1]], [values[2], values[3]]], dtype=dtype)


def run_reference(r, q, init, as_int, ops):
    kalman = OptimizedKalmanFilterFactory(R=r, Q=q).create_filter(box_array(init, as_int))
    states = []
    for op in ops:
        if op[0] == "P":
            kalman.predict()
        else:
            kalman.update(np.expand_dims(box_array(op[1], as_int).flatten(), 0).T, None, H)
        states.append([float(value) for value in kalman.x.flatten()])
    cov = [float(value) for value in list(kalman.pos_variance.flatten()) + list(kalman.pos_vel_covariance.flatten()) + list(kalman.vel_variance.flatten())]
    return states, cov


def draw_pair(case_index):
    if case_index % 3 != 0:
        return NAMED_PAIRS[(case_index // 3) % 6]
    return rint(50, 1000) / 100.0, rint(1, 100) / 100.0


def ops_with_frames(budget, drop_percent, meas):
    ops = [("P",)]
    frame = 0
    while len(ops) < budget:
        if frame == 0:
            ops.append(("U", meas(0)))
        else:
            ops.append(("P",))
            if len(ops) < budget and rint(0, 99) >= drop_percent:
                ops.append(("U", meas(frame)))
        frame += 1
    return ops


def draw_tracking(_case_index):
    width = rint(10, 300)
    height = rint(10, 300)
    x = rint(0, max(0, FRAME_W - width))
    y = rint(0, max(0, FRAME_H - height))
    vx = rint(-5, 5)
    vy = rint(-5, 5)
    init = [x, y, x + width, y + height]

    def meas(frame):
        return [
            x + vx * frame + rint(-3, 3),
            y + vy * frame + rint(-3, 3),
            x + width + vx * frame + rint(-3, 3),
            y + height + vy * frame + rint(-3, 3),
        ]

    ops = ops_with_frames(rint(1, 20), rint(0, 50), meas)
    return init, True, ops


def draw_jump(_case_index):
    width = rint(10, 300)
    height = rint(10, 300)
    x = rint(0, max(0, FRAME_W - width))
    y = rint(0, max(0, FRAME_H - height))
    init = [x, y, x + width, y + height]
    offsets = [(0, 0, 0, 0)]
    for _ in range(19):
        dx = rint(-300, 300)
        dy = rint(-300, 300)
        dw = rint(-100, 100)
        dh = rint(-100, 100)
        px, py, pz, pw = offsets[-1]
        offsets.append((px + dx, py + dy, pz + dx + dw, pw + dy + dh))

    def meas(frame):
        ox, oy, oz, ow = offsets[frame]
        return [x + ox, y + oy, x + width + oz, y + height + ow]

    ops = ops_with_frames(rint(1, 20), rint(0, 50), meas)
    return init, True, ops


def draw_gap(_case_index):
    width = rint(10, 300)
    height = rint(10, 300)
    x = rint(0, max(0, FRAME_W - width))
    y = rint(0, max(0, FRAME_H - height))
    init = [x, y, x + width, y + height]
    ops = []
    cx = x
    cy = y
    for _ in range(rint(1, 4)):
        cx += rint(-20, 20)
        cy += rint(-20, 20)
        ops.append(("U", [cx, cy, cx + width, cy + height]))
    for _ in range(rint(6, 16)):
        ops.append(("P",))
    return init, True, ops


def big_corner():
    value = rint(2**20, 2**24)
    return -value if rint(0, 1) else value


def extreme_value(near_zero):
    if near_zero:
        return rint(-8, 8)
    return rint(-(2**24), 2**24)


def draw_extreme(case_index):
    near_zero = case_index % 2 == 1
    init = [big_corner()] + [extreme_value(near_zero) for _ in range(3)]

    def meas(_frame):
        values = [extreme_value(near_zero) for _ in range(4)]
        if rint(0, 2) == 0:
            values[rint(0, 3)] = big_corner()
        return values

    ops = ops_with_frames(rint(1, 20), rint(0, 50), meas)
    return init, True, ops


def float_value():
    bucket = rint(0, 9)
    if bucket == 0:
        value = rint(1, 10) / 1000000.0
    elif bucket == 1:
        value = float(rint(1, 10000000))
    else:
        value = rint(1, 9999999) / [10.0, 100.0, 1000.0][rint(0, 2)]
    return -value if rint(0, 1) else value


def draw_float(case_index):
    init = [float_value() for _ in range(4)]
    budget = rint(1, 17) if case_index % 4 == 0 else rint(1, 20)

    def meas(_frame):
        return [float_value() for _ in range(4)]

    ops = ops_with_frames(budget, rint(0, 50), meas)
    if case_index % 5 == 0 and ops[0][0] == "P" and len(ops) >= 2 and ops[1][0] == "U":
        ops = ops[1:]
    if case_index % 7 == 0 and len(ops) < 20 and len(ops) >= 2 and ops[1][0] == "U":
        ops.insert(2, ("U", meas(0)))
    if case_index % 4 == 0:
        for _ in range(3):
            if len(ops) < 20:
                ops.append(("P",))
    values = list(init) + [value for op in ops if op[0] == "U" for value in op[1]]
    if all(is_int_value(value) for value in values):
        init[0] += 0.5
    return init, False, ops


def build_line(n, r, q, init, ops, states, cov):
    ops_text = " ".join(
        "P" if op[0] == "P" else "U:" + ",".join(hx(value) for value in op[1]) for op in ops
    )
    states_text = " ".join(",".join(hx(value) for value in state) for state in states)
    cov_text = ",".join(hx(value) for value in cov)
    init_text = " ".join(hx(value) for value in init)
    return f"kalman {n} r {hx(r)} q {hx(q)} init {init_text} ops {ops_text} => {states_text} cov {cov_text}"


def longest_predict_run(ops):
    best = 0
    run = 0
    for op in ops:
        if op[0] == "P":
            run += 1
            best = max(best, run)
        else:
            run = 0
    return best


def main():
    lines = []
    stats = {
        "tracking": 0,
        "jumps": 0,
        "gaps": 0,
        "extreme": 0,
        "floats": 0,
        "both_pu": 0,
        "run3p": 0,
        "noninteger": 0,
        "bigint": 0,
        "nonfinite_excluded": 0,
    }
    pair_counts = [0] * len(NAMED_PAIRS)
    named_keys = {(hx(r), hx(q)): name for (r, q), name in zip(NAMED_PAIRS, NAMED_NAMES)}
    drawers = [("tracking", 350, draw_tracking), ("jumps", 250, draw_jump), ("gaps", 250, draw_gap), ("extreme", 150, draw_extreme), ("floats", 200, draw_float)]
    case_index = 0
    for kind, target, drawer in drawers:
        written = 0
        attempts = 0
        while written < target:
            attempts += 1
            assert attempts <= target * 40, f"the {kind} draw loop did not settle"
            r, q = draw_pair(case_index)
            case_index += 1
            init, as_int, ops = drawer(case_index)
            states, cov = run_reference(r, q, init, as_int, ops)
            if any(not math.isfinite(value) for state in states for value in state) or any(
                not math.isfinite(value) for value in cov
            ):
                stats["nonfinite_excluded"] += 1
                continue
            corner_values = list(init) + [value for op in ops if op[0] == "U" for value in op[1]]
            if any(op[0] == "P" for op in ops) and any(op[0] == "U" for op in ops):
                stats["both_pu"] += 1
            if longest_predict_run(ops) >= 3:
                stats["run3p"] += 1
            if any(not is_int_value(value) for value in corner_values):
                stats["noninteger"] += 1
            if any(is_int_value(value) and abs(float(value)) >= 2**20 for value in corner_values):
                stats["bigint"] += 1
            named = named_keys.get((hx(r), hx(q)))
            if named is not None:
                pair_counts[NAMED_NAMES.index(named)] += 1
            lines.append(build_line(len(lines) + 1, r, q, init, ops, states, cov))
            stats[kind] += 1
            written += 1
    assert len(lines) == 1200, len(lines)
    assert stats["tracking"] == 350 and stats["jumps"] == 250 and stats["gaps"] == 250
    assert stats["extreme"] == 150 and stats["floats"] == 200
    assert stats["both_pu"] >= 1000, stats["both_pu"]
    assert stats["run3p"] >= 300, stats["run3p"]
    assert stats["noninteger"] >= 200, stats["noninteger"]
    assert stats["bigint"] >= 150, stats["bigint"]
    assert all(count >= 50 for count in pair_counts), pair_counts
    pair_text = " ".join(f"{name} {count}" for name, count in zip(NAMED_NAMES, pair_counts))
    header = [
        "# kalman goldens: the state and covariance bits norfair 2.3.0 OptimizedKalmanFilter (norfair/filter.py 148-227) produces, driven as Frigate's tracker drives it: predict, then update(measurement, None, H) with H the identity(4) over zeros((4, 4)) block",
        f"# source norfair-2.3.0 sdist sha256 {SDIST_SHA256} (PyPI), package tree hash {TREE_SHA256} (find . -name '*.py' | LC_ALL=C sort | xargs sha256sum | sha256sum under norfair/)",
        f"# numpy {np.__version__} (goldens shell; norfair declares numpy 1.x; every operation on this path is an elementwise IEEE double add, multiply or divide, so the bits do not depend on it)",
        f"# seed {SEED}",
        f"# counts cases {len(lines)} tracking 350 jumps 250 gaps 250 extreme 150 floats 200 both_pu {stats['both_pu']} run3p {stats['run3p']} noninteger {stats['noninteger']} bigint {stats['bigint']} {pair_text} nonfinite_excluded {stats['nonfinite_excluded']}",
        "# floors both_pu 1000 run3p 300 noninteger 200 bigint 150 named_each 50",
    ]
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text("\n".join(header + lines) + "\n", encoding="utf-8")
    summary = " ".join(f"{name}={stats[name]}" for name in sorted(stats))
    pairs = " ".join(f"{name}={count}" for name, count in zip(NAMED_NAMES, pair_counts))
    print(f"track_kalman: {len(lines)} cases; {pairs}; {summary}")


if __name__ == "__main__":
    main()
