#!/usr/bin/env bash
# The live camera tile used to embed go2rtc's own WebRTC player page, proxied
# end-to-end by the deployed nginx configuration. There is no /go2rtc/
# location in that configuration, so a URL under that prefix has nowhere to
# match but this site's own SPA fallback -- an iframe that loads the app into
# itself at HTTP 200, with nothing about the response distinguishing it from
# a working tile. This script fails the build if that literal reappears
# anywhere in the UI's own source, so a regression is caught before it ships,
# not discovered by someone watching a tile load itself forever.
#
# Issue #12 item U1 (INV-7) replaced that iframe with a native
# MediaSource/WebSocket player against G2's fMP4-over-WebSocket transport, and
# removed the embedded player page for good -- so the same reasoning now also
# denies the specific page URL the iframe pointed at
# (/live/webrtc/webrtc.html), even though it happens to fall under /go2rtc/'s
# sibling /live/webrtc/ prefix rather than /go2rtc/ itself. INV-7 requires
# this literal gone from the UI's own source unconditionally, not only as an
# iframe src.
#
# Usage: check_no_go2rtc.sh [path]
#   No argument: scans the default set (crates/corvette-ui/src).
#   With an argument: scans exactly the given path, for testing against a
#   fixture without touching the default set.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:-$REPO/crates/corvette-ui/src}"

if [[ ! -e "$TARGET" ]]; then
  echo "check_no_go2rtc: no such path: $TARGET" >&2
  exit 1
fi

if hits="$(grep -rn -- '/go2rtc/' "$TARGET")"; then
  echo "check_no_go2rtc: found '/go2rtc/' references:" >&2
  echo "$hits" >&2
  exit 1
fi

if hits="$(grep -rn -- '/live/webrtc/webrtc.html' "$TARGET")"; then
  echo "check_no_go2rtc: found '/live/webrtc/webrtc.html' references:" >&2
  echo "$hits" >&2
  exit 1
fi

echo "check_no_go2rtc: no '/go2rtc/' or '/live/webrtc/webrtc.html' references under $TARGET"
