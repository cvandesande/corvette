#!/usr/bin/env bash
# Enforces INV-3: the deployed system and every registry are read-only to
# agents. Greps tracked scripts and the evidence directory for verbs that
# would mutate a cluster, push an image, or push to a git remote.
#
# Fails closed. Every condition that stops a target from being enumerated,
# read, or matched exits non-zero with a diagnostic naming the cause: a check
# that cannot establish what it covered has not passed, and reporting success
# from a scan set of unknown size is the failure mode this script exists to
# not have.
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
  # Read the tracked set through a plain command substitution, not a process
  # substitution: `mapfile < <(git ls-files ...)` reports mapfile's status,
  # which is success on zero lines, so a git failure would hand this script an
  # empty scan set and it would announce a pass over nothing.
  if ! tracked_scripts="$(cd "$REPO" && git ls-files 'scripts/*.sh')"; then
    echo "check_no_mutation: cannot enumerate tracked scripts: git ls-files failed in $REPO" >&2
    exit 1
  fi
  # An empty listing is a different fault from a failed one and gets its own
  # message, but it is equally not a pass: this script lives in the directory
  # it scans, so zero tracked scripts means the enumeration found the wrong
  # tree, not that the tree is clean.
  if [[ -z "$tracked_scripts" ]]; then
    echo "check_no_mutation: no tracked scripts/*.sh under $REPO -- scan set is empty, refusing to pass" >&2
    exit 1
  fi
  targets=()
  while IFS= read -r script; do
    targets+=("$REPO/$script")
  done <<<"$tracked_scripts"
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
# against content only, never against the path -- testing the path let a
# marker in a file's name exempt every line in that file, which is a
# file-scoped skip, not the line-scoped one this exists to provide. Matched as
# a plain substring, not a pattern, so the marker itself cannot be disguised
# by regex metacharacters.
FIXTURE_MARKER='NOT-EXECUTED-FIXTURE'

# Writes the given file to stdout with every backslash continuation joined onto
# the line that opened it, one empty line per line consumed.
#
# A forbidden verb split across a trailing backslash is one shell statement but
# two grep lines, and grep matches within one line. The empty-line padding keeps
# the output the same length as the input, so a statement still sits on the
# number of the line it starts at and grep -n keeps reporting real line numbers.
# A carriage return between the backslash and the end of line still counts as a
# continuation, so a CRLF file cannot hide a wrapped verb. Over-joining -- a
# line ending in an escaped backslash, which the shell would not continue --
# can only produce an extra reported match for a human to dismiss, never a
# missed one.
join_continuations() {
  awk -- '
    {
      held = held $0
      if (held ~ /\\\r?$/) {
        sub(/\\\r?$/, "", held)
        consumed++
        next
      }
      print held
      for (i = 0; i < consumed; i++) print ""
      held = ""
      consumed = 0
    }
    END {
      if (held != "" || consumed > 0) {
        print held
        for (i = 0; i < consumed; i++) print ""
      }
    }
  ' "$1"
}

scratch_dir="$(mktemp -d)"
trap 'rm -rf "$scratch_dir"' EXIT
joined_file="$scratch_dir/joined"
file_list="$scratch_dir/files"

# -L follows symlinked files and directories, so a symlink inside the scan set
# adds coverage rather than hiding a target behind it. Enumerating the files
# here, instead of handing the directories to grep -R, is what lets every
# failure below name the path it happened on.
if ! find -L "${targets[@]}" -type f -print0 >"$file_list"; then
  echo "check_no_mutation: cannot enumerate files in the scan set (find failed)" >&2
  exit 1
fi
mapfile -t -d '' scan_files <"$file_list"

fail=0
combined_pattern="$(IFS='|'; printf '%s' "${forbidden_patterns[*]}")"
for file in "${scan_files[@]}"; do
  # grep -I skips a file it classifies as binary -- one NUL byte anywhere is
  # enough -- and says nothing, which would hide every forbidden verb in that
  # file. Take grep's own text/binary verdict up front and fail on it: a file
  # in the scan set that cannot be read as text has not been checked. This also
  # catches an unreadable file, whose grep status is 2 and whose diagnostic
  # grep prints for itself.
  if [[ -s "$file" ]] && ! grep -qI -e '' -- "$file"; then
    echo "check_no_mutation: cannot scan as text: $file" >&2
    fail=1
    continue
  fi

  if ! join_continuations "$file" >"$joined_file"; then
    echo "check_no_mutation: cannot read $file" >&2
    fail=1
    continue
  fi

  # grep exits 1 for "no match" and 2 for a real error; collapsing the two with
  # `|| true` is how an unscannable file becomes a silent pass. -a keeps grep
  # from re-applying the binary heuristic the text check above already ruled on.
  grep_status=0
  hits="$(grep -naE -e "$combined_pattern" -- "$joined_file")" || grep_status=$?
  if [[ "$grep_status" -gt 1 ]]; then
    echo "check_no_mutation: grep failed with status $grep_status scanning $file" >&2
    fail=1
    continue
  fi
  [[ "$grep_status" -eq 0 ]] || continue

  while IFS= read -r hit; do
    # grep prints "line:content" here because it is given exactly one file, and
    # that file is this script's own temporary copy, so no scanned path -- and
    # no colon in one -- can shift the content field.
    line="${hit%%:*}"
    content="${hit#*:}"
    if [[ "$content" == *"$FIXTURE_MARKER"* ]]; then
      continue
    fi
    echo "check_no_mutation: forbidden verb match: $file:$line:$content" >&2
    fail=1
  done <<<"$hits"
done

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "check_no_mutation: no forbidden verbs found"
