"""Goldens for corvette-track's detection merge: the second pass of Frigate's
reduce_detections, plus its clipped, area and intersection helpers, found by
running the source text at v0.17.2 and never by a restated rule.

Run command:

    cd /home/cvandesande/github/corvette && timeout 300 nix develop .#goldens --command python3 -I tools/goldens/track_merge.py <Frigate checkout directory>

Blob-checked and executed unchanged in one namespace holding numpy, defaultdict
and a cv2 stand-in whose dnn.NMSBoxes(boxes, confidences, score_threshold,
nms_threshold) returns np.arange(len(boxes), dtype=np.int32), which keeps every
box in input order; with that stand-in reduce_detections equals Corvette's
consolidate: the first pass is the identity on grouping and order and the
fixture therefore covers the second pass and the helpers. The texts are
frigate/util/object.py blob b8f41e2c3214c22980f5227055aad348c9d6cd10 lines
495-591 (the nested get_consolidated_object_detections is lines 540-587),
frigate/util/image.py blob ea9fb0a0a7f166d7b145c6c3af9941de673aebe3 lines
701-716 (intersection), 719-720 (area) and 752-765 (clipped), and
frigate/const.py blob 41c24f08741e83a886250459dfb5bcb0cdee79a6 lines 46-54
(the label limits).

One expectation diverges from raw Frigate by Corvette decision: a pair whose
later box has an inclusive area of zero raises ZeroDivisionError inside
Frigate, where Corvette skips the pair; merge cases that raise are not written
and are counted as zerodiv_excluded.

Fixture crates/corvette-track/tests/fixtures/merge_cases.txt, # header lines
first (source blobs, the stand-in, the seed, the counts), one case per line:

    merge <n> frame <width> <height> dets <det> <det> ... => <i> <j> ...
    clipped <n> frame <width> <height> det <det> => true|false
    area <n> box x0,y0,x1,y1 => <integer>
    intersection <n> a x0,y0,x1,y1 b x0,y0,x1,y1 => none|x0,y0,x1,y1

A merge line's survivors are the surviving detections' indexes in the input
list, in output order, or - for none. <det> is
label:score_hex8:x0,y0,x1,y1:rx0,ry0,rx1,ry1 with score_hex8 the bits of the
float32 score; the frame is width then height (Frigate's frame_shape is
(height, width)); the detection's area and ratio are computed by this
generator exactly as detect computes them ((x1 - x0) * (y1 - y0) and
(x1 - x0) / max(1, y1 - y0)) and are not written. Output is deterministic.
"""

import hashlib
import struct
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parents[2]
FIXTURE = REPO / "crates/corvette-track/tests/fixtures/merge_cases.txt"
OBJECT_BLOB = "b8f41e2c3214c22980f5227055aad348c9d6cd10"
IMAGE_BLOB = "ea9fb0a0a7f166d7b145c6c3af9941de673aebe3"
CONST_BLOB = "41c24f08741e83a886250459dfb5bcb0cdee79a6"
SEED = 20261123
LABELS = ["person", "car", "face", "dog"]
FRAMES = [(704, 576), (640, 480), (1280, 720), (100, 60)]


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


def build_namespace(obj, img, const):
    assert obj[494] == "def reduce_detections("
    assert obj[590] == "    )"
    assert obj[539].startswith("    def get_consolidated_object_detections")
    assert img[700] == "def intersection(box_a, box_b) -> Optional[list[int]]:"
    assert img[715] == "    )"
    assert img[718] == "def area(box):"
    assert img[719] == "    return (box[2] - box[0] + 1) * (box[3] - box[1] + 1)"
    assert img[751] == "def clipped(obj, frame_shape):"
    assert img[764] == "        return False"
    assert const[45] == "LABEL_CONSOLIDATION_MAP = {"
    assert const[53] == "LABEL_NMS_DEFAULT = 0.4"
    ns = {"defaultdict": defaultdict, "np": np, "Any": object, "Optional": object}

    class Dnn:
        @staticmethod
        def NMSBoxes(boxes, confidences, score_threshold, nms_threshold):
            return np.arange(len(boxes), dtype=np.int32)

    class Cv2:
        dnn = Dnn

    ns["cv2"] = Cv2
    exec(compile("\n".join(const[45:54]), "const.py", "exec"), ns)
    exec(compile("\n".join(img[700:716]), "image.py", "exec"), ns)
    exec(compile("\n".join(img[718:720]), "image.py", "exec"), ns)
    exec(compile("\n".join(img[751:765]), "image.py", "exec"), ns)
    exec(compile("\n".join(obj[494:591]), "object.py", "exec"), ns)
    return ns


def f32bits(x):
    return f"{struct.unpack('>I', struct.pack('>f', float(np.float32(x))))[0]:08x}"


def det_text(det):
    label, score, box, _, _, region = det
    return (
        f"{label}:{f32bits(score)}:"
        f"{box[0]},{box[1]},{box[2]},{box[3]}:"
        f"{region[0]},{region[1]},{region[2]},{region[3]}"
    )


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: track_merge.py <Frigate checkout directory>")
    frigate = sys.argv[1]
    ns = build_namespace(
        source_lines(frigate, "frigate/util/object.py", OBJECT_BLOB),
        source_lines(frigate, "frigate/util/image.py", IMAGE_BLOB),
        source_lines(frigate, "frigate/const.py", CONST_BLOB),
    )
    rng = Lcg(SEED)
    lines = []
    stats = {
        "dropped": 0, "reordered": 0, "limit_edge": 0,
        "shift_keep_person": 0, "shift_drop_person": 0,
        "shift_keep_car": 0, "shift_drop_car": 0,
        "shift_keep_face": 0, "shift_drop_face": 0,
        "ratio_at": 0, "ratio_below": 0, "zerodiv_excluded": 0,
        "clipped_true": 0, "clipped_false": 0,
        "intersection_none": 0, "intersection_some": 0, "intersection_touch": 0,
        "area_negative": 0, "area_zero": 0,
    }
    kind_counts = {"merge": 0, "clipped": 0, "area": 0, "intersection": 0}

    def make_det(label, score, box, region):
        x0, y0, x1, y1 = box
        width = x1 - x0
        height = y1 - y0
        return (label, np.float32(score), tuple(box), width * height, width / max(1, height), tuple(region))

    def distinct(count):
        while True:
            values = [rng.randint(1, 999_999) / 1_000_000 for _ in range(count)]
            if len(set(f32bits(v) for v in values)) == count:
                return values

    def add_merge(frame, dets):
        w, h = frame
        try:
            out = ns["reduce_detections"]((h, w), dets)
        except ZeroDivisionError:
            stats["zerodiv_excluded"] += 1
            return False
        ids = [next(i for i, d in enumerate(dets) if d is o) for o in out]
        if len(ids) < len(dets):
            stats["dropped"] += 1
        if ids != sorted(ids):
            stats["reordered"] += 1
        for a in range(len(out)):
            for b in range(a + 1, len(out)):
                if out[a][0] != out[b][0]:
                    continue
                inter = ns["intersection"](out[a][2], out[b][2])
                if inter is not None and ns["area"](inter) > 0:
                    stats["limit_edge"] += 1
                    break
            else:
                continue
            break
        survivors = " ".join(str(i) for i in ids) if ids else "-"
        dets_text = " ".join(det_text(d) for d in dets)
        lines.append(f"merge {len(lines) + 1} frame {w} {h} dets {dets_text} => {survivors}")
        kind_counts["merge"] += 1
        return True

    def add_clipped(frame, box, region):
        w, h = frame
        det = make_det("person", 0.9, box, region)
        got = ns["clipped"](det, (h, w))
        stats["clipped_true" if got else "clipped_false"] += 1
        lines.append(f"clipped {len(lines) + 1} frame {w} {h} det {det_text(det)} => {'true' if got else 'false'}")
        kind_counts["clipped"] += 1

    def add_area(box):
        got = ns["area"](box)
        if got < 0:
            stats["area_negative"] += 1
        if got == 0:
            stats["area_zero"] += 1
        lines.append(f"area {len(lines) + 1} box {box[0]},{box[1]},{box[2]},{box[3]} => {got}")
        kind_counts["area"] += 1

    def add_intersection(a, b):
        got = ns["intersection"](a, b)
        if got is None:
            result = "none"
            stats["intersection_none"] += 1
        else:
            result = f"{got[0]},{got[1]},{got[2]},{got[3]}"
            stats["intersection_some"] += 1
            if ns["area"](got) == 1:
                stats["intersection_touch"] += 1
        lines.append(
            f"intersection {len(lines) + 1} a {a[0]},{a[1]},{a[2]},{a[3]} "
            f"b {b[0]},{b[1]},{b[2]},{b[3]} => {result}"
        )
        kind_counts["intersection"] += 1

    def frame_choice():
        return FRAMES[rng.randint(0, len(FRAMES) - 1)]

    def random_box(spread):
        return (
            rng.randint(-spread, spread),
            rng.randint(-spread, spread),
            rng.randint(-spread, spread),
            rng.randint(-spread, spread),
        )

    def plain_box(spread):
        x = rng.randint(-spread, spread)
        y = rng.randint(-spread, spread)
        return (x, y, x + rng.randint(1, spread), y + rng.randint(1, spread))

    # ordinary: clusters of overlapping boxes, mixed sizes, labels and order
    for _ in range(1200):
        w, h = frame_choice()
        region = (0, 0, w, h)
        count = rng.randint(1, 8)
        extra = rng.randint(0, 4) == 0
        scores = distinct(count + (2 if extra else 0))
        cx = rng.randint(-20, w + 20)
        cy = rng.randint(-20, h + 20)
        dets = []
        for j in range(count):
            label = LABELS[rng.randint(0, len(LABELS) - 1)]
            iw = rng.randint(1, max(1, min(160, w)))
            ih = rng.randint(1, max(1, min(160, h)))
            x = cx + rng.randint(-max(1, w // 8), max(1, w // 8))
            y = cy + rng.randint(-max(1, h // 8), max(1, h // 8))
            dets.append(make_det(label, scores[j], (x, y, x + iw - 1, y + ih - 1), region))
        if extra:
            label = LABELS[rng.randint(0, len(LABELS) - 1)]
            big = rng.randint(40, 120)
            dets.append(make_det(label, scores[count], (cx, cy, cx + big, cy + big), region))
            dets.append(make_det(label, scores[count + 1], (cx + 1, cy + 1, cx + big - 1, cy + big - 1), region))
        add_merge((w, h), dets)

    # threshold discriminators: shifted pairs at each label's limit
    shifts = [("person", 10), ("car", 20), ("face", 50)]
    for i in range(630):
        label, shift = shifts[i % 3]
        slot = i // 3
        keep = slot < 105
        s = shift if keep else shift - 1
        w, h = frame_choice()
        region = (0, 0, w, h)
        x = rng.randint(0, 400)
        y = rng.randint(0, 300)
        stats[f"shift_{'keep' if keep else 'drop'}_{label}"] += 1
        extra = i % 2 == 0
        scores = distinct(4 if extra else 2)
        pair = [
            make_det(label, scores[0], (x, y, x + 99, y + 9), region),
            make_det(label, scores[1], (x + s, y, x + s + 99, y + 9), region),
        ]
        dets = list(reversed(pair)) if i % 3 == 2 else pair
        if extra:
            other = "dog" if label != "dog" else "person"
            bx = rng.randint(3000, 3500)
            by = rng.randint(3000, 3300)
            if i % 4 == 0:
                small = (bx + 2, by + 2, bx + 5, by + 6)
                stats["ratio_at"] += 1
            else:
                small = (bx + 2, by + 2, bx + 20, by + 2)
                stats["ratio_below"] += 1
            dets = dets + [
                make_det(other, scores[2], small, region),
                make_det(other, scores[3], (bx, by, bx + 19, by + 19), region),
            ]
        add_merge((w, h), dets)

    # threshold discriminators: inclusive-area ratios at and just below 0.05
    for i in range(170):
        w, h = frame_choice()
        region = (0, 0, w, h)
        label = LABELS[rng.randint(0, len(LABELS) - 1)]
        bx = rng.randint(-2000, 2000)
        by = rng.randint(-2000, 2000)
        scores = distinct(2)
        if i % 2 == 0:
            small = (bx + 2, by + 2, bx + 5, by + 6)
            stats["ratio_at"] += 1
        else:
            small = (bx + 2, by + 2, bx + 20, by + 2)
            stats["ratio_below"] += 1
        add_merge((w, h), [
            make_det(label, scores[0], small, region),
            make_det(label, scores[1], (bx, by, bx + 19, by + 19), region),
        ])

    # ties and order: identical boxes, equal stored areas, interleaved labels, chains
    for i in range(400):
        w, h = frame_choice()
        region = (0, 0, w, h)
        kind = i % 4
        if kind == 0:
            count = rng.randint(2, 3)
            scores = distinct(count)
            label = LABELS[rng.randint(0, len(LABELS) - 1)]
            x = rng.randint(-100, 100)
            y = rng.randint(-100, 100)
            box = (x, y, x + rng.randint(1, 60), y + rng.randint(1, 60))
            dets = [make_det(label, scores[j], box, region) for j in range(count)]
        elif kind == 1:
            scores = distinct(2)
            shapes = [(3, 7), (4, 5), (2, 13), (13, 2), (5, 4)]
            one = shapes[rng.randint(0, len(shapes) - 1)]
            two = shapes[rng.randint(0, len(shapes) - 1)]
            label = LABELS[rng.randint(0, len(LABELS) - 1)]
            x = rng.randint(-100, 100)
            y = rng.randint(-100, 100)
            dets = [
                make_det(label, scores[0], (x, y, x + one[0] - 1, y + one[1] - 1), region),
                make_det(label, scores[1], (x + rng.randint(-3, 3), y, x + rng.randint(-3, 3) + two[0] - 1, y + two[1] - 1), region),
            ]
        elif kind == 2:
            count = rng.randint(4, 6)
            scores = distinct(count)
            dets = []
            for j in range(count):
                x = rng.randint(-30, w + 30)
                y = rng.randint(-30, h + 30)
                box = (x, y, x + rng.randint(2, 90), y + rng.randint(2, 90))
                dets.append(make_det(LABELS[j % len(LABELS)], scores[j], box, region))
        else:
            scores = distinct(3)
            label = LABELS[rng.randint(0, len(LABELS) - 1)]
            step = [9, 10, 12][rng.randint(0, 2)]
            x = rng.randint(-100, 100)
            y = rng.randint(-100, 100)
            dets = [
                make_det(label, scores[j], (x + j * step, y, x + j * step + 99, y + 9), region)
                for j in range(3)
            ]
        add_merge((w, h), dets)

    # degenerate: zero and negative widths and heights; Frigate's ZeroDivisionError cases are excluded
    written = 0
    while written < 400:
        w, h = frame_choice()
        region = (0, 0, w, h)
        count = rng.randint(1, 3)
        scores = distinct(count)
        dets = []
        for j in range(count):
            dx = [-1, 0, rng.randint(-20, -2), rng.randint(1, 20)][rng.randint(0, 3)]
            dy = [-1, 0, rng.randint(-20, -2), rng.randint(1, 20)][rng.randint(0, 3)]
            x = rng.randint(-20, 120)
            y = rng.randint(-20, 80)
            label = LABELS[rng.randint(0, len(LABELS) - 1)]
            dets.append(make_det(label, scores[j], (x, y, x + dx, y + dy), region))
        if add_merge((w, h), dets):
            written += 1

    # clipped: boxes near each region edge, regions at set distances from the frame border
    false_combos = [(0, 4), (0, 5), (0, 6), (4, 4), (4, 5), (4, 6), (5, 6), (6, 6), (7, 6)]

    def edge_case(frame, side, dist, off):
        w, h = frame
        r0, r1, r2, r3 = 12, 12, w - 12, h - 12
        if side == 0:
            return (dist, r1, r2, r3), (dist + off, r1 + 8, r2 - 8, r3 - 8)
        if side == 1:
            return (r0, dist, r2, r3), (r0 + 8, dist + off, r2 - 8, r3 - 8)
        if side == 2:
            return (r0, r1, w - dist, r3), (r0 + 8, r1 + 8, w - dist - off, r3 - 8)
        return (r0, r1, r2, h - dist), (r0 + 8, r1 + 8, r2 - 8, h - dist - off)

    for t in range(200):
        side = t % 4
        dist = 6 + (t // 4) % 2
        off = 4 + ((t // 8) % 2)
        frame = frame_choice()
        region, box = edge_case(frame, side, dist, off)
        add_clipped(frame, box, region)

    for t in range(200):
        side = t % 4
        dist, off = false_combos[(t // 4) % len(false_combos)]
        frame = frame_choice()
        region, box = edge_case(frame, side, dist, off)
        add_clipped(frame, box, region)

    distances = [0, 3, 4, 5, 6, 7, 9, 12]
    for _ in range(200):
        w, h = frame_choice()
        d0 = distances[rng.randint(0, len(distances) - 1)]
        d1 = distances[rng.randint(0, len(distances) - 1)]
        d2 = distances[rng.randint(0, len(distances) - 1)]
        d3 = distances[rng.randint(0, len(distances) - 1)]
        region = (d0, d1, w - d2, h - d3)
        box = (
            region[0] + rng.randint(-10, 30),
            region[1] + rng.randint(-10, 30),
            region[2] - rng.randint(-10, 30),
            region[3] - rng.randint(-10, 30),
        )
        add_clipped((w, h), box, region)

    # area: zero, negative and random boxes
    for i in range(60):
        x = rng.randint(-500, 500)
        y = rng.randint(-500, 500)
        if i % 2 == 0:
            add_area((x, y, x - 1, y + rng.randint(0, 40)))
        else:
            add_area((x, y, x + rng.randint(0, 40), y - 1))

    for i in range(80):
        x = rng.randint(-500, 500)
        y = rng.randint(-500, 500)
        if i % 2 == 0:
            add_area((x, y, x - rng.randint(1, 40), y + rng.randint(1, 40)))
        else:
            add_area((x, y, x + rng.randint(1, 40), y - rng.randint(1, 40)))

    for _ in range(160):
        add_area(random_box(500))

    # intersection: disjoint, overlapping, touching and random pairs
    for i in range(110):
        a = plain_box(200)
        gap = rng.randint(1, 60)
        if i % 2 == 0:
            b = (a[2] + gap, rng.randint(-200, 200), a[2] + gap + rng.randint(1, 60), rng.randint(-200, 200) + rng.randint(1, 60))
        else:
            b = (rng.randint(-200, 200), a[3] + gap, rng.randint(-200, 200) + rng.randint(1, 60), a[3] + gap + rng.randint(1, 60))
        add_intersection(a, b)

    for _ in range(100):
        a = plain_box(200)
        b = (
            a[0] + rng.randint(-20, 20),
            a[1] + rng.randint(-20, 20),
            a[2] + rng.randint(-20, 20),
            a[3] + rng.randint(-20, 20),
        )
        add_intersection(a, b)

    for _ in range(40):
        a = plain_box(200)
        b = (a[2], a[3], a[2] + rng.randint(0, 30), a[3] + rng.randint(0, 30))
        add_intersection(a, b)

    for _ in range(50):
        add_intersection(random_box(400), random_box(400))

    assert len(lines) == 4000, len(lines)
    assert kind_counts == {"merge": 2800, "clipped": 600, "area": 300, "intersection": 300}, kind_counts
    assert stats["dropped"] >= 1000, stats
    assert stats["reordered"] >= 300, stats
    assert stats["limit_edge"] >= 200, stats
    assert stats["shift_keep_person"] >= 100, stats
    assert stats["shift_drop_person"] >= 100, stats
    assert stats["shift_keep_car"] >= 100, stats
    assert stats["shift_drop_car"] >= 100, stats
    assert stats["shift_keep_face"] >= 100, stats
    assert stats["shift_drop_face"] >= 100, stats
    assert stats["ratio_at"] >= 150, stats
    assert stats["ratio_below"] >= 150, stats
    assert stats["zerodiv_excluded"] >= 40, stats
    assert stats["clipped_true"] >= 150, stats
    assert stats["clipped_false"] >= 150, stats
    assert stats["intersection_none"] >= 100, stats
    assert stats["intersection_some"] >= 100, stats
    assert stats["intersection_touch"] >= 30, stats
    assert stats["area_negative"] >= 50, stats
    assert stats["area_zero"] >= 20, stats

    header = [
        "# source v0.17.2 frigate/util/object.py blob b8f41e2c3214c22980f5227055aad348c9d6cd10 lines 495-591 (nested get_consolidated_object_detections 540-587)",
        "# source v0.17.2 frigate/util/image.py blob ea9fb0a0a7f166d7b145c6c3af9941de673aebe3 lines 701-716, 719-720, 752-765",
        "# source v0.17.2 frigate/const.py blob 41c24f08741e83a886250459dfb5bcb0cdee79a6 lines 46-54",
        "# stand-in: cv2.dnn.NMSBoxes keeps every box in input order, so the first pass is the identity and the fixture covers the second pass and the helpers",
        "# seed 20261123",
        f"# counts cases 4000 merge 2800 clipped 600 area 300 intersection 300 dropped {stats['dropped']} reordered {stats['reordered']} limit_edge {stats['limit_edge']}",
        f"# counts shift_keep_person {stats['shift_keep_person']} shift_drop_person {stats['shift_drop_person']} shift_keep_car {stats['shift_keep_car']} shift_drop_car {stats['shift_drop_car']} shift_keep_face {stats['shift_keep_face']} shift_drop_face {stats['shift_drop_face']} ratio_at {stats['ratio_at']} ratio_below {stats['ratio_below']} zerodiv_excluded {stats['zerodiv_excluded']}",
        f"# counts clipped_true {stats['clipped_true']} clipped_false {stats['clipped_false']} intersection_none {stats['intersection_none']} intersection_some {stats['intersection_some']} intersection_touch {stats['intersection_touch']} area_negative {stats['area_negative']} area_zero {stats['area_zero']}",
    ]
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text("\n".join(header + lines) + "\n", encoding="utf-8")
    summary = " ".join(f"{name}={stats[name]}" for name in sorted(stats))
    print(f"track_merge: {len(lines)} cases; {summary}")


if __name__ == "__main__":
    main()
