#!/usr/bin/env bash
# Issue #12 item U2's own browser-based Verify step: builds the real UI
# bundle, a real pinned `moq-relay` (R1's own commit,
# `.agents/issue-12/evidence/R1-dependency-pin.md`), and a real
# `corvette-media-bridge` HLS listener (`examples/g3_browser_fixture.rs`,
# the same fixture item G3's own browser check drives), then drives a real
# headless browser's `<moq-watch>`-vs-HLS race
# (`crates/corvette-ui/src/expanded_view.rs`) against both real endpoints via
# `tests/ui/expanded_view.spec.cjs`.
#
# Mirrors `scripts/run_u1_live_view_browser_check.sh`'s own shape: each
# fixture is a plain process printing (or logging) its own bound port, not
# something Playwright's `webServer` config could usefully own, so this
# script starts both itself and hands the resulting origins to Playwright
# via environment variables.
#
# `MOQ_RELAY_BIN` must point at a `moq-relay` binary already built at the
# pinned commit -- the same environment variable convention item G1's own
# integration test uses (`.agents/issue-12/evidence/G1-evidence.md`'s own
# `MOQ_RELAY_BIN=<pinned moq-relay binary> cargo test ...` invocation). This
# script does not build moq-relay itself: it is a separate upstream repo
# (`https://github.com/moq-dev/moq`), not a workspace member.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"

if [[ -z "${PLAYWRIGHT_TEST_PATH:-}" ]]; then
  echo "run_u2_expanded_view_browser_check: PLAYWRIGHT_TEST_PATH is unset -- run under 'nix develop'" >&2
  exit 1
fi
if ! command -v cargo >/dev/null; then
  echo "run_u2_expanded_view_browser_check: cargo is not on PATH -- run under 'nix develop'" >&2
  exit 1
fi
if ! command -v cargo-leptos >/dev/null; then
  echo "run_u2_expanded_view_browser_check: cargo-leptos is not on PATH -- run under 'nix develop'" >&2
  exit 1
fi
if ! command -v playwright >/dev/null; then
  echo "run_u2_expanded_view_browser_check: playwright is not on PATH -- run under 'nix develop'" >&2
  exit 1
fi
if [[ ! -d "$REPO/node_modules/@moq/watch" ]]; then
  echo "run_u2_expanded_view_browser_check: node_modules/@moq/watch is missing -- run 'npm install' first" >&2
  exit 1
fi
if [[ -z "${MOQ_RELAY_BIN:-}" ]]; then
  echo "run_u2_expanded_view_browser_check: MOQ_RELAY_BIN is unset -- point it at a moq-relay binary built at" >&2
  echo "  the pinned commit (.agents/issue-12/evidence/R1-dependency-pin.md), e.g.:" >&2
  echo "  git clone https://github.com/moq-dev/moq /tmp/moq && cd /tmp/moq &&" >&2
  echo "  git checkout 7b73c43381a7f9c309e3045a8f0f858aa32ef48c && cargo build --release --bin moq-relay" >&2
  exit 1
fi
if [[ ! -x "$MOQ_RELAY_BIN" ]]; then
  echo "run_u2_expanded_view_browser_check: MOQ_RELAY_BIN ('$MOQ_RELAY_BIN') is not an executable file" >&2
  exit 1
fi

echo "Building the UI bundle (same steps as 'make check-ui')"
(cd "$REPO" && NO_COLOR=false cargo leptos build --release --split)
cp "$REPO/crates/corvette-ui/public/app.html" "$REPO/target/site/index.html"

echo "Building the browser-check fixture (real corvette-media-bridge G3 code)"
cargo build --quiet -p corvette-media-bridge --example g3_browser_fixture

hls_log="$(mktemp)"
hls_pid=""
moq_log="$(mktemp)"
moq_pid=""

cleanup() {
  if [[ -n "$hls_pid" ]]; then
    kill "$hls_pid" 2>/dev/null || true
    wait "$hls_pid" 2>/dev/null || true
  fi
  if [[ -n "$moq_pid" ]]; then
    kill "$moq_pid" 2>/dev/null || true
    wait "$moq_pid" 2>/dev/null || true
  fi
  rm -f "$hls_log" "$moq_log"
}
trap cleanup EXIT

echo "Starting the HLS fixture"
"$REPO/target/debug/examples/g3_browser_fixture" >"$hls_log" 2>&1 &
hls_pid=$!

hls_port=""
for _ in $(seq 1 100); do
  if [[ -s "$hls_log" ]] && grep -q '^LISTENING ' "$hls_log"; then
    hls_port="$(grep '^LISTENING ' "$hls_log" | head -1 | awk '{print $2}')"
    break
  fi
  if ! kill -0 "$hls_pid" 2>/dev/null; then
    echo "run_u2_expanded_view_browser_check: the HLS fixture exited before binding a port" >&2
    cat "$hls_log" >&2
    exit 1
  fi
  sleep 0.1
done
if [[ -z "$hls_port" ]]; then
  echo "run_u2_expanded_view_browser_check: the HLS fixture never printed its own bound port" >&2
  cat "$hls_log" >&2
  exit 1
fi
echo "HLS fixture listening on 127.0.0.1:$hls_port"

# Two ephemeral loopback ports (QUIC/WebTransport, plus a plain-HTTP
# fingerprint-bootstrap listener -- see below), a self-signed cert for
# "localhost" (matching R1's own precedent, `.agents/issue-12/evidence/
# R1-dependency-pin.md`), and public (unauthenticated) access -- the same
# shape R1's own two-relay QUIC handshake check used, just with a real
# browser as the second peer instead of a second `moq-relay` process.
moq_port="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')"
moq_web_port="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')"
"$MOQ_RELAY_BIN" \
  --server-bind "127.0.0.1:$moq_port" \
  --web-http-listen "127.0.0.1:$moq_web_port" \
  --tls-generate localhost \
  --auth-public / \
  --log-level info \
  >"$moq_log" 2>&1 &
moq_pid=$!

moq_ready=""
for _ in $(seq 1 100); do
  # moq-relay's own tracing output carries ANSI color escapes even when
  # redirected to a file, which can split a literal "listening addr=..."
  # match across escape codes -- the unbroken "127.0.0.1:<port>" substring
  # itself is not, so that alone is what this checks for.
  if grep -q "127.0.0.1:$moq_port" "$moq_log" 2>/dev/null; then
    moq_ready="1"
    break
  fi
  if ! kill -0 "$moq_pid" 2>/dev/null; then
    echo "run_u2_expanded_view_browser_check: moq-relay exited before binding a port" >&2
    cat "$moq_log" >&2
    exit 1
  fi
  sleep 0.1
done
if [[ -z "$moq_ready" ]]; then
  echo "run_u2_expanded_view_browser_check: moq-relay never logged that it bound $moq_port" >&2
  cat "$moq_log" >&2
  exit 1
fi
echo "moq-relay listening on 127.0.0.1:$moq_port (self-signed, public access)"

# A real browser's WebTransport implementation has no command-line or
# DevTools-Protocol override that bypasses certificate validation for a
# self-signed cert (verified directly: neither `--ignore-certificate-errors`
# nor `--ignore-certificate-errors-spki-list` nor
# `Security.setIgnoreCertificateErrors` changed the outcome one bit --
# every attempt still failed with `QUIC_TLS_CERTIFICATE_UNKNOWN`). The one
# W3C-specified mechanism for exactly this case -- an ephemeral, non-CA
# certificate, which is also why `moq-native`'s own `tls.rs` caps a
# generated cert's lifetime at 14 days ("WebTransport certificates MUST be
# valid for two weeks at most") -- is `serverCertificateHashes`, supplied by
# the PAGE at `new WebTransport(url, options)` call time, not by the
# browser's launch flags. `<moq-watch>` itself never sets this (production
# dials a real cert-manager-issued cert per INV-6, so it has no reason to),
# so `tests/ui/expanded_view.spec.cjs` patches `window.WebTransport` itself,
# for this test only, to attach it -- fetched from moq-relay's own
# `/certificate.sha256` route (`rs/moq-relay/src/web.rs`'s
# `serve_fingerprint`, exposed by `--web-http-listen` above), the same
# fingerprint-bootstrap endpoint `moq-native`'s own client uses for an
# `http://` connect URL (`rs/moq-native/src/quiche.rs`'s `fetch_fingerprint`).
moq_cert_sha256="$(curl -sf "http://127.0.0.1:$moq_web_port/certificate.sha256")"
if [[ -z "$moq_cert_sha256" ]]; then
  echo "run_u2_expanded_view_browser_check: could not fetch moq-relay's own certificate fingerprint" >&2
  exit 1
fi
echo "moq-relay certificate fingerprint: $moq_cert_sha256"

echo "Booting the real expanded view against both real endpoints"
cd "$REPO"
U2_MOQ_RELAY_URL="https://127.0.0.1:$moq_port/anon" \
  U2_MOQ_RELAY_CERT_SHA256_HEX="$moq_cert_sha256" \
  U2_HLS_ORIGIN="http://127.0.0.1:$hls_port" \
  playwright test tests/ui/expanded_view.spec.cjs
