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

The shell carries numpy, OpenCV and scipy from the locked nixpkgs, so each golden records the version it was produced under. The OpenCV version inside the deployed Frigate image has not been inspected; a golden is evidence about the locked nixpkgs version.

`versions.py` prints the Python, numpy, cv2 and scipy versions of the shell it runs in, and exits non-zero when a package is missing.

`normalize.py` writes `crates/corvette-yolo/tests/fixtures/normalize_table.txt` and `normalize_spots.txt`: the float32 bits of Frigate's nchw float input transform at every byte value and at 612 sampled positions of a fixed image; it quotes `frigate/object_detection/base.py` and `util.py` at `v0.17.2` verbatim at its top.

`nms.py` writes `crates/corvette-yolo/tests/fixtures/nms_cases.txt`: the kept indices of `cv2.dnn.NMSBoxes` for seeded random sets and hand-built edge cases, with every f32 value written so it reads back bit-exact; it quotes the filter, sort, and suppression lines of OpenCV `4.11.0`'s `nms.inl.hpp` at its top.

`decode.py` writes `crates/corvette-yolo/tests/fixtures/decode_cases.txt`: the detection rows of Frigate's `__post_process_nms_yolo` for seeded random tensors and hand-built cases, f32 values bit-exact; it quotes `frigate/util/model.py` at `v0.17.2` verbatim at its top.

`motion_shape.py` writes `crates/corvette-motion/tests/fixtures/shape_cases.txt`: the motion frame sizes for sample frame sizes and frame heights; it quotes the motion-frame-size and gray-plane lines of `frigate/motion/improved_motion.py` at `v0.17.2` verbatim at its top.

`motion_resize.py` writes `crates/corvette-motion/tests/fixtures/resize_cases.txt`: the bytes `cv2.INTER_NEAREST` produces for seeded random grey pictures at sample source and destination sizes; it quotes the resize call lines of `frigate/motion/improved_motion.py` at `v0.17.2` and the nearest-neighbor index lines of OpenCV `4.11.0`'s `resize.cpp` verbatim at its top.

`motion_blur.py` writes `crates/corvette-motion/tests/fixtures/blur_cases.txt`: the bytes of `scipy.ndimage.gaussian_filter(sigma=1, radius=1)` on seeded, ramp, checkerboard and flat pictures, and the kernel weights' bit patterns; it quotes the Frigate call lines and the scipy `v1.16.0` lines verbatim at its top.

`motion_contrast_bounds.py` writes `crates/corvette-motion/tests/fixtures/contrast_bounds_cases.txt`: the pair of 4th and 96th percentiles numpy returns, truncated to 8 bits, for seeded, two-level and hand-built pictures; it quotes the Frigate lines and the numpy 1.26.4 and 2.5.1 percentile code verbatim at its top.

`motion_contrast_history.py` writes `crates/corvette-motion/tests/fixtures/contrast_history_cases.txt`: the pictures numpy produces for the contrast stretch with its 50-pair history on seeded picture sequences that wrap the history and hold flat pictures; it quotes the Frigate lines and numpy's mean code verbatim at its top.

`motion_delta.py` writes `crates/corvette-motion/tests/fixtures/delta_cases.txt`: the bytes `cv2.absdiff`, `cv2.threshold` (binary, maximum 255) and `cv2.dilate` (default 3×3 kernel, one iteration) produce for seeded, sparse and single-pixel grey pictures; it quotes the Frigate call lines and the OpenCV `4.11.0` lines verbatim at its top.

`motion_average.py` writes `crates/corvette-motion/tests/fixtures/average_cases.txt` from the pinned `opencv-python-headless` 4.11.0.86 wheel (IPP on), whose directory is its argument; it holds the float32 average and the 8-bit conversion after every frame of seeded sequences; see the file's docstring for the run command.

`motion_contours.py` writes `crates/corvette-motion/tests/fixtures/contours_cases.txt` from the same pinned wheel, whose directory is its argument: the outer contours `cv2.findContours(RETR_EXTERNAL, CHAIN_APPROX_SIMPLE)` returns for seeded and hand-built masks, each reduced to its `cv2.boundingRect` and twice its `cv2.contourArea`; see the file's docstring for the run command.

`motion_decision.py` writes `crates/corvette-motion/tests/fixtures/decision_cases.txt`: the boxes, motion-fraction bits, calibrating flag and average weight of the detector's contour loop, calibration rules and average-update rule, transcribed in pure Python for scaling, filtering, threshold-boundary, state-sequence and seeded random contour sequences; it quotes the Frigate lines verbatim at its top; see the file's docstring for the run command.

`motion_detector.py` writes `crates/corvette-motion/tests/fixtures/detector_cases.txt`: the boxes, calibrating flag and float32-average hash of Frigate's deployed `ImprovedMotionDetector` frame by frame over seeded grey sequences, the class run unchanged under the same pinned wheel with stand-in imports; its arguments are the extracted wheel directory and a Frigate checkout; see the file's docstring for the run command.

`motion_polygon.py` writes `crates/corvette-motion/tests/fixtures/polygon_cases.txt` from the same pinned wheel: the frame-size picture the deployed `create_mask` text, executed unchanged at `v0.17.2`, writes for relative polygon strings at the three deployed masks, hand-built edge shapes, overlaps and seeded random polygons; its arguments are the extracted wheel directory and a Frigate checkout; see the file's docstring for the run command.

`motion_crop.py` writes `crates/corvette-motion/tests/fixtures/crop_pad_cases.txt` and `i420_rgb_cases.txt` from the same pinned wheel: the square canvas Frigate's deployed `yuv_crop_and_resize` returns for seeded frames and regions, and the bytes `cv2.cvtColor(..., cv2.COLOR_YUV2RGB_I420)` returns for seeded and flat canvases; the Frigate text at `v0.17.2` is blob-checked and run unchanged; its arguments are the extracted wheel directory and a Frigate checkout; see the file's docstring for the run command.

`motion_shrink.py` writes `crates/corvette-motion/tests/fixtures/shrink_cases.txt` from the same pinned wheel: the zero set of the deployed shrink of a mask picture, the quoted `cv2.resize(..., INTER_AREA)` followed by `== 0` of `frigate/motion/improved_motion.py` at `v0.17.2`, on the three deployed camera masks, single-kept-pixel pictures at three small geometries, flat edge pictures and seeded random 0/255 pictures; its arguments are the extracted wheel directory and a Frigate checkout; see the file's docstring for the run command.

`motion_resize_rgb.py` writes `crates/corvette-motion/tests/fixtures/resize_rgb_cases.txt` from the same pinned wheel: the bytes `cv2.resize(..., INTER_LINEAR)` returns for seeded and flat interleaved RGB pictures at sample source and destination sizes, as exact or tolerant lines (the tolerant lines carry the wheel's SIMD vertical-kernel residual of one level on a small fraction of the bytes); it quotes the OpenCV `4.11.0` `resize.cpp` fixed-point lines at its top; its argument is the extracted wheel directory; see the file's docstring for the run command.

`motion_tensor.py` writes `crates/corvette-motion/tests/fixtures/tensor_cases.txt` from the same pinned wheel: the bytes Frigate's deployed `create_tensor_input` returns for seeded frames, regions and model sizes, as exact or tolerant lines (the tolerant lines carry the wheel's SIMD resize residual of one level on a small fraction of the bytes); the Frigate text at `v0.17.2` is blob-checked and run unchanged; its arguments are the extracted wheel directory and a Frigate checkout; see the file's docstring for the run command.

`motion_regions.py` writes `crates/corvette-motion/tests/fixtures/region_cases.txt`: the minimum region size, the square region around a box and the square region around a cluster that Frigate's deployed `get_min_region_size`, `calculate_region` and `get_cluster_region` return for sample, edge and seeded random models, frames, boxes and clusters; the Frigate text at `v0.17.2` is blob-checked and run unchanged; its argument is a Frigate checkout; see the file's docstring for the run command. It also writes `crates/corvette-motion/tests/fixtures/cluster_cases.txt`: the containment, area, cluster boundary and cluster-candidate grouping that Frigate's deployed `box_inside`, `area`, `get_cluster_boundary` and `get_cluster_candidates` return for seeded random boxes, clusters in first-member order. A third fixture, `crates/corvette-motion/tests/fixtures/region_float_cases.txt`, holds 900 float-bounds boxes and the region Frigate's `calculate_region` returns for them at the three multipliers.

`motion_grid.py` writes `crates/corvette-motion/tests/fixtures/grid_cases.txt`: the grid-adjusted region and the startup regions that Frigate's deployed `get_region_from_grid`, `get_cluster_region_from_grid` and `get_startup_regions` return for seeded grids, frames, boxes and clusters; the Frigate text at `v0.17.2` is blob-checked and run unchanged; its argument is a Frigate checkout; see the file's docstring for the run command.

`motion_reference.py` is a runner, not a fixture generator: it feeds a raw luma file (concatenated grey pictures, one byte per pixel) to the deployed `ImprovedMotionDetector` text at `v0.17.2`, executed unchanged under the same pinned wheel, and writes one line per picture (`index`, `calibrating`, boxes) in the format `corvette-detect-tools motion-detect` writes, so the two outputs can be compared with `cmp` or `diff`. Its arguments are the extracted wheel directory, a Frigate checkout and the same flags as the Rust command; see the file's docstring for the run command.
