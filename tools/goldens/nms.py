"""Goldens for corvette-yolo's nms_boxes: the keep-set of OpenCV's cv2.dnn.NMSBoxes.

Frigate calls NMSBoxes(boxes, scores, 0.4, 0.4) on the YOLO path and
(bboxes, scores, 0.5, iou) on the region merge (frigate/util/model.py at v0.17.2),
both leaving eta and top_k at their defaults; boxes are (x, y, width, height).

Reference opencv/modules/dnn/src/nms.inl.hpp and nms.cpp at 4.11.0:

    if (scores[i] > threshold) { score_index_vec.push_back(...); }
    std::stable_sort(score_index_vec..., SortScorePairDescend<int>);
    keep = overlap <= adaptive_threshold;

Writes the fixture read by crates/corvette-yolo/tests/nms.rs.
"""

import cv2
import numpy as np
import platform
from pathlib import Path

OUT = Path(__file__).resolve().parents[2] / "crates/corvette-yolo/tests/fixtures/nms_cases.txt"
MASK = 0xFFFF_FFFF

DOWN = np.nextafter(np.float32(0.4), np.float32(0.0))
UP = np.nextafter(np.float32(0.4), np.float32(1.0))
EQ_NT = float(np.float32(1.0) - np.float32(1.0 - 2.0 / 5.0))

HAND = [
    ("empty", [], 0.4, 0.4),
    ("single", [(0, 0, 1, 1, 0.9)], 0.4, 0.4),
    ("all_below", [(0, 0, 1, 1, 0.2), (5, 5, 1, 1, 0.3)], 0.4, 0.4),
    ("score_below_and_at", [(0, 0, 1, 1, float(DOWN)), (5, 5, 1, 1, 0.4)], 0.4, 0.4),
    ("score_at_and_above", [(0, 0, 1, 1, 0.4), (5, 5, 1, 1, float(UP))], 0.4, 0.4),
    ("identical_same_score", [(1, 1, 2, 2, 0.9), (1, 1, 2, 2, 0.9)], 0.4, 0.4),
    ("identical_diff_score", [(1, 1, 2, 2, 0.8), (1, 1, 2, 2, 0.9)], 0.4, 0.4),
    ("disjoint", [(0, 0, 1, 1, 0.9), (10, 10, 1, 1, 0.8)], 0.4, 0.4),
    ("zero_width", [(0, 0, 0, 5, 0.9), (1, 1, 1, 1, 0.8)], 0.4, 0.4),
    ("zero_area_both", [(0, 0, 0, 0, 0.9), (0, 0, 0, 0, 0.8)], 0.4, 0.4),
    ("zero_inside", [(0, 0, 0, 0, 0.9), (0, 0, 2, 2, 0.8)], 0.4, 0.4),
    ("chain", [(0, 0, 10, 10, 0.9), (3, 0, 10, 10, 0.85), (7, 0, 10, 10, 0.8)], 0.4, 0.4),
    ("overlap_equal_threshold", [(0, 0, 2, 1, 0.9), (0, 0, 5, 1, 0.8)], 0.4, EQ_NT),
    ("overlap_just_above_threshold",
     [(0, 0, 10, 1, 0.9), (3.700000047683716, 0, 12.049999237060547, 1, 0.8)], 0.4, 0.4),
    ("fractional_double_keeps", [(0.5, 0.5, 5.5, 5.5, 0.9), (0, 0, 4, 4, 0.8)], 0.4, 0.4),
    ("fractional_double_suppresses", [(0, 0, 4.9, 4.9, 0.9), (1.0, 1.0, 4.9, 4.9, 0.8)], 0.4, 0.4),
]


def _random_rows(seed, count, ties):
    state = seed
    rows = []
    for _ in range(count):
        d = []
        for _ in range(5):
            state ^= (state << 13) & MASK
            state ^= state >> 17
            state ^= (state << 5) & MASK
            d.append(np.float32((state >> 8) / 16777216.0))
        w = np.float32(8.0) + d[2] * np.float32(120.0)
        h = np.float32(8.0) + d[3] * np.float32(120.0)
        score = np.floor(d[4] * 16.0) / 16.0 if ties else d[4]
        rows.append((d[0] * np.float32(600.0), d[1] * np.float32(600.0), w, h, score))
    return rows


def _bits(value):
    return f"{np.float32(value).view(np.uint32):08x}"


def _emit(name, head, rows, st, nt):
    b = np.array([r[:4] for r in rows], dtype=np.float32).reshape(-1, 4)
    s = np.array([r[4] for r in rows], dtype=np.float32)
    kept = " ".join(str(int(i)) for i in np.asarray(cv2.dnn.NMSBoxes(b, s, np.float32(st), np.float32(nt))).reshape(-1))
    lines = [f"[{name}]", *head]
    if not head:
        lines += ["box " + " ".join(_bits(v) for v in r) for r in rows]
    lines += [f"st {_bits(st)}", f"nt {_bits(nt)}"]
    lines.append(("want " + kept).rstrip())
    return lines


def main():
    r200 = _random_rows(1, 200, False)
    sections = []
    sections += _emit("random_200", ["random 1 200"], r200, 0.4, 0.4)
    sections += _emit("ties_200", ["random 1 200", "ties"], _random_rows(1, 200, True), 0.4, 0.4)
    sections += _emit("dense_8400", ["random 7 8400"], _random_rows(7, 8400, False), 0.4, 0.4)
    sections += _emit("region_merge_200", ["random 1 200"], r200, 0.5, 0.6)
    for name, rows, st, nt in HAND:
        sections += _emit(name, [], rows, st, nt)
    header = f"# cv2 {cv2.__version__} numpy {np.__version__} python {platform.python_version()}\n"
    OUT.write_text(header + "\n".join(sections) + "\n")
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
