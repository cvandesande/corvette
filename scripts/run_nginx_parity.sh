#!/usr/bin/env bash
# Serves the publish tree through Frigate's own nginx configuration and asserts
# what the deployment would return for it.
#
# The configuration under tests/nginx-parity/vendor is a byte copy of Frigate
# v0.17.2's; tests/nginx-parity/offline.patch carries the edits that let it run
# unprivileged with no backend, and nothing else. The local dev server
# (crates/corvette-ui-server) answers most of these paths too, and disagrees
# with the deployment on the fallback filename, $uri.html, cache headers, MIME
# types, sub_filter and every proxied route -- which is why it cannot stand in
# for this.
#
# Every route assertion states a content type and a body predicate as well as a
# status. Both failure modes this exists to catch return HTTP 200: a client
# route shadowed by a filesystem location, and an iframe that loads this site
# into itself.
#
# Usage: run_nginx_parity.sh [--expect FILE] [--extra-patch FILE]
#   --expect       expectations file naming what a missing /pkg/ asset returns
#                  and what Cache-Control /pkg/ carries. Defaults to the
#                  measured behaviour of the unmodified vendored config.
#   --extra-patch  a further patch applied to the vendored config after the
#                  offline one, for testing a proposed change to it.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
HARNESS="$REPO/tests/nginx-parity"
WORK="$REPO/target/nginx-parity"
PUBLISH_TREE="$REPO/target/site-publish"

# Fixed so the applied patch can be a plain checked-in diff. A port already in
# use is a hard stop below rather than something to work around: the harness
# must know it is talking to its own nginx.
WEB_PORT=18971
INTERNAL_PORT=15000
FRIGATE_STUB_PORT=15001
GO2RTC_STUB_PORT=11984

BASE_URL="http://127.0.0.1:$WEB_PORT"

expectations="$HARNESS/expectations/donor-config"
extra_patch=""

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --expect)
      expectations="$2"
      shift 2
      ;;
    --extra-patch)
      extra_patch="$2"
      shift 2
      ;;
    *)
      echo "run_nginx_parity: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

# Phase 1: refuse to run on anything but a complete, quiet environment.

for tool in nginx curl patch cmp; do
  if ! command -v "$tool" >/dev/null; then
    echo "run_nginx_parity: $tool is not on PATH -- run under 'nix develop'" >&2
    exit 1
  fi
done

if [[ -z "${PLAYWRIGHT_TEST_PATH:-}" ]]; then
  echo "run_nginx_parity: PLAYWRIGHT_TEST_PATH is unset -- run under 'nix develop'" >&2
  exit 1
fi

# Only enough of the tree to establish that a build produced it. Whether it has
# the right shape -- an index.html rather than the build's own app.html, and
# nothing that would displace a donor file -- is what the assertions below
# measure, so asserting it here as well would turn a measurable failure into an
# environment error.
if [[ ! -f "$PUBLISH_TREE/pkg/corvette.wasm" ]]; then
  echo "run_nginx_parity: no publish tree at $PUBLISH_TREE -- run scripts/build_site.sh" >&2
  exit 1
fi

# Connecting rather than listing sockets: this needs no iproute2 and no
# permission to see other users' processes.
for port in "$WEB_PORT" "$INTERNAL_PORT" "$FRIGATE_STUB_PORT" "$GO2RTC_STUB_PORT"; do
  if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
    exec 3>&-
    echo "run_nginx_parity: port $port is already in use" >&2
    exit 1
  fi
done

# Phase 2: read the expectations. Every key is required -- a missing one would
# otherwise turn its assertion into an empty comparison that always passes.

missing_pkg_asset_status=""
missing_pkg_asset_content_type=""
missing_pkg_asset_is_shell=""
pkg_cache_control=""

if [[ ! -f "$expectations" ]]; then
  echo "run_nginx_parity: no such expectations file: $expectations" >&2
  exit 1
fi

while IFS= read -r line; do
  [[ -z "$line" || "$line" == \#* ]] && continue
  key="${line%%=*}"
  value="${line#*=}"
  case "$key" in
    missing_pkg_asset_status) missing_pkg_asset_status="$value" ;;
    missing_pkg_asset_content_type) missing_pkg_asset_content_type="$value" ;;
    missing_pkg_asset_is_shell) missing_pkg_asset_is_shell="$value" ;;
    pkg_cache_control) pkg_cache_control="$value" ;;
    *)
      echo "run_nginx_parity: unknown key in $expectations: $key" >&2
      exit 1
      ;;
  esac
done <"$expectations"

for required in missing_pkg_asset_status missing_pkg_asset_content_type \
  missing_pkg_asset_is_shell pkg_cache_control; do
  if [[ -z "${!required}" ]]; then
    echo "run_nginx_parity: $expectations does not set $required" >&2
    exit 1
  fi
done

case "$missing_pkg_asset_is_shell" in
  yes | no) ;;
  *)
    echo "run_nginx_parity: missing_pkg_asset_is_shell must be yes or no," \
      "got '$missing_pkg_asset_is_shell'" >&2
    exit 1
    ;;
esac

# Phase 3: stage the nginx prefix -- configuration, web root, media root.

rm -rf "$WORK"
mkdir -p "$WORK/conf" "$WORK/logs" "$WORK/cache" "$WORK/web" "$WORK/media" "$WORK/stub" "$WORK/out"

cp "$HARNESS"/vendor/*.conf "$WORK/conf/"
patch -p1 -d "$WORK/conf" --silent -i "$HARNESS/offline.patch"
if [[ -n "$extra_patch" ]]; then
  patch -p1 -d "$WORK/conf" --silent -i "$extra_patch"
fi

# Both of these are generated inside the container from Go templates rather
# than shipped, so there is nothing to vendor. listen.conf follows
# templates/listen.gotmpl with no TLS and no IPv6 section: the internal port
# first, then the one external traffic reaches. base_path.conf is what
# templates/base_path.gotmpl renders when no base path is set -- empty.
printf 'listen %s;\nlisten %s;\n' "$INTERNAL_PORT" "$WEB_PORT" >"$WORK/conf/listen.conf"
: >"$WORK/conf/base_path.conf"

nginx_prefix="$(dirname "$(dirname "$(readlink -f "$(command -v nginx)")")")"
cp "$nginx_prefix/conf/mime.types" "$WORK/conf/mime.types"
cp "$HARNESS/fixtures/stub/upstreams.conf" "$WORK/conf/upstreams.conf"
cp "$HARNESS/fixtures/stub/webrtc.html" "$WORK/stub/webrtc.html"
cp -r "$HARNESS/fixtures/media/." "$WORK/media/"

# The web root is the donor's directory with the publish tree laid over it,
# which is how the shipped image builds it: the site's own index.html displaces
# the donor's, and every other donor file stays.
cp -r "$HARNESS/fixtures/web/." "$WORK/web/"
cp -r "$PUBLISH_TREE/." "$WORK/web/"

for donor in login.html robots.txt notifications-worker.js assets/index-donor.js \
  fonts/donor.woff2 locales/en.json; do
  if [[ ! -f "$WORK/web/$donor" ]]; then
    echo "run_nginx_parity: the overlay displaced a donor file: $donor" >&2
    exit 1
  fi
done

# Phase 4: start the upstream stubs and the nginx under test.

stop_servers() {
  if [[ -n "${web_nginx_pid:-}" ]]; then
    kill "$web_nginx_pid" 2>/dev/null || true
    wait "$web_nginx_pid" 2>/dev/null || true
  fi
  nginx -p "$WORK" -e logs/stub-error.log -c conf/upstreams.conf -s quit 2>/dev/null || true
}
trap stop_servers EXIT

nginx -p "$WORK" -e logs/stub-error.log -c conf/upstreams.conf

# The vendored configuration says `daemon off`, so this nginx holds the
# terminal until it is stopped; stop_servers is what stops it.
nginx -p "$WORK" -e logs/error.log -c conf/nginx.conf &
web_nginx_pid=$!

wait_for_port() {
  local port=$1
  for _ in $(seq 1 100); do
    if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
      exec 3>&-
      return 0
    fi
    sleep 0.1
  done
  echo "run_nginx_parity: nothing accepted a connection on port $port" >&2
  tail -20 "$WORK/logs/error.log" "$WORK/logs/stub-error.log" 2>/dev/null || true
  return 1
}

wait_for_port "$FRIGATE_STUB_PORT"
wait_for_port "$GO2RTC_STUB_PORT"
wait_for_port "$WEB_PORT"

# Phase 5: the assertions.

checks_run=0
checks_failed=0

fail() {
  echo "  FAIL $1" >&2
  checks_failed=$((checks_failed + 1))
}

pass() {
  echo "  ok   $1"
}

# Paths the harness does not model. Their upstreams are stubs and the vod
# module is not in this nginx at all, so an assertion about one of them would
# be measuring the fixture rather than the deployment. The one exception is the
# go2rtc player page, which is a real fixture on purpose.
refuse_unmodelled_path() {
  local path=$1
  case "$path" in
    /live/webrtc/webrtc.html) return 0 ;;
    /api/* | /vod/* | /clips/* | /stream/* | /exports/* | /ws | /ws/* | /live/*)
      echo "run_nginx_parity: $path is proxied or module-backed and is not" \
        "modelled here; measure it against the deployment" >&2
      exit 1
      ;;
  esac
}

# Writes headers to $2.head and the body to $2.body, and prints the status.
http_get() {
  local path=$1 out=$2 header=${3:-}
  local -a curl_args=(--silent --show-error --dump-header "$out.head"
    --output "$out.body" --write-out '%{http_code}')
  if [[ -n "$header" ]]; then
    curl_args+=(--header "$header")
  fi
  curl "${curl_args[@]}" "$BASE_URL$path"
}

header_value() {
  # Last wins: nginx emits Cache-Control twice where an `expires` directive and
  # an add_header both apply, and the assertions here are about what the
  # configuration sets, which is the add_header. An absent header is an empty
  # string rather than a failure, because absence is one of the things asserted.
  { grep -i "^$2:" "$1" || true; } | tail -1 | cut -d: -f2- | tr -d '\r' | sed 's/^ *//'
}

# Removes the one rewrite this configuration performs on any HTML response it
# serves itself, so two documents can be compared for being the same document.
# A response proxied from an upstream is not rewritten, so comparing a proxied
# body against a served one without this would find a difference of exactly one
# script tag and call two copies of the same page different.
strip_injected_script() {
  sed 's|<script>window\.baseUrl="[^"]*";</script>||' "$1"
}

# Asserts a route's status, content type and at least one body predicate.
# Predicates are contains:TEXT or absent:TEXT. Refusing to run without one is
# the point: a check that only reads the status passes on the shell HTML that
# every unmatched path returns.
assert_route() {
  local name=$1 path=$2 want_status=$3 want_type=$4
  shift 4
  if [[ "$#" -eq 0 ]]; then
    echo "run_nginx_parity: $name states no body predicate" >&2
    exit 1
  fi
  refuse_unmodelled_path "$path"
  checks_run=$((checks_run + 1))

  # One file per check, so a failed run leaves every response on disk to read.
  local out="$WORK/out/${name//[^A-Za-z0-9]/-}"
  local status
  status="$(http_get "$path" "$out")"
  local content_type
  content_type="$(header_value "$out.head" content-type)"

  if [[ "$status" != "$want_status" ]]; then
    fail "$name: status $status, expected $want_status"
    return
  fi
  if [[ "$content_type" != "$want_type"* ]]; then
    fail "$name: content-type '$content_type', expected '$want_type'"
    return
  fi
  local predicate text
  for predicate in "$@"; do
    text="${predicate#*:}"
    case "$predicate" in
      contains:*)
        if ! grep -qF -- "$text" "$out.body"; then
          fail "$name: body does not contain '$text'"
          return
        fi
        ;;
      absent:*)
        if grep -qF -- "$text" "$out.body"; then
          fail "$name: body contains '$text'"
          return
        fi
        ;;
      *)
        echo "run_nginx_parity: $name has an unreadable predicate: $predicate" >&2
        exit 1
        ;;
    esac
  done
  pass "$name: $status $content_type"
}

echo "Serving $PUBLISH_TREE over the donor tree at $BASE_URL"
echo "Expectations: $expectations"

# The site's own shell, not the donor's, and the same bytes for every client
# route: this is what the build-time rename to index.html buys.
assert_route "shell at /" / 200 text/html \
  contains:/pkg/corvette.js absent:donor-react-shell
assert_route "shell at /events" /events 200 text/html \
  contains:/pkg/corvette.js absent:donor-react-shell
assert_route "shell at /recordings" /recordings 200 text/html \
  contains:/pkg/corvette.js absent:donor-react-shell

checks_run=$((checks_run + 1))
http_get / "$WORK/out/root" >/dev/null
http_get /events "$WORK/out/events" >/dev/null
http_get /recordings "$WORK/out/recordings" >/dev/null
if cmp -s "$WORK/out/root.body" "$WORK/out/events.body" &&
  cmp -s "$WORK/out/root.body" "$WORK/out/recordings.body"; then
  pass "client routes return the same bytes as /"
else
  fail "client routes do not return the same bytes as /"
fi

# A trailing slash leaves the fallback entirely: /recordings/ is a filesystem
# location with a JSON autoindex, so the two spellings are different resources.
assert_route "/recordings/ is the media autoindex" /recordings/ 200 application/json \
  contains:2026-08-04 contains:directory absent:/pkg/corvette.js

# The donor's login page still resolves, through the $uri.html step, and is not
# the shell.
assert_route "donor login page" /login 200 text/html \
  contains:donor-login-page absent:/pkg/corvette.js
assert_route "donor robots.txt" /robots.txt 200 text/plain contains:User-agent
assert_route "donor locale" /locales/en.json 200 application/json contains:donor
assert_route "donor asset" /assets/index-donor.js 200 application/javascript \
  contains:donor-asset

# The contrast that makes the /pkg/ expectation below meaningful: a miss under
# a nested location is a real 404, because nested locations do not inherit
# try_files.
assert_route "missing asset under /assets/" /assets/does-not-exist.js 404 text/html \
  contains:404

# The go2rtc player page, which is what a live tile must load instead of this
# site's own shell.
assert_route "go2rtc player" /live/webrtc/webrtc.html 200 text/html \
  contains:go2rtc-webrtc-player absent:/pkg/corvette.js

checks_run=$((checks_run + 1))
http_get /live/webrtc/webrtc.html "$WORK/out/player" >/dev/null
if cmp -s <(strip_injected_script "$WORK/out/root.body") \
  <(strip_injected_script "$WORK/out/player.body"); then
  fail "the go2rtc player page is this site's shell"
else
  pass "the go2rtc player page is a different document from /"
fi

# The wasm binary: the right MIME for streaming instantiation, and a
# Content-Length, which proves sub_filter did not rewrite it -- a filtered
# response is chunked and has none.
assert_route "wasm binary" /pkg/corvette.wasm 200 application/wasm \
  absent:/pkg/corvette.js

checks_run=$((checks_run + 1))
http_get /pkg/corvette.wasm "$WORK/out/wasm" >/dev/null
wasm_length="$(header_value "$WORK/out/wasm.head" content-length)"
# 00 61 73 6d is the four-byte preamble every WebAssembly module starts with.
wasm_magic="$(head -c 4 "$WORK/out/wasm.body" | od -An -tx1 | tr -d ' \n')"
if [[ -n "$wasm_length" && "$wasm_magic" == "0061736d" ]]; then
  pass "wasm is unfiltered ($wasm_length bytes) and starts with the wasm preamble"
else
  fail "wasm content-length '$wasm_length', first bytes '$wasm_magic'"
fi

# Unhashed filenames mean any positive freshness lifetime is a window in which
# a browser serves a superseded bundle from a URL that did not change. This
# holds for every configuration; the exact header is the expectations file's.
checks_run=$((checks_run + 1))
wasm_cache="$(header_value "$WORK/out/wasm.head" cache-control)"
wasm_expires="$(header_value "$WORK/out/wasm.head" expires)"
if [[ "$wasm_cache" == *"$pkg_cache_control"* ]] &&
  ! [[ "$wasm_cache" =~ max-age=[1-9] ]] && [[ -z "$wasm_expires" ]]; then
  pass "wasm cache-control '$wasm_cache' with no freshness lifetime"
else
  fail "wasm cache-control '$wasm_cache' expires '$wasm_expires', expected '$pkg_cache_control' and no freshness lifetime"
fi

# A missing file under /pkg/. On the unmodified configuration this is the shell
# at 200, which a loader reports as a parse error rather than a 404.
if [[ "$missing_pkg_asset_is_shell" == yes ]]; then
  assert_route "missing asset under /pkg/" /pkg/does-not-exist.js \
    "$missing_pkg_asset_status" "$missing_pkg_asset_content_type" \
    contains:/pkg/corvette.js
else
  assert_route "missing asset under /pkg/" /pkg/does-not-exist.js \
    "$missing_pkg_asset_status" "$missing_pkg_asset_content_type" \
    absent:/pkg/corvette.js
fi

# Phase 6: what an ingress path prefix does to these files.
#
# The configuration rewrites BASE_PATH tokens in HTML, CSS and JavaScript from
# a request header. This site emits none of those tokens, so the only rule that
# can fire is the one that injects a window.baseUrl script after <body>.

checks_run=$((checks_run + 1))
http_get /pkg/corvette.js "$WORK/out/js-plain" >/dev/null
http_get /pkg/corvette.js "$WORK/out/js-prefixed" 'X-Ingress-Path: /sub' >/dev/null
if cmp -s "$WORK/out/js-plain.body" "$WORK/out/js-prefixed.body"; then
  pass "the loader JavaScript is byte-identical under an ingress path"
else
  fail "the loader JavaScript changed under an ingress path"
fi

checks_run=$((checks_run + 1))
http_get / "$WORK/out/html-prefixed" 'X-Ingress-Path: /sub' >/dev/null
if grep -qF 'window.baseUrl="/"' "$WORK/out/root.body" &&
  grep -qF 'window.baseUrl="/sub/"' "$WORK/out/html-prefixed.body" &&
  cmp -s <(strip_injected_script "$WORK/out/root.body") "$WORK/web/index.html" &&
  cmp -s <(strip_injected_script "$WORK/out/html-prefixed.body") "$WORK/web/index.html"; then
  pass "the shell differs only by the injected window.baseUrl script"
else
  fail "the shell changed under an ingress path beyond the injected script"
fi

# Phase 7: the browser check. Whether the injected script breaks the boot is
# not answerable from response bodies.

echo "Booting the bundle in a browser through this nginx"
if PARITY_BASE_URL="$BASE_URL" playwright test \
  --config "$HARNESS/playwright.config.cjs"; then
  pass "the bundle boots and routes through this nginx"
else
  fail "the bundle did not boot through this nginx"
fi
checks_run=$((checks_run + 1))

# Phase 8: verdict.

echo "run_nginx_parity: $checks_run checks, $checks_failed failed"
if [[ "$checks_failed" -ne 0 ]]; then
  exit 1
fi
