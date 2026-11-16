r"""Runs Frigate's deployed ImprovedMotionDetector over a raw luma file and
writes one line per picture in the same format as corvette-detect-tools
motion-detect, so the two outputs can be compared picture by picture. The
boxes and the calibration flag are measured by running the deployed class;
the setting defaults are read from it: threshold 30, lightning threshold
0.8, contrast improvement on, contour area 10, frame alpha 0.01 and frame
height 100, the deployed MotionConfig defaults the Rust settings match.

The loader is a deliberate copy of the one in tools/goldens/motion_detector.py:
that file runs its fixture generator and rewrites a fixture at import time, so
this script cannot import it; consolidating the two loaders is a separate
item. Each deployed text is refused unless its git blob hashes to the quoted
value, and it then runs unchanged in a fresh namespace; nothing patches, wraps
or subclasses ImprovedMotionDetector and nothing alters its config after
construction.

Run command (the wheel's native libraries need the two loader directories):

    nix develop .#goldens --command env \
        LD_LIBRARY_PATH=/nix/store/fkcbg2c1w29jr5yp9awai9w3v1wvxdk9-zlib-1.3.2/lib:/nix/store/w2nk3f7prwzpm6h13rfxh8lh46yfdysj-gcc-15.3.0-lib/lib \
        python3 -I tools/goldens/motion_reference.py <extracted wheel directory> <Frigate checkout directory> \
        --luma FILE --width W --height H --out FILE [--threshold T] [--contour-area A]
        [--improve-contrast 0|1] [--lightning-threshold L] [--frame-alpha F]
        [--frame-height H] [--mask x1,y1,x2,y2,...]

cv2 comes from the pinned wheel
opencv_python_headless-4.11.0.86-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
(sha256 0e0a27c19dd1f40ddff94976cfe43066fbbe9dfbb2ec1907d66c19caef42a57b) and
the run refuses any other cv2 version.

Reference: the frigate repository, tag v0.17.2,
frigate/motion/improved_motion.py blob b081d37919541e6d1f95762ce084abde81774038
(the whole ImprovedMotionDetector text is executed) and frigate/util/image.py
blob ea9fb0a0a7f166d7b145c6c3af9941de673aebe3 lines 911 to 943 (create_mask
with add_mask, executed whole: a 255-filled picture, polygon interiors
filled 0, and a point beyond 1.0 refused by a lexicographic compare).

Exit codes match the Rust tool: 0 success, 2 usage, 3 input, output or
format error. The per-picture line and the one stdout summary line match the
Rust tool's form; the versions line goes to stderr.
"""

import argparse
import hashlib
import math
import os
import platform
import subprocess
import sys
import types

sys.path.insert(0, sys.argv[1])
import cv2
import numpy as np

MOTION_PATH = "frigate/motion/improved_motion.py"
MOTION_BLOB = "b081d37919541e6d1f95762ce084abde81774038"
IMAGE_PATH = "frigate/util/image.py"
IMAGE_BLOB = "ea9fb0a0a7f166d7b145c6c3af9941de673aebe3"
MASK_SLICE_FIRST = 910
MASK_SLICE_LAST = 943

_deployed = None


def blob_hash(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def die(message):
    print(f"motion_reference: {message}", file=sys.stderr)
    sys.exit(3)


def die_usage(message):
    print(f"motion_reference: error: {message}", file=sys.stderr)
    sys.exit(2)


def git_show(frigate_dir, ref):
    try:
        return subprocess.run(
            ["git", "-C", frigate_dir, "show", ref],
            capture_output=True,
            check=True,
        ).stdout
    except (OSError, subprocess.SubprocessError) as error:
        die(f"cannot read {ref} from {frigate_dir}: {error}")


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
    text = git_show(frigate_dir, "v0.17.2:" + MOTION_PATH)
    assert blob_hash(text) == MOTION_BLOB, "the source does not hash to the quoted blob"
    _stand_ins()
    namespace = {"__name__": "deployed_improved_motion"}
    exec(compile(text.decode("ascii"), MOTION_PATH, "exec"), namespace)
    _deployed = namespace["ImprovedMotionDetector"]
    return _deployed


def deployed_create_mask(frigate_dir):
    """Returns the deployed create_mask text, blob-checked, from lines 911 to 943 of image.py."""
    raw = git_show(frigate_dir, "v0.17.2:" + IMAGE_PATH)
    assert blob_hash(raw) == IMAGE_BLOB, "the source does not hash to the quoted blob"
    namespace = {"np": np, "cv2": cv2}
    exec(
        compile(
            "\n".join(raw.decode("utf-8").splitlines()[MASK_SLICE_FIRST:MASK_SLICE_LAST]) + "\n",
            IMAGE_PATH,
            "exec",
        ),
        namespace,
    )
    return namespace["create_mask"]


def threshold_value(text):
    try:
        value = int(text)
    except ValueError:
        raise argparse.ArgumentTypeError(f"expected a whole number, got {text!r}")
    if not 0 <= value <= 255:
        raise argparse.ArgumentTypeError(f"expected 0 to 255, got {value}")
    return value


def finite_value(text):
    try:
        value = float(text)
    except ValueError:
        raise argparse.ArgumentTypeError(f"expected a number, got {text!r}")
    if not math.isfinite(value):
        raise argparse.ArgumentTypeError(f"expected a finite number, got {text}")
    return value


def parse(argv):
    """Returns the settings flags parsed from argv[3:]; the two directory positions are read by main."""
    parser = argparse.ArgumentParser(
        prog="motion_reference.py",
        description="runs Frigate's deployed ImprovedMotionDetector over a raw luma file",
    )
    parser.add_argument("--luma", required=True, help="raw luma file, width by height bytes per picture")
    parser.add_argument("--width", required=True, type=int)
    parser.add_argument("--height", required=True, type=int)
    parser.add_argument("--out", required=True)
    parser.add_argument("--threshold", type=threshold_value, default=30)
    parser.add_argument("--contour-area", type=int, default=10)
    parser.add_argument("--improve-contrast", choices=("0", "1"), default="1")
    parser.add_argument("--lightning-threshold", type=finite_value, default=0.8)
    parser.add_argument("--frame-alpha", type=finite_value, default=0.01)
    parser.add_argument("--frame-height", type=int, default=100)
    parser.add_argument("--mask", help="x,y fraction pairs, at least three")
    return parser.parse_args(argv[3:])


def mask_picture(frigate_dir, height, width, text):
    """Returns the mask picture the deployed create_mask returns, or all-255 with no --mask."""
    if text is None:
        return np.full((height, width), 255, np.uint8)
    numbers = []
    for part in text.split(","):
        try:
            numbers.append(float(part))
        except ValueError:
            die_usage(f"--mask is not a list of numbers: {text}")
    if len(numbers) < 6 or len(numbers) % 2 == 1:
        die_usage(f"--mask needs at least three x,y pairs: {text}")
    try:
        return deployed_create_mask(frigate_dir)((height, width), text)
    except Exception as error:
        die(f"the deployed create_mask refused --mask {text}: {error}")


def build_detector(frame_height, frame_width, settings, mask, frigate_dir):
    """Returns a deployed detector for (height, width), settings and a frame-sized keep mask or None."""
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


def detect(detector, index, luma, width, height):
    """Feeds one grey picture as the deployed call passes it and returns its boxes and flag."""
    frame = np.full((height * 3 // 2, width), 128, np.uint8)
    frame[0:height, 0:width] = luma.reshape(height, width)
    try:
        boxes = [tuple(int(value) for value in box) for box in detector.detect(frame)]
    except Exception as error:
        die(f"the deployed class failed on picture {index}: {error}")
    return boxes, detector.is_calibrating()


def format_line(index, calibrating, boxes):
    boxes_text = ";".join(",".join(str(value) for value in box) for box in boxes) if boxes else "-"
    return f"{index}\t{'1' if calibrating else '0'}\t{boxes_text}\n"


def open_input(path):
    try:
        return open(path, "rb")
    except OSError as error:
        die(f"cannot read {path}: {error}")


def open_output(path):
    try:
        return open(path, "w")
    except OSError as error:
        die(f"cannot write {path}: {error}")


def replay(source, sink, detector, picture_bytes, width, height, pictures):
    """Feeds each picture in order, writes one line per picture, returns the two counts."""
    with_boxes = 0
    calibrating = 0
    for index in range(pictures):
        chunk = source.read(picture_bytes)
        if len(chunk) != picture_bytes:
            die(f"cannot read picture {index} from {source.name}")
        boxes, is_calibrating = detect(detector, index, np.frombuffer(chunk, np.uint8), width, height)
        with_boxes += 1 if boxes else 0
        calibrating += 1 if is_calibrating else 0
        try:
            sink.write(format_line(index, is_calibrating, boxes))
        except OSError as error:
            die(f"cannot write a line: {error}")
    return with_boxes, calibrating


def settings_for(args):
    return {
        "threshold": args.threshold,
        "lightning_threshold": args.lightning_threshold,
        "improve_contrast": args.improve_contrast == "1",
        "contour_area": args.contour_area,
        "frame_alpha": args.frame_alpha,
        "frame_height": args.frame_height,
    }


def print_versions():
    print("python", platform.python_version(), file=sys.stderr)
    print("numpy", np.__version__, file=sys.stderr)
    import scipy

    print("scipy", scipy.__version__, file=sys.stderr)
    print("cv2", cv2.__version__, file=sys.stderr)
    assert cv2.__version__ == "4.11.0", cv2.__version__


def main():
    print_versions()
    args = parse(sys.argv)
    if args.width < 1 or args.height < 1:
        die(f"no picture shape for {args.width} by {args.height}")
    if args.frame_height < 1:
        die(f"--frame-height must be at least 1, got {args.frame_height}")
    if args.contour_area < 0:
        die_usage(f"--contour-area must not be negative, got {args.contour_area}")
    detector = build_detector(
        args.height, args.width, settings_for(args), mask_picture(sys.argv[2], args.height, args.width, args.mask), sys.argv[2]
    )
    source = open_input(args.luma)
    picture_bytes = args.width * args.height
    size = os.fstat(source.fileno()).st_size
    if size == 0 or size % picture_bytes != 0:
        die(f"{args.luma} is {size} bytes, a whole picture is {picture_bytes} bytes")
    sink = open_output(args.out)
    pictures = size // picture_bytes
    with_boxes, calibrating = replay(source, sink, detector, picture_bytes, args.width, args.height, pictures)
    try:
        sink.close()
    except OSError as error:
        die(f"cannot finish writing {args.out}: {error}")
    print(f"pictures {pictures}, with boxes {with_boxes}, calibrating {calibrating}")


if __name__ == "__main__":
    main()
