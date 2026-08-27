Origin of the files in this directory
======================================

Both files here are third-party JavaScript this repo's own build has no
tooling to fetch or bundle at build time (`cargo-leptos` builds Rust/WASM
only; there is no Node build step wired into `make check-ui`), so -- matching
`tests/nginx-parity/vendor/PROVENANCE`'s own precedent for exactly this kind
of gap -- they are vendored: fetched once from the npm registry, recorded
here with their exact version and integrity hash, and committed as static
assets `cargo-leptos`'s own `assets-dir` (`crates/corvette-ui/public`, see
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

moq-watch.bundle.js
-------------------

A single self-contained IIFE bundle built with `esbuild` from `@moq/watch`'s
own `element` entry point (`@moq/watch/element` -> registers `<moq-watch>` as
a side effect; deliberately not the whole package, which also exports the
publisher-side `<moq-publish>`/`<moq-watch-ui>` elements issue #12 item U2
does not need). `@moq/watch` itself depends on `@moq/hang`, `@moq/net`,
`@moq/msf`, and `@moq/signals`; `esbuild --bundle` resolves and inlines that
whole graph into the one committed file, so nothing here depends on
`node_modules` at runtime.

    Package               @moq/watch
    Version                0.5.1 (exact, root package.json devDependency)
    Registry integrity     sha512-/nXgrZkN4Ves1hRgIPjXMXcWVkddoekA5PTH0INX2fLLZ64qmsPFMaTg9N/tBovWxt/ZEVdoCuQ0BQt4O6Re8A==

    Package               @moq/hang (transitive, via @moq/watch's own "^0.4.1")
    Version resolved       0.4.2 (root package-lock.json)
    Registry integrity     sha512-SfSKvDUqrOymjUX9oprEmTpsq4kfBjRcL70MdurK9ntBF/0gLwIXPqSpUF5tc1KN+lwiBqBjYBDln8GNsv9pIA==

    Build tool             esbuild 0.28.2 (via `npx --yes esbuild@0.28.2`,
                            not added as a project dependency -- used once to
                            produce this checked-in file, the same way this
                            file itself is checked in rather than rebuilt by
                            `make check-ui`)
    Build command           npx --yes esbuild@0.28.2 node_modules/@moq/watch/element.js \
                              --bundle --format=iife \
                              --outfile=crates/corvette-ui/public/vendor/moq-watch.bundle.js
    sha256 (this copy)      5db02563f25e7c1e8834afbd442961cbbf7d8888b86edc73260e7c9a5606985e
    Build output            clean: 0 esbuild warnings or errors

Both packages' own licenses (`hls.js`: Apache-2.0; `@moq/watch`/`@moq/hang`:
MIT OR Apache-2.0) are compatible with this project's own MIT license and
carry no AGPL exposure (INV-1's own concern, though INV-1 itself names only
Go binaries; recorded here as the general license-review the style guide
requires for any new dependency).

Relationship to R1's Cargo-side pin
------------------------------------

Issue #12 item R1 pinned the Rust side of `moq-dev/moq` to git commit
`7b73c43381a7f9c309e3045a8f0f858aa32ef48c` (`.agents/issue-12/evidence/
R1-dependency-pin.md`), reasoning that a later commit's `Import`/`Split` API
changes were unread and therefore unsafe to build against. That same
reasoning applies here: `@moq/watch`/`@moq/hang` are not published to npm at
that exact commit (npm publishes a package version once per release, not
once per commit), so this item verified, by diff, that no version gap exists
for the files this item's own contract actually depends on:

  - npm's `@moq/watch@0.5.0`/`@moq/hang@0.4.0` (the versions declared in the
    pinned commit's own `package.json` files, per R1's evidence) were
    actually published from git commit `f3ff49319d05b3f95ad0929538cf20f58f315578`
    (2026-08-14), which `gh api repos/moq-dev/moq/compare/7b73c433...f3ff493`
    shows is 17 commits BEHIND (an ancestor of) the pinned commit -- i.e.
    older than what R1 verified.
  - The next published versions, `@moq/watch@0.5.1`/`@moq/hang@0.4.1`, were
    published from `8f5a97b2728f657a364437bfe2aa306c6a012103` (2026-08-20),
    which `gh api .../compare/7b73c433...8f5a97b` shows is 22 commits AHEAD
    of (a descendant of) the pinned commit.
  - `gh api .../compare/7b73c433...8f5a97b --jq '.files[].filename'`,
    filtered to `js/watch|js/hang|js/net/src/connection|js/signals`, lists
    only `js/hang/package.json`, `js/watch/package.json`,
    `js/watch/src/broadcast.{ts,test.ts}`, and
    `js/net/src/connection/{accept,browser,connect,handshake}.ts`.
    `js/watch/src/element.ts` (the `<moq-watch>` element itself,
    `expanded_view.rs`'s own documented contract) and
    `js/net/src/connection/reload.ts` (the `.status`/`.closed` surface that
    contract depends on) are **not** in that list -- byte-identical between
    the pinned commit and this published version.
  - The same two files are also absent from `gh api .../compare/
    8f5a97b...23f48def` (the diff to `0.5.2`/`0.4.2`, the current npm
    `latest` as of this item), so the byte-identical range actually extends
    from the pinned commit through every version npm has published since.

Given that, `0.5.1`/`0.4.1`-or-later is a strict superset of the pinned
commit for the two files this item's own contract rests on, and this item
pins the exact versions above (not a caret range) so a future `npm install`
cannot silently move past what was actually diffed here.
