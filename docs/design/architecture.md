# Architecture

Corvette replaces Frigate one boundary at a time: a Leptos UI first, then a
Rust service behind the same nginx router, followed by detection and recording.
The end state is a Rust NVR using ncnn/Vulkan for inference. Planned work and
current status live in [GitHub issue #1][roadmap].

This document describes the target architecture Corvette is converging on.
The replacement is incremental (see below), so a section can describe a
component ahead of the issue that ships it. An undecided target is marked as
open. Current status per component lives in the roadmap issue.

[roadmap]: https://github.com/cvandesande/corvette/issues/1

## Repository boundary

This repository owns the Rust NVR, shared wire types, and Leptos UI.
`frigate-vulkan` owns the existing ncnn detector plugin, Frigate-derived
container builds, and deployment glue while Frigate remains in the pod. The
repositories intentionally have different release inputs: Corvette is pinned to
a Rust toolchain, while `frigate-vulkan` is pinned to a Frigate version.

The UI bundle is the one cross-repository build dependency. Corvette publishes
its static `target/site` output as an OCI artifact that is "citable, stable
and digest-pinnable — a digest names whatever bytes were actually published,
once." The digest pins one specific build's output; producing the same digest
from a second clean build is a separate property, tracked as its own goal in
[issue #13][byte-reproducible-builds]. The
downstream nginx image consumes a `CORVETTE_VERSION` pinned by digest
alongside its Frigate and ncnn inputs. nginx moves into this repository once
Corvette owns both the UI and recording playback.

[byte-reproducible-builds]: https://github.com/cvandesande/corvette/issues/13

## Crate structure

Everything this repository owns lives in one Cargo workspace, as separate crates
— `crates/corvette-api`, `crates/corvette-ui`, `crates/corvette-ui-server`,
`crates/ncnn-sys`, `crates/ncnn-spike` today. This keeps one shared
`rust-toolchain.toml` and Nix devShell, and one issue-linked design/planning
apparatus (`AGENTS.md`, `.agents/issue-*/`), covering every component. A
workspace member crate can still be published to crates.io on its own version
and cadence — the tokio/hyper ecosystem does exactly this — so a crate in this
workspace can remain independently reusable. A crate earns its own repository
once it gains outside interest or a release cadence genuinely independent of
the rest of the workspace.

Where a component is a generic capability with more than one possible backend,
the crate boundary is a narrow trait plus swappable implementation crates
behind it, so each build depends only on the SDK its own backend needs. The
RTSP-restream server (decisions D-6/D-7 in
`.agents/issue-12/DESIGN-live-view.md`, and its own research in
`.agents/issue-12/RESEARCH-rtsp-restream-server.md`) is the first case of this:
a `StreamProvider`-shaped trait boundary, generic and Corvette-independent, so
the crate itself stays reusable outside this project. The detection engine
follows the same shape as ncnn/Vulkan is joined by other accelerators: a small
crate defines the detection trait and shared types (detection results,
bounding boxes, frame format), independent of any backend; `ncnn`/Vulkan is
one implementation crate against that trait, and Coral (Edge TPU) and Hailo
are future sibling implementation crates, each pulling in its own SDK
(`libedgetpu`, HailoRT) on its own.

## Deployment stakes

The Frigate deployment this repository currently targets (`frigate-vulkan` on tirnanog)
is a personal test environment, not production infrastructure. Work through the UI-deploy
phase (issue #2) does not need production-grade rollout ceremony — gated cutovers,
manifest-fidelity guarantees, elaborate review cycles — since a mistake there costs a
redeploy, not real data or uptime for anyone else. Rigor should scale up as later phases
put real data at risk: retention and storage management, the last replacement (see Risk
boundary below), is where correctness actually matters, because that is the point this
system starts holding recordings someone relies on.

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

Live view is Rust-native end to end. The grid tile is a native MSE player over
WebSocket. The expanded view attempts a Rust-built MoQ relay first, over
QUIC/WebTransport (`moq-dev/moq` — issue #12 D-1/D-2), falling back to
HLS/LL-HLS on failure or timeout. A Rust ingest bridge (D-5) and a
from-scratch Rust RTSP-restream server (D-6/D-7) together take over every
role go2rtc played, including Frigate's own `detect`/`record` camera
connections as well as the browser-facing player — the deployed image ships
Corvette's own Rust media components. See
`.agents/issue-12/DESIGN-live-view.md` for the decisions of record and
`.agents/issue-12/PLAN-live-view.md` for the implementation plan.

**Open:** the grid tile's own live MSE source, once go2rtc's replacement is
complete. The current implementation plan sources it from go2rtc's
`/live/mse/api/ws` (`PLAN-live-view.md` BLOCKER-1, reading (a)), which needs
reconciling with D-6/D-7's Rust-native replacement — flagged here as a
decision still to make.

Browser-native `MediaSource` performs decode and render for the MSE grid
tile, the HLS fallback path, and recorded fragments. The single-camera
expanded view carries audio over MoQ alongside video, when the camera itself
provides it — D-4 already scopes the ingest transport to native
H.264/H.265/AAC, so the relay and `hang`'s Web Component carry an audio track
the same way they carry video, with no transcoding.

Recent-events detection boxes are a separate, simpler mechanism: no video
decode is involved at all. The server stores each detection's box coordinates
as data and serves the plain snapshot; the browser draws the box on a canvas
over the `<img>`, computed at render time from stored coordinates rather than
baked into the image at capture time. See
`.agents/issue-5/RESEARCH-review-snapshot-api.md` for the annotation-gap
research this answers.

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
