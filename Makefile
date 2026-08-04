.PHONY: check check-cpp check-nginx-parity check-nix check-no-mutation check-rust check-shell \
	check-site-shape check-ui check-whitespace serve-ui

check: check-rust check-cpp check-shell check-nix check-ui check-whitespace check-site-shape \
	check-nginx-parity check-no-mutation

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

check-site-shape:
	./scripts/build_site.sh
	./scripts/check_site_shape.sh target/site-publish

# Runs after check-site-shape, which stages the publish tree this serves.
check-nginx-parity:
	./scripts/run_nginx_parity.sh

check-no-mutation:
	./scripts/check_no_mutation.sh

check-whitespace:
	git diff --check

serve-ui:
	./scripts/serve_ui.sh
