#!/usr/bin/env bash
# Exercises the publish script's guards, push ordering and digest record with a stub podman:
# unknown argument, unconfirmed or stale push refusal, the confirmed build, and the scratch-repo cases.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="${PUBLISH_SCRIPT:-$ROOT/scripts/publish_detect_worker_image.sh}"
unset CORVETTE_DETECT_WORKER_PUSH_CONFIRM
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
mkdir -p "$SCRATCH/bin"
# Stub bodies write every dollar sign as \044: a literal $ inside single quotes would fail shellcheck.
printf '#!/usr/bin/env bash\nlog=%s/podman.log\necho "\044*" >>"\044log"\ncase "\044{1:-}" in\nbuild)\n  exit "\044{STUB_BUILD_RC:-99}"\n  ;;\nsave)\n  if [ "\044{STUB_SAVE_OK:-0}" != 1 ]; then exit 99; fi\n  out=\n  prev=\n  for a in "\044@"; do\n    if [ "\044prev" = --output ]; then out="\044a"; fi\n    prev="\044a"\n  done\n  if [ -z "\044out" ]; then exit 97; fi\n  : >"\044out"\n  ;;\npush)\n  if [ -z "\044{STUB_PUSH_DIGEST:-}" ]; then exit 99; fi\n  df=\n  prev=\n  for a in "\044@"; do\n    if [ "\044prev" = --digestfile ]; then df="\044a"; fi\n    prev="\044a"\n  done\n  if [ -z "\044df" ]; then exit 97; fi\n  printf "%%s\\n" "\044STUB_PUSH_DIGEST" >"\044df"\n  ;;\n*)\n  exit 99\n  ;;\nesac\n' "$SCRATCH" >"$SCRATCH/bin/podman"
chmod +x "$SCRATCH/bin/podman"
export PATH="$SCRATCH/bin:$PATH"
REPOSITORY="docker.io/cvandesande/corvette-detect-worker"
TODAY="$(date -u +%Y%m%d)"
YESTERDAY="$(date -u -d yesterday +%Y%m%d)"
TAG="$REPOSITORY:$TODAY"
failed=0
run_case() {
  local name="$1"
  shift
  : >"$SCRATCH/$name.out"
  : >"$SCRATCH/$name.err"
  rm -f "$SCRATCH/podman.log"
  rc=0
  timeout 30 "${CASE_SCRIPT:-$SCRIPT}" "$@" </dev/null \
    >"$SCRATCH/$name.out" 2>"$SCRATCH/$name.err" || rc=$?
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
    problem="exit $rc, want 1; stderr: $(tr '\n' ' ' <"$SCRATCH/$name.err")"
  else
    for needle in "$@"; do
      if ! grep -qF -- "$needle" "$SCRATCH/$name.err"; then
        problem="stderr lacked [$needle]: $(tr '\n' ' ' <"$SCRATCH/$name.err")"
        break
      fi
    done
    if [ -z "$problem" ] && [ -s "$SCRATCH/podman.log" ]; then
      problem="podman.log was not empty: $(cat "$SCRATCH/podman.log")"
    fi
  fi
  report "$name" "$problem"
}
name=unknown_argument_is_refused
run_case "$name" --bogus
refused "$name" 'unknown argument: --bogus'
name=push_without_confirmation_is_refused
run_case "$name" --push
refused "$name" 'refusing to push' "CORVETTE_DETECT_WORKER_PUSH_CONFIRM=$TAG"
name=push_with_yesterdays_confirmation_is_refused
export CORVETTE_DETECT_WORKER_PUSH_CONFIRM="$REPOSITORY:$YESTERDAY"
run_case "$name" --push
unset CORVETTE_DETECT_WORKER_PUSH_CONFIRM
refused "$name" 'refusing to push'
name=push_with_todays_confirmation_reaches_the_build
before="$(git -C "$ROOT" status --short -uall .agents/issue-6/evidence)"
export CORVETTE_DETECT_WORKER_PUSH_CONFIRM="$TAG"
run_case "$name" --push
after="$(git -C "$ROOT" status --short -uall .agents/issue-6/evidence)"
unset CORVETTE_DETECT_WORKER_PUSH_CONFIRM
problem="" log="$SCRATCH/podman.log"
if [ "$rc" -eq 0 ]; then
  problem="exit 0, want non-zero (the stub exits 99)"
elif [ ! -s "$log" ] || [ "$(wc -l <"$log")" -ne 1 ]; then
  problem="podman.log does not hold exactly one line: [$(cat "$log" 2>/dev/null)]"
elif ! grep -q '^build ' "$log"; then
  problem="line does not start with build: $(cat "$log")"
elif ! grep -qF -- '--file' "$log"; then
  problem="line lacks --file: $(cat "$log")"
elif ! grep -qF 'docker/Dockerfile.detect-worker' "$log"; then
  problem="line lacks the detect-worker Dockerfile: $(cat "$log")"
elif ! grep -qF "$TAG" "$log"; then
  problem="line lacks today's tag: $(cat "$log")"
elif [ "$before" != "$after" ]; then
  problem="evidence tree changed: before[$before] after[$after]"
fi
report "$name" "$problem"
mkdir -p "$SCRATCH/repo/scripts" "$SCRATCH/repo/docker"
cp "$SCRIPT" "$SCRATCH/repo/scripts/publish_detect_worker_image.sh"
printf 'exit "\044{LAYOUT_STUB_RC:-0}"\n' >"$SCRATCH/repo/scripts/check_detect_worker_oci_layout.sh"
chmod +x "$SCRATCH/repo/scripts/check_detect_worker_oci_layout.sh"
touch "$SCRATCH/repo/docker/Dockerfile.detect-worker"
git init -q "$SCRATCH/repo"
env -u GIT_INDEX_FILE -u GIT_DIR -u GIT_WORK_TREE git -C "$SCRATCH/repo" \
  -c user.name=test -c user.email=test@example.invalid commit -q --allow-empty -m test
SCRATCH_HEAD="$(git -C "$SCRATCH/repo" rev-parse HEAD)"
export CASE_SCRIPT="$SCRATCH/repo/scripts/publish_detect_worker_image.sh"
DIGEST_LOG_PATH="$SCRATCH/repo/.agents/issue-6/evidence/corvette-detect-worker-digest.txt"
export CORVETTE_DETECT_WORKER_PUSH_CONFIRM="$TAG" STUB_BUILD_RC=0 STUB_SAVE_OK=1
name=layout_failure_stops_before_the_push
export LAYOUT_STUB_RC=1 STUB_PUSH_DIGEST="sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
run_case "$name" --push
problem="" log="$SCRATCH/podman.log"
if [ "$rc" -eq 0 ]; then
  problem="exit 0, want non-zero (the layout stub fails)"
elif [ ! -s "$log" ]; then
  problem="podman.log is empty; no stub call ran"
elif ! grep -q '^build ' "$log"; then
  problem="no build line: $(tr '\n' ' ' <"$log")"
elif ! grep -q '^save ' "$log"; then
  problem="no save line: $(tr '\n' ' ' <"$log")"
elif grep -q '^push ' "$log"; then
  problem="push line present: $(tr '\n' ' ' <"$log")"
elif [ -e "$DIGEST_LOG_PATH" ]; then
  problem="digest log exists after a layout failure"
fi
unset LAYOUT_STUB_RC STUB_PUSH_DIGEST
report "$name" "$problem"
name=confirmed_push_records_the_digest
export LAYOUT_STUB_RC=0 STUB_PUSH_DIGEST="sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
run_case "$name" --push
problem="" log="$SCRATCH/podman.log"
if [ "$rc" -ne 0 ]; then
  problem="exit $rc, want 0; stderr: $(tr '\n' ' ' <"$SCRATCH/$name.err")"
elif [ "$(wc -l <"$log")" -ne 3 ]; then
  problem="podman.log does not hold three lines: [$(tr '\n' ' ' <"$log")]"
elif [ "$(sed -n 1p <"$log" | cut -d' ' -f1)" != build ] ||
  [ "$(sed -n 2p <"$log" | cut -d' ' -f1)" != save ] ||
  [ "$(sed -n 3p <"$log" | cut -d' ' -f1)" != push ]; then
  problem="log order is not build save push: [$(tr '\n' ' ' <"$log")]"
elif ! sed -n 3p <"$log" | grep -qF -- '--digestfile'; then
  problem="push line lacks --digestfile: $(sed -n 3p <"$log")"
elif ! sed -n 3p <"$log" | grep -qF -- "$TAG"; then
  problem="push line lacks today's tag: $(sed -n 3p <"$log")"
else
  printf 'tag: %s\ndigest: %s\ncommit: %s\n' "$TAG" "$STUB_PUSH_DIGEST" "$SCRATCH_HEAD" \
    >"$SCRATCH/expected-digest.txt"
  if [ ! -f "$DIGEST_LOG_PATH" ]; then
    problem="no digest log at $DIGEST_LOG_PATH"
  elif ! cmp -s "$SCRATCH/expected-digest.txt" "$DIGEST_LOG_PATH"; then
    problem="digest log differs: [$(tr '\n' ' ' <"$DIGEST_LOG_PATH")]"
  fi
fi
unset LAYOUT_STUB_RC STUB_PUSH_DIGEST
report "$name" "$problem"
name=malformed_digest_is_refused
export LAYOUT_STUB_RC=0 STUB_PUSH_DIGEST=garbage
rm -f "$DIGEST_LOG_PATH"
run_case "$name" --push
problem=""
if [ "$rc" -eq 0 ]; then
  problem="exit 0, want non-zero (the digest is garbage)"
elif ! grep -qF 'no usable digest' "$SCRATCH/$name.err"; then
  problem="stderr lacked [no usable digest]: $(tr '\n' ' ' <"$SCRATCH/$name.err")"
elif [ -e "$DIGEST_LOG_PATH" ]; then
  problem="digest log exists after a malformed digest"
fi
report "$name" "$problem"
if [ "$failed" -ne 0 ]; then
  exit 1
fi
