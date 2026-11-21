#!/usr/bin/env bash
# Exercises the merged-image publish script with a stub podman and stub helpers: the refusals, the
# layout stop on a stub archive, the digest record, and append-with-previous-digest across three pushes.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="${PUBLISH_SCRIPT:-$ROOT/scripts/publish_corvette_image.sh}"
CHECK_SCRIPT="${CHECK_SCRIPT:-$ROOT/scripts/check_corvette_oci_layout.sh}"
unset CORVETTE_PUSH_CONFIRM
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
mkdir -p "$SCRATCH/bin" "$SCRATCH/repo/scripts" "$SCRATCH/repo/docker" "$SCRATCH/stubmembers"
# Stub bodies write every dollar sign as \044: a literal $ inside single quotes would fail shellcheck.
printf '#!/usr/bin/env bash\nlog=%s/podman.log\necho "\044*" >>"\044log"\ncase "\044{1:-}" in\nbuild)\n  exit "\044{STUB_BUILD_RC:-99}"\n  ;;\nsave)\n  if [ "\044{STUB_SAVE_OK:-0}" != 1 ]; then exit 99; fi\n  out=\n  prev=\n  for a in "\044@"; do\n    if [ "\044prev" = --output ]; then out="\044a"; fi\n    prev="\044a"\n  done\n  if [ -z "\044out" ]; then exit 97; fi\n  if [ -n "\044{STUB_SAVE_SEED:-}" ]; then cp "\044STUB_SAVE_SEED" "\044out"; else : >"\044out"; fi\n  ;;\npush)\n  if [ -z "\044{STUB_PUSH_DIGEST:-}" ]; then exit 99; fi\n  df=\n  prev=\n  for a in "\044@"; do\n    if [ "\044prev" = --digestfile ]; then df="\044a"; fi\n    prev="\044a"\n  done\n  if [ -z "\044df" ]; then exit 97; fi\n  printf "%%s\\n" "\044STUB_PUSH_DIGEST" >"\044df"\n  ;;\n*)\n  exit 99\n  ;;\nesac\n' "$SCRATCH" >"$SCRATCH/bin/podman"
chmod +x "$SCRATCH/bin/podman"
export PATH="$SCRATCH/bin:$PATH"
REPOSITORY="docker.io/cvandesande/corvette"
TODAY="$(date -u +%Y%m%d)"
YESTERDAY="$(date -u -d yesterday +%Y%m%d)"
TAG="$REPOSITORY:$TODAY"
DIGEST_A="sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
DIGEST_B="sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
DIGEST_C="sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
: >"$SCRATCH/stubmembers/index.json"
tar -cf "$SCRATCH/stub.oci" -C "$SCRATCH/stubmembers" index.json
cp "$SCRIPT" "$SCRATCH/repo/scripts/publish_corvette_image.sh"
touch "$SCRATCH/repo/docker/Dockerfile.corvette"
install_stub_check() {
  printf 'exit "\044{LAYOUT_STUB_RC:-0}"\n' >"$SCRATCH/repo/scripts/check_corvette_oci_layout.sh"
  chmod +x "$SCRATCH/repo/scripts/check_corvette_oci_layout.sh"
}
install_real_check() {
  cp "$CHECK_SCRIPT" "$SCRATCH/repo/scripts/check_corvette_oci_layout.sh"
  chmod +x "$SCRATCH/repo/scripts/check_corvette_oci_layout.sh"
}
env -u GIT_INDEX_FILE -u GIT_DIR -u GIT_WORK_TREE git init -q "$SCRATCH/repo"
env -u GIT_INDEX_FILE -u GIT_DIR -u GIT_WORK_TREE git -C "$SCRATCH/repo" \
  -c user.name=test -c user.email=test@example.invalid commit -q --allow-empty -m test
SCRATCH_HEAD="$(git -C "$SCRATCH/repo" rev-parse HEAD)"
SCRATCH_SCRIPT="$SCRATCH/repo/scripts/publish_corvette_image.sh"
DIGEST_LOG_PATH="$SCRATCH/repo/.agents/issue-1/evidence/corvette-digest.txt"
failed=0
rc=0
run_case() {
  local name="$1"
  shift
  : >"$SCRATCH/$name.out"
  : >"$SCRATCH/$name.err"
  rm -f "$SCRATCH/podman.log"
  rc=0
  timeout 30 env -u GIT_INDEX_FILE -u GIT_DIR -u GIT_WORK_TREE "$@" </dev/null \
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
run_case "$name" "$SCRIPT" --bogus
refused "$name" 'unknown argument: --bogus'
name=push_without_confirmation_is_refused
run_case "$name" "$SCRIPT" --push
refused "$name" 'refusing to push' "CORVETTE_PUSH_CONFIRM=$TAG"
name=push_with_yesterdays_confirmation_is_refused
run_case "$name" env "CORVETTE_PUSH_CONFIRM=$REPOSITORY:$YESTERDAY" "$SCRIPT" --push
refused "$name" 'refusing to push'
name=push_with_todays_confirmation_reaches_the_build
before="$(git -C "$ROOT" status --short -uall .agents/issue-1/evidence)"
run_case "$name" env "CORVETTE_PUSH_CONFIRM=$TAG" "$SCRIPT" --push
after="$(git -C "$ROOT" status --short -uall .agents/issue-1/evidence)"
problem=""
log="$SCRATCH/podman.log"
if [ "$rc" -eq 0 ]; then
  problem="exit 0, want non-zero (the stub exits 99)"
elif [ ! -s "$log" ] || [ "$(wc -l <"$log")" -ne 1 ]; then
  problem="podman.log does not hold exactly one line: [$(cat "$log" 2>/dev/null)]"
elif ! grep -q '^build ' "$log"; then
  problem="line does not start with build: $(cat "$log")"
elif ! grep -qF -- 'docker/Dockerfile.corvette' "$log"; then
  problem="line lacks the merged Dockerfile: $(cat "$log")"
elif ! grep -qF -- "$TAG" "$log"; then
  problem="line lacks today's tag: $(cat "$log")"
elif [ "$before" != "$after" ]; then
  problem="evidence tree changed: before[$before] after[$after]"
fi
report "$name" "$problem"
install_stub_check
name=default_mode_builds_and_saves_without_pushing
run_case "$name" env -u CORVETTE_PUSH_CONFIRM STUB_BUILD_RC=0 STUB_SAVE_OK=1 \
  "$SCRATCH_SCRIPT"
problem=""
log="$SCRATCH/podman.log"
if [ "$rc" -ne 0 ]; then
  problem="exit $rc, want 0; stderr: $(tr '\n' ' ' <"$SCRATCH/$name.err")"
elif ! grep -qF 'built local OCI archive at' "$SCRATCH/$name.out"; then
  problem="stdout lacks the success message: [$(tr '\n' ' ' <"$SCRATCH/$name.out")]"
elif [ ! -s "$log" ] || [ "$(wc -l <"$log")" -ne 2 ]; then
  problem="podman.log does not hold two lines: [$(tr '\n' ' ' <"$log" 2>/dev/null)]"
elif [ "$(sed -n 1p "$log" | cut -d' ' -f1)" != build ]; then
  problem="log line 1 is not a build: $(sed -n 1p "$log")"
elif [ "$(sed -n 2p "$log" | cut -d' ' -f1)" != save ]; then
  problem="log line 2 is not a save: $(sed -n 2p "$log")"
elif grep -q '^push ' "$log"; then
  problem="push line present without --push: [$(tr '\n' ' ' <"$log")]"
elif [ -e "$DIGEST_LOG_PATH" ]; then
  problem="digest log written in local mode"
fi
report "$name" "$problem"
name=layout_failure_stops_before_the_push
install_real_check
run_case "$name" env -u CORVETTE_PUSH_CONFIRM STUB_BUILD_RC=0 STUB_SAVE_OK=1 \
  STUB_SAVE_SEED="$SCRATCH/stub.oci" "$SCRATCH_SCRIPT"
problem=""
log="$SCRATCH/podman.log"
if [ "$rc" -eq 0 ]; then
  problem="exit 0, want non-zero (the layout check fails on the stub archive)"
elif grep -qF 'built local OCI archive at' "$SCRATCH/$name.out"; then
  problem="success message printed despite the failing layout check"
elif grep -q '^push ' "$log" 2>/dev/null; then
  problem="push line present after a layout failure: [$(tr '\n' ' ' <"$log")]"
elif [ -e "$DIGEST_LOG_PATH" ]; then
  problem="digest log exists after a layout failure"
fi
install_stub_check
report "$name" "$problem"
name=confirmed_push_records_the_digest
run_case "$name" env "CORVETTE_PUSH_CONFIRM=$TAG" STUB_BUILD_RC=0 STUB_SAVE_OK=1 \
  STUB_PUSH_DIGEST="$DIGEST_A" "$SCRATCH_SCRIPT" --push
problem=""
log="$SCRATCH/podman.log"
if [ "$rc" -ne 0 ]; then
  problem="exit $rc, want 0; stderr: $(tr '\n' ' ' <"$SCRATCH/$name.err")"
elif [ "$(wc -l <"$log")" -ne 3 ]; then
  problem="podman.log does not hold three lines: [$(tr '\n' ' ' <"$log")]"
elif [ "$(sed -n 1p "$log" | cut -d' ' -f1)" != build ] ||
  [ "$(sed -n 2p "$log" | cut -d' ' -f1)" != save ] ||
  [ "$(sed -n 3p "$log" | cut -d' ' -f1)" != push ]; then
  problem="log order is not build save push: [$(tr '\n' ' ' <"$log")]"
elif ! sed -n 3p "$log" | grep -qF -- '--digestfile'; then
  problem="push line lacks --digestfile: $(sed -n 3p "$log")"
else
  printf 'tag: %s\ndigest: %s\ncommit: %s\n' "$TAG" "$DIGEST_A" "$SCRATCH_HEAD" \
    >"$SCRATCH/expected-a.txt"
  if [ ! -f "$DIGEST_LOG_PATH" ]; then
    problem="no digest log at $DIGEST_LOG_PATH"
  elif ! cmp -s "$SCRATCH/expected-a.txt" "$DIGEST_LOG_PATH"; then
    problem="digest log differs: [$(tr '\n' ' ' <"$DIGEST_LOG_PATH")]"
  fi
fi
report "$name" "$problem"
name=second_push_appends_and_prints_previous_digest
run_case "$name" env "CORVETTE_PUSH_CONFIRM=$TAG" STUB_BUILD_RC=0 STUB_SAVE_OK=1 \
  STUB_PUSH_DIGEST="$DIGEST_B" "$SCRATCH_SCRIPT" --push
problem=""
if [ "$rc" -ne 0 ]; then
  problem="exit $rc, want 0; stderr: $(tr '\n' ' ' <"$SCRATCH/$name.err")"
elif [ "$(wc -l <"$DIGEST_LOG_PATH")" -ne 6 ]; then
  problem="digest log does not hold two blocks: [$(tr '\n' ' ' <"$DIGEST_LOG_PATH")]"
elif ! cmp -s "$SCRATCH/expected-a.txt" <(sed -n 1,3p "$DIGEST_LOG_PATH"); then
  problem="first block changed by the second push: [$(tr '\n' ' ' <"$DIGEST_LOG_PATH")]"
else
  printf 'tag: %s\ndigest: %s\ncommit: %s\n' "$TAG" "$DIGEST_B" "$SCRATCH_HEAD" \
    >"$SCRATCH/expected-b.txt"
  if ! cmp -s "$SCRATCH/expected-b.txt" <(sed -n 4,6p "$DIGEST_LOG_PATH"); then
    problem="second block differs: [$(sed -n 4,6p "$DIGEST_LOG_PATH" | tr '\n' ' ')]"
  elif ! grep -qF "previous push digest: $DIGEST_A" "$SCRATCH/$name.out"; then
    problem="stdout lacks the previous digest print: [$(tr '\n' ' ' <"$SCRATCH/$name.out")]"
  fi
fi
report "$name" "$problem"
name=third_push_prints_the_second_push_not_the_first
run_case "$name" env "CORVETTE_PUSH_CONFIRM=$TAG" STUB_BUILD_RC=0 STUB_SAVE_OK=1 \
  STUB_PUSH_DIGEST="$DIGEST_C" "$SCRATCH_SCRIPT" --push
problem=""
if [ "$rc" -ne 0 ]; then
  problem="exit $rc, want 0; stderr: $(tr '\n' ' ' <"$SCRATCH/$name.err")"
elif [ "$(wc -l <"$DIGEST_LOG_PATH")" -ne 9 ]; then
  problem="digest log does not hold three blocks: [$(tr '\n' ' ' <"$DIGEST_LOG_PATH")]"
else
  printf 'tag: %s\ndigest: %s\ncommit: %s\n' "$TAG" "$DIGEST_C" "$SCRATCH_HEAD" \
    >"$SCRATCH/expected-c.txt"
  if ! cmp -s "$SCRATCH/expected-c.txt" <(sed -n 7,9p "$DIGEST_LOG_PATH"); then
    problem="third block differs: [$(sed -n 7,9p "$DIGEST_LOG_PATH" | tr '\n' ' ')]"
  elif ! grep -qF "previous push digest: $DIGEST_B" "$SCRATCH/$name.out"; then
    problem="stdout lacks the second push digest as previous: [$(tr '\n' ' ' <"$SCRATCH/$name.out")]"
  elif grep -qF "previous push digest: $DIGEST_A" "$SCRATCH/$name.out"; then
    problem="stdout printed the oldest block as previous, not the second push's"
  fi
fi
report "$name" "$problem"
name=malformed_digest_is_refused
rm -f "$DIGEST_LOG_PATH"
run_case "$name" env "CORVETTE_PUSH_CONFIRM=$TAG" STUB_BUILD_RC=0 STUB_SAVE_OK=1 \
  STUB_PUSH_DIGEST=garbage "$SCRATCH_SCRIPT" --push
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
