#!/usr/bin/env bash
# Issue #12 item U1's own browser-based Verify step: builds the real UI
# bundle and a real `corvette-media-bridge` fMP4-over-WebSocket endpoint
# (`crates/corvette-media-bridge/examples/g2_browser_fixture.rs`, the same
# fixture item G2's own browser check drives), then drives a real headless
# browser's MediaSource/SourceBuffer against the grid tile's own production
# client (`crates/corvette-ui/src/live_view.rs`) end to end.
#
# Mirrors `scripts/run_g2_fmp4_ws_browser_check.sh`'s own shape: the fixture
# is a plain Rust binary printing its own bound port, not an HTTP server
# Playwright's `webServer` config could usefully own, so this script reads
# that port itself and hands the resulting WebSocket origin to Playwright via
# an environment variable `tests/ui/live_view.spec.cjs` reads. The UI server
# itself IS started by Playwright's own `webServer` entry
# (`playwright.config.cjs`), the same way every other `tests/ui/*.spec.cjs`
# run already relies on.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"

if [[ -z "${PLAYWRIGHT_TEST_PATH:-}" ]]; then
  echo "run_u1_live_view_browser_check: PLAYWRIGHT_TEST_PATH is unset -- run under 'nix develop'" >&2
  exit 1
fi
if ! command -v cargo >/dev/null; then
  echo "run_u1_live_view_browser_check: cargo is not on PATH -- run under 'nix develop'" >&2
  exit 1
fi
if ! command -v cargo-leptos >/dev/null; then
  echo "run_u1_live_view_browser_check: cargo-leptos is not on PATH -- run under 'nix develop'" >&2
  exit 1
fi
if ! command -v playwright >/dev/null; then
  echo "run_u1_live_view_browser_check: playwright is not on PATH -- run under 'nix develop'" >&2
  exit 1
fi

echo "Building the UI bundle (same steps as 'make check-ui')"
(cd "$REPO" && NO_COLOR=false cargo leptos build --release --split)
cp "$REPO/crates/corvette-ui/public/app.html" "$REPO/target/site/index.html"

echo "Building the browser-check fixture (real corvette-media-bridge G2 code)"
cargo build --quiet -p corvette-media-bridge --example g2_browser_fixture

fixture_log="$(mktemp)"
fixture_pid=""

stop_fixture() {
  if [[ -n "$fixture_pid" ]]; then
    kill "$fixture_pid" 2>/dev/null || true
    wait "$fixture_pid" 2>/dev/null || true
  fi
  rm -f "$fixture_log"
}
trap stop_fixture EXIT

echo "Starting the fixture"
"$REPO/target/debug/examples/g2_browser_fixture" >"$fixture_log" 2>&1 &
fixture_pid=$!

port=""
for _ in $(seq 1 100); do
  if [[ -s "$fixture_log" ]] && grep -q '^LISTENING ' "$fixture_log"; then
    port="$(grep '^LISTENING ' "$fixture_log" | head -1 | awk '{print $2}')"
    break
  fi
  if ! kill -0 "$fixture_pid" 2>/dev/null; then
    echo "run_u1_live_view_browser_check: the fixture exited before binding a port" >&2
    cat "$fixture_log" >&2
    exit 1
  fi
  sleep 0.1
done

if [[ -z "$port" ]]; then
  echo "run_u1_live_view_browser_check: the fixture never printed its own bound port" >&2
  cat "$fixture_log" >&2
  exit 1
fi

echo "Fixture listening on 127.0.0.1:$port"

echo "Booting the real grid tile against the fixture's own output"
cd "$REPO"
U1_LIVE_VIEW_WS_ORIGIN="ws://127.0.0.1:$port" \
  U1_LIVE_VIEW_CAMERA_NAME="browser-check" \
  playwright test tests/ui/live_view.spec.cjs
