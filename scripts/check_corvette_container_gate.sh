#!/usr/bin/env bash
# The local packaging gate: every command, probe and socket the pod and Compose files will use, run
# against a locally built corvette image under podman with no network and no GPU. One PASS or FAIL
# line per check; exit 0 only if all pass. Socket connects use corvette-detect-service --probe, the
# in-image client, so the image needs no shell socket tool for this gate to run.
set -u

if ! command -v podman >/dev/null 2>&1; then
  echo "SKIP check_corvette_container_gate: podman is not installed"
  exit 2
fi

image="${1:-}"
if [[ -z "$image" ]]; then
  image="$(podman images --sort created --format '{{.Repository}}:{{.Tag}}' |
    grep '^docker\.io/cvandesande/corvette:' | head -n 1)"
fi
if [[ -z "$image" ]]; then
  echo "FAIL image-selection: no local corvette image; pass one as the first argument"
  exit 1
fi

failed=0
pass() { echo "PASS $1"; }
fail() {
  echo "FAIL $1: $2"
  failed=1
}

check_exit() {
  local name="$1" want="$2"
  shift 2
  local out rc
  out="$(timeout 120 podman run --rm --pull=never --network none "$image" "$@" 2>&1)"
  rc=$?
  if [[ "$rc" -eq "$want" ]]; then
    pass "$name"
  else
    fail "$name" "exit $rc, want $want; first line: ${out%%$'\n'*}"
  fi
}

check_exit_and_output() {
  local name="$1" want="$2" needle="$3"
  shift 3
  local out rc
  out="$(timeout 120 podman run --rm --pull=never --network none "$image" "$@" 2>&1)"
  rc=$?
  if [[ "$rc" -eq "$want" && "$out" == *"$needle"* ]]; then
    pass "$name"
  else
    fail "$name" "exit $rc (want $want) or output lacks [$needle]: ${out:0:200}"
  fi
}

check_nonzero_and_output() {
  local name="$1" needle="$2"
  shift 2
  local out rc
  out="$(timeout 120 podman run --rm --pull=never --network none "$image" "$@" 2>&1)"
  rc=$?
  if [[ "$rc" -ne 0 && "$out" == *"$needle"* ]]; then
    pass "$name"
  else
    fail "$name" "exit $rc (want non-zero) or output lacks [$needle]: ${out:0:200}"
  fi
}

check_exit_and_output api-server-usage-documented 1 "CORVETTE_API_SOCKET is not set" corvette-api-server
check_nonzero_and_output media-bridge-usage-documented "CORVETTE_MEDIA_BRIDGE_CONFIG is not set" corvette-media-bridge
check_exit_and_output moq-relay-version-ok 0 "moq-relay" moq-relay --version
check_exit_and_output detect-service-help-ok 0 "exit codes:" corvette-detect-service --help
check_exit detect-worker-help-ok 0 corvette-detect-worker --help
check_exit detect-service-probe-missing-socket-fails-clean 1 \
  corvette-detect-service --probe --socket /nonexistent/status.sock

name=no-command-fails
out="$(timeout 120 podman run --rm --pull=never --network none "$image" 2>&1)"
rc=$?
if [[ "$rc" -ne 0 ]]; then
  pass "$name"
else
  fail "$name" "container with no command: ran to exit $rc, want the runtime's no-command failure: ${out:0:200}"
fi

api_script="$(cat <<'API_GATE'
corvette-api-server &
srv=$!
i=0
until [ -S /run/c/api.sock ] || [ "$i" -ge 50 ]; do i=$((i + 1)); sleep 0.2; done
mode="$(stat -c %a /run/c/api.sock 2>/dev/null)"
kill "$srv" 2>/dev/null
[ "$mode" = "666" ]
API_GATE
)"
name=api-server-binds-socket-0666-as-65532
out="$(timeout 180 podman run --rm --pull=never --network none --user 65532 --tmpfs /run/c:mode=1777 \
  -e CORVETTE_API_SOCKET=/run/c/api.sock "$image" sh -c "$api_script" 2>&1)"
rc=$?
if [[ "$rc" -eq 0 ]]; then
  pass "$name"
else
  fail "$name" "exit $rc: ${out:0:200}"
fi

detect_script="$(cat <<'DETECT_GATE'
printf "worker_socket = \"/run/c/worker.sock\"\nlabels = \"/run/c/labels.txt\"\ndeadline_ms = 5000\ncameras = []\n" > /run/c/config.toml
printf "0 person\n" > /run/c/labels.txt
: > /run/c/param
: > /run/c/bin
corvette-detect-service --config /run/c/config.toml --socket /run/c/status.sock \
  --ffmpeg /usr/bin/ffmpeg --restream-address 127.0.0.1:8554 --worker /bin/false \
  --worker-param /run/c/param --worker-bin /run/c/bin --worker-max-deadline-ms 5000 \
  --worker-health-socket /run/c/health.sock &
srv=$!
i=0
until [ -S /run/c/status.sock ] || [ "$i" -ge 50 ]; do i=$((i + 1)); sleep 0.2; done
mode="$(stat -c %a /run/c/status.sock 2>/dev/null)"
corvette-detect-service --probe --socket /run/c/status.sock
probe_rc=$?
kill "$srv" 2>/dev/null
[ "$mode" = "666" ] && [ "$probe_rc" -eq 0 ]
DETECT_GATE
)"
for uid_run in 0 65532; do
  name="detect-status-socket-0666-as-$uid_run"
  out="$(timeout 180 podman run --rm --pull=never --network none --user "$uid_run" --tmpfs /run/c:mode=1777 \
    "$image" sh -c "$detect_script" 2>&1)"
  rc=$?
  if [[ "$rc" -eq 0 ]]; then
    pass "$name"
  else
    fail "$name" "exit $rc: ${out:0:200}"
  fi
done

if [[ "$failed" -ne 0 ]]; then
  exit 1
fi
