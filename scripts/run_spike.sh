#!/usr/bin/env bash
# Runs the ncnn-from-Rust spike end to end: build, Python reference, Rust
# implementation over the same input, tensor diff, and the device-selection
# checks that the C API extension exists for.
#
# The models and the Python image come from the sibling frigate-vulkan repo.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
FRIGATE_REPO="${FRIGATE_REPO:-$(cd "$REPO/../frigate-vulkan" && pwd)}"
MODELS_DIR="${MODELS_DIR:-$FRIGATE_REPO/models}"
MODEL="${MODEL:-yolov9t-320-2026-2.ncnn.param}"
SIZE="${SIZE:-320}"
ITERS="${ITERS:-2000}"
NCNN_TAG="${NCNN_TAG:-20260526}"
PY_IMAGE="${PY_IMAGE:-frigate-vulkan:py313}"
SPIKE_IMAGE="${SPIKE_IMAGE:-corvette/ncnn-spike:trixie}"
OUT="${OUT:-$REPO/out}"

mkdir -p "$OUT"
rm -f "$OUT"/*.f32

echo "== build =="
docker build -f "$REPO/docker/Dockerfile.spike" --build-arg "NCNN_TAG=$NCNN_TAG" \
  -t "$SPIKE_IMAGE" "$REPO"

common=(--rm --device /dev/dri
  -v "$MODELS_DIR:/models:ro" -v "$OUT:/out"
  -e "MODEL_PARAM=/models/$MODEL" -e "MODEL_SIZE=$SIZE" -e "BENCH_ITERS=$ITERS"
  -e INPUT_F32=/out/input.f32)

# First run generates the input, so the reference goes first deliberately.
echo
echo "== python reference ($PY_IMAGE) =="
docker run "${common[@]}" -v "$REPO/scripts:/scripts:ro" \
  -e OUTPUT_F32=/out/output-python.f32 \
  --entrypoint python3 "$PY_IMAGE" /scripts/reference_infer.py \
  | tee "$OUT/python.txt"

echo
echo "== rust spike ($SPIKE_IMAGE) =="
docker run "${common[@]}" -e OUTPUT_F32=/out/output-rust.f32 "$SPIKE_IMAGE" \
  | tee "$OUT/rust.txt"

echo
echo "== tensor diff =="
docker run --rm -v "$OUT:/out" -v "$REPO/scripts:/scripts:ro" \
  --entrypoint python3 "$PY_IMAGE" \
  /scripts/compare_outputs.py /out/output-python.f32 /out/output-rust.f32

echo
echo "== device selection =="
# Out of range must be refused rather than clamped or ignored.
if docker run "${common[@]}" -e NCNN_DEVICE=99 -e BENCH_ITERS=1 "$SPIKE_IMAGE" 2>&1 | tail -1; then
  echo "FAIL: out-of-range device index was accepted" >&2
  exit 1
fi
# And the software rasterizer must be refused by type, not by name.
if docker run "${common[@]}" -e NCNN_DEVICE=1 -e BENCH_ITERS=1 "$SPIKE_IMAGE" 2>&1 | tail -1; then
  echo "NOTE: device 1 was accepted -- no software rasterizer present on this host?" >&2
fi
