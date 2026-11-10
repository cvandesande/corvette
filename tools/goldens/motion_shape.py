"""Goldens for corvette-motion's size rule: the motion frame size Frigate computes.

Reference: the frigate repository, path frigate/motion/improved_motion.py, tag
v0.17.2, blob b081d37919541e6d1f95762ce084abde81774038, lines 30 to 34 and 73,
copied verbatim:

        self.resize_factor = frame_shape[0] / config.frame_height
        self.motion_frame_size = (
            config.frame_height,
            config.frame_height * frame_shape[1] // frame_shape[0],
        )

        gray = frame[0 : self.frame_shape[0], 0 : self.frame_shape[1]]

frame_shape is (height, width); the gray copy of the brightness plane is
frame_height rows tall and frame_height * frame_shape[1] // frame_shape[0]
columns wide. The Rust side calls that pair's width the motion width.

Writes the fixture read by crates/corvette-motion/tests/shape.rs.
"""

from pathlib import Path

OUT = Path(__file__).resolve().parents[2] / "crates/corvette-motion/tests/fixtures/shape_cases.txt"

CASES = [
    (704, 576, 100),
    (640, 480, 100),
    (1920, 1080, 100),
    (2688, 1520, 100),
    (576, 704, 100),
    (704, 576, 50),
    (704, 576, 200),
    (100, 100, 100),
]


def main() -> None:
    lines = []
    for frame_w, frame_h, config_frame_height in CASES:
        frame_shape = (frame_h, frame_w)
        motion_frame_size = (
            config_frame_height,
            config_frame_height * frame_shape[1] // frame_shape[0],
        )
        lines.append(
            f"{frame_w} {frame_h} {config_frame_height} "
            f"{motion_frame_size[1]} {motion_frame_size[0]}"
        )
    OUT.write_text("\n".join(lines) + "\n", encoding="ascii")


main()
