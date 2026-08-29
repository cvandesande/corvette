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
# U2's vendored @moq/watch/hls.js bundles (crates/corvette-ui/public/vendor/,
# copied into $SITE_ROOT/vendor by cargo-leptos's own assets-dir handling).
# Without this, U2's expanded view ships code that references
# /vendor/moq-watch.bundle.js and /vendor/hls.min.js but the publish tree --
# and everything downstream of it -- never actually contains them.
cp -r "$SITE_ROOT/vendor" "$PUBLISH_ROOT/vendor"

echo "Staged publish tree at $PUBLISH_ROOT"
