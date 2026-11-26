#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: issue-ref.sh check <commit-message-file>" >&2
  exit 2
}

if [[ "${1-}" != check || $# -ne 2 || ! -f "${2-}" ]]; then
  usage
fi

msg="$(grep -Ev '^#([[:blank:]].*)?$' -- "$2" || true)"

case "$(printf '%s\n' "$msg" | head -n 1)" in
  'Merge '* | 'fixup! '* | 'squash! '* | 'Revert '*) exit 0 ;;
esac

if printf '%s\n' "$msg" | grep -iqE '^(Refs|Closes|Fixes|Part of) #[0-9]+' ||
  printf '%s\n' "$msg" | grep -qE '^No issue: .{8,}'; then
  exit 0
fi

echo "Add an issue line to the commit message: Refs, Closes, Fixes or Part of followed by #N, or a No issue: line giving at least 8 characters of reason."
exit 1
