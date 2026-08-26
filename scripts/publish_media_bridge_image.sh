#!/usr/bin/env bash
# Builds Corvette's own media-relay image (issue #12, item P1: adopted
# moq-relay + corvette-media-bridge, one image, two entrypoints, per DT-1)
# into an OCI image and, only when a human explicitly confirms, publishes it
# to the tagged registry repository.
#
# Mirrors publish_site_image.sh's own shape exactly. Default mode never
# contacts a registry: it builds a local OCI layout under out/ and checks its
# shape. Push mode additionally requires CORVETTE_MEDIA_BRIDGE_PUSH_CONFIRM
# in the environment to equal this run's exact dated tag -- a value nobody
# sets by accident, and one that expires the moment the date rolls over, so a
# leftover confirmation left set from an earlier day cannot authorize today's
# push.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
DOCKERFILE="$REPO/docker/Dockerfile.media-bridge"
OCI_DEST="$REPO/out/corvette-media-bridge.oci"
IMAGE_REPOSITORY="docker.io/cvandesande/corvette-media-bridge"
TAG="$IMAGE_REPOSITORY:$(date -u +%Y%m%d)"
DIGEST_LOG="$REPO/.agents/issue-12/evidence/corvette-media-bridge-digest.txt"

push_requested=0
case "${1:-}" in
  "") ;;
  --push) push_requested=1 ;;
  *)
    echo "publish_media_bridge_image: unknown argument: $1 (usage: publish_media_bridge_image.sh [--push])" >&2
    exit 1
    ;;
esac

cd "$REPO"

if [[ "$push_requested" -eq 0 ]]; then
  mkdir -p "$(dirname "$OCI_DEST")"
  docker buildx build --file "$DOCKERFILE" --tag "$TAG" \
    --output "type=oci,dest=$OCI_DEST" "$REPO"
  ./scripts/check_media_bridge_oci_layout.sh "$OCI_DEST"
  echo "publish_media_bridge_image: built local OCI layout at $OCI_DEST (tag $TAG, not pushed)"
  exit 0
fi

publish_flag="--push"
build_args=(docker buildx build --file "$DOCKERFILE" --tag "$TAG")
build_args+=("$publish_flag" "$REPO")

echo "publish_media_bridge_image: push mode requested for $TAG" >&2
echo "publish_media_bridge_image: command that would run: ${build_args[*]} --metadata-file <tempfile>" >&2

confirm="${CORVETTE_MEDIA_BRIDGE_PUSH_CONFIRM:-}"
if [[ "$confirm" != "$TAG" ]]; then
  echo "publish_media_bridge_image: refusing to push -- set CORVETTE_MEDIA_BRIDGE_PUSH_CONFIRM=$TAG to authorize this exact tag" >&2
  exit 1
fi

metadata_file="$(mktemp)"
trap 'rm -f "$metadata_file"' EXIT
"${build_args[@]}" --metadata-file "$metadata_file"

digest="$(jq -r '."containerimage.digest"' "$metadata_file")"
if [[ ! "$digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  echo "publish_media_bridge_image: pushed but buildx metadata had no usable digest: $digest" >&2
  exit 1
fi

commit="$(git -C "$REPO" rev-parse HEAD)"
mkdir -p "$(dirname "$DIGEST_LOG")"
{
  echo "tag: $TAG"
  echo "digest: $digest"
  echo "commit: $commit"
} >"$DIGEST_LOG"

echo "publish_media_bridge_image: pushed $TAG@$digest, recorded at $DIGEST_LOG"
