//! RTSP client for dialing physical IP cameras.
//!
//! `session` is the protocol layer: it performs the Digest-authenticated
//! `DESCRIBE`/`SETUP`/`PLAY` handshake, schedules keep-alives at a real
//! margin against each camera's own declared timeout, and demuxes the
//! resulting interleaved RTP frames. It hands raw per-track RTP payloads to
//! a later depacketization layer; it does not decode them itself.
//!
//! `mock_camera` is a test double this crate's own test suites dial against
//! in place of a real camera; it is never a production dependency.

// `rtsp-types` depends on `url`, whose modern Unicode-conformant IDNA
// backend (`idna_adapter` -> `icu_*`) needs a newer proc-macro toolchain
// than this workspace's existing `leptos`/`corvette-ui` macro chain, so two
// major versions of `syn` coexist in the resolved dependency graph. Both
// `idna` and its `icu_*` dependencies are pinned at the exact versions
// already present in this workspace's `Cargo.lock` before this crate added
// its own dependency on `url` (via `rtsp-types`) -- confirmed by diffing
// `Cargo.lock`, adding this crate's dependencies did not bump a version
// anywhere in that chain. This crate is only the first *natively built*
// crate in the workspace to pull `url` in (`corvette-ui`'s own use of `url`
// is wasm-target-only), which is why this otherwise-preexisting version
// split becomes visible to a native `cargo clippy --workspace` run for the
// first time here. There is no version of `rtsp-types` that avoids
// depending on `url`, and neither `url` nor `idna` offers a feature flag to
// select a non-`icu4x` backend, so no code or dependency change in this
// crate resolves it. Flagged for human review per `AGENTS.md`'s
// clippy-bypass policy rather than silently allowed away.
#![allow(clippy::multiple_crate_versions)]

pub mod session;

#[cfg(feature = "mock-camera")]
pub mod mock_camera;
