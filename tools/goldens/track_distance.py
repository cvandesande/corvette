"""Goldens for corvette_track::distance: the float64 values of Frigate's own
distance function (frigate/track/norfair_tracker.py lines 41 to 71 at v0.17.2,
executed unchanged through frigate_distance) and the float32 cells norfair
2.3.0 ScalarDistance.get_distances builds from them, label gate included, plus
float64-to-float32 cast cells as numpy's assignment produces them.

Run command:

    cd /home/cvandesande/github/corvette && timeout 500 nix develop .#goldens --command python3 -I tools/goldens/track_distance.py /home/cvandesande/github/frigate <extracted norfair directory>

The Frigate text is blob-checked: sha1 of the git blob header plus the file
bytes must equal 84a0f390a3742ab6dc5a339a1b54480570280110, and the executed
slice lines[40:75] is asserted to start with the def distance line and the def
frigate_distance line. The reference norfair is the 2.3.0 sdist
norfair-2.3.0.tar.gz, sha256
8331c7fb2290fe92e9cd73877d43b0a5134fafd8846082002a9b545845d2a79c (PyPI);
extract it with `tar -xzf norfair-2.3.0.tar.gz` and pass the directory that
holds the norfair package. The script asserts the package tree hash
e8626859af72d20d25705e3563f2a6d3f7ae410476e4adaae106c0672bb0d5be — the recipe
`cd <dir>/norfair && find . -name '*.py' | LC_ALL=C sort | xargs sha256sum |
sha256sum`, recomputed here in Python with a plain byte-order sort — and the
version norfair reports.

norfair imports rich and filterpy, which the distance path never touches and
the shell does not carry, so a meta-path import hook gives any module named rich
or filterpy (and their submodules) a stub module whose attribute lookup returns
a dummy class (print for rich.print), and importlib.metadata.version('norfair')
is patched to '2.3.0' — both only so that `import norfair` succeeds.

The shell's numpy (2.x, version recorded in the fixture header) is newer than
the numpy 1.x pin norfair declares; the bits depend on it only through
np.linalg.norm's dot, whose summation order this build records in the header.
Division-by-zero warnings are expected and suppressed.

Fixture crates/corvette-track/tests/fixtures/distance_cases.txt, # header lines
first (sources, numpy and BLAS, the norm note, the seed, counts, floors), one
case per line after them:

    dist <n> det <x1>,<y1>,<x2>,<y2> est <h16>,<h16>,<h16>,<h16> => <h16|nan> <h8|nan>
    matrix <n> cands <label>:<x1>,<y1>,<x2>,<y2> ... objs <label>:<h16>,... ... => <rows>x<cols> <h8|nan> ...
    cast <n> <h16> => <h8|nan>

Integers are decimal, doubles and floats are big-endian hex of their bits; a NaN
result is the word nan, an input NaN keeps its bits. A dist result is the f64
then the f32 of the one-cell table; matrix cells are row-major (one row per
candidate, one column per object) and an empty list is written -; a cast line's
expected value is the cell of np.full((1, 1), inf, float32) after m[0, 0] = x.
Labels are lowercase letters. Output is deterministic.
"""

import hashlib
import importlib.abc
import importlib.machinery
import importlib.metadata
import math
import struct
import subprocess
import sys
import types
import warnings
from pathlib import Path

if len(sys.argv) != 3:
    sys.exit("usage: track_distance.py <Frigate checkout directory> <extracted norfair directory (the one holding norfair/)>")

FRIGATE_DIR = sys.argv[1]
SRC = Path(sys.argv[2]).resolve()
NORFAIR_PACKAGE = SRC / "norfair"
FIXTURE = Path(__file__).resolve().parents[2] / "crates/corvette-track/tests/fixtures/distance_cases.txt"
FRIGATE_FILE = "frigate/track/norfair_tracker.py"
FRIGATE_BLOB = "84a0f390a3742ab6dc5a339a1b54480570280110"
SDIST_SHA256 = "8331c7fb2290fe92e9cd73877d43b0a5134fafd8846082002a9b545845d2a79c"
TREE_SHA256 = "e8626859af72d20d25705e3563f2a6d3f7ae410476e4adaae106c0672bb0d5be"
SEED = 20261125
FRAME_W = 704
FRAME_H = 576
LABELS = ["person", "car", "dog", "a", "b"]
NAN = float("nan")
INF = float("inf")
SUBNORMAL = 2.0 ** -149
OVERFLOW_TIE_BITS = int("47effffff0000000", 16)
F32_MAX_BITS = int("47efffffe0000000", 16)
MIN_F32_NORMAL_BITS = int("3810000000000000", 16)
HAND_CAST_BITS = [
    "3ff0000000000000", "3fb999999999999a", "4170000010000000", "4170000030000000",
    "47effffff0000000", "47efffffefffffff", "47efffffe0000000", "366244ce242c5561",
    "3690000000000000", "3690000000000001", "3696d601ad376ab9", "0000000000000001",
    "3810000000000000", "380fffffbb1dd6a1", "4004000000000000", "48078287f49c4a1d",
    "7ff0000000000000",
]


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

np.seterr(all="ignore")
from norfair.distances import ScalarDistance
from norfair.tracker import Detection, TrackedObject

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
NAMESPACE = {"np": np, "Detection": Detection, "TrackedObject": TrackedObject}
exec(compile("\n".join(LINES[40:75]), FRIGATE_FILE, "exec"), NAMESPACE)
frigate_distance = NAMESPACE["frigate_distance"]
SCALAR = ScalarDistance(frigate_distance)


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


def hx32(value):
    return "nan" if math.isnan(value) else struct.pack(">f", float(value)).hex()


def value64(value):
    return "nan" if math.isnan(value) else hx64(value)


def f64_from_bits(bits):
    return struct.unpack(">d", struct.pack(">Q", bits))[0]


def bits64(value):
    return struct.unpack(">Q", struct.pack(">d", float(value)))[0]


def as_candidate(label, box):
    return Detection(points=np.array([[box[0], box[1]], [box[2], box[3]]]), label=label)


def as_object(label, estimate):
    return types.SimpleNamespace(label=label, estimate=np.array([[estimate[0], estimate[1]], [estimate[2], estimate[3]]], dtype=float))


def cast_cell(value):
    cell = np.full((1, 1), np.inf, dtype=np.float32)
    cell[0, 0] = value
    return float(cell[0, 0])


def draw_tracking():
    width = rint(10, 300)
    height = rint(10, 300)
    x = rint(0, FRAME_W - width)
    y = rint(0, FRAME_H - height)
    det = [x, y, x + width, y + height]
    estimate = [det[index] + rint(-600, 600) / 100.0 for index in range(4)]
    return det, estimate


def draw_far():
    x1 = rint(-1000, 2000)
    y1 = rint(-1000, 2000)
    det = [x1, y1, x1 + rint(-200, 400), y1 + rint(-200, 400)]
    estimate = [rint(-200000, 200000) / 50.0 for _ in range(4)]
    return det, estimate


def draw_exact(index):
    width = rint(10, 300)
    height = rint(10, 300)
    x = rint(0, FRAME_W - width)
    y = rint(0, FRAME_H - height)
    mode = index % 3
    if mode == 2:
        det = [x, y, x + width, y]
        estimate = [float(value) for value in det]
        if index % 4 == 2:
            shift = rint(1, 40)
            estimate = [det[0] + shift, float(det[1]), det[2] + shift, float(det[3])]
        return det, estimate
    det = [x, y, x + width, y + height]
    if mode == 0 or (mode == 1 and index % 5 == 1):
        return det, [float(value) for value in det]
    shift = [rint(-40, 40) for _ in range(4)]
    return det, [det[index2] + shift[index2] for index2 in range(4)]


def log_dim(power_low, power_high):
    return (10.0 ** rint(power_low, power_high)) * (1.0 + rint(0, 999) / 1000.0)


def draw_aspect():
    width = rint(1, 2000)
    height = rint(1, 2000)
    x0 = rint(-1000, 1000) / 10.0
    y0 = rint(-1000, 1000) / 10.0
    det = [0, 0, width, height]
    estimate = [x0, y0, x0 + log_dim(-3, 6), y0 + log_dim(-3, 6)]
    return det, estimate


def draw_degenerate(index):
    base = float(rint(-100, 100))
    mode = index % 10
    if mode < 5:
        det = [int(base), int(base) - 4, int(base), int(base) + 4]
        estimate = [base, base - 4.0, base, base + 4.0]
        return det, estimate
    if mode < 8:
        det_x = int(base) + rint(1, 500)
        det = [det_x, int(base), det_x + rint(1, 300), int(base) + rint(1, 300)]
        estimate = [base, base, base, base + 20.0]
        return det, estimate
    det = [int(base), int(base) + 30, int(base) + 50, int(base)]
    estimate = [base + 7.0, base + 40.0, base, base + 5.0]
    return det, estimate


def draw_large(index):
    mode = index % 4
    if mode == 0:
        x1 = rint(-2147483647, 2147483647)
        y1 = rint(-2147483647, 2147483647)
        det = [x1, y1, x1 + rint(0, 1000), y1 + rint(0, 1000)]
        estimate = [rint(-10 ** 12, 10 ** 12) / 100.0 for _ in range(4)]
        return det, estimate
    if mode == 3:
        x1 = rint(-3000, 3000)
        y1 = rint(-3000, 3000)
        det = [x1, y1, x1 + rint(1, 200), y1 + rint(1, 200)]
        estimate = [rint(-10 ** 12, 10 ** 12) / 100.0 for _ in range(4)]
        return det, estimate
    det = [rint(1, 1000), rint(1, 1000), rint(1001, 2000), rint(1001, 2000)]
    width = 10.0 ** -rint(160, 300) if mode == 1 else 10.0 ** -rint(40, 75)
    estimate = [0.0, 0.0, width, width * rint(1, 5)]
    return det, estimate


NONFINITE_VALUES = [NAN, INF, -INF]


def draw_nonfinite(index):
    estimate = [100.0, 100.0, 200.0, 200.0]
    det = [120, 150, 180, 190]
    if index % 2 == 0:
        slot = index % 12
        estimate[slot % 4] = NONFINITE_VALUES[slot % 3]
    else:
        slot = index % 54
        first = slot % 6
        pair = [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)][first]
        estimate[pair[0]] = NONFINITE_VALUES[(slot // 6) % 3]
        estimate[pair[1]] = NONFINITE_VALUES[(slot // 18) % 3]
    return det, estimate


def draw_dyadic():
    det = [rint(0, 700), rint(0, 570), rint(0, 700) + 1, rint(0, 570) + 1]
    if det[2] < det[0]:
        det[0], det[2] = det[2], det[0]
    if det[3] < det[1]:
        det[1], det[3] = det[3], det[1]
    estimate = [rint(-200000, 200000) / 64.0 for _ in range(4)]
    return det, estimate


DIST_GROUPS = [
    (900, lambda index: draw_tracking()),
    (600, lambda index: draw_far()),
    (300, lambda index: draw_exact(index)),
    (300, lambda index: draw_aspect()),
    (200, draw_degenerate),
    (200, draw_large),
    (150, draw_nonfinite),
    (350, lambda index: draw_dyadic()),
]


def make_dist_line(number, det, estimate):
    candidate = as_candidate("person", det)
    obj = as_object("person", estimate)
    f64 = frigate_distance(candidate, obj)
    f32 = float(SCALAR.get_distances([obj], [candidate])[0, 0])
    det_text = f"{det[0]},{det[1]},{det[2]},{det[3]}"
    est_text = ",".join(hx64(value) for value in estimate)
    return f"dist {number} det {det_text} est {est_text} => {value64(f64)} {hx32(f32)}", f64, f32, estimate


def draw_box():
    width = rint(10, 200)
    height = rint(10, 200)
    x = rint(0, FRAME_W - width)
    y = rint(0, FRAME_H - height)
    return [x, y, x + width, y + height]


def make_matrix_line(number):
    count_c = rint(1, 6)
    count_o = rint(1, 6)
    cands = []
    for _ in range(count_c):
        cands.append((LABELS[rint(0, 4)], draw_box()))
    objs = []
    for _ in range(count_o):
        label = LABELS[rint(0, 4)]
        source = cands[rint(0, count_c - 1)][1]
        estimate = [source[position] + rint(-600, 600) / 100.0 for position in range(4)]
        objs.append((label, estimate))
    if number % 5 == 0:
        nan_slot = rint(0, count_o - 1)
        objs[nan_slot] = (cands[rint(0, count_c - 1)][0], [NAN, 0.0, 100.0, 100.0])
    return number, cands, objs


def make_matrix_empty_line(number):
    count = rint(1, 4)
    entries = [(LABELS[rint(0, 4)], draw_box()) for _ in range(count)]
    if number % 2 == 0:
        return number, entries, []
    return number, [], entries


def write_matrix_parts(tokens):
    return " ".join(f"{label}:{','.join(str(value) for value in box)}" for label, box in tokens)


def write_matrix_objs(tokens):
    return " ".join(f"{label}:{','.join(hx64(value) for value in estimate)}" for label, estimate in tokens)


def tie_normal():
    sign = rint(0, 1) << 63
    exponent = rint(1, 254)
    mantissa = rint(0, 8388607)
    return sign | ((exponent + 896) << 52) | (mantissa << 29) | 0x10000000


def tie_subnormal():
    sign = rint(0, 1) << 63
    multiple = rint(1, 8388607)
    return bits64((multiple + 0.5) * SUBNORMAL) | sign


def cast_groups():
    bits = []
    for _ in range(300):
        sign = rint(0, 1) << 63
        exponent = rint(0, 2046)
        mantissa = (rint(0, 67108863) << 26) | rint(0, 67108863)
        bits.append(sign | (exponent << 52) | mantissa)
    for _ in range(110):
        bits.append(tie_normal())
    for _ in range(40):
        bits.append(tie_subnormal())
    assert len(bits) == 450, len(bits)
    near_max = [OVERFLOW_TIE_BITS + offset for offset in (-5, -4, -3, -2, -1, 0, 1, 2, 3)]
    near_max.extend((F32_MAX_BITS, F32_MAX_BITS + 1, F32_MAX_BITS - 1))
    for value in near_max:
        bits.extend((value, (1 << 63) | value))
    while len(bits) < 550:
        bits.append((rint(0, 1) << 63) | (OVERFLOW_TIE_BITS + rint(-8, 8)))
    for _ in range(70):
        bits.append(bits64(rint(1, 8388607) * SUBNORMAL) | (rint(0, 1) << 63))
    for _ in range(55):
        bits.append(tie_subnormal())
    for _ in range(25):
        bits.append(bits64(rint(1, 3) / 8.0 * SUBNORMAL) | (rint(0, 1) << 63))
    assert len(bits) == 700, len(bits)
    for _ in range(84):
        bits.append(rint(1, 4503599627370495))
    bits.extend((0, 1 << 63))
    for offset in (-3, -2, -1, 0, 1, 2, 3):
        bits.append(MIN_F32_NORMAL_BITS + offset)
        bits.append((1 << 63) | (MIN_F32_NORMAL_BITS + offset))
    assert len(bits) == 800, len(bits)
    for _ in range(40):
        bits.append(bits64(rint(0, 1000000) / 10000.0))
    for _ in range(40):
        bits.append(bits64(rint(0, 1000000) / 1000000.0))
    bits.extend(int(text, 16) for text in HAND_CAST_BITS)
    bits.extend((0x7FF0000000000000, 0xFFF0000000000000, 0x7FF8000000000000))
    return bits


def main():
    warnings.simplefilter("ignore")
    dist_lines = []
    dist_stats = {"dist_nan64": 0, "dist_inf64": 0, "dist_zero64": 0, "dist_finite64_inf32": 0, "dist_nonfinite_est": 0, "dist_bad_dim": 0}
    number = 0
    for target, drawer in DIST_GROUPS:
        for index in range(target):
            number += 1
            det, estimate = drawer(index)
            line, f64, f32, estimate = make_dist_line(number, det, estimate)
            dist_lines.append(line)
            det_width = float(det[2] - det[0])
            det_height = float(det[3] - det[1])
            est_width = estimate[2] - estimate[0]
            est_height = estimate[3] - estimate[1]
            if math.isnan(f64):
                dist_stats["dist_nan64"] += 1
            if math.isinf(f64):
                dist_stats["dist_inf64"] += 1
            if f64 == 0.0:
                dist_stats["dist_zero64"] += 1
            if math.isfinite(f64) and math.isinf(f32):
                dist_stats["dist_finite64_inf32"] += 1
            if any(not math.isfinite(value) for value in estimate):
                dist_stats["dist_nonfinite_est"] += 1
            if est_width <= 0.0 or est_height <= 0.0 or det_width <= 0.0 or det_height <= 0.0:
                dist_stats["dist_bad_dim"] += 1
    assert len(dist_lines) == 3000, len(dist_lines)

    matrix_lines = []
    gated_cells = 0
    empty_side_cases = 0
    nan_cells = 0
    for number in range(1, 601):
        index, cands, objs = make_matrix_line(number)
        table = SCALAR.get_distances([as_object(label, estimate) for label, estimate in objs], [as_candidate(label, box) for label, box in cands])
        gated_cells += sum(1 for label, _ in cands for own, _ in objs if own != label)
        nan_cells += int(np.count_nonzero(np.isnan(table)))
        cands_text = " ".join(f"{label}:{box[0]},{box[1]},{box[2]},{box[3]}" for label, box in cands)
        objs_text = write_matrix_objs(objs)
        cells = " ".join(hx32(float(value)) for value in table.flatten())
        matrix_lines.append(f"matrix {index} cands {cands_text} objs {objs_text} => {len(cands)}x{len(objs)} {cells}")
    for number in range(601, 701):
        index, cands, objs = make_matrix_empty_line(number)
        empty_side_cases += 1
        cands_text = write_matrix_parts(cands) if cands else "-"
        objs_text = write_matrix_objs(objs) if objs else "-"
        matrix_lines.append(f"matrix {index} cands {cands_text} objs {objs_text} => {len(cands)}x{len(objs)}")
    assert len(matrix_lines) == 700, len(matrix_lines)

    cast_lines = []
    cast_stats = {"cast_ties": 0, "cast_overflow": 0, "cast_subnormal": 0, "cast_underflow": 0}
    all_bits = cast_groups()
    assert len(all_bits) == 900, len(all_bits)
    for number, bits in enumerate(all_bits, start=1):
        value = f64_from_bits(bits)
        out = cast_cell(value)
        cast_lines.append(f"cast {number} {bits:016x} => {hx32(out)}")
        exponent = (bits >> 52) & 0x7FF
        out_bits = struct.unpack(">I", struct.pack(">f", out))[0]
        if 897 <= exponent <= 1150 and (bits & 0x1FFFFFFF) == 0x10000000:
            cast_stats["cast_ties"] += 1
        if exponent < 2047 and math.isinf(out):
            cast_stats["cast_overflow"] += 1
        if out_bits & 0x7FFFFFFF != 0 and (out_bits >> 23) & 0xFF == 0:
            cast_stats["cast_subnormal"] += 1
        if math.isfinite(value) and value != 0.0 and out == 0.0:
            cast_stats["cast_underflow"] += 1
    assert len(cast_lines) == 900, len(cast_lines)

    floors = {
        "dist_nan64": 100,
        "dist_inf64": 100,
        "dist_zero64": 100,
        "dist_finite64_inf32": 20,
        "dist_nonfinite_est": 100,
        "dist_bad_dim": 150,
        "matrix_gated_cells": 1000,
        "matrix_empty_side_cases": 50,
        "matrix_nan_cells": 20,
        "cast_ties": 100,
        "cast_overflow": 50,
        "cast_subnormal": 100,
        "cast_underflow": 30,
    }
    stats = dict(dist_stats)
    stats["matrix_gated_cells"] = gated_cells
    stats["matrix_empty_side_cases"] = empty_side_cases
    stats["matrix_nan_cells"] = nan_cells
    stats.update(cast_stats)
    for name, floor in floors.items():
        assert stats[name] >= floor, f"floor {name}: {stats[name]} < {floor}"

    config = np.show_config(mode="dicts")
    blas = config.get("Build Dependencies", {}).get("blas", {})
    blas_name = blas.get("name", "unknown")
    blas_version = blas.get("version", "unknown")
    counts = {"dist": 3000, "matrix": 700, "cast": 900}
    counts.update(stats)
    header = [
        "# distance goldens: the float64 values of Frigate's distance (norfair_tracker.py lines 41 to 71 at v0.17.2, run unchanged as frigate_distance), the float32 cells norfair 2.3.0 ScalarDistance.get_distances builds from them (label gate included), and float64-to-float32 cast cells",
        f"# source frigate v0.17.2 {FRIGATE_FILE} blob {FRIGATE_BLOB}, norfair-2.3.0 sdist sha256 {SDIST_SHA256} (PyPI), package tree hash {TREE_SHA256} (find . -name '*.py' | LC_ALL=C sort | xargs sha256sum | sha256sum under norfair/)",
        f"# numpy {np.__version__} blas {blas_name} {blas_version} (goldens shell; norfair declares numpy 1.x)",
        "# the f64 result is float(np.linalg.norm(change)): sqrt of numpy's dot of the change vector with itself, which sums the four squares strictly left to right on this build",
        f"# seed {SEED}",
        "# counts " + " ".join(f"{name} {value}" for name, value in counts.items()),
        "# floors " + " ".join(f"{name} {value}" for name, value in floors.items()),
    ]
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text("\n".join(header + dist_lines + matrix_lines + cast_lines) + "\n", encoding="utf-8")
    summary = " ".join(f"{name}={stats[name]}" for name in sorted(stats))
    print(f"track_distance: dist 3000 matrix 700 cast 900; {summary}")


if __name__ == "__main__":
    main()
