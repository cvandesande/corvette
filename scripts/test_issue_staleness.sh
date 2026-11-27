#!/usr/bin/env bash
# Checks issue_staleness.sh against fixture issues on stdin and a temporary
# repo of dated commits; no network. Failures print FAIL and set the exit code.
set -euo pipefail
unset GIT_INDEX_FILE GIT_DIR GIT_WORK_TREE
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$SCRIPT_DIR/issue_staleness.sh"
SCRATCH="$(mktemp -d)"
trap 'rm -r "$SCRATCH"' EXIT

REPO="$SCRATCH/repo"
git init -q "$REPO"
git -C "$REPO" config user.name test
git -C "$REPO" config user.email test@example.invalid
dated_commit() {
  GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid \
  GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid \
  GIT_AUTHOR_DATE="$2" GIT_COMMITTER_DATE="$2" \
    git -C "$REPO" commit -q --allow-empty -m "$1"
}
H='#'
dated_commit "fix the lens: ${H}12 shipped in November" "2026-11-01T12:00:00 +0000"
dated_commit "Issue 5: recording works" "2026-11-02T12:00:00 +0000"
dated_commit "fix ${H}21 old work" "2025-05-01T12:00:00 +0000"
dated_commit "bump ${H}1200" "2026-11-15T12:00:00 +0000"

recent="$(date -u -d '5 days ago' +%FT%TZ)"
cat > "$SCRATCH/issues.json" <<EOF
[
  {"number":1,"title":"Recent unmentioned","updatedAt":"$recent"},
  {"number":5,"title":"Motion ticks","updatedAt":"2026-10-01T00:00:00Z"},
  {"number":9,"title":"Quiet since 2020","updatedAt":"2020-01-01T00:00:00Z"},
  {"number":12,"title":"Recording pipeline","updatedAt":"2026-10-12T00:00:00Z"},
  {"number":21,"title":"Mention predates update","updatedAt":"2026-06-01T00:00:00Z"},
  {"number":120,"title":"Digit boundary probe","updatedAt":"2026-10-01T00:00:00Z"}
]
EOF

failed=0
expect_line() {
  if grep -qxF -- "$2" "$3"; then echo "ok $1"; else failed=1; echo "FAIL $1: missing: $2"; fi
}
expect_no_prefix() {
  if grep -qE "^$2" "$3"; then failed=1; echo "FAIL $1: unexpected line starting: $2"; else echo "ok $1"; fi
}
expect_rc() {
  if [ "$2" -eq "$3" ]; then echo "ok $1"; else failed=1; echo "FAIL $1: exit $2, want $3"; fi
}

run_staleness() {
  local out="$1"
  shift
  ( cd "$REPO" && bash "$SCRIPT" "$@" --json-from-stdin < "$SCRATCH/issues.json" ) \
    > "$SCRATCH/$out" 2> "$SCRATCH/$out.err"
}
run_staleness plain
run_staleness all --all

expect_line commit_newer_than_update_lists "${H}12 20 days behind: Recording pipeline" "$SCRATCH/plain"
expect_line count_line "2 issues behind" "$SCRATCH/plain"
expect_no_prefix commit_older_than_update_not_listed "${H}21 " "$SCRATCH/plain"
expect_no_prefix mention_12_not_for_issue_1 "${H}1 " "$SCRATCH/plain"
expect_no_prefix mention_1200_not_for_issue_120 "${H}120 " "$SCRATCH/plain"
expect_no_prefix quiet_issue_absent_without_all "${H}9 " "$SCRATCH/plain"
expect_line all_lists_unmentioned_2020 "${H}9 no commit mentions it, last updated 2020-01-01" "$SCRATCH/all"
expect_no_prefix all_skips_recent_unmentioned "${H}1 " "$SCRATCH/all"
expect_line all_count_line "2 issues behind, 2 unmentioned" "$SCRATCH/all"
if [ "$(sed -n '3p' "$SCRATCH/all")" = "${H}9 no commit mentions it, last updated 2020-01-01" ]; then
  echo "ok quiet_lines_sorted_oldest_first"
else
  failed=1
  echo "FAIL quiet_lines_sorted_oldest_first: third line was $(sed -n '3p' "$SCRATCH/all")"
fi

if [ "$(head -n 1 "$SCRATCH/plain")" = "#5 32 days behind: Motion ticks" ]; then
  echo "ok behind_lines_sorted_days_desc"
else
  failed=1
  echo "FAIL behind_lines_sorted_days_desc: first line was $(head -n 1 "$SCRATCH/plain")"
fi

mkdir -p "$SCRATCH/emptybin" "$SCRATCH/nogit"
for tool in jq date sort cut; do ln -s "$(command -v "$tool")" "$SCRATCH/nogit/$tool"; done
rc=0
env -i PATH="$SCRATCH/emptybin" "$(command -v bash)" "$SCRIPT" </dev/null >"$SCRATCH/notool.out" 2>&1 || rc=$?
expect_rc missing_tool_exits_2 "$rc" 2
expect_line missing_tool_message "issue_staleness: required tool missing: jq" "$SCRATCH/notool.out"
rc=0
env -i PATH="$SCRATCH/nogit" "$(command -v bash)" "$SCRIPT" --json-from-stdin < "$SCRATCH/issues.json" >"$SCRATCH/nogit.out" 2>&1 || rc=$?
expect_rc missing_git_exits_2 "$rc" 2

exit "$failed"
