#!/usr/bin/env bash
# The only proof, before a human publishes the image, of what the image actually carries.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: check_detect_worker_oci_layout.sh <path-to-oci-tar>" >&2
  exit 2
fi
OCI_TAR="$1"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
DOCKERFILE="$REPO/docker/Dockerfile.detect-worker"

if [[ ! -f "$OCI_TAR" ]]; then
  echo "check_detect_worker_oci_layout: no such file: $OCI_TAR" >&2
  exit 1
fi
if [[ ! -f "$DOCKERFILE" ]]; then
  echo "check_detect_worker_oci_layout: no such file: $DOCKERFILE" >&2
  exit 1
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

if ! tar -xf "$OCI_TAR" -C "$workdir"; then
  echo "check_detect_worker_oci_layout: cannot extract $OCI_TAR" >&2
  exit 1
fi

manifest_digest="$(jq -r '.manifests[0].digest' "$workdir/index.json")"
if [[ ! "$manifest_digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  echo "check_detect_worker_oci_layout: index.json has no usable manifest digest" >&2
  exit 1
fi
manifest_blob="$workdir/blobs/sha256/${manifest_digest#sha256:}"

config_digest="$(jq -r '.config.digest' "$manifest_blob")"
if [[ ! "$config_digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  echo "check_detect_worker_oci_layout: image manifest has no usable config digest" >&2
  exit 1
fi
config_blob="$workdir/blobs/sha256/${config_digest#sha256:}"
if [[ ! -f "$config_blob" ]]; then
  echo "check_detect_worker_oci_layout: missing config blob: $config_digest" >&2
  exit 1
fi

mapfile -t layer_digests < <(jq -r '.layers[].digest' "$manifest_blob")
if [[ "${#layer_digests[@]}" -eq 0 ]]; then
  echo "check_detect_worker_oci_layout: image manifest has no layers" >&2
  exit 1
fi

fail=0
fail_line() {
  printf 'check_detect_worker_oci_layout: FAIL %s: %s\n' "$1" "$2" >&2
  fail=1
}

merged="$workdir/merged"
mkdir -p "$merged"
merged_real="$(realpath -m -- "$merged")"

whiteout_dir_in_tree() {
  local real
  real="$(realpath -m -- "$merged${1:+/$1}")" || return 1
  [[ "$real" == "$merged_real" || "$real" == "$merged_real/"* ]]
}

# A whiteout reached through a symlink from a lower layer could otherwise delete on the host.
merge_layer() {
  local blob="$1" name="$2" member base dir victim
  local -a members=()
  if ! tar -tf "$blob" > "$workdir/members.txt"; then
    echo "check_detect_worker_oci_layout: cannot extract $name" >&2
    exit 1
  fi
  mapfile -t members < "$workdir/members.txt"
  for member in "${members[@]}"; do
    member="${member#./}"
    [[ -n "$member" ]] || continue
    base="${member##*/}"
    dir="${member%/*}"
    [[ "$dir" != "$member" ]] || dir=""
    if [[ "$member" == ".." || "$member" == ..?* || "$member" == */.. || "$member" == */..?* ]]; then
      fail_line layer "member name has a .. component: $member"
      continue
    fi
    if [[ "$base" == ".wh..wh..opq" ]]; then
      if ! whiteout_dir_in_tree "$dir"; then
        fail_line layer "opaque marker resolves outside the tree: $member"
        continue
      fi
      victim="${merged:?}${dir:+/$dir}"
      if [[ -d "$victim" ]]; then
        find "$victim" -mindepth 1 -delete
      fi
    elif [[ "$base" == ".wh."* ]]; then
      if ! whiteout_dir_in_tree "$dir"; then
        fail_line layer "whiteout resolves outside the tree: $member"
        continue
      fi
      rm -rf -- "${merged:?}${dir:+/$dir}/${base#.wh.}"
    fi
  done
  if ! tar --exclude='.wh.*' -xf "$blob" -C "$merged"; then
    echo "check_detect_worker_oci_layout: cannot extract $name" >&2
    exit 1
  fi
}

for layer_digest in "${layer_digests[@]}"; do
  layer_blob="$workdir/blobs/sha256/${layer_digest#sha256:}"
  if [[ ! -f "$layer_blob" ]]; then
    echo "check_detect_worker_oci_layout: missing layer blob: $layer_digest" >&2
    fail=1
    continue
  fi
  merge_layer "$layer_blob" "$layer_digest"
done

resolve_in_tree() {
  local rel="$1" path target i real
  if [[ "$rel" == /* ]]; then
    path="$merged$rel"
  else
    path="$merged/$rel"
  fi
  for ((i = 0; i < 16; i++)); do
    if [[ -L "$path" ]]; then
      target="$(readlink "$path")"
      if [[ "$target" == /* ]]; then
        path="$merged$target"
      else
        path="$(dirname "$path")/$target"
      fi
    elif [[ -f "$path" ]]; then
      real="$(realpath -- "$path")" || return 1
      [[ "$real" == "$merged_real" || "$real" == "$merged_real/"* ]] || return 1
      return 0
    else
      return 1
    fi
  done
  return 1
}

interp="$merged/lib64/ld-linux-x86-64.so.2"
bin="$merged/usr/local/bin/corvette-detect-worker"
libpath="$merged/opt/ncnn/lib:$merged/lib/x86_64-linux-gnu:$merged/usr/lib/x86_64-linux-gnu"

if [[ ! -f "$bin" ]]; then
  fail_line binary "missing: /usr/local/bin/corvette-detect-worker"
elif [[ ! -x "$bin" ]]; then
  fail_line binary "present but not executable: /usr/local/bin/corvette-detect-worker"
fi

if [[ ! -x "$interp" ]]; then
  fail_line usage "no executable dynamic linker at lib64/ld-linux-x86-64.so.2 in the tree"
  fail_line help "no executable dynamic linker at lib64/ld-linux-x86-64.so.2 in the tree"
else
  usage_status=0
  timeout 30 env -i "$interp" --library-path "$libpath" "$bin" \
    < /dev/null > "$workdir/usage.out" 2> "$workdir/usage.err" || usage_status=$?
  usage_err="$(< "$workdir/usage.err")"
  usage_out="$(< "$workdir/usage.out")"
  if [[ "$usage_status" -ne 2 ]]; then
    fail_line usage "exit $usage_status, want 2; stderr first line: ${usage_err%%$'\n'*}"
  elif [[ "$usage_err" != corvette-detect-worker:* ]]; then
    fail_line usage "stderr does not start with corvette-detect-worker: got ${usage_err%%$'\n'*}"
  elif [[ "$usage_err" != *"usage: corvette-detect-worker --param PATH"* ]]; then
    fail_line usage "usage line missing from stderr: ${usage_err%%$'\n'*}"
  elif [[ -n "$usage_out" ]]; then
    fail_line usage "stdout should be empty, got: ${usage_out%%$'\n'*}"
  fi

  help_status=0
  timeout 30 env -i "$interp" --library-path "$libpath" "$bin" --help \
    < /dev/null > "$workdir/help.out" 2> "$workdir/help.err" || help_status=$?
  help_err="$(< "$workdir/help.err")"
  help_all="$(< "$workdir/help.out")"
  help_all+="$help_err"
  if [[ "$help_status" -ne 0 ]]; then
    fail_line help "exit $help_status, want 0; stderr first line: ${help_err%%$'\n'*}"
  elif [[ "$help_all" != *"exit codes:"* ]]; then
    fail_line help "no 'exit codes:' text in --help output: ${help_all%%$'\n'*}"
  fi
fi

# ncnn dlopen()s the Vulkan loader at run time, so only a file assertion can catch it missing.
for lib in opt/ncnn/lib/libncnn.so.1 usr/lib/x86_64-linux-gnu/libvulkan.so.1 usr/lib/x86_64-linux-gnu/libgomp.so.1; do
  if ! resolve_in_tree "$lib"; then
    fail_line library "missing or dangling: $lib"
  fi
done

icd_dir="$merged/usr/share/vulkan/icd.d"
for family in radeon lvp; do
  mapfile -t manifests < <(command find "$icd_dir" -maxdepth 1 -name "${family}_icd*.json" 2>/dev/null | sort)
  if [[ ${#manifests[@]} -eq 0 ]]; then
    fail_line driver "$family: no ${family}_icd*.json manifest in /usr/share/vulkan/icd.d"
    continue
  fi
  for manifest in "${manifests[@]}"; do
    lp="$(jq -r '.ICD.library_path' "$manifest")"
    mf="${manifest#"$merged"/}"
    if [[ -z "$lp" || "$lp" == "null" ]]; then
      fail_line driver "$family: no ICD.library_path in $mf"
    elif [[ "$lp" == /* ]]; then
      if ! resolve_in_tree "$lp"; then
        fail_line driver "$family: library $lp named by $mf is missing"
      fi
    elif ! resolve_in_tree "usr/lib/x86_64-linux-gnu/$lp"; then
      fail_line driver "$family: library $lp named by $mf is missing under /usr/lib/x86_64-linux-gnu"
    fi
  done
done

for lic in usr/share/licenses/ncnn/LICENSE.txt usr/share/licenses/glslang/LICENSE.txt usr/share/licenses/corvette/LICENSE; do
  if [[ ! -s "$merged/$lic" ]]; then
    fail_line licence "missing or empty: $lic"
  fi
done

model_files="$(cd "$merged" && command find . \( -name '*.param' -o -name '*.ncnn.bin' \))"
if [[ -n "$model_files" ]]; then
  fail_line model "model files baked in: ${model_files//$'\n'/ }"
fi

entrypoint="$(jq -c '.config.Entrypoint' "$config_blob")"
if [[ "$entrypoint" != '["/usr/local/bin/corvette-detect-worker"]' ]]; then
  fail_line entrypoint "got $entrypoint, want [\"/usr/local/bin/corvette-detect-worker\"]"
fi

cmd="$(jq -c '.config.Cmd' "$config_blob")"
if [[ "$cmd" != "null" && "$cmd" != "[]" ]]; then
  fail_line cmd "got $cmd, want null or an empty array"
fi

user="$(jq -c '.config.User' "$config_blob")"
if [[ "$user" != "null" && "$user" != '""' ]]; then
  fail_line user "got $user, want null or the empty string"
fi

ld_hit="$(jq -c '(.config.Env // []) | index("LD_LIBRARY_PATH=/opt/ncnn/lib")' "$config_blob")"
if [[ "$ld_hit" == "null" ]]; then
  fail_line ld_library_path "no LD_LIBRARY_PATH=/opt/ncnn/lib entry in the image environment"
fi

for literal in frigate go2rtc mediamtx ffmpeg; do
  if grep -qi -- "$literal" "$DOCKERFILE"; then
    fail_line "denied literal" "'$literal' found in $DOCKERFILE"
  fi
  if grep -qi -- "$literal" "$config_blob"; then
    fail_line "denied literal" "'$literal' found in the image config blob ($config_digest)"
  fi
done

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "check_detect_worker_oci_layout: ok ($OCI_TAR: binary starts through the image's own loader; ncnn, libgomp and the Vulkan loader present; radeon and lvp drivers present with resolvable libraries; licences present; no baked-in model; config matches the pod; no denied literals)"
