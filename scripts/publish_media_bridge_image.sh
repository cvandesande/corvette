#!/usr/bin/env bash
# Builds the media-bridge image with podman, saves it as a local OCI archive and checks its layout.
# With --push it then publishes, only when CORVETTE_MEDIA_BRIDGE_PUSH_CONFIRM equals this
# run's exact dated tag; see publish_site_image.sh for why it must be today's exact tag.
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
  podman build --file "$DOCKERFILE" --tag "$TAG" "$REPO"
  mkdir -p "$(dirname "$OCI_DEST")"
  podman save --format oci-archive --output "$OCI_DEST" "$TAG"
  ./scripts/check_media_bridge_oci_layout.sh "$OCI_DEST"
  echo "publish_media_bridge_image: built local OCI layout at $OCI_DEST (tag $TAG, not pushed)"
  exit 0
fi

echo "publish_media_bridge_image: push mode requested for $TAG" >&2
echo "publish_media_bridge_image: command that would run: podman build --file $DOCKERFILE --tag $TAG $REPO" >&2
echo "publish_media_bridge_image: command that would run: podman push --digestfile <tempfile> $TAG" >&2

confirm="${CORVETTE_MEDIA_BRIDGE_PUSH_CONFIRM:-}"
if [[ "$confirm" != "$TAG" ]]; then
  echo "publish_media_bridge_image: refusing to push -- set CORVETTE_MEDIA_BRIDGE_PUSH_CONFIRM=$TAG to authorize this exact tag" >&2
  exit 1
fi

podman build --file "$DOCKERFILE" --tag "$TAG" "$REPO"

digestfile="$(mktemp)"
trap 'rm -f "$digestfile"' EXIT
podman push --digestfile "$digestfile" "$TAG"
digest="$(tr -d '[:space:]' <"$digestfile")"
if [[ ! "$digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  echo "publish_media_bridge_image: pushed but podman wrote no usable digest: $digest" >&2
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
