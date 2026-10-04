#!/usr/bin/env bash
# Builds Corvette's own read-only API-service image to a local OCI layout (issue #4 A5).
# Push needs CORVETTE_API_SERVER_PUSH_CONFIRM set to this run's dated tag.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
DOCKERFILE="$REPO/docker/Dockerfile.api-server"
OCI_DEST="$REPO/out/corvette-api-server.oci"
IMAGE_REPOSITORY="docker.io/cvandesande/corvette-api-server"
TAG="$IMAGE_REPOSITORY:$(date -u +%Y%m%d)"
DIGEST_LOG="$REPO/.agents/issue-4/evidence/corvette-api-server-digest.txt"

push_requested=0
case "${1:-}" in
  "") ;;
  --push) push_requested=1 ;;
  *)
    echo "publish_api_server_image: unknown argument: $1 (usage: publish_api_server_image.sh [--push])" >&2
    exit 1
    ;;
esac

cd "$REPO"

if [[ "$push_requested" -eq 0 ]]; then
  mkdir -p "$(dirname "$OCI_DEST")"
  docker buildx build --file "$DOCKERFILE" --tag "$TAG" \
    --output "type=oci,dest=$OCI_DEST" "$REPO"
  # buildx writes the OCI layout as one tar (as publish_media_bridge_image.sh
  # already learned), so the layout members are checked inside the archive.
  if ! tar -tf "$OCI_DEST" index.json oci-layout >/dev/null 2>&1; then
    echo "publish_api_server_image: $OCI_DEST is not an OCI layout: index.json or oci-layout missing" >&2
    exit 1
  fi
  echo "publish_api_server_image: built local OCI layout at $OCI_DEST (tag $TAG, not pushed)"
  exit 0
fi

build_args=(docker buildx build --file "$DOCKERFILE" --tag "$TAG" --push "$REPO")

echo "publish_api_server_image: push mode requested for $TAG" >&2
echo "publish_api_server_image: command that would run: ${build_args[*]} --metadata-file <tempfile>" >&2

confirm="${CORVETTE_API_SERVER_PUSH_CONFIRM:-}"
if [[ "$confirm" != "$TAG" ]]; then
  echo "publish_api_server_image: refusing to push -- set CORVETTE_API_SERVER_PUSH_CONFIRM=$TAG to authorize this exact tag" >&2
  exit 1
fi

metadata_file="$(mktemp)"
trap 'rm -f "$metadata_file"' EXIT
"${build_args[@]}" --metadata-file "$metadata_file"

digest="$(jq -r '."containerimage.digest"' "$metadata_file")"
if [[ ! "$digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  echo "publish_api_server_image: pushed but buildx metadata had no usable digest: $digest" >&2
  exit 1
fi

commit="$(git -C "$REPO" rev-parse HEAD)"
mkdir -p "$(dirname "$DIGEST_LOG")"
{
  echo "tag: $TAG"
  echo "digest: $digest"
  echo "commit: $commit"
} >"$DIGEST_LOG"

echo "publish_api_server_image: pushed $TAG@$digest, recorded at $DIGEST_LOG"
