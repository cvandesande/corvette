r"""Runs Frigate's deployed region block over the per-picture box file that
corvette-detect-tools motion-detect writes and writes one line per picture in
the same format as corvette-detect-tools regions, so the two outputs can be
compared picture by picture. The squares are Frigate's own: nothing patches or
edits the block text, its inputs are the recorded motion boxes and calibration
flags.

Run command:

    cd /home/cvandesande/github/corvette && timeout 300 nix develop .#goldens --command python3 -I tools/goldens/regions_reference.py <Frigate checkout directory> --boxes FILE --width W --height H --out FILE [--model-size S]

Reference: the frigate repository, path frigate/util/image.py, tag v0.17.2,
blob ea9fb0a0a7f166d7b145c6c3af9941de673aebe3, lines 291 to 314
(calculate_region) and lines 719 to 720 (area); path frigate/util/object.py,
tag v0.17.2, blob b8f41e2c3214c22980f5227055aad348c9d6cd10, line 35
(GRID_SIZE), lines 270 to 284 (get_min_region_size), lines 151 to 163
(get_cluster_region_from_grid), lines 166 to 213 (get_region_from_grid), lines
313 to 317 (box_inside), lines 368 to 373 (inside_any), lines 376 to 393
(get_cluster_boundary), lines 396 to 443 (get_cluster_candidates), lines 446 to
458 (get_cluster_region) and lines 461 to 492 (get_startup_regions); path
frigate/video.py, tag v0.17.2, blob 38a39740410422ca3d9dae8ea8d42728f514cb87,
lines 899 to 945 (the region block of process_frames); each range asserted at
its first and last text and executed unchanged in one namespace defining np,
Any, math and ModelConfig. get_min_region_size is applied to a stub whose
height and width are --model-size, and its result is the block's
region_min_size for every picture. The video block is dedented and wrapped, its
text not edited, as the body of a function taking the block's free names
(frame_shape, region_min_size, object_boxes, motion_boxes, region_grid,
startup_scan, motion_detector, ptz_moving_at_frame_time, frame_time,
ptz_metrics) and returning regions and startup_scan. startup_scan is true
before the first picture and then whatever the block returned; object_boxes is
empty, region_grid is the empty startup grid, ptz_moving_at_frame_time returns
false, frame_time is 0.0 and the ptz stamp values are 0.0.

Input, one line per picture: index\tcalibrating\tboxes — index counts from 0
with no gap, calibrating is 0 or 1, and boxes is - or x1,y1,x2,y2 groups joined
by ;, every number ASCII digits. A final line without a newline is accepted.

Output, one line per picture: index\tregions — regions is - when the block
returned no square, else the squares x1,y1,x2,y2 sorted ascending by
(x1, y1, x2, y2) and joined by ;. Success writes one stdout line: pictures N,
with regions M, regions R.

Exit codes: 0 success; 2 usage, with the usage text on stderr; 3 input, output
or format error, with the message error: MESSAGE on stderr — a malformed line
names its 1-based line number and an exception from the block names the
picture's index. A number in the file above 2147483647 is an input error.
"""

import hashlib
import re
import subprocess
import sys
import textwrap
from pathlib import Path

import numpy as np
from typing import Any

IMAGE_BLOB = "ea9fb0a0a7f166d7b145c6c3af9941de673aebe3"
OBJECT_BLOB = "b8f41e2c3214c22980f5227055aad348c9d6cd10"
VIDEO_BLOB = "38a39740410422ca3d9dae8ea8d42728f514cb87"
GRID_SIZE = 8
DEFAULT_MODEL_SIZE = 640
MODEL_SIZE_MAX = 8192
# int() refuses very long digit strings; the domains below bound every number,
# so a longer digit string is refused before int() sees it.
COORDINATE_MAX = 2147483647  # i32 maximum: the Rust command's coordinate domain
U64_MAX = 18446744073709551615  # u64 maximum: the Rust command's width and height domain
INDEX_MAX_DIGITS = 18  # a longer index is reported, not echoed
# str.isdigit accepts other scripts' digits; the format is ASCII digits only.
NUMBER = re.compile(r"[0-9]+")
KNOWN_FLAGS = ("boxes", "width", "height", "out", "model-size")
REQUIRED_FLAGS = ("boxes", "width", "height", "out")
USAGE = (
    "usage: regions_reference.py <Frigate checkout directory> "
    "--boxes FILE --width W --height H --out FILE [--model-size S]"
)


class UsageError(Exception):
    """A command line problem: the usage text goes to stderr, exit code 2."""


class InputError(Exception):
    """An input, checkout or output problem: exit code 3."""


class CalibratingDetector:
    def __init__(self, calibrating):
        self.calibrating = calibrating

    def is_calibrating(self):
        return self.calibrating


class Stamp:
    value = 0.0


class PtzMetrics:
    start_time = Stamp()
    stop_time = Stamp()


class ModelStub:
    def __init__(self, model_size):
        self.height = model_size
        self.width = model_size


def digit_value(text, max_digits):
    """Return the value of an ASCII digit string with at most max_digits
    significant digits, or None when it has more.

    Leading zeros are not counted and a string of only zeros is 0.
    """
    digits = text.lstrip("0")
    if len(digits) > max_digits:
        return None
    return int(digits) if digits else 0


def parse_positive_int(flag, text):
    value = digit_value(text, len(str(U64_MAX))) if NUMBER.fullmatch(text) else None
    if value is None or not 1 <= value <= U64_MAX:
        raise UsageError(f"{flag} must be a positive integer, got {text!r}")
    return value


def parse_model_size(text):
    value = digit_value(text, len(str(MODEL_SIZE_MAX))) if NUMBER.fullmatch(text) else None
    if value is None or not 1 <= value <= MODEL_SIZE_MAX:
        raise UsageError(f"--model-size must be 1 to {MODEL_SIZE_MAX}, got {text!r}")
    return value


def parse_args(argv):
    values = {}
    positionals = []
    i = 0
    while i < len(argv):
        token = argv[i]
        if not token.startswith("--"):
            positionals.append(token)
            i += 1
            continue
        name = token[2:]
        if name not in KNOWN_FLAGS:
            raise UsageError(f"unknown flag: {token}")
        if name in values:
            raise UsageError(f"duplicated flag: {token}")
        if i + 1 >= len(argv) or argv[i + 1].startswith("--"):
            raise UsageError(f"flag {token} has no value")
        values[name] = argv[i + 1]
        i += 2
    if len(positionals) != 1:
        raise UsageError("expected exactly one Frigate checkout directory argument")
    for name in REQUIRED_FLAGS:
        if name not in values:
            raise UsageError(f"missing required flag: --{name}")
    return {
        "checkout": positionals[0],
        "boxes": values["boxes"],
        "width": parse_positive_int("--width", values["width"]),
        "height": parse_positive_int("--height", values["height"]),
        "out": values["out"],
        "model_size": parse_model_size(values.get("model-size", str(DEFAULT_MODEL_SIZE))),
    }


def blob_hash(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def source_lines(frigate_dir, path, blob):
    try:
        cmd = ["git", "-C", frigate_dir, "show", f"v0.17.2:{path}"]
        raw = subprocess.run(cmd, capture_output=True, check=True).stdout
    except (OSError, subprocess.SubprocessError) as error:
        raise InputError(f"cannot read v0.17.2:{path} from {frigate_dir}: {error}")
    if blob_hash(raw) != blob:
        raise InputError(f"the source of {path} does not hash to the quoted blob")
    return raw.decode("utf-8").splitlines()


def extract(lines, first, first_text, last, last_text, namespace):
    assert lines[first - 1].startswith(first_text), "unexpected line range"
    assert lines[last - 1].strip() == last_text, "unexpected line range"
    exec(compile("\n".join(lines[first - 1 : last]), "frigate", "exec"), namespace)


def extract_block(lines, namespace):
    assert lines[899 - 1].strip().startswith("# get consolidated regions for tracked objects"), \
        "unexpected line range"
    assert lines[945 - 1].strip() == "startup_scan = False", "unexpected line range"
    body = textwrap.dedent("\n".join(lines[899 - 1 : 945]))
    names = ("frame_shape, region_min_size, object_boxes, motion_boxes, region_grid, "
             "startup_scan, motion_detector, ptz_moving_at_frame_time, frame_time, ptz_metrics")
    source = ("def block(" + names + "):\n" + textwrap.indent(body, "    ")
              + "\n    return regions, startup_scan\n")
    exec(compile(source, "block", "exec"), namespace)


def frigate_namespace(frigate_dir):
    namespace = {"np": np, "Any": Any, "math": __import__("math"), "ModelConfig": Any}
    image = source_lines(frigate_dir, "frigate/util/image.py", IMAGE_BLOB)
    extract(image, 291, "def calculate_region", 314,
            "return (x_offset, y_offset, x_offset + size, y_offset + size)", namespace)
    extract(image, 719, "def area(box)", 720,
            "return (box[2] - box[0] + 1) * (box[3] - box[1] + 1)", namespace)
    obj = source_lines(frigate_dir, "frigate/util/object.py", OBJECT_BLOB)
    extract(obj, 35, "GRID_SIZE", 35, "GRID_SIZE = 8", namespace)
    extract(obj, 270, "def get_min_region_size", 284, "return 320", namespace)
    extract(obj, 313, "def box_inside(b1, b2)", 317, "return False", namespace)
    extract(obj, 368, "def inside_any(box_a, boxes)", 373, "return False", namespace)
    extract(obj, 376, "def get_cluster_boundary(box, min_region)", 393, "]", namespace)
    extract(obj, 396, "def get_cluster_candidates(frame_shape, min_region, boxes)", 443,
            "return [list(tup) for tup in unique]", namespace)
    extract(obj, 446, "def get_cluster_region(frame_shape, min_region, cluster, boxes)", 458,
            ")", namespace)
    extract(obj, 151, "def get_cluster_region_from_grid", 163, ")", namespace)
    extract(obj, 166, "def get_region_from_grid", 213, ")", namespace)
    extract(obj, 461, "def get_startup_regions", 492, "return regions", namespace)
    video = source_lines(frigate_dir, "frigate/video.py", VIDEO_BLOB)
    extract_block(video, namespace)
    return namespace


def parse_boxes(field, line_number):
    if field == "-":
        return []
    boxes = []
    for group in field.split(";"):
        numbers = group.split(",")
        if len(numbers) != 4 or not all(NUMBER.fullmatch(number) for number in numbers):
            raise InputError(f"line {line_number}: box must be x1,y1,x2,y2 of digits: {group!r}")
        box = []
        for number in numbers:
            value = digit_value(number, len(str(COORDINATE_MAX)))
            if value is None or value > COORDINATE_MAX:
                raise InputError(f"line {line_number}: a coordinate is out of range")
            box.append(value)
        boxes.append(tuple(box))
    return boxes


def read_rows(path):
    try:
        data = Path(path).read_bytes()
    except OSError as error:
        raise InputError(f"cannot read boxes file {path}: {error.strerror}")
    if not data:
        raise InputError(f"boxes file is empty: {path}")
    chunks = data.split(b"\n")
    if chunks[-1] == b"":
        chunks.pop()
    rows = []
    for index, chunk in enumerate(chunks):
        line_number = index + 1
        try:
            line = chunk.decode("utf-8")
        except UnicodeDecodeError:
            raise InputError(f"line {line_number}: not valid utf-8")
        fields = line.split("\t")
        if len(fields) != 3:
            raise InputError(
                f"line {line_number}: expected index, calibrating and boxes: found "
                f"{len(fields)} fields"
            )
        index_text, calibrating, boxes_field = fields
        if not NUMBER.fullmatch(index_text):
            raise InputError(f"line {line_number}: index must be digits: {index_text!r}")
        value = digit_value(index_text, INDEX_MAX_DIGITS)
        if value is None:
            raise InputError(f"line {line_number}: index is not the line number")
        if value != index:
            raise InputError(f"line {line_number}: index {value} is not {index}")
        if calibrating not in ("0", "1"):
            raise InputError(f"line {line_number}: calibrating must be 0 or 1: {calibrating!r}")
        rows.append((calibrating == "1", parse_boxes(boxes_field, line_number)))
    return rows


def run(options):
    namespace = frigate_namespace(options["checkout"])
    rows = read_rows(options["boxes"])
    frame_shape = (options["height"], options["width"])
    floor = namespace["get_min_region_size"](ModelStub(options["model_size"]))
    grid = [[{"sizes": []} for _ in range(GRID_SIZE)] for _ in range(GRID_SIZE)]
    lines = []
    with_regions = 0
    total_regions = 0
    startup_scan = True
    for index, (calibrating, boxes) in enumerate(rows):
        try:
            regions, startup_scan = namespace["block"](
                frame_shape, floor, [], boxes, grid, startup_scan,
                CalibratingDetector(calibrating), lambda *ignored: False, 0.0, PtzMetrics())
        except Exception as error:
            raise InputError(f"picture {index}: {error}")
        if regions:
            with_regions += 1
        total_regions += len(regions)
        squares = ";".join(
            ",".join(str(value) for value in square) for square in sorted(regions))
        lines.append(f"{index}\t{squares or '-'}")
    try:
        Path(options["out"]).write_text("".join(line + "\n" for line in lines),
                                        encoding="utf-8")
    except OSError as error:
        raise InputError(f"cannot write {options['out']}: {error.strerror}")
    print(f"pictures {len(rows)}, with regions {with_regions}, regions {total_regions}")


def main():
    try:
        run(parse_args(sys.argv[1:]))
    except UsageError as error:
        print(f"error: {error}", file=sys.stderr)
        print(USAGE, file=sys.stderr)
        sys.exit(2)
    except InputError as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(3)


if __name__ == "__main__":
    main()
