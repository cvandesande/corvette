"""Python reference for the ncnn-from-Rust spike.

Runs the same model through the ncnn *Python* module -- the path
frigate-vulkan's detector uses today -- over a fixed input, and writes the raw
output tensor so the Rust result can be diffed against it rather than merely
compared on latency. The reported statistics are the ones scripts/bench_steady.py
reports, computed the same way.

Environment: MODEL_PARAM, MODEL_SIZE, BENCH_ITERS, NCNN_DEVICE, INPUT_F32,
OUTPUT_F32. The input file is generated if it does not exist, so whichever
implementation runs first fixes the input for the other.
"""

import json
import os
import statistics
import time
from pathlib import Path

import ncnn
import numpy as np


def blob_names(path):
    """The .param parse frigate-vulkan's ncnn.py does, kept here to check it
    against the names ncnn's C API reports directly."""
    layers = []
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        fields = line.split()
        if len(fields) < 4:
            continue
        try:
            inputs, outputs = int(fields[2]), int(fields[3])
        except ValueError:
            continue
        end = 4 + inputs + outputs
        if outputs and len(fields) >= end:
            layers.append(fields[:end])
    input_layer = next((l for l in layers if l[0] == "Input"), None)
    if input_layer is None or not layers:
        raise RuntimeError(f"Could not parse blobs in {path}")
    return input_layer[4 + int(input_layer[2])], layers[-1][4 + int(layers[-1][2])]


param_path = os.environ["MODEL_PARAM"]
bin_path = param_path[: -len(".param")] + ".bin"
size = int(os.environ.get("MODEL_SIZE", "320"))
iters = int(os.environ.get("BENCH_ITERS", "2000"))
input_path = os.environ.get("INPUT_F32")
output_path = os.environ.get("OUTPUT_F32")
input_name, output_name = blob_names(param_path)

if input_path and Path(input_path).exists():
    data = np.fromfile(input_path, dtype=np.float32).reshape(3, size, size)
else:
    data = np.random.default_rng(0).random((3, size, size), dtype=np.float32)
    if input_path:
        data.tofile(input_path)

device = int(os.getenv("NCNN_DEVICE", ncnn.get_default_gpu_index()))
net = ncnn.Net()
net.opt.use_vulkan_compute = True
net.set_vulkan_device(device)
net.opt.use_fp16_packed = False
net.opt.use_fp16_storage = False
net.opt.use_fp16_arithmetic = False
load_start = time.perf_counter()
if net.load_param(param_path) != 0 or net.load_model(bin_path) != 0:
    raise RuntimeError("load failed")
load_ms = (time.perf_counter() - load_start) * 1000


def infer():
    ex = net.create_extractor()
    mat = ncnn.Mat(data)
    if ex.input(input_name, mat) != 0:
        raise RuntimeError("input failed")
    rc, out = ex.extract(output_name)
    if rc != 0:
        raise RuntimeError(f"extract failed: {rc}")
    return np.array(out, dtype=np.float32)


first = infer()
if output_path:
    first.tofile(output_path)

samples = []
for _ in range(iters):
    t0 = time.perf_counter()
    infer()
    samples.append((time.perf_counter() - t0) * 1000)

quarter = max(1, iters // 4)
head = samples[:quarter]
tail = samples[-(iters // 2):]
print("RESULT " + json.dumps({
    "model": Path(param_path).name,
    "size": size,
    "device": ncnn.get_gpu_info(device).device_name(),
    "input_blob": input_name,
    "output_blob": output_name,
    "load_ms": round(load_ms, 1),
    "output_shape": list(first.shape),
    "output_len": int(first.size),
    "iters": iters,
    "head_mean_ms": round(statistics.fmean(head), 3),
    "steady_mean_ms": round(statistics.fmean(tail), 3),
    "steady_median_ms": round(statistics.median(tail), 3),
    "steady_p95_ms": round(sorted(tail)[int(len(tail) * 0.95)], 3),
    "steady_min_ms": round(min(tail), 3),
    "steady_fps": round(1000 / statistics.fmean(tail), 1),
}))
