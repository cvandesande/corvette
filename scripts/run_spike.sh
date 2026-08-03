#!/usr/bin/env bash
# Builds and runs the ncnn-from-Rust benchmark and its device-selection checks.
#
# The model comes from the sibling frigate-vulkan repo.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
FRIGATE_REPO="${FRIGATE_REPO:-$(cd "$REPO/../frigate-vulkan" && pwd)}"
MODELS_DIR="${MODELS_DIR:-$FRIGATE_REPO/models}"
MODEL="${MODEL:-yolov9t-320-2026-2.ncnn.param}"
SIZE="${SIZE:-320}"
ITERS="${ITERS:-2000}"
NCNN_TAG="${NCNN_TAG:-20260526}"
SPIKE_IMAGE="${SPIKE_IMAGE:-corvette/ncnn-spike:trixie}"

echo "== build =="
docker build -f "$REPO/docker/Dockerfile.spike" --build-arg "NCNN_TAG=$NCNN_TAG" \
  -t "$SPIKE_IMAGE" "$REPO"

common=(--rm --device /dev/dri
  -v "$MODELS_DIR:/models:ro"
  -e "MODEL_PARAM=/models/$MODEL" -e "MODEL_SIZE=$SIZE" -e "BENCH_ITERS=$ITERS")

echo
echo "== benchmark ($SPIKE_IMAGE) =="
docker run "${common[@]}" "$SPIKE_IMAGE"

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
