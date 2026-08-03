.PHONY: check check-cpp check-nix check-rust check-shell check-ui check-whitespace serve-ui

check: check-rust check-cpp check-shell check-nix check-ui check-whitespace

check-rust:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --all-features --locked
	cargo test --workspace --all-targets --all-features --locked

check-cpp:
	clang-format --dry-run --Werror crates/ncnn-sys/csrc/c_api_ext.cpp \
		crates/ncnn-sys/csrc/c_api_ext.h

check-shell:
	shellcheck scripts/*.sh

check-nix:
	nixfmt --check flake.nix

check-ui:
	NO_COLOR=false cargo leptos build --release --split
	cp crates/corvette-ui/public/app.html target/site/index.html
	playwright test

check-whitespace:
	git diff --check

serve-ui:
	./scripts/serve_ui.sh
