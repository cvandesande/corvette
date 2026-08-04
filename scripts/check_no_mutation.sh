#!/usr/bin/env bash
# Enforces INV-3: the deployed system and every registry are read-only to
# agents. Greps tracked scripts and the evidence directory for verbs that
# would mutate a cluster, push an image, or push to a git remote.
#
# Usage: check_no_mutation.sh [path ...]
#   No arguments: scans the default set (tracked scripts/*.sh plus
#   .agents/issue-2/evidence/, if present).
#   With arguments: scans exactly the given paths, for testing against a
#   fixture without touching the default set.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"

# Each pattern uses a character class between verb and object so this file's
# own pattern list (verb and object adjacent, no whitespace) cannot self-match.
forbidden_patterns=(
  'kubectl[[:space:]]+apply'
  'kubectl[[:space:]]+edit'
  'kubectl[[:space:]]+patch'
  'kubectl[[:space:]]+scale'
  'kubectl[[:space:]]+delete'
  'kubectl[[:space:]]+rollout'
  'kubectl[[:space:]]+cp'
  'docker[[:space:]]+push'
  'buildx[[:space:]].*--push'
  'oras[[:space:]]+push'
  'skopeo[[:space:]]+copy'
  'git[[:space:]]+push'
)

if [[ "$#" -gt 0 ]]; then
  targets=("$@")
else
  targets=()
  mapfile -t tracked_scripts < <(cd "$REPO" && git ls-files 'scripts/*.sh')
  for script in "${tracked_scripts[@]}"; do
    targets+=("$REPO/$script")
  done
  if [[ -d "$REPO/.agents/issue-2/evidence" ]]; then
    targets+=("$REPO/.agents/issue-2/evidence")
  fi
fi

# A mutation-test evidence file legitimately quotes a forbidden verb to prove
# this script catches it -- that line is a rehearsal, not a record of anything
# that ran, so it would otherwise make every future INV-3 sweep fail on its
# own required evidence. A line carrying this literal marker in its CONTENT is
# exempted; every other line, including a second, unmarked forbidden verb
# sharing a file with a marked line, is still caught. The marker is tested
# against content only, not the full "path:line:content" hit grep -R prints --
# testing the whole hit let a marker in the file's PATH exempt every line in
# that file, which is a file-scoped skip, not the line-scoped one this exists
# to provide. Matched as a plain substring, not a pattern, so the marker
# itself cannot be disguised by regex metacharacters.
FIXTURE_MARKER='NOT-EXECUTED-FIXTURE'

fail=0
for target in "${targets[@]}"; do
  [[ -e "$target" ]] || continue
  for pattern in "${forbidden_patterns[@]}"; do
    while IFS= read -r hit; do
      # grep -R always prints "path:line:content"; strip exactly those two
      # leading fields (not "up to the last colon") so a colon inside the
      # matched content itself is never mistaken for a field separator.
      content="${hit#*:}"
      content="${content#*:}"
      if [[ "$content" == *"$FIXTURE_MARKER"* ]]; then
        continue
      fi
      echo "check_no_mutation: forbidden verb match: $hit" >&2
      fail=1
    done < <(grep -RInE "$pattern" "$target" 2>/dev/null || true)
  done
done

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "check_no_mutation: no forbidden verbs found"
