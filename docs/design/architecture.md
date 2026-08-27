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

Live view is Rust-native end to end. `crates/rtsp-restream` is a standalone,
from-scratch RTSP server (issue #12 D-6/D-7), reusable outside Corvette: five
methods (`OPTIONS`, `DESCRIBE`, `SETUP`, `PLAY`, `TEARDOWN`), one transport
(`RTP/AVP/TCP`, interleaved), no authentication — the narrowed surface every
known consumer, including Frigate's own `detect`/`record` ffmpeg, actually
uses. It owns no socket or transport code of its own; an embedder's async I/O
layer drives one RTSP session per accepted connection against a
caller-supplied stream provider.

`crates/corvette-media-bridge` is the one process that embeds it (D-8). Per
configured camera, it dials that camera once through issue #18's
`corvette-rtsp-client` and fans the resulting broadcast channel of
depacketized frames out to four independent, supervised Tokio tasks — one per
role, so one role's failure for one camera never stops another role for that
camera or any role for another camera (D-8/DP-3/DP-4):

- an RTSP-restream feed, so Frigate's own `detect`/`record` ffmpeg (and any
  other RTSP client) can play that camera back by name;
- a direct MoQ-publish loop, dialing the relay (`moq-relay`, adopted from
  `moq-dev/moq`, D-1/D-2) and publishing the camera's frames as a broadcast
  named after the camera, with no container format anywhere in the path
  (D-9);
- an fMP4-over-WebSocket repackager: the grid tile's own live data source,
  reusing the shared frame stream to build one WebSocket connection per
  viewer, matching go2rtc's own previous MSE-tile transport shape;
- an HLS/LL-HLS packager: the expanded view's fallback backend, likewise
  reusing the shared frame stream, serving N1's `/live/hls/` location when
  the MoQ relay is unreachable or times out.

One Corvette-owned OCI image (`docker/Dockerfile.media-bridge`) carries both
`moq-relay` and `corvette-media-bridge`; a deploying manifest chooses which of
the image's two binaries each container runs.

Kubernetes pods share one network namespace across their containers. Frigate's
own `detect`/`record` ffmpeg dials `rtsp://127.0.0.1:8554/<camera>[_sub]`
inside that shared namespace, and the media-bridge container's RTSP-restream
server now answers that port from a sibling container in the same pod — a
real topology change this document states plainly: detect/record footage
flows through a different container than it did before. Frigate's own
`ffmpeg.inputs` configuration needs no edit, since only which container
answers the port changes. The MoQ relay's own external listener is reached
directly through a dedicated NodePort UDP Service, since nginx has no QUIC/
WebTransport support to proxy it through. See
[API and media contracts](api-contracts.md) for the full
RTSP-restream protocol contract, the media-bridge's broadcast/track naming,
the NodePort choice, and the retired/replaced live-view routes.

The cluster manifest change carrying the port move and the NodePort Service is
drafted and reviewed but not yet applied to the running deployment; every
component described above is implemented and verified in this repository
today. See `.agents/issue-12/DESIGN-live-view.md` for the decisions of record
and `.agents/issue-12/PLAN-live-view.md` for the implementation plan.

Browser-native `MediaSource` performs decode and render for the MSE grid
tile, the HLS fallback path, and recorded fragments. The relay and `hang`'s
Web Component, and the HLS/LL-HLS packager, each carry a configured camera's
H.264/H.265 video track today. Audio follows the same paths once
`corvette-rtsp-client` resolves a camera's SDP audio section — see API
contracts for the current state of that gap.

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
