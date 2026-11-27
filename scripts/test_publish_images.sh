#!/usr/bin/env bash
# Exercises the site, media-bridge and API-server publish scripts with a stub podman and stub helpers:
# the refusals, the confirmed build-then-push and digest record, and the layout and shape stops.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT_DIR="${PUBLISH_SCRIPTS_DIR:-$ROOT/scripts}"
unset CORVETTE_UI_PUSH_CONFIRM CORVETTE_MEDIA_BRIDGE_PUSH_CONFIRM CORVETTE_API_SERVER_PUSH_CONFIRM
SCRATCH="$(mktemp -d)"
trap 'rm -r "$SCRATCH"' EXIT
mkdir -p "$SCRATCH/bin" "$SCRATCH/repo/scripts" "$SCRATCH/repo/docker" "$SCRATCH/seedmembers"
export HELPER_LOG="$SCRATCH/helpers.log"
TODAY="$(date -u +%Y%m%d)"
YESTERDAY="$(date -u -d yesterday +%Y%m%d)"
GOOD_DIGEST="sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
# Stub bodies write every dollar sign as \044: a literal $ inside single quotes would fail shellcheck.
printf '#!/usr/bin/env bash\nlog=%s/podman.log\necho "\044*" >>"\044log"\ncase "\044{1:-}" in\nbuild)\n  exit "\044{STUB_BUILD_RC:-99}"\n  ;;\nsave)\n  if [ "\044{STUB_SAVE_OK:-0}" != 1 ]; then exit 99; fi\n  out=\n  prev=\n  for a in "\044@"; do\n    if [ "\044prev" = --output ]; then out="\044a"; fi\n    prev="\044a"\n  done\n  if [ -z "\044out" ]; then exit 97; fi\n  if [ -n "\044{STUB_SAVE_SEED:-}" ]; then cp "\044STUB_SAVE_SEED" "\044out"; else : >"\044out"; fi\n  ;;\npush)\n  if [ -z "\044{STUB_PUSH_DIGEST:-}" ]; then exit 99; fi\n  df=\n  prev=\n  for a in "\044@"; do\n    if [ "\044prev" = --digestfile ]; then df="\044a"; fi\n    prev="\044a"\n  done\n  if [ -z "\044df" ]; then exit 97; fi\n  printf "%%s\\n" "\044STUB_PUSH_DIGEST" >"\044df"\n  ;;\n*)\n  exit 99\n  ;;\nesac\n' "$SCRATCH" >"$SCRATCH/bin/podman"
chmod +x "$SCRATCH/bin/podman"
export PATH="$SCRATCH/bin:$PATH"
stub_helper() {
  printf '#!/usr/bin/env bash\necho "%s \044*" >>"\044{HELPER_LOG:-/dev/null}"\nexit "\044{%s:-0}"\n' "$1" "$2" >"$SCRATCH/repo/scripts/$1"
  chmod +x "$SCRATCH/repo/scripts/$1"
}
stub_helper build_site.sh STUB_BUILD_SITE_RC
stub_helper check_site_shape.sh STUB_CHECK_SHAPE_RC
stub_helper check_media_bridge_oci_layout.sh LAYOUT_STUB_RC
touch "$SCRATCH/repo/docker/Dockerfile.site" "$SCRATCH/repo/docker/Dockerfile.media-bridge" "$SCRATCH/repo/docker/Dockerfile.api-server"
: >"$SCRATCH/seedmembers/index.json"
: >"$SCRATCH/seedmembers/oci-layout"
tar -cf "$SCRATCH/seed-good.tar" -C "$SCRATCH/seedmembers" index.json oci-layout
tar -cf "$SCRATCH/empty.tar" -T /dev/null
env -u GIT_INDEX_FILE -u GIT_DIR -u GIT_WORK_TREE git init -q "$SCRATCH/repo"
env -u GIT_INDEX_FILE -u GIT_DIR -u GIT_WORK_TREE git -C "$SCRATCH/repo" \
  -c user.name=test -c user.email=test@example.invalid commit -q --allow-empty -m test
SCRATCH_HEAD="$(git -C "$SCRATCH/repo" rev-parse HEAD)"
failed=0
rc=0
run_case() {
  local name="$1"
  shift
  : >"$SCRATCH/${name//\//_}.out"
  : >"$SCRATCH/${name//\//_}.err"
  rm -f "$SCRATCH/podman.log" "$SCRATCH/helpers.log"
  rc=0
  timeout 30 env -u GIT_INDEX_FILE -u GIT_DIR -u GIT_WORK_TREE "$@" </dev/null \
    >"$SCRATCH/${name//\//_}.out" 2>"$SCRATCH/${name//\//_}.err" || rc=$?
}
report() {
  local name="$1" problem="$2"
  if [ -n "$problem" ]; then
    failed=1
    echo "FAIL $name: $problem"
  else
    echo "ok $name"
  fi
}
refused() {
  local name="$1" needle problem=""
  shift
  if [ "$rc" -ne 1 ]; then
    problem="exit $rc, want 1; stderr: $(tr '\n' ' ' <"$SCRATCH/${name//\//_}.err")"
  else
    for needle in "$@"; do
      if ! grep -qF -- "$needle" "$SCRATCH/${name//\//_}.err"; then
        problem="stderr lacked [$needle]: $(tr '\n' ' ' <"$SCRATCH/${name//\//_}.err")"
        break
      fi
    done
    if [ -z "$problem" ] && [ -s "$SCRATCH/podman.log" ]; then
      problem="podman.log was not empty: $(cat "$SCRATCH/podman.log")"
    fi
  fi
  report "$name" "$problem"
}
run_script_cases() {
  local script="$1" confirm_var="$2" repository="$3" digest_rel="$4" prefix="$5"
  local label="${script%.sh}" name problem log case_path digest_path tag before after
  case_path="$SCRATCH/repo/scripts/$script"
  digest_path="$SCRATCH/repo/$digest_rel"
  tag="$repository:$TODAY"
  cp "$SCRIPT_DIR/$script" "$case_path"

  name="$label/unknown_argument_is_refused"
  run_case "$name" env -u "$confirm_var" "$case_path" --bogus
  problem=""
  if [ "$rc" -ne 1 ]; then
    problem="exit $rc, want 1; stderr: $(tr '\n' ' ' <"$SCRATCH/${name//\//_}.err")"
  elif ! grep -qF 'unknown argument: --bogus' "$SCRATCH/${name//\//_}.err"; then
    problem="stderr lacked [unknown argument: --bogus]"
  elif [ -s "$SCRATCH/podman.log" ] || [ -s "$SCRATCH/helpers.log" ]; then
    problem="a command ran: podman[$(cat "$SCRATCH/podman.log" 2>/dev/null)] helpers[$(cat "$SCRATCH/helpers.log" 2>/dev/null)]"
  fi
  report "$name" "$problem"

  name="$label/push_without_confirmation_is_refused"
  run_case "$name" env -u "$confirm_var" "$case_path" --push
  refused "$name" 'refusing to push' "$confirm_var=$tag"

  name="$label/push_with_yesterdays_confirmation_is_refused"
  run_case "$name" env "$confirm_var=$repository:$YESTERDAY" "$case_path" --push
  refused "$name" 'refusing to push'

  name="$label/push_with_todays_confirmation_reaches_the_build"
  before="$(git -C "$ROOT" status --short -uall .agents)"
  run_case "$name" env "$confirm_var=$tag" "$case_path" --push
  after="$(git -C "$ROOT" status --short -uall .agents)"
  problem="" log="$SCRATCH/podman.log"
  if [ "$rc" -eq 0 ]; then
    problem="exit 0, want non-zero (the stub build exits 99)"
  elif [ ! -s "$log" ]; then
    problem="podman.log is empty; no stub call ran"
  elif ! head -n 1 "$log" | grep -q '^build '; then
    problem="first log line does not start with build: $(tr '\n' ' ' <"$log")"
  elif grep -q '^push ' "$log"; then
    problem="push line present: $(tr '\n' ' ' <"$log")"
  elif [ -e "$digest_path" ]; then
    problem="digest log written after a failed build"
  elif [ "$before" != "$after" ]; then
    problem="real-repo evidence changed: before[$before] after[$after]"
  fi
  report "$name" "$problem"

  name="$label/confirmed_push_builds_then_pushes_and_records_the_digest"
  run_case "$name" env "$confirm_var=$tag" STUB_BUILD_RC=0 STUB_PUSH_DIGEST="$GOOD_DIGEST" "$case_path" --push
  problem="" log="$SCRATCH/podman.log"
  if [ "$rc" -ne 0 ]; then
    problem="exit $rc, want 0; stderr: $(tr '\n' ' ' <"$SCRATCH/${name//\//_}.err")"
  elif [ ! -f "$log" ] || [ "$(wc -l <"$log")" -ne 2 ]; then
    problem="podman.log does not hold two lines: [$(tr '\n' ' ' <"$log" 2>/dev/null)]"
  elif [ "$(sed -n 1p "$log" | cut -d' ' -f1)" != build ]; then
    problem="log line 1 is not a build: $(sed -n 1p "$log")"
  elif [ "$(sed -n 2p "$log" | cut -d' ' -f1)" != push ] ||
    ! sed -n 2p "$log" | grep -qF -- '--digestfile' ||
    ! sed -n 2p "$log" | grep -qF -- "$tag"; then
    problem="log line 2 is not a push --digestfile of today's tag: $(sed -n 2p "$log")"
  else
    printf 'tag: %s\ndigest: %s\ncommit: %s\n' "$tag" "$GOOD_DIGEST" "$SCRATCH_HEAD" \
      >"$SCRATCH/expected-digest.txt"
    if [ ! -f "$digest_path" ]; then
      problem="no digest log at $digest_rel"
    elif ! cmp -s "$SCRATCH/expected-digest.txt" "$digest_path"; then
      problem="digest log differs: [$(tr '\n' ' ' <"$digest_path")]"
    fi
  fi
  report "$name" "$problem"

  name="$label/malformed_digest_is_refused"
  rm -f "$digest_path"
  run_case "$name" env "$confirm_var=$tag" STUB_BUILD_RC=0 STUB_PUSH_DIGEST=garbage "$case_path" --push
  problem=""
  if [ "$rc" -ne 1 ]; then
    problem="exit $rc, want 1 (the digest is garbage)"
  elif ! grep -qF -- "$prefix" "$SCRATCH/${name//\//_}.err"; then
    problem="stderr lacks the script name: $(tr '\n' ' ' <"$SCRATCH/${name//\//_}.err")"
  elif ! grep -qF 'garbage' "$SCRATCH/${name//\//_}.err"; then
    problem="stderr lacks the digest: $(tr '\n' ' ' <"$SCRATCH/${name//\//_}.err")"
  elif [ -e "$digest_path" ]; then
    problem="digest log exists after a malformed digest"
  fi
  report "$name" "$problem"

  name="$label/default_mode_builds_and_saves_without_pushing"
  run_case "$name" env -u "$confirm_var" STUB_BUILD_RC=0 STUB_SAVE_OK=1 \
    STUB_SAVE_SEED="$SCRATCH/seed-good.tar" "$case_path"
  problem="" log="$SCRATCH/podman.log"
  if [ "$rc" -ne 0 ]; then
    problem="exit $rc, want 0; stderr: $(tr '\n' ' ' <"$SCRATCH/${name//\//_}.err")"
  elif ! grep -q 'built local OCI layout at' "$SCRATCH/${name//\//_}.out"; then
    problem="stdout lacks the success message: [$(tr '\n' ' ' <"$SCRATCH/${name//\//_}.out")]"
  elif [ ! -f "$log" ] || [ "$(wc -l <"$log")" -ne 2 ]; then
    problem="podman.log does not hold two lines: [$(tr '\n' ' ' <"$log" 2>/dev/null)]"
  elif [ "$(sed -n 1p "$log" | cut -d' ' -f1)" != build ]; then
    problem="log line 1 is not a build: $(sed -n 1p "$log")"
  elif [ "$(sed -n 2p "$log" | cut -d' ' -f1)" != save ] ||
    ! sed -n 2p "$log" | grep -qF -- '--format oci-archive' ||
    ! sed -n 2p "$log" | grep -qF -- '--output' ||
    ! sed -n 2p "$log" | grep -qF -- "$tag"; then
    problem="log line 2 is not save --format oci-archive --output of today's tag: $(sed -n 2p "$log")"
  fi
  report "$name" "$problem"

  if [ "$script" = publish_site_image.sh ]; then
    name="$label/shape_check_failure_stops_before_any_podman_call"
    problem=""
    run_case "$name" env STUB_CHECK_SHAPE_RC=1 "$case_path"
    if [ "$rc" -eq 0 ]; then
      problem="default mode exited 0, want non-zero"
    elif [ -s "$SCRATCH/podman.log" ]; then
      problem="default mode touched podman: [$(tr '\n' ' ' <"$SCRATCH/podman.log")]"
    fi
    run_case "$name" env "$confirm_var=$tag" STUB_CHECK_SHAPE_RC=1 "$case_path" --push
    if [ "$rc" -eq 0 ]; then
      problem="${problem}push mode exited 0, want non-zero; "
    elif [ -s "$SCRATCH/podman.log" ]; then
      problem="${problem}push mode touched podman: [$(tr '\n' ' ' <"$SCRATCH/podman.log")]"
    fi
    report "$name" "$problem"
  else
    name="$label/default_mode_stops_when_the_layout_check_fails"
    if [ "$script" = publish_media_bridge_image.sh ]; then
      run_case "$name" env -u "$confirm_var" STUB_BUILD_RC=0 STUB_SAVE_OK=1 LAYOUT_STUB_RC=1 "$case_path"
    else
      run_case "$name" env -u "$confirm_var" STUB_BUILD_RC=0 STUB_SAVE_OK=1 \
        STUB_SAVE_SEED="$SCRATCH/empty.tar" "$case_path"
    fi
    problem=""
    if [ "$rc" -eq 0 ]; then
      problem="exit 0, want non-zero (the layout check fails)"
    elif grep -q 'built local OCI layout at' "$SCRATCH/${name//\//_}.out"; then
      problem="success message printed despite the failing layout check"
    fi
    report "$name" "$problem"
  fi
}
run_script_cases publish_site_image.sh CORVETTE_UI_PUSH_CONFIRM \
  docker.io/cvandesande/corvette-ui .agents/issue-2/evidence/corvette-ui-digest.txt publish_site_image
run_script_cases publish_media_bridge_image.sh CORVETTE_MEDIA_BRIDGE_PUSH_CONFIRM \
  docker.io/cvandesande/corvette-media-bridge .agents/issue-12/evidence/corvette-media-bridge-digest.txt publish_media_bridge_image
run_script_cases publish_api_server_image.sh CORVETTE_API_SERVER_PUSH_CONFIRM \
  docker.io/cvandesande/corvette-api-server .agents/issue-4/evidence/corvette-api-server-digest.txt publish_api_server_image
if [ "$failed" -ne 0 ]; then
  exit 1
fi
