"""Goldens for corvette-yolo's decode: the detection rows Frigate's post_process_yolo emits.

Reference: the frigate repository, path frigate/util/model.py, tag v0.17.2 (commit
3d4dd3ac4), blob 338303e2d7663bb12928c559e4e18182b632ce4c, lines 187 to 233:
__post_process_nms_yolo at 187 to 226 and post_process_yolo at 229 to 233. Both
functions are copied below verbatim and applied unchanged.

The deployed detector calls them as in frigate-vulkan docker/frigate/ncnn.py at
6ec7cf1, lines 180 to 181:

        output = self._extract(np.ascontiguousarray(tensor_input.squeeze(0)))
        return post_process_yolo([output[np.newaxis, ...]], self.width, self.height)

so this generator calls post_process_yolo([raw[np.newaxis, ...]], 640, 640) with
raw a (50, 8400) float32 array, and keeps the rows with a non-zero confidence.

Writes the fixture read by crates/corvette-yolo/tests/decode.rs.
"""

import platform
from pathlib import Path

import cv2
import numpy as np

OUT = Path(__file__).resolve().parents[2] / "crates/corvette-yolo/tests/fixtures/decode_cases.txt"
MASK = 0xFFFF_FFFF
ANCHORS = 8400
BOX_ROWS = 4
CLASSES = 46
ROWS = BOX_ROWS + CLASSES
BACKGROUND = [0.0, 0.0625, 0.125, 0.1875, 0.25, 0.3125, 0.375]
ST = 0.4
NT = 0.4
# gate-passing anchors and rows returned, computed by the orchestrator from the spec
EXPECTED = {"random_dense": (275, 20), "random_sparse": (11, 8), "random_none": (0, 0),
            "random_all": (8400, 20)}
EQ04 = np.uint32(0x3ECCCCCD).view(np.float32)
UP04 = np.uint32(0x3ECCCCCF).view(np.float32)
NAN_BITS = np.uint32(0x7FC00000).view(np.float32)

def __post_process_nms_yolo(predictions: np.ndarray, width, height) -> np.ndarray:
    predictions = np.squeeze(predictions)

    # transpose the output so it has order (inferences, class_ids)
    if predictions.shape[0] < predictions.shape[1]:
        predictions = predictions.T

    scores = np.max(predictions[:, 4:], axis=1)
    predictions = predictions[scores > 0.4, :]
    scores = scores[scores > 0.4]
    class_ids = np.argmax(predictions[:, 4:], axis=1)

    # Rescale box
    boxes = predictions[:, :4]
    boxes_xyxy = np.ones_like(boxes)
    boxes_xyxy[:, 0] = boxes[:, 0] - boxes[:, 2] / 2
    boxes_xyxy[:, 1] = boxes[:, 1] - boxes[:, 3] / 2
    boxes_xyxy[:, 2] = boxes[:, 0] + boxes[:, 2] / 2
    boxes_xyxy[:, 3] = boxes[:, 1] + boxes[:, 3] / 2
    boxes = boxes_xyxy

    # run NMS
    indices = cv2.dnn.NMSBoxes(boxes, scores, score_threshold=0.4, nms_threshold=0.4)
    detections = np.zeros((20, 6), np.float32)
    for i, (bbox, confidence, class_id) in enumerate(
        zip(boxes[indices], scores[indices], class_ids[indices])
    ):
        if i == 20:
            break

        detections[i] = [
            class_id,
            confidence,
            bbox[1] / height,
            bbox[0] / width,
            bbox[3] / height,
            bbox[2] / width,
        ]

    return detections


def post_process_yolo(output: list[np.ndarray], width: int, height: int) -> np.ndarray:
    if len(output) > 1:
        return __post_process_multipart_yolo(output, width, height)
    else:
        return __post_process_nms_yolo(output[0], width, height)



def _next_u32(state):
    state ^= (state << 13) & MASK
    state ^= state >> 17
    state ^= (state << 5) & MASK
    return state


def _unit(v):
    return np.float32(v / 16777216.0)


def _bits(value):
    return f"{np.float32(value).view(np.uint32):08x}"


def _random_tensor(seed, strong_below):
    raw = np.zeros((ROWS, ANCHORS), dtype=np.float32)
    state = seed
    for anchor in range(ANCHORS):
        v = []
        for _ in range(7):
            state = _next_u32(state)
            v.append(state >> 8)
        raw[0, anchor] = _unit(v[0]) * np.float32(640.0)
        raw[1, anchor] = _unit(v[1]) * np.float32(640.0)
        raw[2, anchor] = np.float32(16.0) + _unit(v[2]) * np.float32(112.0)
        raw[3, anchor] = np.float32(16.0) + _unit(v[3]) * np.float32(112.0)
        for j in range(CLASSES):
            raw[BOX_ROWS + j, anchor] = BACKGROUND[(anchor + 5 * j) % 7]
        if v[4] < strong_below:
            c = (v[5] * 46) >> 24
            raw[BOX_ROWS + c, anchor] = np.float32(0.5) + np.float32(0.5) * _unit(v[6])
    return raw


def _rows(raw):
    out = post_process_yolo([raw[np.newaxis, ...]], 640, 640)
    return [r for r in out if r[1] != 0.0]


def _row_lines(rows):
    lines = [f"st {_bits(ST)}", f"nt {_bits(NT)}"]
    for r in rows:
        lines.append(" ".join(["row", str(int(r[0])), *(_bits(r[k]) for k in (1, 2, 3, 4, 5))]))
    return lines


def _emit_random(name, seed, strong_below):
    raw = _random_tensor(seed, strong_below)
    gate = int(np.count_nonzero(np.max(raw[BOX_ROWS:, :], axis=0) > ST))
    rows = _rows(raw)
    assert (gate, len(rows)) == EXPECTED[name], f"{name}: got {(gate, len(rows))}, expected {EXPECTED[name]}"
    lines = [f"[{name}]", f"random {seed} {strong_below}", f"# gate {gate} rows {len(rows)}"]
    return lines + _row_lines(rows)


def _emit_hand(name, cells):
    raw = np.zeros((ROWS, ANCHORS), dtype=np.float32)
    for row, anchor, value in cells:
        raw[row, anchor] = value
    lines = [f"[{name}]"] + [f"cell {r} {a} {_bits(raw[r, a])}" for r, a, _ in cells]
    return lines + _row_lines(_rows(raw))


def _box(anchor, cx, cy, w, h):
    return [(0, anchor, cx), (1, anchor, cy), (2, anchor, w), (3, anchor, h)]


HAND = {
    "single": _box(100, 320.0, 240.0, 64.0, 96.0) + [(4 + 7, 100, 0.75)],
    "score_equal_gate": _box(10, 100.0, 100.0, 40.0, 40.0) + [(4 + 3, 10, EQ04)]
    + _box(20, 400.0, 100.0, 40.0, 40.0) + [(4 + 3, 20, UP04)],
    "argmax_tie": _box(5, 200.0, 200.0, 50.0, 50.0) + [(4 + 9, 5, 0.625), (4 + 31, 5, 0.625)],
    "nan_class_score": _box(7, 150.0, 150.0, 30.0, 30.0) + [(4 + 2, 7, 0.875), (4 + 40, 7, NAN_BITS)]
    + _box(8, 500.0, 500.0, 30.0, 30.0) + [(4 + 2, 8, 0.75)],
    "quirk_nms_suppression": _box(1, 100.0, 100.0, 40.0, 40.0) + [(4, 1, 0.875)]
    + _box(2, 130.0, 100.0, 40.0, 40.0) + [(4, 2, 0.75)],
    "cap_25": [c for i in range(25) for c in (
        _box(i, 5 * i + 5.5, 0.5, 1 - 10 * (i + 1), 1.0) + [(4, i, 0.9375 - i / 64)])],
}


def main():
    sections = []
    sections += _emit_random("random_dense", 11, 524288)
    sections += _emit_random("random_sparse", 12, 32768)
    sections += _emit_random("random_none", 13, 0)
    sections += _emit_random("random_all", 14, 16777216)
    for name in ["single", "score_equal_gate", "argmax_tie", "nan_class_score",
                 "quirk_nms_suppression", "cap_25"]:
        sections += _emit_hand(name, HAND[name])
    header = f"# cv2 {cv2.__version__} numpy {np.__version__} python {platform.python_version()}\n"
    OUT.write_text(header + "\n".join(sections) + "\n")
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
