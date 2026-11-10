r"""Goldens for corvette-motion's motion decisions: the motion boxes, the motion
fraction, the calibrating flag and the average weight that the deployed
detector's contour loop, calibration rules and average-update rule return for a
list of contours, frame by frame through a sequence whose state carries.

Reference: the frigate repository, path frigate/motion/improved_motion.py, tag
v0.17.2, blob b081d37919541e6d1f95762ce084abde81774038, lines 30 to 34, line
36, line 40, lines 139 to 158, lines 179 to 185 and lines 214 to 230, blank
lines 155 and 182 omitted, copied verbatim:

        self.resize_factor = frame_shape[0] / config.frame_height
        self.motion_frame_size = (
            config.frame_height,
            config.frame_height * frame_shape[1] // frame_shape[0],
        )
        self.motion_frame_count = 0
        self.calibrating = True
        # loop over the contours
        total_contour_area = 0
        for c in contours:
            # if the contour is big enough, count it as motion
            contour_area = cv2.contourArea(c)
            total_contour_area += contour_area
            if contour_area > self.config.contour_area:
                x, y, w, h = cv2.boundingRect(c)
                motion_boxes.append(
                    (
                        int(x * self.resize_factor),
                        int(y * self.resize_factor),
                        int((x + w) * self.resize_factor),
                        int((y + h) * self.resize_factor),
                    )
                )
        pct_motion = total_contour_area / (
            self.motion_frame_size[0] * self.motion_frame_size[1]
        )
        # once the motion is less than 5% and the number of contours is < 4, assume its calibrated
        if pct_motion < 0.05 and len(motion_boxes) <= 4:
            self.calibrating = False
        # if calibrating or the motion contours are > 80% of the image area (lightning, ir, ptz) recalibrate
        if self.calibrating or pct_motion > self.config.lightning_threshold:
            self.calibrating = True
        if len(motion_boxes) > 0:
            self.motion_frame_count += 1
            if self.motion_frame_count >= 10:
                # only average in the current frame if the difference persists for a bit
                cv2.accumulateWeighted(
                    resized_frame,
                    self.avg_frame,
                    0.2 if self.calibrating else self.config.frame_alpha,
                )
        else:
            # when no motion, just keep averaging the frames together
            cv2.accumulateWeighted(
                resized_frame,
                self.avg_frame,
                0.2 if self.calibrating else self.config.frame_alpha,
            )
            self.motion_frame_count = 0

Reference: the frigate repository, path frigate/config/camera/motion.py, tag
v0.17.2, blob 65c03f73176cb2aab15a57e48f032065bc6cbae6, lines 12 to 25, copied
verbatim:

    threshold: int = Field(
        default=30,
        title="Motion detection threshold (1-255).",
        ge=1,
        le=255,
    )
    lightning_threshold: float = Field(
        default=0.8, title="Lightning detection threshold (0.3-1.0).", ge=0.3, le=1.0
    )
    improve_contrast: bool = Field(default=True, title="Improve Contrast")
    contour_area: Optional[int] = Field(default=10, title="Contour Area")
    delta_alpha: float = Field(default=0.2, title="Delta Alpha")
    frame_alpha: float = Field(default=0.01, title="Frame Alpha")
    frame_height: Optional[int] = Field(default=100, title="Frame Height")

Run (no argument):
    nix develop .#goldens --command python3 -I tools/goldens/motion_decision.py

Nothing is measured here: the oracle below is a pure Python transcription of
the quoted lines and the deployed class is not run. What is read is what the
oracle returns for every frame: the boxes, the motion fraction, the
calibrating flag and the average weight, in Python's int() truncation, float
multiplication and true division.

Writes the fixture read by crates/corvette-motion/tests/decision.rs. One
header line "counts scale ... filter ... fraction ... states ... random ..."
holds the number of sequences per kind; the kinds appear in that order. Each
sequence is one line "sequence <kind> <frame_width> <frame_height>
<lightning> <contour_area> <frame_alpha> <frame_height_setting> <frames>"
where lightning and frame_alpha are repr floats, followed by <frames> lines
"frame <count> <x> <y> <w> <h> <twice_area>... = <boxes> <x1> <y1> <x2>
<y2>... <fraction_bits> <calibrating> <weight>", where the count is followed
by that many groups of five contour integers, boxes by that many groups of
four, fraction_bits is the 16 lowercase hex digits of the IEEE-754 big-endian
bits of the fraction, calibrating is 0 or 1, and weight is - or the 16 hex
digits of the weight's bits. A sequence's frames run in order through one
oracle; each sequence starts a fresh one. random.Random(20261110 + 1000 *
kind_number + index) is the source of randomness, kind_number counting from 0
in the header's order and index within the kind from 0.
"""

import random
import struct
from pathlib import Path

OUT = Path(__file__).resolve().parents[2] / "crates/corvette-motion/tests/fixtures/decision_cases.txt"

SCALE_GEOMETRIES = [
    (1280, 720, 100),
    (704, 576, 100),
    (640, 480, 100),
    (1920, 1080, 100),
    (2560, 1440, 100),
    (3840, 2160, 100),
    (1280, 720, 50),
    (1280, 720, 150),
    (704, 576, 50),
    (704, 576, 150),
    (640, 480, 120),
    (1280, 720, 90),
]

FRACTION_PAIRS = [(0.05, 0.8), (0.3, 0.3), (0.7, 0.7), (0.8, 0.8), (1.0, 1.0)]
RANDOM_LIGHTNINGS = [0.3, 0.5, 0.7, 0.8, 0.9, 1.0]
RANDOM_AREAS = [0, 5, 10, 30]
RANDOM_ALPHAS = [0.0, 0.01, 0.05, 0.2, 0.5, 1.0]
RANDOM_FRAMES = 40

lines = []
counts = {"scale": 0, "filter": 0, "fraction": 0, "states": 0, "random": 0}


class Oracle:
    def __init__(self, frame_width, frame_height, lightning, contour_area, frame_alpha, setting):
        self.resize_factor = frame_height / setting
        self.motion_height = setting
        self.motion_width = setting * frame_width // frame_height
        self.lightning = lightning
        self.contour_min = contour_area
        self.frame_alpha = frame_alpha
        self.motion_frame_count = 0
        self.calibrating = True

    def step(self, contours):
        motion_boxes = []
        total_contour_area = 0
        for x, y, w, h, twice_area in contours:
            contour_area = twice_area / 2
            total_contour_area += contour_area
            if contour_area > self.contour_min:
                motion_boxes.append(
                    (
                        int(x * self.resize_factor),
                        int(y * self.resize_factor),
                        int((x + w) * self.resize_factor),
                        int((y + h) * self.resize_factor),
                    )
                )
        pct_motion = total_contour_area / (self.motion_height * self.motion_width)
        if pct_motion < 0.05 and len(motion_boxes) <= 4:
            self.calibrating = False
        if self.calibrating or pct_motion > self.lightning:
            self.calibrating = True
        if len(motion_boxes) > 0:
            self.motion_frame_count += 1
            if self.motion_frame_count >= 10:
                average_weight = 0.2 if self.calibrating else self.frame_alpha
            else:
                average_weight = None
        else:
            average_weight = 0.2 if self.calibrating else self.frame_alpha
            self.motion_frame_count = 0
        return motion_boxes, pct_motion, self.calibrating, average_weight


def hex_bits(value):
    return struct.pack(">d", value).hex()


def frame_line(oracle, contours):
    boxes, fraction, calibrating, weight = oracle.step(contours)
    fields = ["frame", str(len(contours))]
    for contour in contours:
        fields += [str(v) for v in contour]
    fields.append("=")
    fields.append(str(len(boxes)))
    for box in boxes:
        fields += [str(v) for v in box]
    fields.append(hex_bits(fraction))
    fields.append("1" if calibrating else "0")
    fields.append("-" if weight is None else hex_bits(weight))
    return " ".join(fields)


def emit(kind, frame_width, frame_height, lightning, area, alpha, setting, frames):
    oracle = Oracle(frame_width, frame_height, lightning, area, alpha, setting)
    lines.append(
        " ".join(
            [
                "sequence",
                kind,
                str(frame_width),
                str(frame_height),
                repr(lightning),
                str(area),
                repr(alpha),
                str(setting),
                str(len(frames)),
            ]
        )
    )
    for frame in frames:
        lines.append(frame_line(oracle, frame))
    counts[kind] += 1


def motion_pixels(frame_width, frame_height, setting):
    return setting * (setting * frame_width // frame_height)


for index, (width, height, setting) in enumerate(SCALE_GEOMETRIES):
    mh = setting
    mw = setting * width // height
    contours = []
    for i in range(max(mw, mh) + 40):
        x = i % mw
        y = i % mh
        contours.append((x, y, 1 + (i * 7) % (mw - x), 1 + (i * 5) % (mh - y), 22))
    assert {c[0] for c in contours} == set(range(mw))
    assert {c[1] for c in contours} == set(range(mh))
    assert mw in {c[0] + c[2] for c in contours}
    assert mh in {c[1] + c[3] for c in contours}
    emit("scale", width, height, 0.8, 10, 0.01, setting, [contours[i:i + 50] for i in range(0, len(contours), 50)])

for index, area in enumerate([0, 1, 10, 11, 100]):
    times = []
    for t in [0, 1, 2, 2 * area - 1, 2 * area, 2 * area + 1, 2 * area + 2, 2 * area + 3, 1000]:
        if t >= 0 and t not in times:
            times.append(t)
    emit("filter", 1280, 720, 0.8, area, 0.01, 100, [[(i, i, 2, 2, t) for i, t in enumerate(times)]])

index = 0
for width, height in [(1280, 720), (704, 576)]:
    pixels = motion_pixels(width, height, 100)
    for target, lightning in FRACTION_PAIRS:
        raw = 2 * pixels * target
        assert abs(raw - round(raw)) <= 1e-6
        base = round(raw)
        for delta in (-1, 0, 1):
            total = base + delta
            for box_count in (1, 4, 5, 6):
                rest = total - 22 * (box_count - 1)
                assert rest > 0
                contours = [(0, 0, 1, 1, 22)] * (box_count - 1) + [(0, 0, 1, 1, rest)]
                for start_flag in (0, 1):
                    frames = ([[]] if start_flag else []) + [contours]
                    emit("fraction", width, height, lightning, 10, 0.01, 100, frames)
                    index += 1
assert index == 240

single = lambda t: [(0, 0, 1, 1, t)]
assert 15930 / 17700 == 0.9
assert 8850 / 17700 == 0.5
assert 354 / 17700 == 0.02
assert 15045 / 17700 == 0.85
assert 14160 / 17700 == 0.8
assert abs(400 / 17700 - 0.0226) < 1e-4
emit("states", 1280, 720, 0.8, 10, 0.01, 100, [[] for _ in range(14)])
emit(
    "states", 1280, 720, 0.8, 10, 0.01, 100,
    [single(31860) for _ in range(3)] + [single(17700) for _ in range(2)]
    + [single(708) for _ in range(3)] + [[] for _ in range(3)],
)
emit(
    "states", 1280, 720, 0.8, 10, 0.01, 100,
    [[] for _ in range(2)] + [single(40) for _ in range(25)] + [[]]
    + [single(40) for _ in range(12)] + [[]],
)
emit(
    "states", 1280, 720, 0.8, 10, 0.01, 100,
    [[(0, 0, 1, 1, 22)] * 5 for _ in range(6)] + [[(0, 0, 1, 1, 22)] * 4]
    + [[(0, 0, 1, 1, 22)] * 5 for _ in range(6)],
)
emit(
    "states", 1280, 720, 0.8, 10, 0.01, 100,
    [[] for _ in range(3)] + [single(30090) for _ in range(12)] + [[] for _ in range(2)]
    + [single(28320)] + [single(28321)] + [[]],
)
emit("states", 1280, 720, 0.8, 10, 0.01, 100, [[(0, 0, 1, 1, 20)] * 40 for _ in range(10)])


def random_contour(rng, mw, mh, twice_area):
    x = rng.randint(0, mw - 1)
    y = rng.randint(0, mh - 1)
    return (x, y, rng.randint(1, mw - x), rng.randint(1, mh - y), twice_area)


for index in range(40):
    rng = random.Random(20261110 + 1000 * 4 + index)
    width, height, setting = rng.choice(SCALE_GEOMETRIES[:6])
    lightning = rng.choice(RANDOM_LIGHTNINGS)
    area = rng.choice(RANDOM_AREAS)
    alpha = rng.choice(RANDOM_ALPHAS)
    mw = setting * width // height
    pixels = setting * mw
    frames = []
    mode = None
    for _ in range(RANDOM_FRAMES):
        if mode is None or rng.random() >= 0.8:
            mode = rng.choice([m for m in (0, 1, 2) if m != mode])
        contours = []
        if mode == 0:
            contours = [random_contour(rng, mw, mh, rng.randint(0, 60)) for _ in range(rng.randint(0, 3))]
        elif mode == 1:
            contours = [random_contour(rng, mw, mh, rng.randint(10, 600)) for _ in range(rng.randint(5, 9))]
        else:
            count = rng.randint(1, 4)
            total = int(rng.uniform(0.5, 1.0) * 2 * pixels)
            base, extra = divmod(total, count)
            contours = [
                random_contour(rng, mw, mh, base + (1 if k < extra else 0)) for k in range(count)
            ]
        frames.append(contours)
    emit("random", width, height, lightning, area, alpha, setting, frames)

header = "counts " + " ".join(
    field for kind in ("scale", "filter", "fraction", "states", "random") for field in (kind, str(counts[kind]))
)
OUT.write_text("\n".join([header] + lines) + "\n", encoding="ascii")
