# Roadmap: from a Frigate derivative to an NVR

Where this is going, and why. Adapted 2026-08-03 from the plan originally written
in the `frigate-vulkan` repo, once the ncnn spike proved the premise this
repository exists for.

The arc is to stop being a Frigate derivative: a Rust/Leptos UI first, then a
Rust NVR behind it, with the ncnn/Vulkan detector as the part that was always the
point. **That work is this repository.** `frigate-vulkan` keeps the detector
plugin and the container packaging; nothing here is pinned to `FRIGATE_VERSION`,
and nothing there is pinned to a Rust toolchain.

## Status

| | State |
| --- | --- |
| **Stage 2, the ncnn-from-Rust spike** | **Done, 2026-08-03.** The premise holds; see [ncnn-spike.md](ncnn-spike.md) |
| **Stage 0, the Leptos UI** | **In progress, started 2026-08-03.** Live view, event browsing and availability-aware recording playback work. Cargo Leptos splits the recording route into a lazy WASM payload; timeline zoom is next. |

Packaging work -- the distroless split pod, retiring the Frigate donor image --
belongs to `frigate-vulkan` and is parked there, not tracked here. It does not
block any stage below: the split pod makes replacing one component at a time
easier, but the strangler works against the current single-container image too,
because nginx is already the router either way.

> That plan was deleted rather than kept in two places. To read it:
> `git show ada11cb:docs/distroless-split-plan.md` in `frigate-vulkan`.

## Sequence

| Stage | Scope | Repo | Frigate after |
| --- | --- | --- | --- |
| 0 | Leptos UI against Frigate's existing API | **this repo** | untouched |
| 1 | Backend trim -- overlay fork of `app.py`, `api/fastapi_app.py`, `api/classification.py` | `frigate-vulkan` | -400MB, no embeddings |
| 2 | **ncnn-from-Rust spike** -- **done**, see [ncnn-spike.md](ncnn-spike.md) | **this repo** | untouched |
| 3 | Rust service in the pod, read-only routes first (stats, config, event list) | **this repo** | shares `/api` |
| 4 | Rust detection pipeline: frame ingest, motion, ncnn, tracking | **this repo** | detector retired |
| 5 | Rust recording, events, retention | **this repo** | Frigate retired |

### Stage 0 progress

The first reviewable foundation is complete:

- `crates/corvette-ui` provides a responsive Leptos CSR application shell.
- `crates/corvette-api` owns the shared subset of Frigate's `/api/config`
  contract, and the UI discovers and orders enabled cameras through it.
- Camera cards use go2rtc's maintained MSE player, keeping media decode outside
  WASM and the stream inside one TCP connection.
- The event history shows Frigate review activity from the last six hours with
  review thumbnails, unambiguous browser-local timestamps, camera names and
  entered zones. It can be filtered to alerts, detections or significant motion;
  mobile layouts initially collapse the list to four entries.
- Events is also a distinct `/events` route with an uncollapsed review feed, a
  touch-friendly 21-day range calendar with highest-severity activity dots, and
  the same activity filters. Days without retained footage are dimmed using the
  shared recording-availability contract, and selected reviews play Frigate's
  high-resolution review clip. Calendar queries use local-day boundaries,
  including 23- and 25-hour daylight-saving days.
- Continuous recordings can be selected by camera with common range shortcuts
  or by tapping the start and end of a calendar range. A single player follows
  the selected point on the availability timeline.
- Calendar selections accept start/end times and summarize each day's highest
  review severity as motion, detection or alert. Days without retained footage
  are disabled using Frigate's camera-specific recording summary.
- Recordings is a distinct `/recordings` client-side route, while the dashboard
  keeps live view and recent activity focused on initial navigation.
- Selected recording ranges expose retained spans on a timeline. Its playhead
  snaps gaps to available footage, shows review activity and playable motion
  recording ranges across the full selection, and uses Frigate's low-resolution
  preview videos for responsive seeking. Activity playback advances through the
  next chronological motion, detection or alert and stops after the final item.
- `make serve-ui` forwards Frigate and go2rtc from Kubernetes and serves the UI
  with live reload; camera discovery, playback and recent events are verified
  against the running `icams` deployment in Chromium through Playwright.
- The pinned Rust toolchain includes `wasm32-unknown-unknown`, and the Nix
  development shell provides Cargo Leptos, Node.js and a Chromium-only Playwright
  browser bundle.
- `make check` builds an optimized WASM bundle in addition to running the
  repository's Rust, C++, shell, Nix and whitespace checks.

Cargo Leptos builds the UI with `--split` for development and release. The local
server preserves the Frigate and go2rtc proxies and serves the application shell
as the fallback for direct client-side route navigation. The release artifact is
plain static content under `target/site`, including a separate recording-route
WASM payload that is fetched only when Recordings is opened.

The next timeline slice is adding zoom for long recording ranges.

Known recording-browser issues:

- Frigate returns a JSON `400` when a selected range has no recordings, but a
  `<video>` element reports that as an unsupported MIME type. Preflight the VOD
  mapping and show Frigate's actual error before assigning the media URL.
- A reported July 31 selection requested August 1. Verify date-to-timestamp
  conversion across the browser timezone and daylight-saving boundaries when
  the first committed Playwright regression suite is added.

Stage 0 is worth doing on its own merits even if nothing after it happens: it
replaces 21MB of React, and it is what makes stage 1 safe, since a UI you own is
a contract you own. The split-WASM build is now proved; replacing the React
application is not. Note the dependency though -- stage 0 is where the Leptos
bet is placed, and the case for Leptos over a TypeScript framework rests on
stages 3-5 actually landing. See "UI stack" below.

Two facts make the whole thing stageable rather than a rewrite-or-nothing bet.

**nginx is already the router.** `location /api/` proxies to `upstream
frigate_api`, a literal `127.0.0.1:5001`. Adding a Rust service to the pod means
adding a second upstream and moving routes across one at a time -- the strangler
pattern falls out of the architecture that already exists, with no new machinery
and a per-route rollback.

**The hard part of the UI is already handled by components being kept.** Live
video is go2rtc (WebRTC/MSE on 1984) and recording playback is `nginx-vod` HLS on
`/vod/`. Frigate's React app embeds those; it does not implement them. A Leptos
UI does the same. What remains is CRUD over JSON and images -- events, review,
config, stats, snapshots -- which is squarely what Leptos is good at.

## Why two repositories

This repo owns the Rust: the NVR and the Leptos UI. `frigate-vulkan` keeps the
ncnn/Vulkan detector plugin, the container builds and the deployment glue. The
split was worth making at the outset rather than extracting later, for three
reasons:

- **Different languages, different lifecycles.** Nothing here is pinned to
  `FRIGATE_VERSION`, and nothing there is pinned to a Rust toolchain.
- **`frigate-vulkan`'s reason to exist survives either outcome.** Its detector
  plugin plus packaging is useful whether or not the NVR ever ships. Keeping them
  separate means a stalled `corvette` does not strand the working thing -- which
  is exactly the situation as of today.
- **The boundary is already container-shaped.** nginx routes between containers
  by upstream, so the strangler in stages 3-5 works across two repos with no
  extra machinery.

The UI belongs here, not there, because the argument for Leptos is shared serde
types with the Rust NVR -- splitting those across repos would forfeit it. That
creates the one real cross-repo dependency: **the nginx image needs this repo's
built UI bundle.** Publish it as an OCI artifact and let the nginx stage `COPY
--from` it, pinned by a `CORVETTE_VERSION` ARG alongside `FRIGATE_VERSION` and
`NCNN_TAG`. That is the same shape as the donor image being retired there, with
the difference that this one is ours, small, and pinned by digest.

Expect the nginx container to migrate here eventually -- once it is serving this
repo's UI and this repo's recordings, it has no reason to live there. Until stage
5 it still serves Frigate's `/vod/` and media, so it stays put.

## Stage 2 in hindsight

Stage 2 was taken out of dependency order deliberately: everything after it
assumed ncnn was usable from Rust with Vulkan, and that was the one assumption
whose failure invalidated the whole plan.

It held. The C API covers the entire inference path and the output is bitwise
identical to the Python detector's, at the same speed. The one gap -- ncnn's C
API has no GPU enumeration, only selection -- is closed by
`crates/ncnn-sys/csrc/c_api_ext.cpp`, which turned out cheaper than the plan
originally assumed: it compiles against ncnn's *installed* headers, so it is a
translation unit of ours rather than a patch, and there is no fork to rebase on an
`NCNN_TAG` bump. Full result, including what was deliberately not covered, in
[ncnn-spike.md](ncnn-spike.md).

Two findings that change later stages slightly:

- `ncnn_net_get_input_name`/`get_output_name` make the detector's
  `_parse_blob_names` unnecessary here -- stage 4 drops that function rather than
  porting it.
- ncnn's Vulkan instance is process-wide, so a lost device still needs a new
  process. Whatever supervises the detector in stage 4 has to be able to restart
  it.

## UI stack: Leptos, and why the video path is not the risk

The instinct is that WASM is a poor fit for a video-heavy UI. It is not, because
**WASM never touches video frames.** Frigate's bundle drives playback with
`hls.js`, `jsmpeg`, `RTCPeerConnection` and `MediaSource`/`SourceBuffer` -- all of
which run in the browser's native media pipeline. The framework creates a
`<video>` element, negotiates, and renders the chrome around it; decode happens in
the browser's C++ stack whether the app is React, Leptos or vanilla JS. The
question is ecosystem access, not throughput.

**Decision: use go2rtc's maintained WebRTC player for live video, drive recording
MSE with fMP4 directly, and do not take a dependency on hls.js.** Frigate already
proxies go2rtc's player and signalling socket through authenticated nginx routes,
so embedding that player keeps stream negotiation with the component that owns
the protocol. Recordings are the only reason Frigate needs `hls.js` -- Chrome and
Firefox will not play HLS natively in `<video>`, only Safari will. But the nginx
config already sets `vod_hls_container_format fmp4`, and the `/stream/` location
already advertises `application/dash+xml`, so the segments can be fed to
`MediaSource` from Rust directly. That removes the largest JS dependency the UI
would otherwise have to wrap.

**Leptos is justified by the Rust NVR, not by the UI.** The compounding win is
shared serde types across the boundary that is today pydantic on one side and
hand-written TypeScript on the other; fine-grained reactivity also suits a live
dashboard of event feed, camera state and stats over a websocket. Both are real,
but the first is contingent -- if stages 3-5 never land, most of the argument goes
with them. Were this a UI replacement alone, **SolidJS or Svelte** would be the
better pick: the same fine-grained reactivity model, native access to `hls.js` and
the charting ecosystem, much smaller than React. **Dioxus** is the alternative
Rust option, worth revisiting only if a desktop or mobile client ever matters.

Worth keeping in proportion: an NVR UI is lists, a grid of `<video>` elements and
a timeline scrubber. It is not a heavy reactive application, and the framework
choice should not become the project.

## Architecture: x86_64 and aarch64, one of them tested

`flake.nix` builds for both. **arm64 is unvalidated, not unsupported**, and the
distinction is the whole of this section: what is missing is hardware to test on,
not code.

Nothing here is x86-specific. The FFI is `c_int`- and pointer-shaped, the C++ shim
touches no intrinsics, and ncnn's Vulkan backend is mobile-first -- if anything it
is better exercised on ARM than on desktop x86. A Nix build on an aarch64 host
compiles ncnn natively, so the cross-compilation problem never arises; it is only
the *container* path that would need a native arm64 builder or QEMU for the
longest stage of the build.

The amd64-only decision this inherited belonged to `frigate-vulkan`, and does not
transfer. That repo exists to pin down RADV behaviour on specific AMD discrete
cards -- gfx803, gfx906 -- so for it, multi-arch doubles a soak matrix whose whole
point is hardware that no arm64 machine has. Neither of those is true here: this
repo is bindings and an NVR, and its correctness is not a property of one vendor's
driver.

## Contracts a Rust NVR must honour

These are what the rest of the pod depends on, and they are small enough to write
down:

- **Review media availability is explicit and independent.** A review record can
  outlive its thumbnail, snapshot or recording, and each asset can have a
  different retention policy. Corvette's API must report thumbnail/snapshot and
  playable-clip availability as separate current capabilities. Clients must not
  infer video availability from the existence of a review or discover it by
  assigning a media URL and interpreting a `404`.
- **`vod_mode mapped` + `vod_upstream_location /api`.** nginx-vod asks `/api` for
  a JSON mapping of a playback request to files on disk, then reads them itself.
  Whatever serves `/api` has to answer that, or recording playback stops working.
- **`/media/frigate` layout.** nginx serves `clips/`, `recordings/` and
  `exports/` directly with `root /media/frigate`, so the Rust writer keeps the
  layout or the nginx config changes with it.
- **`/dev/shm/go2rtc.yaml`.** go2rtc does not read Frigate's `config.yml`; the
  file is rendered for it. A Rust NVR generates it directly, which is one of the
  few places where replacing Frigate makes the deployment simpler rather than
  harder.
- **`/tmp/cache/birdseye`** only matters if birdseye is kept. It probably should
  not be.

## What is actually being rewritten

Not the detector. `frigate-vulkan`'s `docker/frigate/ncnn.py` is 181 lines and
already exists; of the 4,587 LOC in `detectors/`, nearly all is other runtimes
(OpenVINO, TensorRT, Hailo, degirum, EdgeTPU) that the project never loads. The
rewrite is the ~54k LOC that makes Frigate an NVR rather than a detector, and it
is worth being clear-eyed about which parts are hard:

- **Frame ingest** is not hard -- Frigate spawns ffmpeg per camera with rawvideo
  to a pipe. Rust does the same; no libav binding required.
- **Tracking** is bounded -- norfair is Kalman + Hungarian assignment, and Rust
  has equivalents (`similari`) or it can be ported.
- **Config schema** is 3,202 LOC of pydantic. A serde equivalent is real work, but
  a new UI means a smaller schema is allowed.
- **Retention and storage management** is the genuinely risky one. It is subtle,
  it is where bugs destroy recordings rather than merely erroring, and it deserves
  to be last.

## Open questions

- ~~**How to restore GPU enumeration for the Rust port.**~~ Settled 2026-08-03 by
  the spike: `c_api_ext` compiled against ncnn's installed headers, in
  `crates/ncnn-sys/csrc`. No fork, and `GpuInfo::type()` came with it. See
  [ncnn-spike.md](ncnn-spike.md).
- **Which model, and under what licence.** The detector currently runs YOLOv9
  weights that are AGPL-3.0 from Ultralytics (GPL-3.0 upstream), which is fine for
  a private deployment and a real constraint on anything shipped. A permissive
  alternative means replacing the YOLO-generic post-processing, not editing
  config. Decide before stage 5, not during it.
