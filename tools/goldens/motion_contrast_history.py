r"""Goldens for corvette-motion's contrast stretch: the 8-bit picture the
deployed detector's improve-contrast step writes for every picture of a seeded
sequence, stretched with the average of the last 50 pairs of percentile bounds.

Reference: the frigate repository, path frigate/motion/improved_motion.py, tag
v0.17.2, blob b081d37919541e6d1f95762ce084abde81774038, lines 25, 43 to 45 and
88 to 106, line 87 (a TODO comment) and line 100 (a blank line) omitted, copied
verbatim:

        contrast_frame_history=50,
        self.contrast_values = np.zeros((contrast_frame_history, 2), np.uint8)
        self.contrast_values[:, 1:2] = 255
        self.contrast_values_index = 0
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

Reference: the numpy repository, path numpy/core/_methods.py as shipped by the
v1.26.4 wheel, blob 0fc070b34c381ecdf8e8bb0d015bb799313a232e, lines 111 to 113,
118 and 121 to 122 of function _mean, copied verbatim:

    if dtype is None:
        if issubclass(arr.dtype.type, (nt.integer, nt.bool_)):
            dtype = mu.dtype('f8')
    ret = umr_sum(arr, axis, dtype, out, keepdims, where=where)
            ret = um.true_divide(
                    ret, rcount, out=ret, casting='unsafe', subok=False)

Reference: the numpy repository, path numpy/_core/_methods.py, version 2.5.1,
blob 1c29831bca209e63dc7e06a2d0f83fa986fc3f98, lines 125 to 127, 132 and 134 to
135 of function _mean, copied verbatim:

    if dtype is None:
        if issubclass(arr.dtype.type, (nt.integer, nt.bool)):
            dtype = mu.dtype('f8')
    ret = umr_sum(arr, axis, dtype, out, keepdims, where=where)
        ret = um.true_divide(
                ret, rcount, out=ret, casting='unsafe', subok=False)

Writes the fixture read by crates/corvette-motion/tests/contrast_history.rs:
one line per record, first field the kind. "counts 6 830" first. "sequence
<mode> <sw> <sh> <frames>" starts a sequence and the next <frames> lines are
its frames, in order, each sequence starting from a fresh history. "frame <seed>
<lo> <span> <output>": the sw by sh picture of the seeded generator of
motion_contrast_bounds.py (state = seed; for each byte state = (state * 1664525
+ 1013904223) mod 2**32 and the byte is state >> 24, row-major; the pixel is lo
plus the byte modulo span; the generator asserts lo + span - 1 <= 255; a frame
of span 1 is flat) and what the stretch returns for it: lowercase hex of its
bytes, row-major, when the mode is hex; the 16 lowercase hex digits of the
FNV-1a 64-bit hash of its bytes (offset basis 0xcbf29ce484222325, prime
0x100000001b3, per byte hash = (hash ^ byte) * prime mod 2**64) when the mode
is fnv.

The numbers are what numpy writes, version 2.5.1 as printed by versions.py in
the goldens shell and 1.26.4, the deployed pin, in a separate environment; the
fixture produced by both is byte-identical.
"""

from pathlib import Path

import numpy as np

OUT = Path(__file__).resolve().parents[2] / "crates/corvette-motion/tests/fixtures/contrast_history_cases.txt"

SEQUENCES = [
    ("hex", 8, 6, 140, 1000, 37, 120, 2, 53, 130, 11, 5),
    ("hex", 5, 5, 140, 2000, 61, 200, 2, 29, 56, 9, 0),
    ("hex", 3, 3, 140, 3000, 7, 200, 1, 5, 56, None, None),
    ("hex", 2, 2, 140, 4000, 13, 250, 1, 3, 5, None, None),
    ("fnv", 122, 100, 135, 5000, 29, 100, 2, 47, 150, 17, 3),
    ("fnv", 64, 48, 135, 6000, 43, 100, 2, 31, 150, 13, 8),
]


class ContrastStretch:
    def __init__(self) -> None:
        contrast_frame_history = 50
        self.contrast_values = np.zeros((contrast_frame_history, 2), np.uint8)
        self.contrast_values[:, 1:2] = 255
        self.contrast_values_index = 0

    def apply(self, resized_frame):
        input_shape = resized_frame.shape
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
        assert resized_frame.dtype == np.uint8 and resized_frame.shape == input_shape
        return resized_frame


def lcg_bytes(seed: int, count: int) -> bytes:
    state = seed
    out = bytearray(count)
    for i in range(count):
        state = (state * 1664525 + 1013904223) % (1 << 32)
        out[i] = state >> 24
    return bytes(out)


def fnv1a_hex(data: bytes) -> str:
    value = 0xCBF29CE484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001B3) % (1 << 64)
    return f"{value:016x}"


def main() -> None:
    lines = [f"counts {len(SEQUENCES)} {sum(seq[3] for seq in SEQUENCES)}"]
    for (mode, sw, sh, frames, seed_base, lo_mult, lo_mod, span_base, span_mult, span_mod, flat_mod, flat_rem) in SEQUENCES:
        lines.append(f"sequence {mode} {sw} {sh} {frames}")
        stretch = ContrastStretch()
        for k in range(frames):
            seed = seed_base + k
            lo = (lo_mult * k) % lo_mod
            span = span_base + (span_mult * k) % span_mod
            if flat_mod is not None and k % flat_mod == flat_rem:
                span = 1
            assert lo + span - 1 <= 255, (seed, lo, span)
            raw = np.frombuffer(lcg_bytes(seed, sw * sh), dtype=np.uint8).reshape(sh, sw)
            picture = (raw.astype(np.int64) % span + lo).astype(np.uint8)
            out = stretch.apply(picture)
            field = fnv1a_hex(out.tobytes()) if mode == "fnv" else out.tobytes().hex()
            lines.append(f"frame {seed} {lo} {span} {field}")
    assert lines[0] == "counts 6 830"
    assert len(lines) == 837
    OUT.write_text("\n".join(lines) + "\n", encoding="ascii")


main()
