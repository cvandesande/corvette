#!/usr/bin/env bash
# The live camera tile embeds go2rtc's own WebRTC player page, proxied
# end-to-end by the deployed nginx configuration. There is no /go2rtc/
# location in that configuration, so a URL under that prefix has nowhere to
# match but this site's own SPA fallback -- an iframe that loads the app into
# itself at HTTP 200, with nothing about the response distinguishing it from
# a working tile. This script fails the build if that literal reappears
# anywhere in the UI's own source, so a regression is caught before it ships,
# not discovered by someone watching a tile load itself forever.
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

echo "check_no_go2rtc: no '/go2rtc/' references under $TARGET"
