#!/usr/bin/env bash
# Exercises the publish script's guards with a stub docker: unknown argument,
# unconfirmed or stale push refusal, and the confirmed build invocation.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="${PUBLISH_SCRIPT:-$ROOT/scripts/publish_detect_worker_image.sh}"
unset CORVETTE_DETECT_WORKER_PUSH_CONFIRM
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
mkdir -p "$SCRATCH/bin"
cat >"$SCRATCH/bin/docker" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >>"$SCRATCH/docker.log"
exit 99
EOF
chmod +x "$SCRATCH/bin/docker"
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
  rm -f "$SCRATCH/docker.log"
  rc=0
  timeout 30 "$SCRIPT" "$@" </dev/null \
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
    if [ -z "$problem" ] && [ -s "$SCRATCH/docker.log" ]; then
      problem="docker.log was not empty: $(cat "$SCRATCH/docker.log")"
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
problem="" log="$SCRATCH/docker.log"
if [ "$rc" -eq 0 ]; then
  problem="exit 0, want non-zero (the stub always exits 99)"
elif [ ! -s "$log" ] || [ "$(wc -l <"$log")" -ne 1 ]; then
  problem="docker.log does not hold exactly one line: [$(cat "$log" 2>/dev/null)]"
elif ! grep -q '^buildx build' "$log"; then
  problem="line does not start with buildx build: $(cat "$log")"
elif ! grep -qF -- '--push' "$log"; then
  problem="line lacks --push: $(cat "$log")"
elif ! grep -qF -- '--metadata-file' "$log"; then
  problem="line lacks --metadata-file: $(cat "$log")"
elif ! grep -qF 'docker/Dockerfile.detect-worker' "$log"; then
  problem="line lacks the detect-worker Dockerfile: $(cat "$log")"
elif ! grep -qF "$TAG" "$log"; then
  problem="line lacks today's tag: $(cat "$log")"
elif [ "$before" != "$after" ]; then
  problem="evidence tree changed: before[$before] after[$after]"
fi
report "$name" "$problem"
if [ "$failed" -ne 0 ]; then
  exit 1
fi
