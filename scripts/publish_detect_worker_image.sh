#!/usr/bin/env bash
# Builds the detection worker image as a local OCI layout and never contacts a registry.
# With --push it publishes only when CORVETTE_DETECT_WORKER_PUSH_CONFIRM equals this
# run's exact dated tag.
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
  mkdir -p "$(dirname "$OCI_DEST")"
  docker buildx build --file "$DOCKERFILE" --tag "$TAG" \
    --output "type=oci,dest=$OCI_DEST" "$REPO"
  ./scripts/check_detect_worker_oci_layout.sh "$OCI_DEST"
  echo "publish_detect_worker_image: built local OCI layout at $OCI_DEST (tag $TAG, not pushed)"
  exit 0
fi

publish_flag="--push"
build_args=(docker buildx build --file "$DOCKERFILE" --tag "$TAG")
build_args+=("$publish_flag" "$REPO")

echo "publish_detect_worker_image: push mode requested for $TAG" >&2
echo "publish_detect_worker_image: command that would run: ${build_args[*]} --metadata-file <tempfile>" >&2

confirm="${CORVETTE_DETECT_WORKER_PUSH_CONFIRM:-}"
if [[ "$confirm" != "$TAG" ]]; then
  echo "publish_detect_worker_image: refusing to push -- set CORVETTE_DETECT_WORKER_PUSH_CONFIRM=$TAG to authorize this exact tag" >&2
  exit 1
fi

metadata_file="$(mktemp)"
trap 'rm -f "$metadata_file"' EXIT
"${build_args[@]}" --metadata-file "$metadata_file"

digest="$(jq -r '."containerimage.digest"' "$metadata_file")"
if [[ ! "$digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  echo "publish_detect_worker_image: pushed but buildx metadata had no usable digest: $digest" >&2
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
