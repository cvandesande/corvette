.PHONY: check check-cpp check-nginx-parity check-nix check-no-go2rtc check-no-mutation \
	check-no-trailing-slash-hrefs check-oci-layout check-rust check-standards \
	check-standards-range check-shell check-site-shape check-ui check-whitespace serve-ui

check: check-rust check-cpp check-shell check-nix check-ui check-whitespace check-site-shape \
	check-nginx-parity check-no-mutation check-no-go2rtc check-no-trailing-slash-hrefs \
	check-oci-layout

check-rust:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
	cargo test --workspace --all-targets --all-features --locked

check-cpp:
	clang-format --dry-run --Werror crates/ncnn-sys/csrc/c_api_ext.cpp \
		crates/ncnn-sys/csrc/c_api_ext.h

check-shell:
	shellcheck $(shell git ls-files '*.sh')

check-standards:
	bash docs/standards/gate.sh d8 staged
	bash docs/standards/gate.sh comment staged
	bash docs/standards/gate.sh obs deps
	bash docs/standards/gate.sh selftest
	bash docs/standards/gate.sh comment selftest
	bash docs/standards/gate.sh d8 selftest
	bash docs/standards/gate.sh obs selftest

check-standards-range:
	@if [ -z '$(BASE)' ]; then echo "check-standards-range: BASE is empty; pass a range base, e.g. make check-standards-range BASE=HEAD~1"; exit 2; fi
	bash docs/standards/gate.sh d8 range $(BASE)..HEAD
	bash docs/standards/gate.sh comment check $(BASE)..HEAD
	bash docs/standards/gate.sh obs deps

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

check-no-go2rtc:
	./scripts/check_no_go2rtc.sh

check-no-trailing-slash-hrefs:
	./scripts/check_no_trailing_slash_hrefs.sh

# Local-only: builds an OCI layout under out/, no registry contact.
check-oci-layout:
	./scripts/publish_site_image.sh
	./scripts/check_oci_layout.sh out/corvette-ui.oci

check-whitespace:
	git diff --check

serve-ui:
	./scripts/serve_ui.sh
