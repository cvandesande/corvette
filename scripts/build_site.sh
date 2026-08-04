#!/usr/bin/env bash
# Builds a publish-ready staging tree at target/site-publish/ without mutating
# target/site, which a running dev server (scripts/serve_ui.sh) may be serving.
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
