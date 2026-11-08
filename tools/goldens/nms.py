"""Goldens for corvette-yolo's nms_boxes: the keep-set of OpenCV's cv2.dnn.NMSBoxes.

Frigate calls cv2.dnn.NMSBoxes on the YOLO path at frigate/util/model.py v0.17.2
lines 28 and 209, keyword form score_threshold=0.4, nms_threshold=0.4, and on the
region merge at frigate/util/object.py v0.17.2 lines 527 to 529:

    indices = cv2.dnn.NMSBoxes(
        boxes, confidences, 0.5, LABEL_NMS_MAP.get(label, LABEL_NMS_DEFAULT)
    )

both leaving eta and top_k at their defaults; boxes are (x, y, width, height).

Reference OpenCV 4.11.0, git blob hashes: modules/dnn/src/nms.cpp
c51a6305581b86530f585e3e359238ae1b6120d4, modules/dnn/src/nms.inl.hpp
7b84839c02c6550534526f3bc696a58dbec463d6,
modules/core/include/opencv2/core/types.hpp df4fd21885dedda99c5d0d1e059d34aa8e9d975e.
Quoted verbatim, with file and line:

nms.inl.hpp 40 to 43, the strict score filter:
        if (scores[i] > threshold)
        {
            score_index_vec.push_back(std::make_pair(scores[i], i));
        }
nms.inl.hpp 47 to 48, the descending sort:
    std::stable_sort(score_index_vec.begin(), score_index_vec.end(),
                     SortScorePairDescend<int>);
nms.inl.hpp 90, the keep test:
            keep = overlap <= adaptive_threshold;
nms.cpp 19, the overlap of a pair:
    return 1.f - static_cast<float>(jaccardDistance(a, b));
types.hpp 2045 to 2048, jaccardDistance's early return:
    if ((Aa + Ab) <= std::numeric_limits<_Tp>::epsilon()) {
        // jaccard_index = 1 -> distance = 0
        return 0.0;
    }

The Rust port in crates/corvette-yolo/src/nms.rs transcribes types.hpp 1851,
1860, 1930 to 1962, 2020 to 2025 and 2040 to 2053.

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

EXTREME = [
    ("neg_width", [(0, 0, 5, 5, 0.9), (2, 2, -3, 5, 0.8)], 0.4, 0.4),
    ("neg_height", [(0, 0, 5, 5, 0.9), (2, 2, 5, -3, 0.8)], 0.4, 0.4),
    ("tiny_area_pair", [(0, 0, 3e-9, 3e-9, 0.9), (2e-9, 0, 3e-9, 3e-9, 0.8)], 0.4, 0.4),
    ("coords_1e30", [(0, 0, 1e30, 1e30, 0.9), (1e30, 1e30, 1e30, 1e30, 0.8)], 0.4, 0.4),
    ("touching", [(0, 0, 2, 2, 0.9), (2, 0, 2, 2, 0.8)], 0.4, 0.4),
    ("same_box_four",
     [(1, 1, 3, 3, 0.9), (1, 1, 3, 3, 0.8), (1, 1, 3, 3, 0.7), (1, 1, 3, 3, 0.6)], 0.4, 0.4),
    ("nt_zero_overlap", [(0, 0, 2, 2, 0.9), (1, 1, 2, 2, 0.8)], 0.4, 0.0),
    ("nt_one_overlap", [(0, 0, 2, 2, 0.9), (1, 1, 2, 2, 0.8)], 0.4, 1.0),
    ("zero_area_nt_one", [(0, 0, 0, 0, 0.9), (0, 0, 0, 0, 0.8)], 0.4, 1.0),
    ("score_exactly_one", [(0, 0, 1, 1, 1.0), (5, 5, 1, 1, 0.9)], 0.4, 0.4),
    ("negative_origin_overlap",
     [(-5, -5, 10, 10, 0.9), (-3, -3, 10, 10, 0.8)], 0.4, 0.4),
    ("negative_origin_disjoint",
     [(-10, 0, 5, 5, 0.9), (0, 0, 5, 5, 0.8)], 0.4, 0.4),
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
    for name, rows, st, nt in EXTREME:
        sections += _emit(name, [], rows, st, nt)
    header = f"# cv2 {cv2.__version__} numpy {np.__version__} python {platform.python_version()}\n"
    OUT.write_text(header + "\n".join(sections) + "\n")
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
