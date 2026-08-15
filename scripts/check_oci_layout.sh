#!/usr/bin/env bash
# Verifies an OCI image layout built by publish_site_image.sh: the merged
# filesystem the image would overlay must contain index.html and
# pkg/corvette.wasm under /site, and nothing outside /site/ at all. This is
# the same publish shape check_site_shape.sh enforces on the staging tree,
# re-asserted on the artifact a human would actually push -- the staging
# tree can be correct and the image still wrong if the Dockerfile ever grows
# a second COPY.
set -euo pipefail

OCI_TAR="${1:?usage: check_oci_layout.sh <path-to-oci-tar>}"

if [[ ! -f "$OCI_TAR" ]]; then
  echo "check_oci_layout: no such file: $OCI_TAR" >&2
  exit 1
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

tar -xf "$OCI_TAR" -C "$workdir"

manifest_digest="$(jq -r '.manifests[0].digest' "$workdir/index.json")"
if [[ ! "$manifest_digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  echo "check_oci_layout: index.json has no usable manifest digest" >&2
  exit 1
fi
manifest_blob="$workdir/blobs/sha256/${manifest_digest#sha256:}"

mapfile -t layer_digests < <(jq -r '.layers[].digest' "$manifest_blob")
if [[ "${#layer_digests[@]}" -eq 0 ]]; then
  echo "check_oci_layout: image manifest has no layers" >&2
  exit 1
fi

fail=0
merged="$workdir/merged"
mkdir -p "$merged"
for layer_digest in "${layer_digests[@]}"; do
  layer_blob="$workdir/blobs/sha256/${layer_digest#sha256:}"
  if [[ ! -f "$layer_blob" ]]; then
    echo "check_oci_layout: missing layer blob: $layer_digest" >&2
    fail=1
    continue
  fi
  tar -xf "$layer_blob" -C "$merged"
done

# Top level of the merged filesystem must be exactly "site" -- this is what
# makes "nothing outside /site/" mechanical rather than a per-file scan.
mapfile -t top_level < <(find "$merged" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)
if [[ "${top_level[*]}" != "site" ]]; then
  echo "check_oci_layout: unexpected top-level paths in image: [${top_level[*]}], want [site]" >&2
  fail=1
fi

if [[ ! -s "$merged/site/index.html" ]]; then
  echo "check_oci_layout: missing or empty /site/index.html" >&2
  fail=1
fi

if [[ ! -f "$merged/site/pkg/corvette.wasm" ]]; then
  echo "check_oci_layout: missing /site/pkg/corvette.wasm" >&2
  fail=1
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "check_oci_layout: $OCI_TAR matches the expected shape"
