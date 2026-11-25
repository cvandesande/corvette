#!/usr/bin/env bash
set -euo pipefail

OCI_TAR="${1:?usage: check_media_bridge_oci_layout.sh <path-to-oci-tar>}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
DOCKERFILE="$REPO/docker/Dockerfile.media-bridge"

if [[ ! -f "$OCI_TAR" ]]; then
  echo "check_media_bridge_oci_layout: no such file: $OCI_TAR" >&2
  exit 1
fi
if [[ ! -f "$DOCKERFILE" ]]; then
  echo "check_media_bridge_oci_layout: no such file: $DOCKERFILE" >&2
  exit 1
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

tar -xf "$OCI_TAR" -C "$workdir"

manifest_digest="$(jq -r '.manifests[0].digest' "$workdir/index.json")"
if [[ ! "$manifest_digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  echo "check_media_bridge_oci_layout: index.json has no usable manifest digest" >&2
  exit 1
fi
manifest_blob="$workdir/blobs/sha256/${manifest_digest#sha256:}"

config_digest="$(jq -r '.config.digest' "$manifest_blob")"
if [[ ! "$config_digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  echo "check_media_bridge_oci_layout: image manifest has no usable config digest" >&2
  exit 1
fi
config_blob="$workdir/blobs/sha256/${config_digest#sha256:}"
if [[ ! -f "$config_blob" ]]; then
  echo "check_media_bridge_oci_layout: missing config blob: $config_digest" >&2
  exit 1
fi

mapfile -t layer_digests < <(jq -r '.layers[].digest' "$manifest_blob")
if [[ "${#layer_digests[@]}" -eq 0 ]]; then
  echo "check_media_bridge_oci_layout: image manifest has no layers" >&2
  exit 1
fi

fail=0
merged="$workdir/merged"
mkdir -p "$merged"
for layer_digest in "${layer_digests[@]}"; do
  layer_blob="$workdir/blobs/sha256/${layer_digest#sha256:}"
  if [[ ! -f "$layer_blob" ]]; then
    echo "check_media_bridge_oci_layout: missing layer blob: $layer_digest" >&2
    fail=1
    continue
  fi
  tar -xf "$layer_blob" -C "$merged"
done

# Run it via the layout's own loader --library-path: no root or Docker needed.
interp="$merged/lib64/ld-linux-x86-64.so.2"
libpath="$merged/lib/x86_64-linux-gnu:$merged/usr/lib/x86_64-linux-gnu"

check_binary_present_and_executable() {
  local path="$1"
  local label="$2"
  if [[ ! -f "$merged$path" ]]; then
    echo "check_media_bridge_oci_layout: missing $label: $path" >&2
    fail=1
    return 1
  fi
  if [[ ! -x "$merged$path" ]]; then
    echo "check_media_bridge_oci_layout: $label is present but not executable: $path" >&2
    fail=1
    return 1
  fi
  return 0
}

if check_binary_present_and_executable /usr/local/bin/corvette-media-bridge "corvette-media-bridge binary"; then
  if [[ ! -x "$interp" ]]; then
    echo "check_media_bridge_oci_layout: no dynamic linker found in layout at /lib64/ld-linux-x86-64.so.2 -- cannot prove corvette-media-bridge runs" >&2
    fail=1
  else
# No CLI flags; the exact fail-closed message rules out a linker error.
    bridge_status=0
    bridge_output="$(env -u CORVETTE_MEDIA_BRIDGE_CONFIG "$interp" --library-path "$libpath" "$merged/usr/local/bin/corvette-media-bridge" 2>&1)" || bridge_status=$?
    if [[ "$bridge_status" -eq 0 ]]; then
      echo "check_media_bridge_oci_layout: corvette-media-bridge exited 0 with CORVETTE_MEDIA_BRIDGE_CONFIG unset, expected it to fail closed on the missing env var" >&2
      fail=1
    elif [[ "$bridge_output" != *"environment variable CORVETTE_MEDIA_BRIDGE_CONFIG is not set"* ]]; then
      echo "check_media_bridge_oci_layout: corvette-media-bridge did not run as expected from the built layout:" >&2
      echo "$bridge_output" >&2
      fail=1
    fi
  fi
fi

# INV-1: Dockerfile source is authoritative; blob history covers last stage.
denied_literals=(mediamtx bluenviron winkmichael)
for literal in "${denied_literals[@]}"; do
  if grep -qi -- "$literal" "$DOCKERFILE"; then
    echo "check_media_bridge_oci_layout: denied literal '$literal' found in $DOCKERFILE" >&2
    fail=1
  fi
  if grep -qi -- "$literal" "$config_blob"; then
    echo "check_media_bridge_oci_layout: denied literal '$literal' found in the built layout's config blob ($config_digest)" >&2
    fail=1
  fi
done

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "check_media_bridge_oci_layout: $OCI_TAR matches the expected shape (corvette-media-bridge present, executable, and runnable; no denied third-party literals found)"
