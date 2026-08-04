#!/usr/bin/env bash
# Builds the release bundle and stages a publish-ready copy of it at
# target/site-publish/.
#
# The build below rewrites target/site, so running this replaces the bundle a
# dev server (scripts/serve_ui.sh) is serving, exactly as `make check` already
# does. Only the staging copy leaves target/site alone. Check for a listening
# dev server before running either.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
SITE_ROOT="$REPO/target/site"
PUBLISH_ROOT="$REPO/target/site-publish"
APP_HTML="$REPO/crates/corvette-ui/public/app.html"

cd "$REPO"
NO_COLOR=false cargo leptos build --release --split

rm -rf "$PUBLISH_ROOT"
mkdir -p "$PUBLISH_ROOT"
cp "$APP_HTML" "$PUBLISH_ROOT/index.html"
cp -r "$SITE_ROOT/pkg" "$PUBLISH_ROOT/pkg"

echo "Staged publish tree at $PUBLISH_ROOT"
