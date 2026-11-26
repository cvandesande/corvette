#!/usr/bin/env bash
# Installs pre-commit and commit-msg, refusing to overwrite a foreign hook.
set -eu

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT" || exit 1
HOOKS="$(git rev-parse --git-path hooks)"

precommit_text() {
	cat <<'HOOK'
#!/usr/bin/env bash
set -eu
cd "$(git rev-parse --show-toplevel)" || exit 1
make check-standards
staged="$(git diff --cached --name-only)"
if grep -qE '(\.rs|Cargo\.(toml|lock))$' <<<"$staged"; then
	if command -v cargo >/dev/null 2>&1; then
		make check-rust
	else
		nix develop --no-write-lock-file "$(git rev-parse --show-toplevel)" --command make check-rust
	fi
fi
HOOK
}

commitmsg_text() {
	cat <<'HOOK'
#!/usr/bin/env bash
bash docs/standards/issue-ref.sh check "$1"
HOOK
}

install_hook() {
	local name="$1" want="$2" note="$3" path
	path="$HOOKS/$name"
	if [ -e "$path" ]; then
		if cmp -s "$want" "$path"; then
			echo "$name hook already installed and identical: $path"
			return 0
		fi
		echo "install_hooks.sh: refusing to overwrite a different $name hook: $path" >&2
		echo "remove or merge it first; this installer never touches other hooks" >&2
		exit 1
	fi
	cp "$want" "$path"
	chmod 755 "$path"
	echo "installed $path (mode 755)$note"
}

WANT="$(mktemp)"
WANT_MSG="$(mktemp)"
trap 'rm -f "$WANT" "$WANT_MSG"' EXIT
precommit_text > "$WANT"
commitmsg_text > "$WANT_MSG"

install_hook pre-commit "$WANT" ": make check-standards, plus make check-rust when Rust is staged"
install_hook commit-msg "$WANT_MSG" ": issue-ref check on every commit message"
