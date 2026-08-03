# Architecture

Corvette replaces Frigate one boundary at a time: a Leptos UI first, then a
Rust service behind the same nginx router, followed by detection and recording.
The end state is a Rust NVR using ncnn/Vulkan for inference. Planned work and
current status live in [GitHub issue #1][roadmap].

[roadmap]: https://github.com/cvandesande/corvette/issues/1

## Repository boundary

This repository owns the Rust NVR, shared wire types, and Leptos UI.
`frigate-vulkan` owns the existing ncnn detector plugin, Frigate-derived
container builds, and deployment glue while Frigate remains in the pod. The
repositories intentionally have different release inputs: Corvette is pinned to
a Rust toolchain, while `frigate-vulkan` is pinned to a Frigate version.

The UI bundle is the one cross-repository build dependency. Corvette publishes
its static `target/site` output as an OCI artifact. The downstream nginx image
consumes a `CORVETTE_VERSION` pinned by digest alongside its Frigate and ncnn
inputs. nginx moves into this repository once Corvette owns both the UI and
recording playback.

## Incremental replacement

nginx is already the router for the deployed system. Frigate's API is an
upstream at `127.0.0.1:5001`, so a Rust service can join the pod as another
upstream. Routes move individually, preserving a per-route rollback to Frigate.

The replacement order follows the dependency and risk boundaries:

1. Deploy the Leptos UI against Frigate's existing APIs.
2. Introduce read-only Rust routes for configuration, statistics, and events.
3. Replace frame ingest, motion processing, ncnn inference, and tracking.
4. Replace recording, event creation, and retention.
5. Retire Frigate and move nginx ownership into Corvette.

The ncnn work was proved early because every server-side phase depends on it.
The [ncnn spike](../ncnn-spike.md) drives the complete inference path through
ncnn's C API with output identical to the Python binding.

## Media boundary

WASM does not decode video. Live streams remain in go2rtc's maintained MSE or
WebRTC player, and browser-native `MediaSource` handles recording fragments.
The Leptos application owns negotiation and playback controls while decode and
rendering remain in the browser media stack.

Frigate's nginx-vod integration remains in place until Corvette owns recording.
The Rust service must satisfy the mapping and filesystem contracts in
[API and media contracts](api-contracts.md) before that boundary can move.

## Shared Rust boundary

`crates/corvette-api` owns wire contracts shared by the browser and the future
service. Pure domain behavior can also move into shared crates when both sides
need the same implementation, particularly activity ranges, recording segment
merging, and playback availability. Server-authoritative work stays on the
server; sharing Rust does not justify sending raw storage telemetry to browsers.

## Detection process

ncnn's Vulkan instance is process-wide. A lost device therefore requires a new
process rather than rebuilding one detector object. The detection service must
run under supervision that can restart it.

ncnn exposes model input and output names through its C API, so Corvette reads
them directly instead of carrying Frigate's model-file blob-name parser. GPU
enumeration uses Corvette's installed-header C++ shim; it is a translation unit
of this repository rather than an ncnn fork.

## Supported architectures

The Nix flake builds for x86_64 and aarch64. aarch64 is unvalidated rather than
unsupported: the FFI is pointer- and C-integer-shaped, the C++ shim uses no
architecture-specific intrinsics, and ncnn's Vulkan backend supports ARM.
Native aarch64 hardware validation remains tracked in GitHub.

## Risk boundary

Frame ingest is an ffmpeg raw-video pipe, and tracking is a bounded assignment
problem. Configuration is substantial but can be smaller than Frigate's schema
because Corvette owns its client. Retention and storage management are the
highest-risk subsystem because defects can destroy recordings; they remain the
last replacement and require failure and recovery testing.
