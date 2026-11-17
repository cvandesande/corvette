"""Goldens for corvette_track::match_greedy: the greedy pair selection norfair
2.3.0 Tracker.match_dets_and_objs (norfair/tracker.py lines 352 to 385) performs
and the split into matched pairs, unmatched candidates and unmatched objects that
Tracker._update_objects_in_place (lines 286 to 350) returns, for seeded float32
distance tables, so a Rust port can be checked pair for pair and index for index.

Run command:

    cd /home/cvandesande/github/corvette && timeout 500 nix develop .#goldens --command python3 -I tools/goldens/track_match.py <extracted norfair directory>

The reference is the norfair 2.3.0 sdist norfair-2.3.0.tar.gz, sha256
8331c7fb2290fe92e9cd73877d43b0a5134fafd8846082002a9b545845d2a79c (PyPI);
extract it with `tar -xzf norfair-2.3.0.tar.gz` and pass the directory that
holds the norfair package. The script asserts the package tree hash
e8626859af72d20d25705e3563f2a6d3f7ae410476e4adaae106c0672bb0d5be — the recipe
`cd <dir>/norfair && find . -name '*.py' | LC_ALL=C sort | xargs sha256sum |
sha256sum`, recomputed here in Python with a plain byte-order sort — and the
version norfair reports.

norfair imports rich and filterpy, which the matching path never touches and the
shell does not carry, so a meta-path import hook gives any module named rich or
filterpy (and their submodules) a stub module whose attribute lookup returns a
dummy class (print for rich.print), and importlib.metadata.version('norfair')
is patched to '2.3.0' — both only so that `import norfair` succeeds.

The shell's numpy (2.x, version recorded in the fixture header) is newer than
the numpy 1.x pin norfair declares; every comparison on this path is a float32
table cell against the float32 rounding of the threshold, so the results do not
depend on it. Frigate configures five trackers with the threshold values 2.5,
3.75, 2, 2.5 and 3 (frigate/track/norfair_tracker.py lines 128, 133, 145, 161
and 167 at v0.17.2) and does not override the matching; the two integers are
passed to norfair as Python ints, as Frigate does. Every draw runs under
np.errstate(all="ignore") and with warnings suppressed; NaN tables are expected
to raise ValueError from norfair and are recorded as matcherr lines.

Fixture crates/corvette-track/tests/fixtures/match_cases.txt, # header lines
first (sources, seed, numpy, the Frigate threshold note, counts, floors), one
case per line after them:

    match <n> thr <h16> table <rows>x<cols> <h8> <h8> ... => <pairs> ; <uc> ; <uo>
    matcherr <n> thr <h16> table <rows>x<cols> <h8> <h8> ... => error

A <h16> is the big-endian hex of the float64 threshold and an <h8> the
big-endian hex of a float32 cell (a NaN cell keeps its bits); a table with no
cells is written `table <rows>x<cols> -`; <pairs> is the space-separated
<candidate>,<object>,<h8 of the cell> triples in selection order, <uc> and <uo>
are the space-separated ascending unmatched candidate and object indices, each
of the three written - when empty. Rows are candidates, columns are objects.
Output is deterministic.
"""

import hashlib
import importlib.abc
import importlib.machinery
import importlib.metadata
import math
import struct
import sys
import types
import warnings
from pathlib import Path

if len(sys.argv) != 2:
    sys.exit("usage: track_match.py <extracted norfair directory (the one holding norfair/)>")

SRC = Path(sys.argv[1]).resolve()
NORFAIR_PACKAGE = SRC / "norfair"
FIXTURE = Path(__file__).resolve().parents[2] / "crates/corvette-track/tests/fixtures/match_cases.txt"
SDIST_SHA256 = "8331c7fb2290fe92e9cd73877d43b0a5134fafd8846082002a9b545845d2a79c"
TREE_SHA256 = "e8626859af72d20d25705e3563f2a6d3f7ae410476e4adaae106c0672bb0d5be"
SEED = 20261126
HUGE_THRESHOLD = 3.4028235677973366e38
FRIGATE_THRESHOLDS = [2.5, 3.75, 2, 2.5, 3]
NAN_BIT_PATTERNS = [0x7FC00000, 0xFFC00000, 0x7FC00001, 0x7F800001, 0xFF800001]


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
from norfair.tracker import Detection, Tracker


class Lcg:
    def __init__(self, seed):
        self.state = seed

    def randint(self, low, high):
        self.state = (self.state * 6364136223846793005 + 1442695040888963407) & 0xFFFFFFFFFFFFFFFF
        return low + (self.state >> 33) % (high - low + 1)


rng = Lcg(SEED)


def rint(low, high):
    return rng.randint(low, high)


def hx64(value):
    return struct.pack(">d", float(value)).hex()


def bits32(value):
    return struct.unpack(">I", struct.pack(">f", float(value)))[0]


def f32_hex(value):
    return f"{bits32(value):08x}"


def f32_from_bits(bits):
    return np.uint32(bits).view(np.float32)


def f64_from_bits(bits):
    return struct.unpack(">d", struct.pack(">Q", bits))[0]


def f32_uniform(high):
    mantissa = (rint(0, 8388607) << 26) | rint(0, 67108863)
    return np.float32(mantissa / 2**49 * high)


def frigate_threshold():
    return FRIGATE_THRESHOLDS[rint(0, 4)]


class _Obj:
    def __init__(self, index, hits):
        self.index = index
        self.last_distance = None
        self.hits = hits

    def hit(self, detection, period):
        self.hits.append((detection.data, self.index))


class _Table:
    def __init__(self, table):
        self.table = table

    def get_distances(self, objects, candidates):
        return self.table


TR = Tracker(distance_function=lambda a, b: 0.0, distance_threshold=2.5)
POINTS = np.array([[0, 0], [1, 1]])


def candidates_for(rows):
    return [Detection(points=POINTS, data=index, label="a") for index in range(rows)]


def run_match_line(number, rows, cols, cells, threshold):
    if rows == 0 or cols == 0:
        table = np.zeros((rows, cols), dtype=np.float32)
    else:
        table = np.array(cells, dtype=np.float32).reshape(rows, cols)
    hits = []
    objects = [_Obj(index, hits) for index in range(cols)]
    candidates = candidates_for(rows)
    unmatched_candidates, matched_objects, unmatched_objects = TR._update_objects_in_place(
        _Table(table), threshold, objects, candidates, 1
    )
    det_idxs, obj_idxs = TR.match_dets_and_objs(table, threshold)
    pair_order = [(int(det), int(obj)) for det, obj in zip(det_idxs, obj_idxs)]
    assert hits == pair_order, f"case {number}: hit order {hits} differs from {pair_order}"
    unmatched_cand_indices = [cand.data for cand in unmatched_candidates]
    unmatched_obj_indices = [obj.index for obj in unmatched_objects]
    assert unmatched_cand_indices == sorted(unmatched_cand_indices), f"case {number}"
    assert unmatched_obj_indices == sorted(unmatched_obj_indices), f"case {number}"
    for position, obj in enumerate(matched_objects):
        candidate, object_index = pair_order[position]
        assert bits32(obj.last_distance) == bits32(table[candidate, object_index]), f"case {number}"
    cells_text = " ".join(f32_hex(value) for value in table.reshape(-1)) if rows * cols else "-"
    pairs_text = " ".join(
        f"{candidate},{object_index},{f32_hex(table[candidate, object_index])}"
        for candidate, object_index in pair_order
    ) or "-"
    uc_text = " ".join(str(index) for index in unmatched_cand_indices) or "-"
    uo_text = " ".join(str(index) for index in unmatched_obj_indices) or "-"
    table_text = f"table {rows}x{cols} {cells_text}"
    line = f"match {number} thr {hx64(threshold)} {table_text} => {pairs_text} ; {uc_text} ; {uo_text}"
    return line, table, threshold


def run_matcherr_line(number, rows, cols, cells, threshold):
    table = np.array(cells, dtype=np.float32).reshape(rows, cols)
    nan_count = rint(1, min(4, rows * cols))
    positions = set()
    while len(positions) < nan_count:
        positions.add(rint(0, rows * cols - 1))
    for position in sorted(positions):
        table.reshape(-1)[position] = f32_from_bits(NAN_BIT_PATTERNS[rint(0, 4)])
    hits = []
    objects = [_Obj(index, hits) for index in range(cols)]
    raised = False
    try:
        TR._update_objects_in_place(_Table(table), threshold, objects, candidates_for(rows), 1)
    except ValueError as error:
        raised = True
        assert "nan values" in str(error), str(error)
    assert raised, f"case {number}: no ValueError for a table holding NaN cells"
    cells_text = " ".join(f32_hex(value) for value in table.reshape(-1))
    nan_bits = [bits32(value) for value in table.reshape(-1) if math.isnan(float(value))]
    line = f"matcherr {number} thr {hx64(threshold)} table {rows}x{cols} {cells_text} => error"
    return line, nan_bits


def draw_gate_cell():
    if rint(0, 99) < 35:
        return np.float32("inf")
    if rint(0, 3) == 0:
        return f32_uniform(1.0)
    return f32_uniform(8.0)


def draw_value_cells(values, count):
    return [values[rint(0, len(values) - 1)] for _ in range(count)]


TIE_VALUES = [np.float32(v) for v in (0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, float("inf"))]
TIE_VALUES.append(f32_from_bits(0x401FFFFF))

SIGN_VALUES = [
    np.float32(0.0), np.float32(-0.0), np.float32(1.0), np.float32(-1.0), np.float32(-2.0),
    np.float32("inf"), np.float32("-inf"), np.float32(1e-45), np.float32(-1e-45),
    np.float32(3.4028234663852886e38),
]
SIGN_THRESHOLDS = [
    0.0, -0.0, -1.5, 1e-45, -1e-45, float("inf"), float("-inf"), float("nan"),
    3.4028234663852886e38, HUGE_THRESHOLD, 1e300, 1.0,
]


def draw_sign_cell():
    index = rint(0, 10)
    if index == 10:
        return np.float32(rint(-1000000, 1000000) / 1000.0)
    return SIGN_VALUES[index]


def draw_random_f32():
    sign = rint(0, 1)
    exponent = rint(0, 254)
    mantissa = rint(0, 8388607)
    return f32_from_bits((sign << 31) | (exponent << 23) | mantissa)


def draw_random_f64():
    sign = rint(0, 1)
    exponent = rint(0, 2046)
    mantissa = (rint(0, 8388607) << 26) | rint(0, 67108863)
    return f64_from_bits((sign << 63) | (exponent << 52) | mantissa)


def generate_match_cases():
    lines = []
    stats = {
        "empty_side": 0, "pairs3": 0, "no_pair": 0, "both_unmatched": 0, "cell_at_threshold": 0,
        "first_pick_tie": 0, "threshold_edge": 0, "nan_threshold": 0, "inf_threshold": 0,
        "ninf_threshold": 0, "zero_threshold": 0, "negative_threshold": 0, "huge_threshold": 0,
        "signed_zero": 0, "inf_cells": 0, "ninf_cases": 0,
    }
    number = 0

    def take(rows, cols, cells, threshold):
        nonlocal number
        number += 1
        line, table, thr = run_match_line(number, rows, cols, cells, threshold)
        lines.append(line)
        count_stats(stats, table, thr, lines[-1])

    # 900 tracking rows, not 800: the brief's groups sum to 2900 while its total,
    # floors and completion condition all say 3000; the first group carries the extra 100.
    for _ in range(900):
        rows = rint(1, 8)
        cols = rint(1, 8)
        take(rows, cols, [draw_gate_cell() for _ in range(rows * cols)], frigate_threshold())
    for _ in range(600):
        rows = rint(1, 6)
        cols = rint(1, 6)
        take(rows, cols, draw_value_cells(TIE_VALUES, rows * cols), frigate_threshold())
    for index in range(400):
        rows = rint(1, 5)
        cols = rint(1, 5)
        if index % 3 == 0:
            mantissa = (rint(0, 8388607) << 26) | rint(0, 67108863)
            threshold = 0.5 + mantissa / 2**49 * 5.5
        elif index % 3 == 1:
            threshold = frigate_threshold()
        else:
            threshold = float(np.float32(0.5 + rint(0, 8388607) / 8388607 * 5.5))
        rounded = np.float32(float(threshold))
        below = np.nextafter(rounded, np.float32("-inf"))
        above = np.nextafter(rounded, np.float32("inf"))
        forced = [below, below, rounded, above]
        cells = [draw_gate_cell() for _ in range(rows * cols)]
        for position in range(min(4, rows * cols)):
            cells[position] = forced[position]
        take(rows, cols, cells, threshold)
    for index in range(300):
        rows = rint(1, 5)
        cols = rint(1, 5)
        take(rows, cols, [draw_sign_cell() for _ in range(rows * cols)], SIGN_THRESHOLDS[index % 12])
    for index in range(300):
        if index < 100:
            take(0, rint(1, 5), [], frigate_threshold())
        elif index < 200:
            take(rint(1, 5), 0, [], frigate_threshold())
        else:
            take(0, 0, [], frigate_threshold())
    for _ in range(150):
        rows = rint(9, 30)
        cols = rint(9, 30)
        take(rows, cols, [draw_gate_cell() for _ in range(rows * cols)], frigate_threshold())
    for index in range(350):
        rows = rint(1, 6)
        cols = rint(1, 6)
        cells = [draw_random_f32() for _ in range(rows * cols)]
        threshold = draw_random_f64()
        if index % 4 == 0:
            threshold = float(cells[rint(0, len(cells) - 1)])
        take(rows, cols, cells, threshold)
    assert len(lines) == 3000, len(lines)
    return lines, stats


def count_stats(stats, table, threshold, line):
    threshold = float(threshold)
    rows, cols = table.shape
    pairs_part, uc_part, uo_part = line.split("=>")[1].split(";")
    pairs = [] if pairs_part.split() == ["-"] else pairs_part.split()
    if rows == 0 or cols == 0:
        stats["empty_side"] += 1
    if len(pairs) >= 3:
        stats["pairs3"] += 1
    if rows > 0 and cols > 0 and not pairs:
        stats["no_pair"] += 1
    if uc_part.split() != ["-"] and uo_part.split() != ["-"]:
        stats["both_unmatched"] += 1
    limit = np.float32(threshold)
    flat = table.reshape(-1)
    if math.isfinite(threshold) and any(bits32(v) == bits32(limit) for v in flat):
        stats["cell_at_threshold"] += 1
    below = [v for v in flat if v < limit]
    if below and sum(1 for v in below if v == min(below)) >= 2:
        stats["first_pick_tie"] += 1
    if math.isfinite(threshold) and float(np.float32(threshold)) != threshold:
        edge_bits = {
            bits32(limit),
            bits32(np.nextafter(limit, np.float32("-inf"))),
            bits32(np.nextafter(limit, np.float32("inf"))),
        }
        if any(bits32(v) in edge_bits for v in flat):
            stats["threshold_edge"] += 1
    if math.isnan(threshold):
        stats["nan_threshold"] += 1
    if threshold == float("inf"):
        stats["inf_threshold"] += 1
    if threshold == float("-inf"):
        stats["ninf_threshold"] += 1
    if threshold == 0.0:
        stats["zero_threshold"] += 1
    if math.isfinite(threshold) and threshold < 0.0:
        stats["negative_threshold"] += 1
    if math.isfinite(threshold) and abs(threshold) >= HUGE_THRESHOLD:
        stats["huge_threshold"] += 1
    cell_bits = {bits32(v) for v in flat}
    if 0x00000000 in cell_bits and 0x80000000 in cell_bits:
        stats["signed_zero"] += 1
    stats["inf_cells"] += sum(1 for v in flat if bits32(v) == 0x7F800000)
    if any(bits32(v) == 0xFF800000 for v in flat):
        stats["ninf_cases"] += 1


def generate_matcherr_cases():
    lines = []
    stats = {"single_nan": 0, "odd_nan_bits": 0}
    for index in range(400):
        rows = rint(1, 8)
        cols = rint(1, 8)
        kind = index % 3
        if kind == 0:
            cells = [draw_gate_cell() for _ in range(rows * cols)]
            threshold = frigate_threshold()
        elif kind == 1:
            cells = draw_value_cells(TIE_VALUES, rows * cols)
            threshold = frigate_threshold()
        else:
            cells = [draw_sign_cell() for _ in range(rows * cols)]
            threshold = SIGN_THRESHOLDS[index % 12]
        line, nan_bits = run_matcherr_line(index + 1, rows, cols, cells, threshold)
        lines.append(line)
        if len(nan_bits) == 1:
            stats["single_nan"] += 1
        if any(bits != 0x7FC00000 for bits in nan_bits):
            stats["odd_nan_bits"] += 1
    assert len(lines) == 400, len(lines)
    return lines, stats


FLOORS = {
    "empty_side": 150, "pairs3": 500, "no_pair": 200, "both_unmatched": 300,
    "cell_at_threshold": 150, "first_pick_tie": 300, "threshold_edge": 100,
    "nan_threshold": 10, "inf_threshold": 10, "ninf_threshold": 10, "zero_threshold": 20,
    "negative_threshold": 40, "huge_threshold": 10, "signed_zero": 30, "inf_cells": 2000,
    "ninf_cases": 20, "single_nan": 100, "odd_nan_bits": 100,
}


def main():
    warnings.simplefilter("ignore")
    match_lines, match_stats = generate_match_cases()
    err_lines, err_stats = generate_matcherr_cases()
    stats = dict(match_stats)
    stats.update(err_stats)
    for name, floor in FLOORS.items():
        assert stats[name] >= floor, f"floor {name}: {stats[name]} < {floor}"
    counts = {"match": 3000, "matcherr": 400}
    counts.update(stats)
    header = [
        "# match goldens: the pair-selection order, float32 pair distance and unmatched indices norfair 2.3.0 Tracker._update_objects_in_place (norfair/tracker.py 286-350) and Tracker.match_dets_and_objs (352-385) produce for seeded float32 distance tables",
        f"# source norfair-2.3.0 sdist sha256 {SDIST_SHA256} (PyPI), package tree hash {TREE_SHA256} (find . -name '*.py' | LC_ALL=C sort | xargs sha256sum | sha256sum under norfair/), functions Tracker._update_objects_in_place (tracker.py 286-350) and Tracker.match_dets_and_objs (352-385)",
        f"# seed {SEED}",
        f"# numpy {np.__version__} (goldens shell; norfair declares numpy 1.x; every comparison on this path is a float32 cell against the float32 rounding of the threshold)",
        "# Frigate configures five trackers with the threshold values 2.5, 3.75, 2, 2.5, 3 (frigate/track/norfair_tracker.py lines 128, 133, 145, 161, 167 at v0.17.2) and does not override the matching; 2 and 3 are passed as ints, as Frigate does",
        "# counts " + " ".join(f"{name} {counts[name]}" for name in counts),
        "# floors " + " ".join(f"{name} {FLOORS[name]}" for name in FLOORS),
    ]
    content = "\n".join(header + match_lines + err_lines) + "\n"
    assert len(content.encode("utf-8")) <= 3_000_000, "fixture over three megabytes"
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text(content, encoding="utf-8")
    summary = " ".join(f"{name}={stats[name]}" for name in sorted(stats))
    print(f"track_match: match 3000 matcherr 400; {summary}")


with np.errstate(all="ignore"):
    main()
