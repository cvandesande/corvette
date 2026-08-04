#!/usr/bin/env bash
# Enforces INV-1 (publish shape) on a staging tree built by build_site.sh: the
# tree must contain exactly index.html and pkg/**, with the expected pkg
# bundle files present and no donor files an overlay onto /opt/frigate/web
# would otherwise displace.
set -euo pipefail

TREE="${1:-target/site-publish}"

if [[ ! -d "$TREE" ]]; then
  echo "check_site_shape: no such directory: $TREE" >&2
  exit 1
fi

fail=0

# Top level must be exactly index.html and pkg -- nothing else, nothing missing.
mapfile -t top_level < <(find "$TREE" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)
expected_top_level=(index.html pkg)
if [[ "${top_level[*]}" != "${expected_top_level[*]}" ]]; then
  echo "check_site_shape: unexpected top level: got [${top_level[*]}], want [${expected_top_level[*]}]" >&2
  fail=1
fi

if [[ ! -s "$TREE/index.html" ]]; then
  echo "check_site_shape: missing or empty index.html" >&2
  fail=1
fi

for required in pkg/corvette.js pkg/corvette.wasm pkg/corvette.css; do
  if [[ ! -f "$TREE/$required" ]]; then
    echo "check_site_shape: missing required file: $required" >&2
    fail=1
  fi
done

# Donor files this overlay must never displace inside /opt/frigate/web.
deny_list=(app.html login.html assets fonts locales robots.txt notifications-worker.js)
for name in "${deny_list[@]}"; do
  while IFS= read -r hit; do
    echo "check_site_shape: forbidden path present: $hit" >&2
    fail=1
  done < <(find "$TREE" -name "$name")
done

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "check_site_shape: $TREE matches the publish shape"
