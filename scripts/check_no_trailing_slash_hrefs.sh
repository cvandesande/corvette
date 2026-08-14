#!/usr/bin/env bash
# Deployed nginx already owns a set of URL prefixes (`/recordings/`,
# `/exports/`, `/api/`, and their siblings) and matches the trailing-slash and
# no-slash spellings of a path as two different resources -- `/recordings`
# renders this site's own shell, `/recordings/` is nginx's own JSON directory
# autoindex. The client router must never link to one of its own routes with
# a trailing slash, or the link silently leaves the SPA and lands on whatever
# nginx location happens to match instead.
#
# This scans for the concrete failure mode: a self-route `href="..."`
# attribute whose value ends in a `/`. A bare `href="/"` (the root route) is
# not a violation -- there is no narrower path a trailing slash could have
# been added to -- so it is deliberately excluded.
#
# Usage: check_no_trailing_slash_hrefs.sh [path]
#   No argument: scans the default set (crates/corvette-ui/src).
#   With an argument: scans exactly the given path, for testing against a
#   fixture without touching the default set.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:-$REPO/crates/corvette-ui/src}"

if [[ ! -e "$TARGET" ]]; then
  echo "check_no_trailing_slash_hrefs: no such path: $TARGET" >&2
  exit 1
fi

if hits="$(grep -rnE -- 'href="/[^"]+/"' "$TARGET")"; then
  echo "check_no_trailing_slash_hrefs: found self-route href(s) ending in '/':" >&2
  echo "$hits" >&2
  exit 1
fi

echo "check_no_trailing_slash_hrefs: no trailing-slash self-route hrefs under $TARGET"
