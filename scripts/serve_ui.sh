#!/usr/bin/env bash
# Serves the UI against a Frigate service reached through Kubernetes.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
FRIGATE_KUBECONFIG="${FRIGATE_KUBECONFIG:-$HOME/dockers/talos/tirnanog/generated/kubeconfig}"
FRIGATE_NAMESPACE="${FRIGATE_NAMESPACE:-icams}"
FRIGATE_SERVICE="${FRIGATE_SERVICE:-frigate}"
FRIGATE_POD_SELECTOR="${FRIGATE_POD_SELECTOR:-app.kubernetes.io/name=frigate}"
FRIGATE_LOCAL_PORT="${FRIGATE_LOCAL_PORT:-5000}"
GO2RTC_LOCAL_PORT=11984
UI_ADDRESS="${UI_ADDRESS:-127.0.0.1}"
UI_PORT="${UI_PORT:-8080}"
forward_pids=()

stop_forwards() {
  for pid in "${forward_pids[@]}"; do
    # A process may already have exited after reporting a forwarding error.
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  done
}
trap stop_forwards EXIT INT TERM

frigate_pod="$(
  kubectl --kubeconfig "$FRIGATE_KUBECONFIG" -n "$FRIGATE_NAMESPACE" \
    get pods -l "$FRIGATE_POD_SELECTOR" --field-selector status.phase=Running \
    -o jsonpath='{.items[0].metadata.name}'
)"
if [[ -z "$frigate_pod" ]]; then
  echo "No running Frigate pod matched selector $FRIGATE_POD_SELECTOR." >&2
  exit 1
fi

kubectl --kubeconfig "$FRIGATE_KUBECONFIG" -n "$FRIGATE_NAMESPACE" \
  port-forward "svc/$FRIGATE_SERVICE" "$FRIGATE_LOCAL_PORT:5000" &
forward_pids+=("$!")
kubectl --kubeconfig "$FRIGATE_KUBECONFIG" -n "$FRIGATE_NAMESPACE" \
  port-forward "pod/$frigate_pod" "$GO2RTC_LOCAL_PORT:1984" &
forward_pids+=("$!")

for _ in {1..50}; do
  for pid in "${forward_pids[@]}"; do
    if ! kill -0 "$pid" 2>/dev/null; then
      wait "$pid"
    fi
  done
  if curl --fail --silent --output /dev/null \
    "http://127.0.0.1:$FRIGATE_LOCAL_PORT/api/config" \
    && curl --fail --silent --output /dev/null \
      "http://127.0.0.1:$GO2RTC_LOCAL_PORT/stream.html"; then
    break
  fi
  sleep 0.1
done

if ! curl --fail --silent --output /dev/null \
  "http://127.0.0.1:$FRIGATE_LOCAL_PORT/api/config" \
  || ! curl --fail --silent --output /dev/null \
    "http://127.0.0.1:$GO2RTC_LOCAL_PORT/stream.html"; then
  echo "Frigate and go2rtc did not become available through their forwards." >&2
  exit 1
fi

echo "Open http://$UI_ADDRESS:$UI_PORT/ in your browser."
cd "$REPO/crates/corvette-ui"
NO_COLOR=false trunk serve --address "$UI_ADDRESS" --port "$UI_PORT"
