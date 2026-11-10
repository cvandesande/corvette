#!/usr/bin/env bash
# Builds the detection worker image with podman, saves it as a local OCI archive and checks its layout.
# With --push it then publishes, only when CORVETTE_DETECT_WORKER_PUSH_CONFIRM equals this
# run's exact dated tag; the push comes after the layout check passes.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
DOCKERFILE="$REPO/docker/Dockerfile.detect-worker"
OCI_DEST="$REPO/out/corvette-detect-worker.oci"
IMAGE_REPOSITORY="docker.io/cvandesande/corvette-detect-worker"
TAG="$IMAGE_REPOSITORY:$(date -u +%Y%m%d)"
DIGEST_LOG="$REPO/.agents/issue-6/evidence/corvette-detect-worker-digest.txt"

push_requested=0
case "${1:-}" in
  "") ;;
  --push) push_requested=1 ;;
  *)
    echo "publish_detect_worker_image: unknown argument: $1 (usage: publish_detect_worker_image.sh [--push])" >&2
    exit 1
    ;;
esac

cd "$REPO"

if [[ "$push_requested" -eq 0 ]]; then
  podman build --file "$DOCKERFILE" --tag "$TAG" "$REPO"
  mkdir -p "$(dirname "$OCI_DEST")"
  podman save --format oci-archive --output "$OCI_DEST" "$TAG"
  ./scripts/check_detect_worker_oci_layout.sh "$OCI_DEST"
  echo "publish_detect_worker_image: built local OCI archive at $OCI_DEST (tag $TAG, not pushed)"
  exit 0
fi

echo "publish_detect_worker_image: push mode requested for $TAG" >&2
echo "publish_detect_worker_image: command that would run: podman build --file $DOCKERFILE --tag $TAG $REPO" >&2
echo "publish_detect_worker_image: command that would run: podman save --format oci-archive --output $OCI_DEST $TAG" >&2
echo "publish_detect_worker_image: command that would run: ./scripts/check_detect_worker_oci_layout.sh $OCI_DEST" >&2
echo "publish_detect_worker_image: command that would run: podman push --digestfile <tempfile> $TAG" >&2

confirm="${CORVETTE_DETECT_WORKER_PUSH_CONFIRM:-}"
if [[ "$confirm" != "$TAG" ]]; then
  echo "publish_detect_worker_image: refusing to push -- set CORVETTE_DETECT_WORKER_PUSH_CONFIRM=$TAG to authorize this exact tag" >&2
  exit 1
fi

podman build --file "$DOCKERFILE" --tag "$TAG" "$REPO"
mkdir -p "$(dirname "$OCI_DEST")"
podman save --format oci-archive --output "$OCI_DEST" "$TAG"
./scripts/check_detect_worker_oci_layout.sh "$OCI_DEST"

digestfile="$(mktemp)"
trap 'rm -f "$digestfile"' EXIT
podman push --digestfile "$digestfile" "$TAG"
digest="$(tr -d '[:space:]' <"$digestfile")"
if [[ ! "$digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  echo "publish_detect_worker_image: pushed but podman wrote no usable digest: $digest" >&2
  exit 1
fi

commit="$(git -C "$REPO" rev-parse HEAD)"
mkdir -p "$(dirname "$DIGEST_LOG")"
{
  echo "tag: $TAG"
  echo "digest: $digest"
  echo "commit: $commit"
} >"$DIGEST_LOG"

echo "publish_detect_worker_image: pushed $TAG@$digest, recorded at $DIGEST_LOG"
