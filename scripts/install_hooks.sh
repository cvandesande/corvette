#!/usr/bin/env bash
# Installs this repository's pre-commit hook: the standards gates on every commit,
# plus the Rust checks when Rust files are staged. Refuses to overwrite a foreign hook.
set -eu

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT" || exit 1
HOOK_PATH="$(git rev-parse --git-path hooks)/pre-commit"

hook_text() {
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

WANT="$(mktemp)"
trap 'rm -f "$WANT"' EXIT
hook_text > "$WANT"

if [ -e "$HOOK_PATH" ]; then
	if cmp -s "$WANT" "$HOOK_PATH"; then
		echo "pre-commit hook already installed and identical: $HOOK_PATH"
		exit 0
	fi
	echo "install_hooks.sh: refusing to overwrite a different pre-commit hook: $HOOK_PATH" >&2
	echo "remove or merge it first; this installer never touches other hooks" >&2
	exit 1
fi

cp "$WANT" "$HOOK_PATH"
chmod 755 "$HOOK_PATH"
echo "installed $HOOK_PATH (mode 755): make check-standards, plus make check-rust when Rust is staged"
