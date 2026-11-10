# Golden test vector generators

`tools/goldens` is where the generators of golden test vectors for Corvette's Rust code live. That code will reproduce the arithmetic of OpenCV and numpy routines. Each generator added here quotes the reference source it reproduces verbatim — its path, version, and blob hash — and writes small fixtures that are committed beside the Rust test that reads them.

Generators run in a dedicated dev shell:

```sh
nix develop .#goldens
```

Check the shell with:

```sh
python3 -I tools/goldens/versions.py
```

The shell carries numpy and OpenCV from the locked nixpkgs, so each golden records the version it was produced under. The OpenCV version inside the deployed Frigate image has not been inspected; a golden is evidence about the locked nixpkgs version.

`versions.py` prints the Python, numpy, and cv2 versions of the shell it runs in, and exits non-zero when a package is missing.

`normalize.py` writes `crates/corvette-yolo/tests/fixtures/normalize_table.txt` and `normalize_spots.txt`: the float32 bits of Frigate's nchw float input transform at every byte value and at 612 sampled positions of a fixed image; it quotes `frigate/object_detection/base.py` and `util.py` at `v0.17.2` verbatim at its top.

`nms.py` writes `crates/corvette-yolo/tests/fixtures/nms_cases.txt`: the kept indices of `cv2.dnn.NMSBoxes` for seeded random sets and hand-built edge cases, with every f32 value written so it reads back bit-exact; it quotes the filter, sort, and suppression lines of OpenCV `4.11.0`'s `nms.inl.hpp` at its top.

`decode.py` writes `crates/corvette-yolo/tests/fixtures/decode_cases.txt`: the detection rows of Frigate's `__post_process_nms_yolo` for seeded random tensors and hand-built cases, f32 values bit-exact; it quotes `frigate/util/model.py` at `v0.17.2` verbatim at its top.

`motion_shape.py` writes `crates/corvette-motion/tests/fixtures/shape_cases.txt`: the motion frame sizes for sample frame sizes and frame heights; it quotes the motion-frame-size and gray-plane lines of `frigate/motion/improved_motion.py` at `v0.17.2` verbatim at its top.
