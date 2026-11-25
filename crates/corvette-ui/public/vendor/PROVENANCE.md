Origin of the files in this directory
======================================

The file here is third-party JavaScript this repo's own build has no
tooling to fetch or bundle at build time (`cargo-leptos` builds Rust/WASM
only; there is no Node build step wired into `make check-ui`), so -- matching
`tests/nginx-parity/vendor/PROVENANCE`'s own precedent for exactly this kind
of gap -- it is vendored: fetched once from the npm registry, recorded
here with its exact version and integrity hash, and committed as a static
asset `cargo-leptos`'s own `assets-dir` (`crates/corvette-ui/public`, see
the workspace `Cargo.toml`) copies verbatim to the site root, served at
`/vendor/<file>`.

hls.min.js
----------

A straight copy of `hls.js`'s own official UMD build, already an npm
devDependency for issue #12 item G3's own browser check
(`crates/corvette-media-bridge/tests/browser/g3_hls.spec.cjs`) and pinned
exactly (no caret) in this repo's root `package.json`.

    Package               hls.js
    Version               1.7.1
    Registry integrity    sha512-DlzIkeBAS9IIQ432k3BUf3HlwbsR0+trB1i2lDdN2gUkNkrehFurh0/48M5c1/EjlDkdGng1gwZIpwyPxvdZ/g==
    Source in the tarball dist/hls.min.js
    Copied                cp node_modules/hls.js/dist/hls.min.js crates/corvette-ui/public/vendor/hls.min.js
    sha256 (this copy)    6cfad701a61fb8a99add5e84449e64661169b0652bf44ceb2a28465c8817b5f1

`hls.js`'s own license (Apache-2.0) is compatible with this project's own MIT
license and carries no AGPL exposure; recorded here as the general
license-review the style guide requires for any new dependency.
