#!/usr/bin/env bash
# Stages the release bundle's publish tree at target/site-publish: index.html,
# pkg, vendor, manifest.json and icons. The build rewrites target/site, so a
# listening dev server (scripts/serve_ui.sh) must be checked for first.
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
cp -r "$SITE_ROOT/vendor" "$PUBLISH_ROOT/vendor"
cp "$SITE_ROOT/manifest.json" "$PUBLISH_ROOT/manifest.json"
cp -r "$SITE_ROOT/icons" "$PUBLISH_ROOT/icons"

echo "Staged publish tree at $PUBLISH_ROOT"
