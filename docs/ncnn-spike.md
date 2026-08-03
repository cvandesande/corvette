# ncnn-from-Rust spike

**The assumption holds.** ncnn's C API drives the detector's entire inference
path from Rust on Vulkan, and produces output **bitwise identical** to the
Python detector on the same input, at the same speed. The one gap -- GPU
enumeration -- is closed by ~90 lines of C++ of ours, with no fork of ncnn.

Run 2026-08-03 on liltig (Strix Halo, RADV GFX1151, Mesa 26.1.5 on the host /
25.x in the images), ncnn tag 20260526, against the same YOLOv9-t models
frigate-vulkan deploys. This is stage 2 of the roadmap in
`frigate-vulkan/docs/distroless-split-plan.md`, done early and deliberately out
of dependency order: everything after it rests on this result.

## Result

Same model, same fixed input tensor, same options (fp32 everywhere, one fresh
extractor and Mat per iteration), 2000 iterations at 320 and 600 at 640.
`steady` is the last half of the run -- the sustained state Frigate actually
lives in, per `frigate-vulkan/scripts/bench_steady.py`.

| | Python (`ncnn` module) | Rust (C API) |
| --- | --- | --- |
| 320 steady mean | 4.829 ms (207.1 fps) | 4.917 ms (203.4 fps) |
| 320 steady p95 | 5.751 ms | 5.730 ms |
| 640 steady mean | 7.532 ms (132.8 fps) | 7.851 ms (127.4 fps) |
| load | 774 / 790 ms | 842 / 795 ms |
| output | 105000 / 420000 floats | identical, `max_abs_diff = 0` |

Not "close" -- **every element of both output tensors is bitwise equal**
(105000/105000 at 320, 420000/420000 at 640). Both bindings drive the same
shaders on the same device through the same `Net`, so the numerics never had a
chance to diverge; the value of measuring it is that a mistake in Mat layout,
`cstep` handling or blob naming would have shown up here as a mismatch rather
than as a plausible-looking wrong answer later.

**Rust buys no inference speed, and the differences above are noise.** An
earlier run of the same harness had Rust ahead at 320 (4.821 against 4.976) --
the sign flips between runs, which is the honest summary: ±2% run to run, on a
card whose clocks move under sustained load. Anything real would have to be
larger than that. This is the expected result and it is fine: the detector was
never the bottleneck this project was going to fix.

The same binary built by Nix and run **natively on the host** -- no container,
Mesa 26.1.5 against the image's 25.x, a separately built ncnn -- produced the
same tensor again, bitwise, at 4.979 ms. Two toolchains, two Mesa generations,
one answer.

## The C API gap, and what closes it

Confirmed against `src/c_api.h` at 20260526 by writing the bindings: every call
the detector makes is present -- net create/load, `set_vulkan_device`, the
`use_vulkan_compute` and three `use_fp16_*` options, `mat_create_external_3d`,
extractor input/extract. What is absent is discovery: `get_gpu_count`,
`get_default_gpu_index` and `GpuInfo` are C++ only.

`crates/ncnn-sys/csrc/c_api_ext.cpp` closes it -- seven entry points over
ncnn's own `gpu.h`, compiled by `build.rs` against the installed headers.
**It is a translation unit of ours, not a patch**: no fork, no vendored tree,
nothing to rebase on an ncnn bump. That is cheaper than the plan assumed.

It also comes out *better* than the Python detector's version of the same
check. `docker/frigate/ncnn.py:105-109` selects a device explicitly because
lavapipe enumerates beside RADV, but the Python module exposes only the device
name, so "is this the software rasterizer?" can only be answered by matching on
a string. `GpuInfo::type()` answers it structurally:

```
ncnn-spike: vulkan device 0: AMD Radeon Graphics (RADV GFX1151) [driver=radv type=integrated score=21]
ncnn-spike: vulkan device 1: llvmpipe (LLVM 19.1.7, 256 bits) [driver=llvmpipe type=cpu score=4]
ncnn-spike: device 1 (llvmpipe (LLVM 19.1.7, 256 bits)) is a software rasterizer; set ALLOW_CPU=1 to use it deliberately
```

Both guards are exercised by `scripts/run_spike.sh`: an out-of-range index is
refused rather than clamped, and a `type=cpu` device is refused unless asked
for by name. The stake, measured on this host with `ALLOW_CPU=1`:
**63.9 ms on lavapipe against 4.8 ms on RADV, 13x**, at a steady 15.7 fps --
slow enough to wreck detection, fast enough to look like it is working.

## Also found

**The `.param` parser is not needed.** `ncnn.py:_parse_blob_names` walks the
parameter file to find the input and output blobs because the Python module
does not surface them. The C API does -- `ncnn_net_get_input_name` /
`ncnn_net_get_output_name` -- and the two agree (`in0` / `out0`) on both
models. A Rust port drops that function rather than porting it.

**ncnn's Vulkan instance is process-wide**, as the Python detector's
`NcnnDeviceLost` comment already records. Nothing about Rust changes that: a
lost device still needs a new process, so whatever supervises the detector in
`corvette` has to be able to restart it.

## Not covered

Deliberately -- the spike was scoped to the assumption, not to the port:

- **fp16 paths.** Only fp32 was compared, matching the validated
  CPU/GPU-parity configuration. The `use_fp16_*` setters are bound and work;
  their numerics were not diffed.
- **Device-lost behaviour.** No GPU reset was induced, so the Rust side of
  `NcnnDeviceLost` is untested. `extract` returning nonzero is handled.
- **Concurrency.** One extractor at a time on one thread. Frigate runs one
  detector process per detector, so this matches how it is used today.
- **arm64.** amd64 only, consistent with the packaging decision.
- **Discrete AMD cards.** This host is an integrated GFX1151. The gfx803 and
  gfx906 behaviour this project exists to pin down -- the Vega 20 SMU
  downclock, the gfx ring timeouts -- is not exercised by anything here.

## Reproducing

```
scripts/run_spike.sh                                  # 320, 2000 iterations
MODEL=yolov9t-640-2026-2.ncnn.param SIZE=640 ITERS=600 scripts/run_spike.sh
```

Needs `frigate-vulkan:py313` built for the reference side, and the models in
`../frigate-vulkan/models`. Both are overridable; see the top of the script.

Without Docker, on a host that has a Vulkan driver:

```
nix run .#ncnn-spike        # MODEL_PARAM=... and the rest via the environment
nix develop                 # Rust 1.97.1, ncnn, and python3+numpy for the diff
```

`nix build .#ncnn` builds the same pinned ncnn tag the container does, with
system glslang rather than the submodule. Two things about it are worth
knowing, because both fail silently rather than loudly: ncnn's generated
`ncnn.pc` joins `${prefix}` with absolute paths, and ncnn **dlopens**
`libvulkan.so.1` instead of linking it -- so `fixupPhase`'s RPATH shrinking
drops the loader and every enumeration comes back empty. `flake.nix` handles
both.
