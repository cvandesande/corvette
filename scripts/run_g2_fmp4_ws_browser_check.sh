#!/usr/bin/env bash
# Issue #12 item G2's own browser-based Verify step: builds and runs
# `examples/g2_browser_fixture.rs` (this crate's real fMP4/WebSocket code,
# fed a real H.264 SPS/PPS/IDR triple), then drives a real headless browser's
# MediaSource/SourceBuffer against its actual output via
# `crates/corvette-media-bridge/tests/browser/g2_fmp4_ws.spec.cjs`.
#
# Mirrors `scripts/run_nginx_parity.sh`'s own shape: this item's fixture is a
# plain Rust binary printing its own bound port, not an HTTP server
# Playwright's `webServer` config could usefully own -- so this script reads
# that port itself and hands the resulting `ws://` URL to Playwright via an
# environment variable, rather than teaching the fixture or the config about
# each other.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
HARNESS="$REPO/crates/corvette-media-bridge/tests/browser"

if [[ -z "${PLAYWRIGHT_TEST_PATH:-}" ]]; then
  echo "run_g2_fmp4_ws_browser_check: PLAYWRIGHT_TEST_PATH is unset -- run under 'nix develop'" >&2
  exit 1
fi
if ! command -v cargo >/dev/null; then
  echo "run_g2_fmp4_ws_browser_check: cargo is not on PATH -- run under 'nix develop'" >&2
  exit 1
fi
if ! command -v playwright >/dev/null; then
  echo "run_g2_fmp4_ws_browser_check: playwright is not on PATH -- run under 'nix develop'" >&2
  exit 1
fi

echo "Building the browser-check fixture"
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
    echo "run_g2_fmp4_ws_browser_check: the fixture exited before binding a port" >&2
    cat "$fixture_log" >&2
    exit 1
  fi
  sleep 0.1
done

if [[ -z "$port" ]]; then
  echo "run_g2_fmp4_ws_browser_check: the fixture never printed its own bound port" >&2
  cat "$fixture_log" >&2
  exit 1
fi

echo "Fixture listening on 127.0.0.1:$port"

echo "Booting a real MSE SourceBuffer against the fixture's own output"
G2_FMP4_WS_URL="ws://127.0.0.1:$port/browser-check" playwright test \
  --config "$HARNESS/playwright.config.cjs"
