r"""Goldens for corvette-motion's MotionDetector: for each frame of a seeded
grey-picture sequence, the motion boxes and calibration flag the deployed
ImprovedMotionDetector.detect returns and the 64-bit FNV-1a hash (offset
0xcbf29ce484222325, prime 0x100000001b3) of the float32 bytes of its average
after that frame, plus the detector's own keep plane of the motion picture.

The numbers are what the deployed class itself writes: the script takes the
Frigate checkout directory as its second argument, reads
frigate/motion/improved_motion.py at tag v0.17.2 from it with git show, refuses
the text unless its bytes hash to the blob quoted below, installs stand-in
modules for its frigate imports, compiles and executes the text in a fresh
module namespace and runs the class unchanged. cv2 comes from the pinned wheel
opencv_python_headless-4.11.0.86-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
(sha256
0e0a27c19dd1f40ddff94976cfe43066fbbe9dfbb2ec1907d66c19caef42a57b), the directory
it was extracted into being the first argument; the wheel's native libraries
need two loader directories, so the full run command is:

    nix develop .#goldens --command env \
        LD_LIBRARY_PATH=/nix/store/6s4wm3zl2y0v5g998n2g88k1zh3vf2pr-zlib-1.3.2/lib:/nix/store/0ba5kj6r0zhfwd84l5vzccqvx7amlq2c-gcc-15.3.0-lib/lib \
        python3 -I tools/goldens/motion_detector.py <extracted wheel directory> <Frigate checkout directory>

Measured with the wheel and the goldens shell: boxes, calibration flag, average
hash and keep plane, frame by frame, by running the class. The versions the run
that wrote the fixture printed: Python 3.14.6, numpy 2.5.1, scipy 1.18.0,
cv2 4.11.0 with version-control line "4.11.0" and cv2.ipp.useIPP() True. The
script prints them again on every run and refuses to write the fixture unless
cv2 is 4.11.0. Read, not measured: the quoted lines below; the autotracking
branches, the disabled-motion early return and the debug-image branches, which
the stand-ins and settings keep unreachable. The deployed image's own numpy,
scipy and OpenCV builds are not inspected here: this fixture is evidence about
the versions printed above, nothing else.

Reference: the frigate repository, path frigate/motion/improved_motion.py, tag
v0.17.2, blob b081d37919541e6d1f95762ce084abde81774038, lines 73 to 80 (grey
slice and resize), 85 to 106 (contrast stretch), 111 to 116 (mask zeroing,
after the stretch, then the blur), 124 to 137 (difference, threshold, dilation,
contours) and 234 to 246 (update_mask, INTER_AREA resize then np.where), copied
verbatim:

        gray = frame[0 : self.frame_shape[0], 0 : self.frame_shape[1]]

        # resize frame
        resized_frame = cv2.resize(
            gray,
            dsize=(self.motion_frame_size[1], self.motion_frame_size[0]),
            interpolation=self.interpolation,
        )

        # Improve contrast
        if self.config.improve_contrast:
            # TODO tracking moving average of min/max to avoid sudden contrast changes
            min_value = np.percentile(resized_frame, 4).astype(np.uint8)
            max_value = np.percentile(resized_frame, 96).astype(np.uint8)
            # skip contrast calcs if the image is a single color
            if min_value < max_value:
                # keep track of the last 50 contrast values
                self.contrast_values[self.contrast_values_index] = [
                    min_value,
                    max_value,
                ]
                self.contrast_values_index += 1
                if self.contrast_values_index == len(self.contrast_values):
                    self.contrast_values_index = 0

                avg_min, avg_max = np.mean(self.contrast_values, axis=0)

                resized_frame = np.clip(resized_frame, avg_min, avg_max)
                resized_frame = (
                    ((resized_frame - avg_min) / (avg_max - avg_min)) * 255
                ).astype(np.uint8)

        # mask frame
        # this has to come after contrast improvement
        # Setting masked pixels to zero, to match the average frame at startup
        resized_frame[self.mask] = [0]

        resized_frame = gaussian_filter(resized_frame, sigma=1, radius=self.blur_radius)

        # compare to average
        frameDelta = cv2.absdiff(resized_frame, cv2.convertScaleAbs(self.avg_frame))

        # compute the threshold image for the current frame
        thresh = cv2.threshold(
            frameDelta, self.config.threshold, 255, cv2.THRESH_BINARY
        )[1]

        # dilate the thresholded image to fill in holes, then find contours
        # on thresholded image
        thresh_dilated = cv2.dilate(thresh, None, iterations=1)
        contours = cv2.findContours(
            thresh_dilated, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE
        )
        contours = grab_cv2_contours(contours)

    def update_mask(self) -> None:
        resized_mask = cv2.resize(
            self.config.mask,
            dsize=(self.motion_frame_size[1], self.motion_frame_size[0]),
            interpolation=cv2.INTER_AREA,
        )
        self.mask = np.where(resized_mask == [0])

        # Reset motion detection state when mask changes
        # so motion detection can quickly recalibrate with the new mask
        self.avg_frame = np.zeros(self.motion_frame_size, np.float32)
        self.calibrating = True
        self.motion_frame_count = 0

The contour loop, the calibration rules and the average-update rule (lines 139
to 158, 179 to 185 and 214 to 230 of the same blob) are quoted verbatim in
tools/goldens/motion_decision.py.

Reference: the frigate repository, path frigate/util/image.py, tag v0.17.2,
blob ea9fb0a0a7f166d7b145c6c3af9941de673aebe3, lines 273 to 283
(grab_cv2_contours, quoted verbatim as the stand-in body) and 925 to 943
(add_mask, filling a polygon with zero on a 255-filled picture, which the
masked and random kinds construct with cv2.fillPoly), copied verbatim:

def grab_cv2_contours(cnts):
    # if the length the contours tuple returned by cv2.findContours
    # is '2' then we are using either OpenCV v2.4, v4-beta, or
    # v4-official
    if len(cnts) == 2:
        return cnts[0]

    # if the length of the contours tuple is '3' then we are using
    # either OpenCV v3, v4-pre, or v4-alpha
    elif len(cnts) == 3:
        return cnts[1]

def add_mask(mask: str, mask_img: np.ndarray):
    points = mask.split(",")

    # masks and zones are saved as relative coordinates
    # we know if any points are > 1 then it is using the
    # old native resolution coordinates
    if any(x > "1.0" for x in points):
        raise Exception("add mask expects relative coordinates only")

    contour = np.array(
        [
            [
                int(float(points[i]) * mask_img.shape[1]),
                int(float(points[i + 1]) * mask_img.shape[0]),
            ]
            for i in range(0, len(points), 2)
        ]
    )
    cv2.fillPoly(mask_img, pts=[contour], color=(0))

Writes the fixture crates/corvette-motion/tests/detector.rs reads: a first line
"counts object N1 flash N2 masked N3 flat N4 random N5", then for each sequence
a line "sequence <kind> <index> <frame_width> <frame_height> <setting>
<threshold> <lightning> <improve_contrast> <contour_area> <frame_alpha>
<frames>" (lightning and frame_alpha are Python repr floats, improve_contrast
is 0 or 1), then its mask line — "mask -" for no mask (the configuration holds
the all-255 picture) or "mask <motion_width * motion_height characters 0 or 1>",
row-major over the motion picture, 0 where the detector's own mask index lists
the pixel — then <frames> lines "frame <frame_width * frame_height * 2
lowercase hex digits> = <box_count> <x1> <y1> <x2> <y2> ... <calibrating>
<average_hash>" with calibrating 0 or 1 and average_hash 16 lowercase hex
digits. The motion picture is motion_width = setting * frame_width //
frame_height by motion_height = setting. A sequence's frames run in order
through one detector; state carries from frame to frame and starts fresh with
each sequence line. The kinds appear in the header's order.

Geometries (frame_width, frame_height, setting) with factor f = frame_height /
setting: g0 24x18 setting 9, g1 30x18 setting 6, g2 35x25 setting 15, g3 16x12
setting 12. Scene model, every kind: the background value at column x, row y is
60 + 4 * ((3 * x + 5 * y) % 16); noise, the amplitude of an independent uniform
integer added to every pixel of every frame, is drawn from the sequence's
random.Random(20261110 + 1000 * kind_number + index) (kind number from 0 in the
header's order, index within the kind from 0); the picture is clipped 0 to 255.

Kinds. object: 4 sequences on g0, g1, g2, g3 in that order, 60 frames each,
threshold 30, lightning 0.8, improve_contrast 1 for g0 and g1 and 0 for g2 and
g3, contour_area 2, frame_alpha 0.01, noise 2; from frame index 30 through 54 a
square of side s = round(3 * f) and value 220 with top row min(round(4 * f),
frame_height - s) and left column min(round((2 + i - 30) * f), frame_width -
s). flash: 2 sequences on g0 and g2, 50 frames each, improve_contrast 0,
threshold 30, lightning 0.8, contour_area 2, frame_alpha 0.01, noise 2; frames
35, 36 and 37 add 100 to every pixel before the clip. masked: 3 sequences on
g0, g2 and g3, 50 frames each, improve_contrast 1, threshold 30, lightning 0.8,
contour_area 2, frame_alpha 0.01, noise 2; columns x0 = int(0.6 * frame_width)
and up hold 200 instead of the background and are masked by a fillPoly rectangle
covering them; frames 25 through 44 also draw a square of value 255 and side
max(2, round(3 * f)) with top row 2 and left column 1 + (i - 25) % max(1, x0 -
s - 1), and a square of value 0 with the same side at top row frame_height - s -
1 and left column x0 + 1; the same frames run a second time with the all-255
mask only to assert the runs differ in an average hash. flat: 2 sequences on g1
and g3, 30 frames each, improve_contrast 1, threshold 30, lightning 0.8,
contour_area 2, frame_alpha 0.01, noise 0; frames 0 to 11 hold 90 everywhere,
frames 12 to 19 hold 40 on the left half of the columns and 200 on the rest,
frames 20 to 29 hold 120 everywhere; the detector's contrast_values_index must
not change on a flat frame and must change on a two-level frame. random: 6
sequences on g0, g1, g2, g3, g0, g1, 40 frames each; threshold from 10, 20, 30,
50; lightning from 0.5, 0.8, 1.0; improve_contrast 0 or 1; contour_area from 0,
2, 5, 10; frame_alpha from 0.01, 0.05, 0.2, 0.5; noise from 0, 1, 2, 4; each
frame draws 0 to 3 rectangles of random position, size and value, and with
probability 0.1 a global offset of -120 to 120 on every pixel; with probability
1/3 the sequence masks a random rectangle of at least a tenth of the frame.
"""

import hashlib
import platform
import random
import subprocess
import sys
import types
from pathlib import Path

sys.path.insert(0, sys.argv[1])
import cv2
import numpy as np

OUT = Path(__file__).resolve().parents[2] / "crates/corvette-motion/tests/fixtures/detector_cases.txt"

MOTION_PATH = "frigate/motion/improved_motion.py"
MOTION_BLOB = "b081d37919541e6d1f95762ce084abde81774038"
GEOMETRIES = [(24, 18, 9), (30, 18, 6), (35, 25, 15), (16, 12, 12)]
FNV_OFFSET = 0xcbf29ce484222325
FNV_PRIME = 0x100000001b3
KINDS = ["object", "flash", "masked", "flat", "random"]
SEED = 20261110

_deployed = None


def fnv1a(data):
    value = FNV_OFFSET
    for byte in data:
        value ^= byte
        value = (value * FNV_PRIME) % (1 << 64)
    return value


def blob_hash(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


class _Any:
    pass


class _Stopped:
    @staticmethod
    def is_set():
        return True


class _Ptz:
    def __init__(self):
        self.autotracker_enabled = _Any()
        self.autotracker_enabled.value = 0
        self.motor_stopped = _Stopped()
        self.stop_time = _Any()
        self.stop_time.value = 0


class _Config:
    def __init__(self, frame_height, mask):
        self.enabled = True
        self.frame_height = frame_height
        self.mask = mask


def _stand_ins():
    if "frigate.motion" in sys.modules:
        return
    frigate = types.ModuleType("frigate")
    camera = types.ModuleType("frigate.camera")
    camera.PTZMetrics = _Any
    config = types.ModuleType("frigate.config")
    config.MotionConfig = _Any
    motion = types.ModuleType("frigate.motion")

    class MotionDetector:
        pass

    motion.MotionDetector = MotionDetector
    util = types.ModuleType("frigate.util")
    util_image = types.ModuleType("frigate.util.image")

    def grab_cv2_contours(cnts):
        # if the length the contours tuple returned by cv2.findContours
        # is '2' then we are using either OpenCV v2.4, v4-beta, or
        # v4-official
        if len(cnts) == 2:
            return cnts[0]

        # if the length of the contours tuple is '3' then we are using
        # either OpenCV v3, v4-pre, or v4-alpha
        elif len(cnts) == 3:
            return cnts[1]

    util_image.grab_cv2_contours = grab_cv2_contours
    frigate.camera = camera
    frigate.config = config
    frigate.motion = motion
    frigate.util = util
    util.image = util_image
    for module in [frigate, camera, config, motion, util, util_image]:
        sys.modules[module.__name__] = module


def deployed_class(frigate_dir):
    """Returns the deployed ImprovedMotionDetector class, loaded once from the checkout."""
    global _deployed
    if _deployed is not None:
        return _deployed
    text = subprocess.run(
        ["git", "-C", frigate_dir, "show", "v0.17.2:" + MOTION_PATH],
        capture_output=True,
        check=True,
    ).stdout
    assert blob_hash(text) == MOTION_BLOB, "the source does not hash to the quoted blob"
    _stand_ins()
    namespace = {"__name__": "deployed_improved_motion"}
    exec(compile(text.decode("ascii"), MOTION_PATH, "exec"), namespace)
    _deployed = namespace["ImprovedMotionDetector"]
    return _deployed


def build_detector(frame_height, frame_width, settings, mask, frigate_dir):
    """Returns a deployed detector for (height, width), settings (threshold, lightning_threshold,
    improve_contrast, contour_area, frame_alpha, frame_height) and a frame-sized keep mask or None."""
    detector_class = deployed_class(frigate_dir)
    if mask is None:
        mask = np.full((frame_height, frame_width), 255, np.uint8)
    config = _Config(settings["frame_height"], mask)
    config.threshold = settings["threshold"]
    config.lightning_threshold = settings["lightning_threshold"]
    config.improve_contrast = settings["improve_contrast"]
    config.contour_area = settings["contour_area"]
    config.frame_alpha = settings["frame_alpha"]
    return detector_class((frame_height, frame_width), config, 5, ptz_metrics=_Ptz())


def detect(detector, rows):
    """Feeds one grey picture (a list of int rows) as the deployed call passes it and records."""
    height, width = len(rows), len(rows[0])
    frame = np.full((height * 3 // 2, width), 128, np.uint8)
    frame[0:height, 0:width] = np.clip(np.array(rows), 0, 255).astype(np.uint8)
    boxes = [tuple(int(value) for value in box) for box in detector.detect(frame)]
    return boxes, detector.is_calibrating(), fnv1a(detector.avg_frame.tobytes())


def keep_string(detector):
    height, width = detector.motion_frame_size
    rows, columns = detector.mask
    masked = {(int(row), int(column)) for row, column in zip(rows, columns)}
    return "".join("0" if (y, x) in masked else "1" for y in range(height) for x in range(width))


def background(width, height):
    return [[60 + 4 * ((3 * x + 5 * y) % 16) for x in range(width)] for y in range(height)]


def add_noise(rows, rng, noise):
    for row in rows:
        for position, value in enumerate(row):
            row[position] = value + rng.randint(-noise, noise)


def rectangle(rows, top, left, side, value):
    for y in range(top, top + side):
        for x in range(left, left + side):
            rows[y][x] = value


def mask_rectangle(width, height, corners):
    mask = np.full((height, width), 255, np.uint8)
    cv2.fillPoly(mask, pts=[np.array(corners, np.int32)], color=(0))
    return mask


def settings(threshold, lightning, improve, area, alpha, setting):
    return {
        "threshold": threshold,
        "lightning_threshold": lightning,
        "improve_contrast": improve,
        "contour_area": area,
        "frame_alpha": alpha,
        "frame_height": setting,
    }


def run(kind, index, geometry, options, frigate_dir):
    """Returns the sequence's three kinds of line plus its per-frame results."""
    width, height, setting = geometry
    rng = random.Random(SEED + 1000 * KINDS.index(kind) + index)
    frames, mask = options(width, height, setting, rng)
    detector = build_detector(height, width, options.settings, mask, frigate_dir)
    lines = [
        " ".join(
            [
                "sequence",
                kind,
                str(index),
                str(width),
                str(height),
                str(setting),
                str(options.settings["threshold"]),
                repr(options.settings["lightning_threshold"]),
                "1" if options.settings["improve_contrast"] else "0",
                str(options.settings["contour_area"]),
                repr(options.settings["frame_alpha"]),
                str(len(frames)),
            ]
        ),
        "mask -" if mask is None else "mask " + keep_string(detector),
    ]
    results = [detect(detector, rows) for rows in frames]
    lines += [
        "frame " + np.clip(np.array(rows), 0, 255).astype(np.uint8).tobytes().hex()
        + " = "
        + " ".join(
            [str(len(boxes))]
            + [str(value) for box in boxes for value in box]
            + ["1" if calibrating else "0", f"{average_hash:016x}"]
        )
        for rows, (boxes, calibrating, average_hash) in zip(frames, results)
    ]
    return lines, results, mask


def attach(options, current_settings):
    options.settings = current_settings
    return options


def object_options(index):
    def options(width, height, setting, rng):
        factor = height / setting
        base = background(width, height)
        frames = []
        for i in range(60):
            rows = [row[:] for row in base]
            if 30 <= i <= 54:
                side = round(3 * factor)
                top = min(round(4 * factor), height - side)
                left = min(round((2 + i - 30) * factor), width - side)
                rectangle(rows, top, left, side, 220)
            add_noise(rows, rng, 2)
            frames.append(rows)
        return frames, None

    return attach(
        options,
        settings(30, 0.8, index < 2, 2, 0.01, GEOMETRIES[index][2]),
    )


def flash_options(index):
    def options(width, height, setting, rng):
        base = background(width, height)
        frames = []
        for i in range(50):
            rows = [row[:] for row in base]
            if i in (35, 36, 37):
                for row in rows:
                    for position, value in enumerate(row):
                        row[position] = value + 100
            add_noise(rows, rng, 2)
            frames.append(rows)
        return frames, None

    geometry = GEOMETRIES[[0, 2][index]]
    return attach(options, settings(30, 0.8, False, 2, 0.01, geometry[2]))


def masked_options(index):
    def options(width, height, setting, rng):
        factor = height / setting
        x0 = int(0.6 * width)
        side = max(2, round(3 * factor))
        frames = []
        for i in range(50):
            rows = [[200 if x >= x0 else value for x, value in enumerate(row)] for row in background(width, height)]
            if 25 <= i <= 44:
                rectangle(rows, 2, 1 + (i - 25) % max(1, x0 - side - 1), side, 255)
                rectangle(rows, height - side - 1, x0 + 1, side, 0)
            add_noise(rows, rng, 2)
            frames.append(rows)
        mask = mask_rectangle(
            width,
            height,
            [(x0, 0), (width - 1, 0), (width - 1, height - 1), (x0, height - 1)],
        )
        return frames, mask

    geometry = GEOMETRIES[[0, 2, 3][index]]
    return attach(options, settings(30, 0.8, True, 2, 0.01, geometry[2]))


def flat_options(index):
    def options(width, height, setting, rng):
        frames = []
        for i in range(30):
            if i < 12:
                rows = [[90] * width for _ in range(height)]
            elif i < 20:
                rows = [[40 if x < width // 2 else 200 for x in range(width)] for _ in range(height)]
            else:
                rows = [[120] * width for _ in range(height)]
            add_noise(rows, rng, 0)
            frames.append(rows)
        return frames, None

    geometry = GEOMETRIES[[1, 3][index]]
    return attach(options, settings(30, 0.8, True, 2, 0.01, geometry[2]))


RANDOM_GEOMETRY = [0, 1, 2, 3, 0, 1]


def random_options(index):
    def options(width, height, setting, rng):
        mask = None
        if rng.random() < 1.0 / 3.0:
            while True:
                side_w = rng.randint(1, width)
                side_h = rng.randint(1, height)
                if side_w * side_h * 10 >= width * height:
                    break
            left = rng.randrange(width - side_w + 1)
            top = rng.randrange(height - side_h + 1)
            mask = mask_rectangle(
                width,
                height,
                [(left, top), (left + side_w - 1, top), (left + side_w - 1, top + side_h - 1), (left, top + side_h - 1)],
            )
        frames = []
        for _ in range(40):
            rows = [row[:] for row in background(width, height)]
            for _ in range(rng.randint(0, 3)):
                left = rng.randrange(width)
                top = rng.randrange(height)
                side_w = rng.randint(1, width - left)
                side_h = rng.randint(1, height - top)
                value = rng.randint(0, 255)
                for y in range(top, top + side_h):
                    for x in range(left, left + side_w):
                        rows[y][x] = value
            if rng.random() < 0.1:
                offset = rng.randint(-120, 120)
                for row in rows:
                    for position, value in enumerate(row):
                        row[position] = value + offset
            add_noise(rows, rng, RANDOM_NOISE[index])
            frames.append(rows)
        return frames, mask

    geometry = GEOMETRIES[RANDOM_GEOMETRY[index]]
    return attach(
        options,
        settings(
            [10, 20, 30, 50][index % 4],
            [0.5, 0.8, 1.0][index % 3],
            index % 2 == 1,
            [0, 2, 5, 10][index % 4],
            [0.01, 0.05, 0.2, 0.5][index % 4],
            geometry[2],
        ),
    )


RANDOM_NOISE = [0, 1, 2, 4, 0, 1]


def assert_object(index, results):
    calibrating = [flag for _boxes, flag, _hash in results]
    boxes = [found for found, _flag, _hash in results]
    first = next((i for i, flag in enumerate(calibrating) if not flag), None)
    assert first is not None and first <= 20, (index, first)
    if index in (2, 3):
        assert not calibrating[29], (index, 29)
    assert any(boxes), index
    run_length = best = 0
    for found in boxes:
        run_length = run_length + 1 if found else 0
        best = max(best, run_length)
    assert best >= 10, (index, best)


def assert_flash(index, results):
    calibrating = [flag for _boxes, flag, _hash in results]
    assert not calibrating[34], (index, 34)
    assert calibrating[35] and calibrating[36] and calibrating[37], (index, 35, 36, 37)
    assert not calibrating[45], (index, 45)


def assert_flat(index, results, detector_state):
    before, after = detector_state
    for i, changed in enumerate(zip(before, after)):
        flat_frame = i < 12 or i >= 20
        assert (changed[0] != changed[1]) != flat_frame, (index, i)


def main():
    print("python", platform.python_version())
    print("numpy", np.__version__)
    import scipy

    print("scipy", scipy.__version__)
    print("cv2", cv2.__version__)
    for line in cv2.getBuildInformation().splitlines():
        if "Version control" in line:
            print(line.strip())
    print("cv2.ipp.useIPP()", cv2.ipp.useIPP())
    assert cv2.__version__ == "4.11.0", cv2.__version__
    frigate_dir = sys.argv[2]
    blocks = {"object": [], "flash": [], "masked": [], "flat": [], "random": []}
    sequences = {"object": 4, "flash": 2, "masked": 3, "flat": 2, "random": 6}

    for index in range(4):
        lines, results, _mask = run("object", index, GEOMETRIES[index], object_options(index), frigate_dir)
        assert_object(index, results)
        blocks["object"] += lines

    for index in range(2):
        lines, results, _mask = run("flash", index, GEOMETRIES[[0, 2][index]], flash_options(index), frigate_dir)
        assert_flash(index, results)
        blocks["flash"] += lines

    for index in range(3):
        width, height, setting = GEOMETRIES[[0, 2, 3][index]]
        options = masked_options(index)
        lines, results, mask = run("masked", index, (width, height, setting), options, frigate_dir)
        assert "0" in lines[1] and "1" in lines[1], index
        # the second run with the all-255 mask exists only for the difference assertion
        frames, _ignored = options(width, height, setting, random.Random(SEED + 1000 * 2 + index))
        plain = build_detector(height, width, options.settings, None, frigate_dir)
        second = [detect(plain, rows) for rows in frames]
        assert any(first[2] != other[2] for first, other in zip(results, second)), index
        blocks["masked"] += lines

    for index in range(2):
        geometry = GEOMETRIES[[1, 3][index]]
        options = flat_options(index)
        width, height, setting = geometry
        detector = build_detector(height, width, options.settings, None, frigate_dir)
        frames, _mask = options(width, height, setting, random.Random(SEED + 1000 * 3 + index))
        before, after = [], []
        for rows in frames:
            before.append(detector.contrast_values_index)
            detect(detector, rows)
            after.append(detector.contrast_values_index)
        lines, results, _mask = run("flat", index, geometry, options, frigate_dir)
        assert_flat(index, results, (before, after))
        blocks["flat"] += lines

    seen = set()
    any_box = False
    for index in range(6):
        lines, results, _mask = run("random", index, GEOMETRIES[RANDOM_GEOMETRY[index]], random_options(index), frigate_dir)
        for i, (boxes, flag, _hash) in enumerate(results):
            if i > 20:
                seen.add(flag)
            any_box = any_box or bool(boxes)
        assert any(bool(boxes) for boxes, _flag, _hash in results), index
        blocks["random"] += lines
    assert seen == {True, False}, seen
    assert any_box

    counts = "counts " + " ".join(f"{kind} {sequences[kind]}" for kind in KINDS)
    text = "\n".join([counts] + [line for kind in KINDS for line in blocks[kind]]) + "\n"
    OUT.write_text(text, encoding="ascii")


main()
