"""Goldens for corvette-track's reduce_detections: both passes of Frigate's
reduce_detections at v0.17.2, run with the real OpenCV 4.11.0 — no stand-in —
so the first pass (the per-label cv2.dnn.NMSBoxes call on Frigate's integer
box tuples) is checked against OpenCV itself, and the second pass again against
Frigate's own text.

Run command (the two LD_LIBRARY_PATH entries name the directories holding the
current libz.so.1 and libgcc_s.so.1, needed by the wheel's native libraries;
see motion_average.py's docstring, and do not copy store paths: they do not
survive a garbage collection):

    cd /home/cvandesande/github/corvette && timeout 500 nix develop .#goldens --command env LD_LIBRARY_PATH=<libz.so.1 directory>:<libgcc_s.so.1 directory> python3 -I tools/goldens/track_reduce.py <Frigate checkout directory> <wheel directory>

The wheel is the pinned opencv_python_headless-4.11.0.86-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl, sha256 0e0a27c19dd1f40ddff94976cfe43066fbbe9dfbb2ec1907d66c19caef42a57b; its cv2 is asserted 4.11.0. The texts are blob-checked and executed unchanged in one namespace: frigate/util/object.py blob b8f41e2c3214c22980f5227055aad348c9d6cd10 lines 495-591 (the whole reduce_detections, first pass 501-538 and second pass 540-587), frigate/util/image.py blob ea9fb0a0a7f166d7b145c6c3af9941de673aebe3 lines 701-716 (intersection), 719-720 (area) and 752-765 (clipped), and frigate/const.py blob 41c24f08741e83a886250459dfb5bcb0cdee79a6 lines 46-54 (the label limits).

One expectation diverges from raw Frigate by Corvette decision: a pair whose
later box has an inclusive area of zero raises ZeroDivisionError inside
Frigate, where Corvette skips the pair; reduce cases that raise are not
written and are counted as zerodiv_excluded.

The first-pass-only header counts (first_pass_drop_above, large_nms_drop) are
counting aids computed by calling cv2.dnn.NMSBoxes with the group form of
object.py 501-538; every written expectation comes from the whole function.

Fixture crates/corvette-track/tests/fixtures/reduce_cases.txt, # header lines
first (the cv2 line, the source blobs, the seed, the counts), one case per
line after them:

    reduce <n> frame <width> <height> dets <det> <det> ... => <i> <j> ...

Survivors are the surviving detections' indexes in the input list, in output
order, or - for none. <det> is label:score_hex8:x0,y0,x1,y1:rx0,ry0,rx1,ry1
with score_hex8 the bits of the float32 score; the frame is width then height
(Frigate's frame_shape is (height, width)); the detection's area and ratio are
computed by this generator exactly as detect computes them ((x1 - x0) * (y1 -
y0) and (x1 - x0) / max(1, y1 - y0)) and are not written. Output is
deterministic.
"""

import hashlib
import os
import struct
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

if len(sys.argv) != 3:
    sys.exit("usage: track_reduce.py <Frigate checkout directory> <wheel directory>")
sys.path.insert(0, sys.argv[2])
assert os.path.isdir(
    os.path.join(sys.argv[2], "opencv_python_headless-4.11.0.86.dist-info")
), "the pinned opencv_python_headless 4.11.0.86 dist-info is missing from the wheel directory"

import numpy as np
import cv2

assert cv2.__version__ == "4.11.0", cv2.__version__

REPO = Path(__file__).resolve().parents[2]
FIXTURE = REPO / "crates/corvette-track/tests/fixtures/reduce_cases.txt"
OBJECT_BLOB = "b8f41e2c3214c22980f5227055aad348c9d6cd10"
IMAGE_BLOB = "ea9fb0a0a7f166d7b145c6c3af9941de673aebe3"
CONST_BLOB = "41c24f08741e83a886250459dfb5bcb0cdee79a6"
SEED = 20261117
LABELS = ["person", "car", "face", "dog", "bicycle"]
FRAMES = [(704, 576), (640, 480), (1280, 720), (100, 60)]
FIXED_SCORES = [
    np.float32(0.5),
    np.nextafter(np.float32(0.5), np.float32(1.0)),
    np.nextafter(np.float32(0.5), np.float32(0.4)),
    np.float32(0.51),
    np.float32(0.6),
    np.float32(0.7),
    np.float32(0.9),
]


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
    assert obj[526] == "            indices = cv2.dnn.NMSBoxes("
    assert obj[539].startswith("    def get_consolidated_object_detections")
    assert obj[590] == "    )"
    assert img[700] == "def intersection(box_a, box_b) -> Optional[list[int]]:"
    assert img[715] == "    )"
    assert img[718] == "def area(box):"
    assert img[719] == "    return (box[2] - box[0] + 1) * (box[3] - box[1] + 1)"
    assert img[751] == "def clipped(obj, frame_shape):"
    assert img[764] == "        return False"
    assert const[45] == "LABEL_CONSOLIDATION_MAP = {"
    assert const[53] == "LABEL_NMS_DEFAULT = 0.4"
    ns = {"defaultdict": defaultdict, "np": np, "Any": object, "Optional": object, "cv2": cv2}
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
    frigate = sys.argv[1]
    ns = build_namespace(
        source_lines(frigate, "frigate/util/object.py", OBJECT_BLOB),
        source_lines(frigate, "frigate/util/image.py", IMAGE_BLOB),
        source_lines(frigate, "frigate/const.py", CONST_BLOB),
    )
    rng = Lcg(SEED)
    lines = []
    stats = {
        "dropped": 0,
        "first_pass_drop_above": 0,
        "score_drop": 0,
        "clipped_low_survive": 0,
        "limit_keep": 0,
        "limit_drop": 0,
        "reordered": 0,
        "label_vanishes": 0,
        "zerodiv_excluded": 0,
        "large_nms_drop": 0,
    }
    kind_counts = {
        "ordinary": 0,
        "exact_pairs": 0,
        "clipped_cases": 0,
        "score_threshold": 0,
        "order_ties": 0,
        "degenerate": 0,
        "large": 0,
    }

    def make_det(label, score, box, region):
        x0, y0, x1, y1 = box
        width = x1 - x0
        height = y1 - y0
        return (label, np.float32(score), tuple(box), width * height, width / max(1, height), tuple(region))

    def frame_choice():
        return FRAMES[rng.randint(0, len(FRAMES) - 1)]

    def score_choice():
        if rng.randint(0, 1) == 0:
            return FIXED_SCORES[rng.randint(0, len(FIXED_SCORES) - 1)]
        return rng.randint(300001, 999999) / 1_000_000

    def first_pass_positions(frame_shape, dets):
        groups = defaultdict(list)
        for position, det in enumerate(dets):
            groups[det[0]].append((position, det))
        kept = []
        for label, members in groups.items():
            boxes = [(d[2][0], d[2][1], d[2][2] - d[2][0], d[2][3] - d[2][1]) for _, d in members]
            confidences = [0.6 if ns["clipped"](d, frame_shape) else d[1] for _, d in members]
            indices = cv2.dnn.NMSBoxes(
                boxes, confidences, 0.5, ns["LABEL_NMS_MAP"].get(label, ns["LABEL_NMS_DEFAULT"])
            )
            for index in indices:
                kept.append(members[int(index)][0])
        return kept

    def add_reduce(kind, frame, dets, limit_mode=None, large=False):
        w, h = frame
        try:
            out = ns["reduce_detections"]((h, w), dets)
        except ZeroDivisionError:
            stats["zerodiv_excluded"] += 1
            return None
        ids = [next(i for i, d in enumerate(dets) if d is o) for o in out]
        idset = set(ids)
        kept_first = set(first_pass_positions((h, w), dets))
        if len(ids) < len(dets):
            stats["dropped"] += 1
        if ids != sorted(ids):
            stats["reordered"] += 1
        if {d[0] for d in dets} - {d[0] for d in out}:
            stats["label_vanishes"] += 1
        first_pass_drop = False
        score_drop = False
        clipped_low_kept = False
        for position, det in enumerate(dets):
            is_clipped = bool(ns["clipped"](det, (h, w)))
            conf = 0.6 if is_clipped else float(det[1])
            if conf > 0.5 and position not in kept_first:
                first_pass_drop = True
            if not is_clipped and float(det[1]) <= 0.5 and position not in idset:
                score_drop = True
            if is_clipped and float(det[1]) <= 0.5 and position in idset:
                clipped_low_kept = True
        if first_pass_drop:
            stats["first_pass_drop_above"] += 1
            if large:
                stats["large_nms_drop"] += 1
        if score_drop:
            stats["score_drop"] += 1
        if clipped_low_kept:
            stats["clipped_low_survive"] += 1
        if limit_mode == "keep" and 0 in idset and 1 in idset:
            stats["limit_keep"] += 1
        if limit_mode == "drop" and len(idset & {0, 1}) == 1:
            stats["limit_drop"] += 1
        survivors = " ".join(str(i) for i in ids) if ids else "-"
        dets_text = " ".join(det_text(d) for d in dets)
        lines.append(f"reduce {len(lines) + 1} frame {w} {h} dets {dets_text} => {survivors}")
        kind_counts[kind] += 1
        return ids

    # 1. ordinary clusters: boxes 10 to 300 pixels inside the frame, many
    # near-duplicates (a base box plus copies jittered up to 3 pixels per edge)
    for i in range(1000):
        w, h = frame_choice()
        region = (0, 0, w, h)
        count = rng.randint(2, 10)
        near = i % 2 == 0
        sw = rng.randint(10, max(10, min(300, w - 1)))
        sh = rng.randint(10, max(10, min(300, h - 1)))
        bx = rng.randint(0, max(0, w - 1 - sw))
        by = rng.randint(0, max(0, h - 1 - sh))
        base = (bx, by, bx + sw, by + sh)
        dets = []
        for _ in range(count):
            label = LABELS[rng.randint(0, len(LABELS) - 1)]
            if near:
                box = (
                    base[0] + rng.randint(-3, 3),
                    base[1] + rng.randint(-3, 3),
                    base[2] + rng.randint(-3, 3),
                    base[3] + rng.randint(-3, 3),
                )
            else:
                bw = rng.randint(10, max(10, min(300, w - 1)))
                bh = rng.randint(10, max(10, min(300, h - 1)))
                x = rng.randint(0, max(0, w - 1 - bw))
                y = rng.randint(0, max(0, h - 1 - bh))
                box = (x, y, x + bw, y + bh)
            dets.append(make_det(label, score_choice(), box, region))
        add_reduce("ordinary", (w, h), dets)

    # 2. exact-threshold pairs: IoU exactly the label's NMS limit (7x7 shifted
    # by 3k, 8x8 by 2k), one cell above it (shift 2k, 1k) and one below (4k, 3k)
    pair_kinds = [("person", 7, 3, 2), ("dog", 7, 3, 2), ("bicycle", 7, 3, 2), ("face", 7, 3, 2), ("car", 8, 2, 1)]
    for i in range(800):
        label, size0, limit_shift, over_shift = pair_kinds[i % 5]
        mode = (i // 5) % 3
        shift = limit_shift if mode == 0 else (over_shift if mode == 1 else limit_shift + 1)
        k = rng.randint(1, 40)
        size = size0 * k
        sh = shift * k
        w, h = frame_choice()
        region = (0, 0, w, h)
        x = rng.randint(0, max(0, w - 1 - size - sh))
        y = rng.randint(0, max(0, h - 1 - size - sh))
        axis = i % 2
        dx = sh if axis == 0 else 0
        dy = sh if axis == 1 else 0
        box_a = (x, y, x + size, y + size)
        box_b = (x + dx, y + dy, x + dx + size, y + dy + size)
        score_a, score_b = (0.9, 0.7) if (i // 2) % 2 == 0 else (0.7, 0.9)
        pair = [make_det(label, score_a, box_a, region), make_det(label, score_b, box_b, region)]
        dets = list(reversed(pair)) if (i // 6) % 2 == 1 else pair
        if i % 3 == 0:
            other = LABELS[(i + 1) % 5]
            if other == label:
                other = LABELS[(i + 2) % 5]
            ox = rng.randint(0, max(0, w - 21))
            oy = rng.randint(0, max(0, h - 21))
            dets = dets + [make_det(other, 0.6, (ox, oy, ox + 20, oy + 20), region)]
        limit_mode = {0: "keep", 1: "drop", 2: None}[mode]
        add_reduce("exact_pairs", (w, h), dets, limit_mode=limit_mode)

    # 3. clipped: regions more than 5 pixels from every frame border, box edges
    # exactly 4, 5 and 6 pixels inside each region edge, low and high scores
    for i in range(500):
        w, h = frame_choice()
        d0 = rng.randint(6, max(6, (w - 44) // 2))
        d1 = rng.randint(6, max(6, (h - 44) // 2))
        d2 = rng.randint(6, max(6, (w - 44) // 2))
        d3 = rng.randint(6, max(6, (h - 44) // 2))
        region = (d0, d1, w - d2, h - d3)
        side = i % 4
        off = 4 + (i // 4) % 3
        kind = (i // 4) % 4
        if side == 0:
            clipped_box = (region[0] + off, region[1] + 8, region[0] + off + 16, region[1] + 24)
        elif side == 1:
            clipped_box = (region[0] + 8, region[1] + off, region[0] + 24, region[1] + off + 16)
        elif side == 2:
            clipped_box = (region[2] - off - 16, region[1] + 8, region[2] - off, region[1] + 24)
        else:
            clipped_box = (region[0] + 8, region[3] - off - 16, region[0] + 24, region[3] - off)
        deep = (region[0] + 10, region[1] + 10, region[0] + 26, region[1] + 26)
        low = [0.3, 0.4, 0.5][i % 3]
        high = [0.6, 0.7, 0.9][i % 3]
        if kind == 0:
            dets = [make_det("person", low, clipped_box, region)]
        elif kind == 1:
            dets = [make_det("person", high, clipped_box, region)]
        elif kind == 2:
            pair = [
                make_det("person", low, clipped_box, region),
                make_det("person", 0.6, deep, region),
            ]
            dets = list(reversed(pair)) if i % 2 == 1 else pair
        else:
            pair = [
                make_det("person", low, clipped_box, region),
                make_det("person", low, deep, region),
            ]
            dets = list(reversed(pair)) if i % 2 == 1 else pair
        add_reduce("clipped_cases", (w, h), dets)

    # 4. score threshold: 0.5, one float32 step up and down, 0.4999, 0.5001
    score_cases = [
        np.float32(0.5),
        np.nextafter(np.float32(0.5), np.float32(1.0)),
        np.nextafter(np.float32(0.5), np.float32(0.4)),
        np.float32(0.4999),
        np.float32(0.5001),
    ]
    for i in range(400):
        score = score_cases[i % 5]
        w, h = frame_choice()
        region = (0, 0, w, h)
        if i % 3 == 0:
            x = rng.randint(0, max(0, w - 11))
            y = rng.randint(0, max(0, h - 11))
            dets = [make_det(LABELS[i % 5], score, (x, y, x + 10, y + 10), region)]
        else:
            count = rng.randint(2, 4)
            x = rng.randint(0, max(0, w - 11))
            y = rng.randint(0, max(0, h - 11))
            dets = [make_det("person", score, (x, y, x + 10, y + 10), region)]
            for j in range(1, count):
                dets.append(make_det("person", 0.9 if j == 1 else score, (x + j, y, x + j + 10, y + 10), region))
        add_reduce("score_threshold", (w, h), dets)

    # 5. order and ties: equal scores, equal areas with different shapes,
    # first-seen label order, a whole label group dropped between two others
    shapes16 = [(2, 8), (4, 4), (1, 16), (8, 2)]
    for i in range(400):
        kind = i % 4
        w, h = frame_choice()
        region = (0, 0, w, h)
        if kind == 0:
            score = FIXED_SCORES[4 + (i // 4) % 3]
            x = rng.randint(0, max(0, w - 31))
            y = rng.randint(0, max(0, h - 31))
            count = rng.randint(2, 3)
            dets = [
                make_det(LABELS[rng.randint(0, 4)], score, (x, y, x + 20, y + 20), region)
                for _ in range(count)
            ]
        elif kind == 1:
            one = shapes16[rng.randint(0, 3)]
            two = shapes16[rng.randint(0, 3)]
            x = rng.randint(0, max(0, w - 21))
            y = rng.randint(0, max(0, h - 21))
            dets = [
                make_det("person", 0.9, (x, y, x + one[0], y + one[1]), region),
                make_det("person", 0.7, (x + 1, y, x + 1 + two[0], y + two[1]), region),
            ]
        elif kind == 2:
            order = LABELS[(i // 4) % 5 :] + LABELS[: (i // 4) % 5]
            x = rng.randint(0, max(0, w - 61))
            y = rng.randint(0, max(0, h - 11))
            scores5 = [0.95, 0.9, 0.8, 0.7, 0.6]
            dets = [
                make_det(order[j], scores5[j], (x + j * 10, y, x + j * 10 + 10, y + 10), region)
                for j in range(5)
            ]
            dets.append(make_det(order[0], 0.55, (x, y, x + 10, y + 10), region))
        else:
            middle = ["car", "dog"][(i // 4) % 2]
            x = rng.randint(0, max(0, w - 51))
            y = rng.randint(0, max(0, h - 11))
            dets = [
                make_det("person", 0.9, (x, y, x + 10, y + 10), region),
                make_det(middle, 0.4, (x + 20, y, x + 30, y + 10), region),
                make_det("person", 0.7, (x + 40, y, x + 50, y + 10), region),
            ]
        add_reduce("order_ties", (w, h), dets)

    # 6. degenerate: zero and negative widths and heights, mixed with normal
    # boxes; Frigate's ZeroDivisionError cases are excluded and counted. A
    # zero-inclusive-area box only divides when it sorts after the current box
    # in its label group and both survived the first pass, so every other case
    # is that explicit pair with scores above the NMS score floor.
    written = 0
    attempts = 0
    while written < 400:
        w, h = frame_choice()
        region = (0, 0, w, h)
        label = LABELS[rng.randint(0, 4)]
        if attempts % 2 == 0:
            x = rng.randint(-2000, 2000)
            y = rng.randint(-2000, 2000)
            x2 = rng.randint(3000, 5000)
            y2 = rng.randint(3000, 5000)
            dets = [
                make_det(label, 0.6, (x, y, x + 10, y + 10), region),
                make_det(label, 0.7, (x2, y2, x2 - 1, y2 - 300), region),
            ]
        else:
            count = rng.randint(1, 3)
            scores = [0.6, 0.7, 0.8][:count]
            dets = []
            for j in range(count):
                dx = [-1, 0, rng.randint(-20, -2), rng.randint(1, 20)][rng.randint(0, 3)]
                dy = [-1, 0, rng.randint(-20, -2), rng.randint(1, 20)][rng.randint(0, 3)]
                x = rng.randint(-20, 120)
                y = rng.randint(-20, 80)
                dets.append(make_det(label, scores[j], (x, y, x + dx, y + dy), region))
            if attempts % 4 == 1:
                x3 = rng.randint(0, max(0, w - 11))
                y3 = rng.randint(0, max(0, h - 11))
                dets.append(make_det(label, 0.9, (x3, y3, x3 + 10, y3 + 10), region))
        if add_reduce("degenerate", (w, h), dets) is not None:
            written += 1
        attempts += 1

    # 7. large: edges within plus or minus 16777216, sizes into the millions so
    # inclusive areas reach 2^45 to 2^50
    for i in range(500):
        w, h = frame_choice()
        region = (0, 0, w, h)
        kind = i % 4
        sw = rng.randint(6_000_000, 33_000_000)
        sh = rng.randint(6_000_000, 33_000_000)
        x = rng.randint(-16_777_216, 16_777_216 - sw)
        y = rng.randint(-16_777_216, 16_777_216 - sh)
        big = (x, y, x + sw, y + sh)
        if kind == 0:
            count = rng.randint(2, 3)
            scores = [0.62, 0.71, 0.83][:count]
            dets = [make_det(LABELS[rng.randint(0, 4)], scores[j], big, region) for j in range(count)]
        elif kind == 1:
            jitter = (
                big[0] + rng.randint(1, 3),
                big[1] + rng.randint(1, 3),
                big[2] - rng.randint(1, 3),
                big[3] - rng.randint(1, 3),
            )
            dets = [
                make_det("person", 0.9, big, region),
                make_det("person", 0.8, jitter, region),
            ]
        elif kind == 2:
            sx = x + rng.randint(10, sw - 110)
            sy = y + rng.randint(10, sh - 110)
            dets = [
                make_det("person", 0.9, big, region),
                make_det("person", 0.8, (sx, sy, sx + 100, sy + 100), region),
            ]
        else:
            shift_x = rng.randint(10, 1000)
            shift_y = rng.randint(10, 1000)
            dets = [
                make_det("person", 0.9, big, region),
                make_det("person", 0.8, (x + shift_x, y + shift_y, x + shift_x + sw, y + shift_y + sh), region),
            ]
        add_reduce("large", (w, h), dets, large=True)

    assert len(lines) == 4000, len(lines)
    assert kind_counts == {
        "ordinary": 1000,
        "exact_pairs": 800,
        "clipped_cases": 500,
        "score_threshold": 400,
        "order_ties": 400,
        "degenerate": 400,
        "large": 500,
    }, kind_counts
    assert stats["dropped"] >= 1500, stats
    assert stats["first_pass_drop_above"] >= 600, stats
    assert stats["score_drop"] >= 300, stats
    assert stats["clipped_low_survive"] >= 200, stats
    assert stats["limit_keep"] >= 150, stats
    assert stats["limit_drop"] >= 150, stats
    assert stats["reordered"] >= 300, stats
    assert stats["label_vanishes"] >= 100, stats
    assert stats["zerodiv_excluded"] >= 40, stats
    assert stats["large_nms_drop"] >= 100, stats

    header = [
        "# cv2 4.11.0 (opencv_python_headless 4.11.0.86 wheel)",
        "# source v0.17.2 frigate/util/object.py blob b8f41e2c3214c22980f5227055aad348c9d6cd10 lines 495-591 (both passes)",
        "# source v0.17.2 frigate/util/image.py blob ea9fb0a0a7f166d7b145c6c3af9941de673aebe3 lines 701-716, 719-720, 752-765",
        "# source v0.17.2 frigate/const.py blob 41c24f08741e83a886250459dfb5bcb0cdee79a6 lines 46-54",
        "# divergence: a pair whose later box has an inclusive area of zero raises ZeroDivisionError in Frigate, where Corvette skips the pair; such cases are not written and are counted as zerodiv_excluded",
        f"# seed {SEED}",
        f"# counts cases 4000 ordinary 1000 exact_pairs 800 clipped_cases 500 score_threshold 400 order_ties 400 degenerate 400 large 500 dropped {stats['dropped']} first_pass_drop_above {stats['first_pass_drop_above']} score_drop {stats['score_drop']} clipped_low_survive {stats['clipped_low_survive']} limit_keep {stats['limit_keep']} limit_drop {stats['limit_drop']} reordered {stats['reordered']} label_vanishes {stats['label_vanishes']} zerodiv_excluded {stats['zerodiv_excluded']} large_nms_drop {stats['large_nms_drop']}",
    ]
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text("\n".join(header + lines) + "\n", encoding="utf-8")
    summary = " ".join(f"{name}={stats[name]}" for name in sorted(stats))
    kinds = " ".join(f"{name}={kind_counts[name]}" for name in sorted(kind_counts))
    print(f"track_reduce: {len(lines)} cases; {kinds}; {summary}")


if __name__ == "__main__":
    main()
