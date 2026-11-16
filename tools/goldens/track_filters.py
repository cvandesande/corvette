"""Goldens for corvette-track's object filters: the first stage at which
Frigate's deployed is_object_filtered drops a candidate, found by ablation
and never by a restated rule.

Run command:

    cd /home/cvandesande/github/corvette && timeout 300 nix develop .#goldens --command python3 -I tools/goldens/track_filters.py <Frigate checkout directory>

Reference: the frigate repository, path frigate/util/object.py, tag v0.17.2,
blob b8f41e2c3214c22980f5227055aad348c9d6cd10, lines 216 to 267
(is_object_filtered); line 216 and line 267 are asserted and the range is
executed unchanged in one namespace.

Fixture crates/corvette-track/tests/fixtures/filter_cases.txt, one case per
line (floats written with Python's repr, which Rust's f64 parse reads back
exactly; scores are 32-bit values and Rust parses them as f32; no tabs):

    set <id> <tracked labels, comma separated>
    filter <set id> <label> <min_area> <max_area> <min_score> <min_ratio> <max_ratio> <mask|->
    case <set id> <label> <score> <x_min> <y_min> <x_max> <y_max> <area> <ratio> <stage>

The header lines come first. A mask is <width>x<height>:<row>/.../<row> with
digit 1 for an open cell and 0 for a masked one; it is 255 and 0 bytes in the
numpy uint8 array Frigate's create_mask makes. <stage> is None, NotTracked,
MinArea, MaxArea, MinScore, MinRatio, MaxRatio or Masked: for k in 1..6 the
function is re-run with the label's settings where every check after stage k
cannot fire (min_area 0, max_area 2**63, min_score and min_ratio -inf,
max_ratio inf, mask None), and the first k that drops the candidate names its
stage; not-tracked is the same function with an empty filter map. The first k
must disagree with nothing: the full call is asserted to return True exactly
when a stage was found. Set A is the hand table's settings; B is every filter
at its defaults with no mask; C gives person an open 1x1 mask and car a masked
1x1 mask, both with narrow area and ratio limits. Draws come from one
Lcg(SEED): 150 rounds of targeted cases, two per stage outcome and four for
None, each built
from a parameter range that forces that stage with margin, then random fill to
3,000 cases drawing areas, ratios and scores at every limit exactly, one below
and one above (areas), the next float either side (math.nextafter, ratios) and
the 32-bit neighbours of each min_score, and boxes from 0 to twice the mask
sizes so the clamps and the midpoint rule are hit.
"""

import hashlib
import math
import struct
import subprocess
import sys
import types
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parents[2]
FIXTURE = REPO / "crates/corvette-track/tests/fixtures/filter_cases.txt"
OBJECT_BLOB = "b8f41e2c3214c22980f5227055aad348c9d6cd10"
SEED = 20261121
TOTAL = 3000
FLOOR = 150
STAGES = ["NotTracked", "MinArea", "MaxArea", "MinScore", "MinRatio", "MaxRatio", "Masked"]


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


def extract(lines, namespace):
    assert lines[215] == "def is_object_filtered(obj, objects_to_track, object_filters):"
    assert lines[266] == "    return False"
    exec(compile("\n".join(lines[215:267]), "frigate", "exec"), namespace)


def f32(x):
    return struct.unpack("f", struct.pack("f", x))[0]


def settings(min_area=0, max_area=24000000, min_score=0.5, min_ratio=0.0, max_ratio=24000000.0, mask=None):
    return types.SimpleNamespace(min_area=min_area, max_area=max_area, min_score=min_score, min_ratio=min_ratio, max_ratio=max_ratio, mask=mask)


def make_mask(rows):
    return np.array([[255 if ch == "1" else 0 for ch in row] for row in rows], dtype=np.uint8)


def mask_text(arr):
    rows = ["".join("0" if v == 0 else "1" for v in row) for row in arr]
    return f"{len(arr[0])}x{len(arr)}:" + "/".join(rows)


def neutral(spec, k):
    kept = types.SimpleNamespace(**vars(spec))
    if k < 2:
        kept.max_area = 2**63
    if k < 3:
        kept.min_score = float("-inf")
    if k < 4:
        kept.min_ratio = float("-inf")
    if k < 5:
        kept.max_ratio = float("inf")
    if k < 6:
        kept.mask = None
    return kept


def first_stage(fn, obj, track, filters):
    if fn(obj, track, {}):
        return "NotTracked"
    label = obj[0]
    if label not in filters:
        return "None"
    for k in range(1, 7):
        stage_filters = dict(filters)
        stage_filters[label] = neutral(filters[label], k)
        if fn(obj, track, stage_filters):
            return STAGES[k]
    return "None"


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: track_filters.py <Frigate checkout directory>")
    namespace = {}
    extract(source_lines(sys.argv[1], "frigate/util/object.py", OBJECT_BLOB), namespace)
    fn = namespace["is_object_filtered"]
    sets = {
        "A": (["person", "car", "dog"], {
            "person": settings(400, 100000, 0.7, 0.5, 3.0, make_mask(["0111", "1111", "1111"])),
            "car": settings(mask=make_mask(["1111", "1111", "1110"])),
            "bird": settings(),
        }),
        "B": (["person", "car"], {
            "person": settings(), "car": settings(), "bird": settings(),
        }),
        "C": (["person", "car", "dog"], {
            "person": settings(50, 1000, 0.2, 0.25, 4.0, make_mask(["1"])),
            "car": settings(50, 1000, 0.2, 0.25, 4.0, make_mask(["0"])),
            "bird": settings(),
        }),
    }
    rng = Lcg(SEED)
    header = []
    for sid in sorted(sets):
        track, filters = sets[sid]
        header.append("set " + sid + " " + ",".join(track))
        for label in sorted(filters):
            spec = filters[label]
            m = "-" if spec.mask is None else mask_text(spec.mask)
            header.append(
                f"filter {sid} {label} {spec.min_area} {spec.max_area} "
                f"{repr(spec.min_score)} {repr(spec.min_ratio)} {repr(spec.max_ratio)} {m}"
            )

    cases = []

    def add(sid, label, score, box, area, ratio):
        track, filters = sets[sid]
        obj = (label, score, box, area, ratio, None)
        stage = first_stage(fn, obj, track, filters)
        assert fn(obj, track, filters) == (stage != "None"), (sid, obj, stage)
        cases.append(
            f"case {sid} {label} {repr(score)} {box[0]} {box[1]} {box[2]} {box[3]} {area} {repr(ratio)} {stage}"
        )

    def box():
        return (rng.randint(0, 8), rng.randint(0, 8), rng.randint(0, 8), rng.randint(0, 8))

    def loose_score():
        return f32(rng.randint(0, 1000) / 1000)

    def loose_area():
        return rng.randint(0, 1000000)

    def loose_ratio():
        return rng.randint(0, 20000) / 1000

    for _ in range(FLOOR):
        add("A", "dog", loose_score(), box(), loose_area(), loose_ratio())
        add("B", "person", f32(rng.randint(510, 1000) / 1000), box(), rng.randint(0, 200000), rng.randint(0, 1000000) / 1000)
        add("C", "dog", loose_score(), box(), loose_area(), loose_ratio())
        add("B", "car", f32(rng.randint(510, 1000) / 1000), box(), rng.randint(0, 200000), rng.randint(0, 1000000) / 1000)
        add("A", "cat", loose_score(), box(), loose_area(), loose_ratio())
        add("C", "bird", loose_score(), box(), loose_area(), loose_ratio())
        add("A", "person", loose_score(), box(), rng.randint(0, 399), loose_ratio())
        add("C", "car", loose_score(), box(), rng.randint(0, 49), loose_ratio())
        add("A", "person", loose_score(), box(), rng.randint(100001, 1000000), loose_ratio())
        add("C", "car", loose_score(), box(), rng.randint(1001, 5000), loose_ratio())
        add("A", "person", f32(rng.randint(1, 699) / 1000), box(), rng.randint(400, 100000), loose_ratio())
        add("C", "car", f32(rng.randint(1, 19) / 100), box(), rng.randint(50, 1000), loose_ratio())
        add("A", "person", f32(0.75), box(), rng.randint(400, 100000), rng.randint(0, 499) / 1000)
        add("C", "car", f32(0.5), box(), rng.randint(50, 1000), rng.randint(0, 249) / 1000)
        add("A", "person", f32(0.75), box(), rng.randint(400, 100000), rng.randint(3001, 9000) / 1000)
        add("C", "car", f32(0.5), box(), rng.randint(50, 1000), rng.randint(4100, 99000) / 1000)
        add("A", "person", f32(0.9), (0, rng.randint(0, 8), 1, 0), rng.randint(400, 100000), rng.randint(500, 3000) / 1000)
        add("C", "car", f32(0.5), box(), rng.randint(50, 1000), rng.randint(250, 4000) / 1000)

    counts = {}
    for line in cases:
        stage = line.rsplit(" ", 1)[1]
        counts[stage] = counts.get(stage, 0) + 1
    for stage in ["None"] + STAGES:
        assert counts.get(stage, 0) >= FLOOR, (stage, counts)

    def at_limit(limit, whole):
        offset = rng.randint(-1, 1)
        if whole:
            value = int(limit) + offset
            return value if value >= 0 else 0
        return limit + offset * math.ulp(float(limit)) if limit != 0 else float(offset)

    def near_score(min_score):
        pick = rng.randint(0, 3)
        if pick == 0:
            return f32(min_score)
        if pick == 1:
            return f32(math.nextafter(min_score, math.inf))
        if pick == 2:
            return f32(math.nextafter(min_score, -math.inf))
        return loose_score()

    while len(cases) < TOTAL:
        sid = ["A", "B", "C"][rng.randint(0, 2)]
        label = ["person", "car", "dog", "cat", "bird"][rng.randint(0, 4)]
        spec = sets[sid][1].get(label)
        if spec is not None and rng.randint(0, 3) == 0:
            limit = spec.min_area if rng.randint(0, 1) == 0 else spec.max_area
            area = at_limit(limit, True)
        else:
            area = loose_area()
        if spec is not None and rng.randint(0, 3) == 0:
            limit = spec.min_ratio if rng.randint(0, 1) == 0 else spec.max_ratio
            ratio = at_limit(limit, False)
        else:
            ratio = loose_ratio()
        score = near_score(spec.min_score) if spec is not None else loose_score()
        add(sid, label, score, box(), area, ratio)

    assert len(cases) == TOTAL, len(cases)
    counts = {}
    for line in cases:
        stage = line.rsplit(" ", 1)[1]
        counts[stage] = counts.get(stage, 0) + 1
    for stage in ["None"] + STAGES:
        assert counts.get(stage, 0) >= FLOOR, (stage, counts)
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text("\n".join(header + cases) + "\n", encoding="utf-8")
    summary = " ".join(f"{stage}={counts[stage]}" for stage in ["None"] + STAGES)
    print(f"track_filters: {len(cases)} cases; {summary}")


if __name__ == "__main__":
    main()
