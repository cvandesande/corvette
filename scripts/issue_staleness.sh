#!/usr/bin/env bash
# Usage: issue_staleness.sh [--all] [--json-from-stdin]
# --all also lists issues no commit mentions that have been quiet 30+ days.
set -euo pipefail

US=$'\x1f'

ALL=0
FROM_STDIN=0
for arg in "$@"; do
  case "$arg" in
    --all) ALL=1 ;;
    --json-from-stdin) FROM_STDIN=1 ;;
    *) echo "issue_staleness: unknown argument: $arg" >&2; exit 2 ;;
  esac
done

for tool in jq git date sort cut; do
  command -v "$tool" >/dev/null 2>&1 || { echo "issue_staleness: required tool missing: $tool" >&2; exit 2; }
done
if [ "$FROM_STDIN" -eq 0 ]; then
  for tool in gh timeout; do
    command -v "$tool" >/dev/null 2>&1 || { echo "issue_staleness: required tool missing: $tool" >&2; exit 2; }
  done
fi

if [ "$FROM_STDIN" -eq 1 ]; then
  issues_json=$(cat)
elif ! issues_json=$(timeout 30 gh issue list --state open --limit 100 --json number,title,updatedAt 2>/dev/null); then
  echo "issue_staleness: gh could not fetch open issues" >&2
  exit 2
fi

NOW=$(date -u +%s)
behind_lines=()
quiet_lines=()

while IFS="$US" read -r num updated title; do
  newest=0
  # Digit bounds stop a reference matching inside a longer digit run.
  while IFS= read -r stamp; do
    [ -n "$stamp" ] || continue
    stamp_epoch=$(date -d "$stamp" +%s)
    if [ "$stamp_epoch" -gt "$newest" ]; then
      newest=$stamp_epoch
    fi
  done < <(git log -i -E --grep="#${num}([^0-9]|\$)" --grep="issue ${num}([^0-9]|\$)" --format=%cI)
  updated_epoch=$(date -d "$updated" +%s)
  if [ "$newest" -gt "$updated_epoch" ]; then
    days=$(( (newest - updated_epoch) / 86400 ))
    behind_lines+=("${days}${US}#${num} ${days} days behind: ${title}")
  elif [ "$ALL" -eq 1 ] && [ "$newest" -eq 0 ] && [ $(( (NOW - updated_epoch) / 86400 )) -ge 30 ]; then
    quiet=$(( (NOW - updated_epoch) / 86400 ))
    quiet_lines+=("${quiet}${US}#${num} no commit mentions it, last updated ${updated%%T*}")
  fi
done < <(printf '%s' "$issues_json" | jq -r --arg us "$US" '.[] | [.number, .updatedAt, .title] | join($us)')

if [ "${#behind_lines[@]}" -gt 0 ]; then
  printf '%s\n' "${behind_lines[@]}" | sort -t"$US" -k1,1nr | cut -d"$US" -f2-
fi
if [ "$ALL" -eq 1 ]; then
  if [ "${#quiet_lines[@]}" -gt 0 ]; then
    printf '%s\n' "${quiet_lines[@]}" | sort -t"$US" -k1,1nr | cut -d"$US" -f2-
  fi
  echo "${#behind_lines[@]} issues behind, ${#quiet_lines[@]} unmentioned"
else
  echo "${#behind_lines[@]} issues behind"
fi
exit 0
