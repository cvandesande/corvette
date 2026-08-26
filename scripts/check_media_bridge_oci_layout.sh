#!/usr/bin/env bash
# Verifies an OCI image layout built by publish_media_bridge_image.sh (issue
# #12, item P1): the merged filesystem the image would run must contain both
# adopted/owned binaries -- moq-relay and corvette-media-bridge -- present,
# executable, and actually runnable (not just present with the right file
# mode); and INV-1's denied third-party-Go-streaming-server literals must not
# appear in the Dockerfile that built this image or in the built layout's own
# dependency/build manifest.
#
# Mirrors scripts/check_oci_layout.sh's own shape (extract the OCI tar's
# layers into one merged filesystem, assert against that), extended with an
# execution check neither predecessor needed: docker/Dockerfile.site's own
# image is inert (an overlay payload, never run on its own), but both
# binaries this image ships are real running processes, so "present with the
# executable bit set" is not enough evidence that the copy actually produced
# a working binary -- a build that copied the wrong architecture, or a binary
# missing a shared library the runtime stage forgot to install, would still
# pass a file-presence check.
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

# --- Both binaries: present, executable, and actually runnable -------------
#
# The dynamic linker is itself part of the merged filesystem (copied in by
# the runtime base image's own layer), so it can be invoked directly against
# the extracted binaries with an explicit --library-path -- no chroot, no
# root, and no Docker daemon needed to prove the copy produced a working,
# correctly linked executable. This is the same technique `patchelf
# --print-rpath`-style tooling and distro build systems use to test a binary
# before it is installed system-wide.
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

if check_binary_present_and_executable /usr/local/bin/moq-relay "moq-relay binary"; then
  if [[ ! -x "$interp" ]]; then
    echo "check_media_bridge_oci_layout: no dynamic linker found in layout at /lib64/ld-linux-x86-64.so.2 -- cannot prove moq-relay runs" >&2
    fail=1
  else
    version_status=0
    version_output="$("$interp" --library-path "$libpath" "$merged/usr/local/bin/moq-relay" --version 2>&1)" || version_status=$?
    if [[ "$version_status" -ne 0 ]]; then
      echo "check_media_bridge_oci_layout: moq-relay --version failed to run from the built layout:" >&2
      echo "$version_output" >&2
      fail=1
    elif [[ "$version_output" != moq-relay\ * ]]; then
      echo "check_media_bridge_oci_layout: moq-relay --version produced unexpected output: $version_output" >&2
      fail=1
    fi
  fi
fi

if check_binary_present_and_executable /usr/local/bin/corvette-media-bridge "corvette-media-bridge binary"; then
  if [[ ! -x "$interp" ]]; then
    echo "check_media_bridge_oci_layout: no dynamic linker found in layout at /lib64/ld-linux-x86-64.so.2 -- cannot prove corvette-media-bridge runs" >&2
    fail=1
  else
    # corvette-media-bridge (issue #12 G1) takes no CLI arguments at all --
    # its own src/main.rs reads configuration only from the
    # CORVETTE_MEDIA_BRIDGE_CONFIG env var (src/config.rs::load_from_env) --
    # so there is no --help/--version flag to assert a zero exit against.
    # Running it with that env var unset is still a real, mechanical
    # execution proof: a binary that were the wrong architecture, missing a
    # shared library, or otherwise not actually runnable would fail with a
    # linker/exec error here, not with this exact, known application-level
    # message. Asserting the precise message (not just a nonzero exit) rules
    # out a coincidental nonzero exit from a linker failure being mistaken
    # for a real run.
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

# --- INV-1: no third-party Go streaming server literal anywhere -------------
#
# The Dockerfile source is the primary check: it is the only place a stray
# `RUN curl .../mediamtx` or `COPY --from=... mediamtx` could be added, and
# every stage that could add one is written directly in this one file. The
# built layout's own config blob is checked too, per the plan's own Do step,
# though it is a weaker secondary check: buildkit's OCI export records
# `history[].created_by` only for the FINAL stage's own instructions, not for
# any earlier stage discarded after its COPY --from -- so this catches a
# denied literal only if it reached the final stage's own commands or
# environment, not one confined to a discarded builder stage. That asymmetry
# is exactly why the Dockerfile-source check above is the one this repo's own
# INV-1 ENFORCED-HOW text names as authoritative for P1.
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

echo "check_media_bridge_oci_layout: $OCI_TAR matches the expected shape (both binaries present, executable, and runnable; no INV-1 denied literals found)"
