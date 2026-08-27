# Implementation plan — issue #12: replace go2rtc with a Rust-native media transport

Phase: PLAN (`docs/standards/AGENTIC_PIPELINE_PLAYBOOK.md` §3–§6). Untracked working
document — do not `git add` or commit (`AGENTS.md`: `.agents/` is outside git history).
**This is a full redraft of the prior `PLAN-live-view.md`**, which was drafted against
decisions D-1 through D-5 only and never reached human approval (its own gate line said
"awaiting human review"). Since that draft, four more decisions — D-6, D-7, D-8, D-9 —
were approved the same day in `.agents/issue-12/DESIGN-live-view.md`, and issue #18 (the
camera-facing RTSP client, D-8's other named participant) shipped in full. Those five
facts invalidate the prior draft's BLOCKER-1 framing, its G1 item, and its packaging/
exposure items in ways a patch could not honestly paper over — hence a full rewrite
rather than an amendment.

Sources read in full for this redraft: `.agents/issue-12/DESIGN-live-view.md` (D-1
through D-9, all gate-approved), `.agents/issue-12/DESIGN-review-report.md` (D-1..D-5
fact review), `.agents/issue-12/DESIGN-review-report-D6-D7.md` (D-6/D-7 fact review,
VERDICT: PASS, two non-decisive numeric corrections), `.agents/issue-12/RESEARCH-moq-
rust-candidates.md`, `RESEARCH-rtsp-moq-bridge.md`, `RESEARCH-webtransport-ingress.md`,
`RESEARCH-frigate-ingest-boundary.md`, `RESEARCH-rtsp-restream-server.md`,
`RESEARCH-d4-ingest-source.md`, the prior (superseded) `PLAN-live-view.md` (read as a
structural/house-style reference), `.agents/issue-18/PLAN-camera-rtsp-client.md` in full
(DONE, all 8 items closed, including two real-camera-discovered follow-ups) plus its
`C1-evidence.md`/`C2-evidence.md`, the actual shipped source at
`crates/corvette-rtsp-client/src/client.rs` and `src/client/task.rs`, issue #12's own
body (`gh issue view 12`), `docs/design/architecture.md`'s "Media boundary" section
(which already carries an **"Open"** note this plan resolves into a named question, see
below), `docs/design/api-contracts.md`, `crates/corvette-ui/src/dashboard.rs`, and
`tests/nginx-parity/vendor/nginx.conf`.

Profile: **Critical** overall — a new externally-reachable network listener
(NodePort/UDP for MoQ), a new *inbound* RTSP listener replacing a role Frigate's own
detect/record pipeline depends on for correctness-critical footage, a media-relay
service, and per-camera concurrent tasks each independently trip the Critical row of
the playbook's own table. Per-item profiles below narrow this only where an item's own
triggers clearly support Significant or Routine.

**PLAN GATE: APPROVED (human, 2026-08-26), as drafted.** Implementation of every item
below is authorized. No item carries a status other than UNSTARTED yet — approval
releases the plan, it does not itself close any item.

Naming (§5): letters are this feature's own work breakdown. `R`, `G`, `P`, `K`, `N`, `U`,
`W`, `V` carry forward from the superseded draft where their content survives
substantively unchanged; `X` and `F` are new phases this redraft adds. Numbers inside a
letter are items. Roadmap issue numbers (#1–#18) are never item ids.

---

## How this redraft resolves the prior BLOCKER-1 — read this before anything else

The superseded draft's BLOCKER-1 asked one question: *does go2rtc keep running after
this issue ships, for anything at all?* It offered three readings, (a)/(b)/(c), and
wrote its U1/N2 items under reading (a) (go2rtc keeps running for everything except the
embedded WebRTC iframe) with a STOP-AND-ASK if the human picked otherwise.

**That question is now answered, directly, by decisions of record, not by this plan's
own guess: no.** D-6 states plainly that fully replacing go2rtc means replacing its
role in Frigate's own `detect`/`record` camera-ingest path, not only its live-view role,
and commits to a from-scratch Rust RTSP-restream server to do it. D-7 chose to build
that server fully from scratch. D-8 puts it in the same process as the camera-facing
RTSP client (issue #18, now shipped) and the MoQ ingest bridge. D-9 removes MPEG-TS from
the ingest bridge's own path, since it no longer has a TS-producing go2rtc to read from.
Read together, these are closer to the old draft's reading (b) than (a) — but concretely
specified, not a guess: **go2rtc's process is fully replaced, everywhere it currently
runs in this deployment, by Rust components this plan builds.** This redraft does not
carry BLOCKER-1 forward as an open question; it is closed, and every item below is
written against that closure.

**Closing BLOCKER-1 this way opened two new, narrower questions neither D-6 through D-9
nor either research pass answered.** This plan did not invent answers to either when
first drafted — it named them and gated the affected items on a human decision, the same
way the prior draft gated U1/N2 on BLOCKER-1 itself. **Both are now answered, by the
human, 2026-08-26 — see the RESOLVED note under each.** The historical framing below (why
each was open, and the three options actually weighed) is kept verbatim, because it is
still the record of what was considered, not because either question is still pending.

**OPEN-1 — what serves the grid tile's live MSE data source, once go2rtc is gone?**
`docs/design/architecture.md`'s own "Media boundary" section already carries this as an
explicit, unresolved "Open" note (added ahead of this redraft): *"the grid tile's own
live MSE source, once go2rtc's replacement is complete... needs reconciling with D-6/
D-7's Rust-native replacement — flagged here as a decision still to make."* Nothing in
D-6 through D-9 produces an MSE/fMP4-over-WebSocket output: D-6/D-7's RTSP-restream
server speaks RTSP to Frigate's ffmpeg (and, per F-7, possibly other RTSP clients); D-8/
D-9's MoQ ingest bridge speaks MoQ to `hang`'s Web Component for the *expanded* view.
Neither is what a native MSE `<video>`/`SourceBuffer` grid tile, per issue #12's own
Scope wording ("mirroring Frigate's own `MsePlayer.tsx`"), needs to connect to. This is
not a gap this plan can guess its way past — three genuinely different, materially
different-cost answers exist, and picking one is a human, not an implementation,
decision:

- (a) Build a new MSE/fMP4-over-WebSocket output as a further consumer of the shared
  per-camera frame stream (G1, below) — a new component, unscoped by any D-n decision,
  comparable in shape to G1's own MoQ-publish logic but emitting fMP4 boxes over a
  WebSocket instead of MoQ objects over QUIC. Keeps the grid tile's technology
  unchanged from what issue #12's Scope text describes.
- (b) The grid tile also uses MoQ via `hang`'s Web Component, the same mechanism U2
  already builds for the expanded view — abandons the "native MSE, mirroring
  `MsePlayer.tsx`" framing in issue #12's own Scope section, which would need
  correcting, but reuses U2's already-planned client and G1's already-planned publish
  path with no new server-side component.
- (c) The grid tile falls back to the same HLS/LL-HLS path U2's expanded view uses on
  MoQ failure — simplest engineering-wise (no new component at all, reuses N1/U2's own
  work) but HLS's segment-interval latency is a real UX regression from go2rtc's
  existing low-latency MSE tile for a security-camera grid meant to show *now*.

**RESOLVED — option (a), human decision, 2026-08-26.** Build a new, small, dedicated
server-side component — item **G2** below — that subscribes to the same in-process
per-camera frame stream every other consumer in this plan already reads from (the D-8
broadcast channel) and repackages the existing Annex-B H.264/H.265 NAL units into fMP4
boxes delivered over a plain WebSocket to the grid tile: no transcoding, structurally
comparable in shape and cost to this plan's own X2 (RTP packetizer) and G1's own
MoQ-publish logic, both of which already do "frame stream in, different wire format out"
work in this exact plan. The human's own stated reasoning, recorded here verbatim as the
decision rationale, not summarized: *this is the cheapest of the three options
computationally — no extra decode, minimal per-frame repackaging cost, and a single
lightweight WebSocket per viewer (the same transport go2rtc's own MSE tile already uses
today, so this preserves current grid behavior/latency) — versus reusing MoQ/`hang` for
the grid (which would multiply real per-tile WebTransport/QUIC connection overhead for no
capability the grid actually needs) or falling back to HLS for the grid (segment-based
I/O and HTTP-polling overhead, and a worse latency profile for a security-camera grid
meant to show video now.* See **DP-4** below for where G2 lands architecturally (inside
the same `corvette-media-bridge` process as G1, not a separate component or process) and
item **G2** for its full Premise/Do/Verify/Mutation/Scope guard.

**OPEN-2 — what serves the HLS/LL-HLS fallback path at all, once go2rtc is gone?**
This is a second instance of the same problem, found independently while verifying N1's
and U2's own premises for this redraft, not named by any prior document. The superseded
draft's N1 item assumed `/live/hls/` proxies to *go2rtc's own* HLS handler
(`internal/hls/` in go2rtc's source) — the same assumption U2's fallback step made for
the expanded view's own HLS `<video>` element. Neither `RESEARCH-moq-rust-candidates.md`
nor `RESEARCH-webtransport-ingress.md` nor any D-n decision states that `moq-dev/moq`'s
own stack (`moq-relay`/`hang`) produces HLS output at all — grepped directly, in this
redraft's own drafting, and not found. Issue #12's Done-when clause requires "HLS/LL-HLS
as fallback" in the same breath as "no go2rtc process in the deployed image" — those two
clauses are now in direct tension the same way BLOCKER-1's own two source clauses were.
Options, not decided here:

- (a) Build a new HLS-packaging output as a further consumer of the shared per-camera
  frame stream (parallel to OPEN-1(a)) — another new, unscoped component.
- (b) Drop the HLS fallback requirement; U2's failure mode on an unreachable MoQ relay
  becomes a retry/error state instead of a working fallback — a real regression from
  issue #12's own stated Done-when clause, which would need correcting on the issue
  itself, not silently reinterpreted.
- (c) Keep a narrow, HLS-only remux process alive solely for this path — a materially
  smaller ask than what D-6 rejected (D-6 was about the *detect/record RTSP-restream*
  role specifically), but nothing in D-6/D-7's text authorizes it either; a human would
  need to say so explicitly.

**RESOLVED — option (a), human decision, 2026-08-26.** Build a new, small, dedicated
HLS/LL-HLS packager — item **G3** below — as a further consumer of the same shared
per-camera frame stream (the D-8 broadcast channel), landing inside the same
`corvette-media-bridge` process as G1 and G2, per **DP-4** below. The human's own stated
reasoning, recorded here as the decision rationale: a real HLS fallback has genuine
resilience value — for networks that block the UDP transport WebTransport/QUIC needs,
and for browsers lacking WebTransport support at all — and dropping it (option (b))
would walk back issue #12's own promised "HLS/LL-HLS as fallback" Done-when clause,
while keeping a narrow go2rtc-derived remux process alive (option (c)) would leave a
residual go2rtc dependency this issue exists to remove entirely. See item **G3** for its
full Premise/Do/Verify/Mutation/Scope guard.

**Both OPEN-1 and OPEN-2 are now resolved (2026-08-26), so no item remains blocked on
either question.** U1's Do (grid tile, OPEN-1) and N1/U2's HLS-fallback Do steps
(OPEN-2) are fully specifiable now, against G2 and G3 respectively — see each item
below. Gate **G-10** is accordingly closed; its text is kept below as the historical
record of what it guarded against, not as an active block.

---

## Item status (updated as items close; the plan itself is not re-litigated)

| Item | Status | Commits | Evidence |
| --- | --- | --- | --- |
| R1 | DONE | `793ccc1` | `.agents/issue-12/evidence/R1-dependency-pin.md` |
| X1 | DONE | `dea735f` | `.agents/issue-12/evidence/X1-mutation.log` |
| X2 | DONE | `9942146` | `.agents/issue-12/evidence/X2-mutation.log` |
| X3 | DONE | `d4941fa` | `.agents/issue-12/evidence/X3-mutation.log` |
| G1 | DONE | `9876cde` | `.agents/issue-12/evidence/G1-evidence.md`, `.agents/issue-12/evidence/G1-mutation.log` |
| G2 | DONE | `8efe91a` | `.agents/issue-12/evidence/G2-mutation.log` |
| G3 | DONE | `ab9c767` | `.agents/issue-12/evidence/G3-mutation.log` |
| P1 | DONE | `5e35d71` | `.agents/issue-12/evidence/P1-mutation.log` |
| F1 | DONE | `frigate-vulkan@e872baa` | `.agents/issue-12/evidence/F1-mutation.log` |
| K1 | DONE | `ada8767` | `.agents/issue-12/evidence/K1-manifest-draft.yaml`, `.agents/issue-12/evidence/K1-mutation.log` |
| N1 | DONE | `frigate-vulkan@f2335a3`, `corvette@fd46fed`, `corvette@a10e64d` | `.agents/issue-12/evidence/N1-mutation.log` |
| N2 | DONE | `frigate-vulkan@496d23a`, `corvette@12946ef` | `.agents/issue-12/evidence/N2-mutation.log` |
| U1 | DONE | `ba48fbf` | `.agents/issue-12/evidence/U1-mutation.log` |
| U2 | DONE | `2bc4e34` | `.agents/issue-12/evidence/U2-mutation.log` |
| W1 | UNSTARTED | — | — |
| V1 | UNSTARTED | — | — |
| V2 | UNSTARTED (ongoing orchestrator duty) | — | — |

No item has begun.

---

## Decisions of record — quoted verbatim, not re-litigated

From `.agents/issue-12/DESIGN-live-view.md`, all human-approved 2026-08-16. D-6/D-7 are
independently fact-reviewed (`DESIGN-review-report-D6-D7.md`, PASS); D-8/D-9 explicitly
skipped fact review by direct human instruction ("skip the fact check, it's approved") —
a real, recorded gap this plan inherits rather than smoothing over. If an item finds one
of these unimplementable **as stated**, that is STOP-AND-ASK (gate G-7), never a
substitution.

```
D-1  Build the MoQ relay + camera-ingest bridge in Rust, rather than adopting a
     third-party Go media server (mediamtx-moq, mainline MediaMTX). Adopting
     moq-dev/moq's own moq-relay binary largely as-is is consistent with D-1's
     rationale (avoid a second Go server this issue would only replace again).

D-2  Base the Rust implementation on moq-dev/moq (moq-net/moq-relay/hang), not
     cloudflare/moq-rs. Decisive factor: activity, embeddability, and Nix-tooling
     fit — not a transport-incompatibility claim (corrected after fact review;
     IETF-transport interop with cloudflare/moq-rs remains untested either way).

D-3  All three exposure mechanisms (hostPort, NodePort, LoadBalancer) are supported
     as implementation options; NodePort is chosen for this deployment (human
     judgment: "more secure than hostPort"). The exposure mechanism is a
     configuration point, not hard-coded.

D-4  (Superseded by D-9 below; kept here verbatim for the historical record of what
     was originally approved.) Ingest transport is go2rtc's MPEG-TS HTTP endpoint
     (api/stream.ts?src=<camera>), not its WS/fMP4 endpoint and not raw RTSP.

D-5  The ingest bridge is a small in-repo Rust binary against moq-mux/moq-net
     directly, not a subprocess wrapping the moq-cli binary. This part of D-5's
     rationale is unaffected by D-9: the bridge is still in-repo, still direct
     against moq-mux/moq-net, still not a moq-cli subprocess — only its upstream
     frame source changed (D-9).

D-6  Fully replacing go2rtc requires a new Rust-native RTSP-restream server, built
     as its own standalone, reusable crate — not the direct-to-camera alternative
     (repointing Frigate's ffmpeg.inputs at each camera directly), and not assumed
     to be the same component as the D-5 ingest bridge. Resolves
     RESEARCH-frigate-ingest-boundary.md DP-1: this deployment routes both `detect`
     and `record` through go2rtc's own RTSP restream (confirmed live against the
     running pod by the D-6/D-7 fact review, not only a dated snapshot) — an
     optional Frigate pattern (Frigate's own reference config points ffmpeg
     directly at cameras by default), but this deployment opted into it, so
     removing go2rtc here removes something both roles actually depend on today.
     Human instruction: build the restream server as its own standalone crate,
     architected for reuse outside Corvette, not folded into any Corvette-only
     crate by default.

D-7  Build the D-6 RTSP-restream server fully from scratch, against
     rtsp-types/sdp-types directly — not on top of rtsp-runtime (a real,
     license-compatible, but six-week-old single-maintainer sans-IO session
     engine), and not by revisiting the AGPL question for bairelay-rtsp (the most
     complete existing candidate, but AGPL-3.0-or-later, incompatible with
     Corvette's MIT license without a licensing decision this plan does not make).
     Rationale: preference for the project's own engineering discipline over a
     young unproven dependency, and no interest in AGPL's copyleft obligations.
     Narrowed scope accepted alongside it: only 5 RTSP methods (OPTIONS, DESCRIBE,
     SETUP, PLAY, TEARDOWN) and one transport mode (TCP-interleaved) are actually
     required by any known consumer — go2rtc's own deployed RTSP server, read
     directly from source, implements exactly this set and rejects every UDP
     Transport request; Frigate's own ffmpeg preset is hard-coded to request TCP.
     No authentication is required either — no known consumer sends any.

D-8  The camera-dialing client (issue #18, D-1, now shipped), the RTSP-restream
     server (D-6/D-7), and the MoQ ingest bridge (D-5) run in the SAME PROCESS,
     sharing each camera's frames through an in-process channel
     (tokio::sync::broadcast, bounded capacity) — not separate processes, not an
     HTTP hop between them. Avoids an otherwise-redundant repackage/unpack
     round-trip (RTP-packetize then immediately re-depacketize) a separate-process
     design would require.

D-9  Re-decides D-4: the MoQ ingest bridge consumes each camera's frames directly
     off the D-8 broadcast channel and publishes them via
     moq_mux::codec::h264/h265/aac's Split/Import layer directly — no MPEG-TS or
     any other container anywhere in the pipeline. Requires upstream frames
     already in Annex-B (video) / raw non-ADTS (AAC) — issue #18's own D-1
     depacketizer produces exactly this shape, confirmed shipped
     (`crates/corvette-rtsp-client/src/depacketize`).
```

**A fact neither D-6 nor D-7 states explicitly, but which this plan's own drafting
confirms and relies on:** Kubernetes pods share one network namespace across all their
containers. go2rtc's RTSP-restream role today runs *inside* the same container as
Frigate's own Python process (an s6-overlay sibling service, `RESEARCH-frigate-ingest-
boundary.md` F-6); Frigate's ffmpeg dials it at `rtsp://127.0.0.1:8554/<camera>[_sub]`
(F-4). Because a pod's containers share `localhost`, moving that RTSP-restream role to
a *sibling* container (this plan's G1, packaged by P1, following the same DT-1 topology
the prior draft already established) requires **no change to Frigate's own
`ffmpeg.inputs` configuration at all** — `127.0.0.1:8554` keeps working; only what
answers that port changes, from go2rtc (same container) to the new media-bridge
(sibling container). This is a real efficiency finding worth stating plainly, not an
assumption: F1 and K1 below build on it directly.

**DP-3 (superseded, restated for the new architecture).** The prior draft decided "one
Tokio task per camera, inside a single process," reasoning from D-5's "small in-repo
binary." D-8 now names that single process explicitly (client + restream server +
ingest bridge, together), which is a larger claim than the prior draft's own DP-3 was
written against, but the same reasoning applies with one refinement: a camera's frames
now feed **two independent roles** (the RTSP-restream server's own fan-out and the MoQ
ingest bridge's own publish loop), not one. **This plan decides DP-3 as: one supervised
Tokio task per camera per role** — a camera's RTSP-restream-feeding task and its
MoQ-publish task are independent, so one role's failure for one camera does not stop
the other role for that same camera, matching INV-5 below. This is the direct
architectural descendant of issue #18's own C1 per-camera-task pattern, one level
composed.

**DT-1 (superseded in one respect, otherwise carried forward).** The prior draft decided
the relay and bridge ship as two sibling containers in the existing Frigate `StatefulSet`
pod, from one Corvette-owned OCI image with two entrypoints. That topology is unchanged
by D-6 through D-9 and is carried forward as-is. **What changes:** the second
container's own responsibility grows from "read go2rtc's TS export, publish MoQ" to
"host the RTSP-restream server AND publish MoQ" — it now also binds the port Frigate's
own `detect`/`record` ffmpeg dials (8554), a role no container in this pod held before
except go2rtc itself, co-located with Frigate rather than sibling to it. This raises the
blast radius of a bug in this container from "live view degrades" to "Frigate's own
recording pipeline breaks" — reflected in this redraft's risk profile for G1/K1/F1
below, all raised relative to the prior draft's equivalent items.

**DP-4 (new, human decision 2026-08-26, resolving where OPEN-1's and OPEN-2's new
components land).** Both new components the human authorized — the grid tile's
fMP4-over-WebSocket repackager (**G2**, resolving OPEN-1) and the HLS/LL-HLS packager
(**G3**, resolving OPEN-2) — land *inside the same `corvette-media-bridge` process as
G1*, as further per-camera(-per-role) tasks subscribing to the same D-8 broadcast
channel, not as separate components, processes, or containers. This follows directly
from D-8's own same-process rationale (avoid a redundant repackage/unpack round-trip
between processes) and from the human's own OPEN-1 rationale (a single lightweight
WebSocket per viewer, no new process boundary); nothing about G2/G3's purpose calls for
a different topology than G1 already established. **DP-3 is extended by this decision**
to cover these two new roles explicitly — see INV-5's restated text below.

---

## Phases, ordered by dependency and risk (§4)

| Phase | Content | Why it sits here |
| --- | --- | --- |
| **R** | Pin the exact `moq-dev/moq` commit; verify `moq-relay` and the `moq-mux` codec-level `Split`/`Import` API build standalone | Nothing downstream can honestly claim a working dependency until this closes. |
| **X** | The D-6/D-7 RTSP-restream server, as its own standalone crate (`crates/rtsp-restream`) — X1 protocol/session/SDP core, X2 RTP packetizer, X3 async I/O + fan-out layer | The one entirely new phase this redraft adds. Sized across three items per F-14's own 3,000–6,000-line estimate for a from-scratch build — too large for one item, mirroring how issue #18 split its own, comparably-sized client into four. Independent of R1's MoQ work; can proceed in parallel with it. |
| **G** | The media-bridge process (`crates/corvette-media-bridge`): wires issue #18's shipped `corvette-rtsp-client` to X3's `StreamProvider` trait and to the MoQ-publish logic directly (D-8/D-9), plus the OPEN-1/OPEN-2-resolved fMP4-over-WebSocket repackager (G2) and HLS/LL-HLS packager (G3), both landing in this same process per DP-4 | Needs R1 (MoQ codec API) and X1–X3 (the restream server) both closed — this is the integration point where every prior D-n decision actually becomes one running process. |
| **P** | Package `moq-relay` (adopted) and `corvette-media-bridge` into one Corvette-owned OCI image (DT-1) | Needs G1's binary to exist. |
| **F** | `frigate-vulkan`'s own Dockerfile: stop go2rtc's process from starting at all | Independent of the Rust build chain (different repo); genuinely new work this redraft adds, required for D-6's "fully replace" to actually hold — without it, go2rtc and the new media-bridge container would both try to serve camera restream, and Frigate's own s6 supervision still expects go2rtc present. |
| **K** | Cluster exposure: StatefulSet container addition, the MoQ NodePort UDP Service port, the RTSP port's move from the frigate container to the media-bridge container, TLS secret mount — drafted only | Needs P1's image shape and F1's confirmation that go2rtc no longer occupies port 8554. |
| **N** | `frigate-vulkan`'s nginx: add `/live/hls/`, now serving G3's HLS packager (N1, OPEN-2 resolved); retire every go2rtc-proxied route (N2, five routes) | N2 is unconditional for every route that proxies to go2rtc specifically, since go2rtc is confirmed fully gone; the *replacement* for `/live/mse/api/ws` now names G2 directly (OPEN-1 resolved). |
| **U** | UI: grid tile (U1, now wired to G2's WebSocket/fMP4 output per OPEN-1); expanded view (U2, MoQ-first, HLS fallback now wired to G3 per OPEN-2) | Both OPEN-1 and OPEN-2 are resolved; U1 and U2's fallback step are both fully specifiable. |
| **W** | `docs/design/api-contracts.md` and `architecture.md` updates | Last: describes what actually shipped, including which OPEN-1/OPEN-2 answers were chosen. |
| **V** | Cutover evidence, once a human has pushed P1's/F1's images and applied K1's manifest | Needs multiple human gates crossed; now also verifies Frigate's own recording pipeline, not only live-view UI routes — the new, higher-stakes check this redraft adds. |

---

## Invariants (§4) — binding, pasted verbatim into every implement and review brief

Eight. Absolute, mechanically checkable, code-enforced. The enforcement mechanism is
named for each; where it does not exist yet, the item that creates it is named.

```
INV-1  NO THIRD-PARTY GO STREAMING SERVER, AND NO AGPL-LICENSED CODE, IN THE BUILD.
       Nothing under docker/ or frigate-vulkan's donor Dockerfile installs,
       downloads, or COPYs a mediamtx, mediamtx-moq, or other Go-language
       streaming binary (D-1). Nothing under crates/rtsp-restream or
       crates/corvette-media-bridge is copied or adapted from bairelay-rtsp or
       lvqr-rtsp, both AGPL-3.0-or-later and explicitly rejected by D-7.
       ENFORCED-HOW: grep — a build check (created in P1) denying `mediamtx`,
       `bluenviron`, `winkmichael` as literals in any touched Dockerfile; a
       license-manifest check (created in X1) denying `bairelay`, `lvqr` as
       dependency names anywhere in Cargo.lock, and confirming
       crates/rtsp-restream's own declared license is not AGPL.

INV-2  DIGEST, NOT TAG, FOR THE DEPLOYED MEDIA-BRIDGE/RELAY IMAGES. Every
       reference to the media-relay image (P1) or the frigate-vulkan image (F1)
       in a drafted manifest (K1) carries `@sha256:<64 hex>` once a real digest
       exists; before that, an explicit, greppable placeholder token
       (`PENDING-DIGEST`) that cannot be mistaken for a real one.
       ENFORCED-HOW: grep — K1's own check fails if a bare `image:` line lacks
       either `@sha256:[0-9a-f]{64}` or the exact placeholder token.

INV-3  THE DEPLOYED SYSTEM, AND THE IaC REPO THAT DESCRIBES IT, ARE READ-ONLY TO
       AGENTS. Permitted: `kubectl get|describe`, `kubectl exec … curl|cat|ls`,
       `git show`, `git -C ~/dockers/kubernetes log|show|diff` (read-only).
       Forbidden anywhere in any item, script, or evidence run: `kubectl
       apply|edit|patch|scale|delete|rollout|cp`, `docker push`, `buildx …
       --push`, `oras push`, `skopeo copy`, `git push`, and any `git commit`,
       `git add`, or working-tree edit inside `~/dockers/kubernetes` at all.
       ENFORCED-HOW: grep — `scripts/check_no_mutation.sh` scans tracked
       scripts and `.agents/issue-12/evidence/` for those verbs plus a check
       that no item's diff touches any path under `~/dockers/kubernetes`.

INV-4  NO STATUS-CODE-ONLY ASSERTIONS. Every new or changed route-parity row
       (N1's `/live/hls/`, N2's retired routes) and every new restream-server/
       media-bridge health check asserts content type AND a body-shape
       predicate, never a bare status code.
       ENFORCED-HOW: test — `scripts/route_parity.sh` already refuses to run a
       row missing either field; N1/N2 add rows under the same schema.

INV-5  PER-CAMERA-PER-ROLE TASK ISOLATION (DP-3, extended by DP-4). Inside the
       media-bridge process, one camera's RTSP-restream-feeding task, its
       MoQ-publish task, its fMP4/WebSocket-repackaging task (G2), and its
       HLS-packaging task (G3) are all independent of each other and of every
       other camera's tasks. One role's failure for one camera never
       terminates the process, any other role for that same camera, or any
       task belonging to another camera.
       ENFORCED-HOW: two checks, matching issue #18's own INV-2 pattern.
       (a) A panic in one task never terminates the process or any unrelated
       task — holds unconditionally under Tokio's per-task panic isolation
       regardless of supervision, verified directly by running multiple
       cameras/roles (now four roles per camera, not two) and confirming each
       keeps producing independently.
       (b) A panicking task's own role actually recovers rather than staying
       permanently dead — needs restart-on-panic supervision; G1's, G2's,
       G3's, and X3's own mutation cycles (breaking that supervision) falsify
       (b) specifically. Word this mutation precisely: it targets
       restart-on-panic recovery for the SAME failed unit, not cross-unit
       isolation, which Tokio already guarantees regardless — see issue #18's
       C1 evidence (`8d47fe6`) for the exact wording error this phrasing
       avoids repeating.

INV-6  THE EXTERNAL QUIC/WEBTRANSPORT LISTENER TERMINATES REAL TLS, NEVER A
       SELF-SIGNED OR EPHEMERAL CERT, IN THE DEPLOYED CONFIGURATION. K1's
       drafted manifest mounts the existing cert-manager Secret into the relay
       container; P1's image never launches the relay with a self-signed-cert
       flag as its deployed command.
       ENFORCED-HOW: grep — K1's manifest check asserts the drafted Secret
       mount name matches the existing cert Secret; P1's build check denies
       any `--self-signed`/`--generate-cert`-shaped flag in the image's baked
       -in deployed `command:` args.

INV-7  THE EMBEDDED GO2RTC WEBRTC PLAYER PAGE IS GONE FROM THE UI. No
       `<iframe … src="/live/webrtc/webrtc.html…"` (or any literal
       `/live/webrtc/webrtc.html`) remains under `crates/corvette-ui/src` once
       U1 lands. True under every OPEN-1 answer, so enforced regardless of
       which is chosen.
       ENFORCED-HOW: grep — extend `scripts/check_no_go2rtc.sh` with the
       denied literal `/live/webrtc/webrtc.html`.

INV-8  NO CONTAINER FORMAT ANYWHERE IN THE RESTREAM-SERVER OR MOQ-BRIDGE
       PIPELINE (D-9). No MPEG-TS, no fMP4, appears anywhere in
       crates/rtsp-restream or crates/corvette-media-bridge's own MoQ-publish
       path. G2 (fMP4-over-WebSocket) and G3 (HLS/LL-HLS) are explicitly
       exempted from this invariant within their own modules only — they are
       a deliberately new, named container boundary the human authorized via
       OPEN-1/OPEN-2, not an accidental reintroduction of one into the
       MoQ-publish or restream-server path itself.
       ENFORCED-HOW: grep — a check (created in X1/G1) denying `mpeg2ts`,
       `mp4box`, `moq_mux::container` as literals in crates/rtsp-restream and
       in corvette-media-bridge's MoQ-publish module specifically (not in
       G2/G3's own modules, which are the named exemption).
```

Every review ends with the invariant sweep across all eight, including for items that
"obviously can't" violate them.

---

## Agents in flight — ceiling (§3, §5)

**Ceiling: 2. Default: 1.**

Every corvette-repo item (R1, X1, X2, X3, G1, G2, G3, P1, K1, U1, U2, W1) shares one
`Cargo.lock`, one `target/` directory, and (for U1/U2) the nginx-parity harness's ports.
Two corvette items never run concurrently, even in separate worktrees, for the reason
issue #2's and the prior issue #12 draft's plans both gave: a shared build cache/harness
collision is not solved by worktree isolation alone. X1→X2→X3→G1 are additionally serial
by dependency in practice.

**K1 counts as a corvette-repo item** for this rule (INV-3 confines its writes to
`.agents/issue-12/evidence/` inside this repo).

The only sanctioned second slot is **one `frigate-vulkan` item (F1, N1, or N2) alongside
one corvette item**. F1, N1, and N2 do not run concurrently with each other — F1 touches
the Dockerfile/s6-overlay layer, N1/N2 touch the vendored `nginx.conf`; different files,
but the same donor-drift-guard mechanism and the same repository's own build state, so
this plan keeps the conservative one-at-a-time rule the prior draft used for N1/N2 and
extends it to F1.

Exceeding 2, or running two corvette items in parallel, is a human decision.

---

## Risk profile (§0)

| Item | Profile | Trigger that sets it |
| --- | --- | --- |
| R1 | Significant | External dependency contract grounding; nothing deployed or concurrent yet. |
| X1 | Significant | New protocol-parsing code accepting external byte streams (RTSP messages) is bitstream-correctness risk, but no concurrency or credentials of its own — mirrors issue #18's D1, not its S1. |
| X2 | Significant | Bitstream-correctness risk (wrong RTP packetization silently corrupts what Frigate records) — no concurrency of its own. |
| X3 | Critical | Concurrency (per-connection async tasks) accepting a real network listener — a named Critical trigger, same class as issue #18's C1. |
| G1 | Critical | Concurrency (per-camera-per-role tasks, DP-3) PLUS this is the actual process Frigate's own recording pipeline now depends on for correctness — raised relative to the prior draft's G1 for exactly the reason DT-1's amendment states. |
| G2 | Significant | Further per-camera-per-role task (DP-4) in the same process as G1, but this role only feeds the UI's grid tile — not Frigate's recording path — so it does not inherit G1's Critical rating; bitstream-correctness risk only (wrong fMP4 framing breaks the tile, not recorded footage). |
| G3 | Significant | Same reasoning as G2: a further per-camera-per-role task (DP-4) serving only the UI's fallback path, not Frigate's recording path; HLS segmentation/manifest logic is more involved than G2's but carries the same risk ceiling. |
| P1 | Critical | Produces the exact artifact a human is later asked to push and a pod is asked to run. |
| F1 | Critical | Deployment/build-layer change to an image Frigate's own detect/record startup depends on; a mistake here breaks recording, not just live view — new, and rated at least as seriously as any other item in this plan. |
| K1 | Critical | New externally-reachable network listener (NodePort/UDP) AND moves the port Frigate's own ffmpeg depends on from one container to another — higher blast radius than the prior draft's K1. |
| N1 | Critical | Deployment behavior — nginx config that ships to the pod; now serves G3's HLS packager, per OPEN-2's resolution. |
| N2 | Critical | Same deployment surface as N1; unconditionally removes five routes (not two) that go2rtc's absence makes permanently dead. |
| U1 | Significant | UI behavior against an established contract — G2's WebSocket/fMP4 contract, per OPEN-1's resolution (see U1's own Premise). |
| U2 | Significant | New UI surface, new third-party JS dependency, dual-protocol fallback logic; MoQ-first path unaffected by either OPEN question, fallback step now wired to G3, per OPEN-2's resolution. |
| W1 | Routine | Documentation only, no behavior change, mechanically checkable. |
| V1 | Critical | Cutover evidence against the running deployment; now also verifies Frigate's own recording continues, not only live-view UI routes. |
| V2 | orchestrator | Issue edits are orchestrator prose actions, not a risk-profiled implementation item. |

---

## Cost assignment (§6) — visible at the plan gate

Per `AGENTS.md`'s model policy: **Sonnet is the floor for both implement and review;
effort is chosen per item from its presumed difficulty, not a fixed value.** No item
claims an Opus exception.

| Item | Repo | Model | Effort | Review model | Pipeline | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| R1 | corvette | sonnet | medium-high | sonnet | full | Unfamiliar crate surface; now also verifies the codec-level `Split`/`Import` API (not just `moq-relay`/`hang`), since G1 depends on it directly, per D-9. |
| X1 | corvette | sonnet | high | sonnet | full | Protocol-correctness core; mirrors issue #18 D1's own assignment reasoning (bitstream correctness, no concurrency of its own). |
| X2 | corvette | sonnet | high | sonnet | full | RTP packetization edge cases (fragmentation thresholds, AU-header framing) are exactly where an under-thought implementation silently produces frames Frigate's ffmpeg can't parse without ever erroring loudly. |
| X3 | corvette | sonnet | high | sonnet | full | Concurrency + a real network listener is the named Critical risk; high effort to think through per-connection failure modes. |
| G1 | corvette | sonnet | high | sonnet | full | Integration point for every D-n decision in this plan; raised-stakes item per DT-1's amendment (a bug here can break Frigate's own recording, not just live view). |
| G2 | corvette | sonnet | high | sonnet | full | Structurally comparable to X2's own packetizer and G1's MoQ-publish logic (frame stream in, different wire format out); high effort for fMP4 box-framing correctness, though the risk ceiling is lower than G1's (UI-only, not recording-path). |
| G3 | corvette | sonnet | high | sonnet | full | HLS/LL-HLS segmentation and manifest generation is more involved than G2's fMP4 framing (playlist rotation, segment boundaries); high effort for the same reason, same UI-only risk ceiling as G2. |
| P1 | corvette | sonnet | high | sonnet | full | Review carries a named re-verification duty: independently confirm no push occurred and both binaries are present and runnable. |
| F1 | frigate-vulkan | sonnet | high | sonnet | full | Modifies a startup-dependency graph Frigate's own process supervision relies on; high effort for the "does Frigate still start cleanly with go2rtc absent" question, which no prior research fully answered. |
| K1 | corvette | sonnet | high | sonnet | full | Review carries a named re-verification duty: confirm INV-3 directly, and confirm the RTSP port's container reassignment does not silently duplicate or drop a port declaration. |
| N1 | frigate-vulkan | sonnet | high | sonnet | full | Premise now includes OPEN-2's own gate; effort covers both the mechanical nginx work and the STOP-AND-ASK judgment of whether OPEN-2 has been answered before proceeding on the HLS-specific content. |
| N2 | frigate-vulkan | sonnet | medium | sonnet | full | Now unconditional for four named routes (broader than the prior draft's two), but each removal is mechanically simple once go2rtc's absence is confirmed. |
| U1 | corvette | sonnet | medium-high | sonnet | full | OPEN-1 resolved to G2's WebSocket/fMP4 contract — a known, bounded integration surface (swap an iframe for a WebSocket-backed MSE player), not the open-ended shape the earlier placeholder held space for. |
| U2 | corvette | sonnet | high | sonnet | full | Genuinely novel dual-protocol client logic; high effort for the fallback-timeout design. |
| W1 | both | sonnet | medium-high | sonnet | **batched** | Docs-only; effort raised over the prior draft's given the number of decisions (D-6..D-9, both OPEN questions, the port-move finding) that must be recorded accurately. |
| V1 | corvette | sonnet | high | sonnet | full | Re-executes the parity diff, the image-ID match, AND a recording-continuity check itself rather than trusting any item's own log. |
| V2 | — | orchestrator | — | — | orchestrator | Issue edits and roadmap updates are orchestrator prose actions. |

**Batched vs. full pipeline.** Exactly one batch: **W1**. Everything else gets the full
per-item implement→review pipeline.

Every agent returns the §7 cost line: `model / effort / tool-calls / approx-tokens`. The
orchestrator records the harness's numbers, not the agent's self-count.

---

## Items

Each item carries exactly five fields: **Premise / Do / Verify / Mutation / Scope
guard**. Evidence files land in `.agents/issue-12/evidence/` (untracked) with the commit
hash embedded in each file.

### Phase R — pin the dependency, verify it builds

```
R1  Pin moq-dev/moq's commit; verify moq-relay AND the codec-level Split/Import API
    build standalone
    REPO: corvette          MODEL: sonnet   EFFORT: medium-high   PIPELINE: full

Premise
  moq-dev/moq pushes daily. VERIFY THE PREMISE FIRST: fetch the actual current HEAD
  (`gh api repos/moq-dev/moq/commits/main`) and decide, recording the reasoning,
  whether to pin the exact commit `RESEARCH-d4-ingest-source.md` verified
  (`7b73c43381a7f9c309e3045a8f0f858aa32ef48c`, 2026-08-16 — the newest commit any
  research pass in this issue actually read code at) or a newer one. D-9's own
  recommendation (no MPEG-TS anywhere) depends on `moq_mux::codec::h264::{Split,
  Import}`, `codec::h265::{Split, Import}`, and `codec::aac::Import` existing and
  building standalone — this was confirmed by reading source
  (`RESEARCH-d4-ingest-source.md` F-1/F-2), not by an independent executable
  build; that gap is what this item closes, alongside the prior draft's own
  `moq-relay`-standalone-build verification.

Do
  1. Pin the chosen commit as a direct git dependency for `moq-net`, `moq-mux`,
     `hang` in the workspace `Cargo.toml`.
  2. In an isolated scratch checkout (a temp directory, not this workspace), clone
     the pinned commit and run `cargo build --release --bin moq-relay` with no
     config beyond command-line flags; confirm it starts and accepts a QUIC
     connection from a second local invocation with no Redis, no `moq-api`.
  3. In the same scratch checkout, write and run a small throwaway program calling
     `moq_mux::codec::h264::Split::new()` / `h264::Import::new(...)` /
     `codec::aac::Import::new(...)` directly against a synthetic Annex-B NAL
     sequence and a synthetic raw AAC access unit (no MPEG-TS involved) — confirm
     both build and produce a non-error result, closing the specific gap named in
     the Premise. This does not need to be exhaustive; it only needs to prove the
     API surface `RESEARCH-d4-ingest-source.md` F-1/F-2 read from source actually
     compiles and runs at the commit this item pins.
  4. Confirm `js/hang`'s Web Component package name/version for U2 to vendor later.
  5. Record all of the above in `.agents/issue-12/evidence/R1-dependency-pin.md`.

Verify (executed)
  - `cargo build --release --bin moq-relay` (scratch checkout) exits 0, no Redis,
    no `moq-api`; the built binary accepts a real QUIC connection.
  - The step-3 throwaway program compiles and runs to completion with no panic.
  - `cargo build --workspace` in THIS repo, with the new git dependency added,
    succeeds under the workspace's own `[workspace.lints]`.

Mutation
  N/A — Significant item, oracle is self-evident (builds/connects or does not).

Scope guard
  Do NOT write any Corvette-specific code against these crates yet (X1–X3, G1). Do
  NOT touch any Dockerfile (P1). Do NOT add a network-facing service anywhere. The
  scratch checkout and throwaway program are disposable and not committed.
```

### Phase X — the RTSP-restream server (D-6/D-7), a standalone reusable crate

```
X1  crates/rtsp-restream: RTSP message/session state machine + SDP generation
    REPO: corvette          MODEL: sonnet   EFFORT: high     PIPELINE: full

Premise
  D-6/D-7 quoted above, independently fact-reviewed (DESIGN-review-report-D6-D7.md,
  PASS) — the narrowed protocol surface (OPTIONS/DESCRIBE/SETUP/PLAY/TEARDOWN, one
  transport mode, no auth) was verified byte-for-byte against go2rtc's own
  `pkg/rtsp/server.go` at the pinned commit in that review; this item does not need
  to re-verify it, only implement to it. `rtsp-types`/`sdp-types` are the two
  dependencies D-7 commits to (no session-engine dependency) — confirm their
  currently-published versions and public API shape directly before writing
  code (crates.io + a direct read of each crate's own `lib.rs`/`README.md`), since
  the research pinned specific commits that may have moved since.
  RESEARCH-rtsp-restream-server.md's "Reusable-crate framing" section is the shape
  of this item's `StreamProvider`-boundary Do step — re-read it before writing the
  trait; it establishes nothing about Corvette, MoQ, or Frigate needs to appear in
  this crate for the split to work.

Do
  1. New library crate `crates/rtsp-restream`, workspace member,
     `license = "MIT OR Apache-2.0"` (matching `rtsp-types`/`sdp-types`/
     `rtsp-runtime`'s own ecosystem convention, chosen here — a plan-level call,
     not a D-n text — specifically because D-6 requires this crate to be usable
     outside Corvette, and dual MIT/Apache-2.0 is the more interoperable choice
     for a crate meant for external reuse than Corvette's own bare MIT).
  2. Depend on `rtsp-types`/`sdp-types` only.
  3. Implement the server-side RTSP session state machine: `OPTIONS` (advertise
     exactly OPTIONS/DESCRIBE/SETUP/PLAY/TEARDOWN — do not replicate go2rtc's own
     advertise-but-not-implement PAUSE gap; advertise only what this crate
     actually implements), `DESCRIBE` (build an SDP body from a `StreamInfo` the
     trait boundary in step 5 supplies), `SETUP` (allocate a session id; accept
     only `Transport: RTP/AVP/TCP;interleaved=N-M`, reject anything else with
     `461 Unsupported Transport`, matching go2rtc's own behavior exactly), `PLAY`,
     `TEARDOWN`. No Digest/Basic auth (D-7/F-12: no established consumer needs
     it).
  4. SDP generation (`RESEARCH-rtsp-restream-server.md` F-13): fmtp templates for
     H.264 (`packetization-mode=1;profile-level-id=...;sprop-parameter-sets=...`),
     H.265 (`sprop-vps/sps/pps`), AAC (`mpeg4-generic`,
     `config=<AudioSpecificConfig hex>`) — built from parameter-set bytes the
     `StreamInfo` carries.
  5. Define the crate's public, Corvette-agnostic boundary: a `StreamProvider`
     trait (`describe(&self, name: &str) -> Option<StreamInfo>`,
     `subscribe(&self, name: &str) -> Option<FrameReceiver>`) and a `Frame`/
     `FrameReceiver` shape carrying Annex-B NAL units / raw AAC access units plus
     a timestamp — defined independently in this crate, with **no** dependency on
     `corvette-rtsp-client` at all (the reusability requirement: this crate must
     not know Corvette's own client crate exists). Read `bairelay-rtsp`'s own
     `StreamProvider` trait shape (`RESEARCH-rtsp-restream-server.md` F-5's
     citation) as design inspiration only — do not copy its code (INV-1, AGPL).
  6. Add the license-manifest check named in INV-1.

Verify (executed)
  - Unit tests for OPTIONS/DESCRIBE/SETUP/PLAY/TEARDOWN against a fake in-memory
    `StreamProvider`.
  - SDP fmtp golden-output tests per codec (H.264/H.265/AAC), checked against the
    exact template shapes `RESEARCH-rtsp-restream-server.md` F-13 cites.
  - Rejected-transport test: a `SETUP` with a UDP `Transport:` header receives
    `461 Unsupported Transport`, not accepted.
  - Unknown-method test: falls to a clean, documented error response, not a panic.
  - `cargo clippy --workspace --all-targets --all-features --locked`, `cargo test
    --workspace`, the new license-manifest check.

Mutation (Significant, oracle not self-evident → required)
  Remove the TCP-only transport gate (accept any `Transport:` value) → the
  rejected-transport test must FAIL (a UDP request is wrongly accepted) → restore
  → PASS. Paste both runs into `.agents/issue-12/evidence/X1-mutation.log`.

Scope guard
  Do NOT implement any TcpListener/socket I/O here (X3). Do NOT implement RTP
  packetization (X2). Do NOT reference `corvette-rtsp-client`, `moq_mux`,
  `moq_net`, or `frigate` anywhere in this crate's source — a grep check (part of
  the license-manifest check) denies these literals. Do NOT copy code from
  `bairelay-rtsp` or `lvqr-rtsp` beyond reading their public API shape as a design
  reference (INV-1).
```

```
X2  crates/rtsp-restream: RTP packetizer (Annex-B/raw-AAC frames → RTP payloads)
    REPO: corvette          MODEL: sonnet   EFFORT: high     PIPELINE: full

Premise
  X1 CLOSED. This item is the mirror image of issue #18's own D1 item: where D1
  turned camera RTP into Annex-B/raw-AAC frames, this item turns Annex-B/raw-AAC
  frames (the same shape, confirmed against issue #18's shipped
  `crates/corvette-rtsp-client/src/depacketize` output type) back into RTP
  payloads for delivery to Frigate's ffmpeg over X3's interleaved channel. Re-read
  D-9's own text and issue #18's D1 evidence for the exact input shape before
  writing code.

Do
  1. H.264 (RFC 6184) packetizer: split each Annex-B access unit into NAL units;
     emit single-NAL RTP packets for small NALs, FU-A fragmentation for large
     ones. Choose and justify a fragmentation-size threshold (there is no real MTU
     limit over TCP-interleaved, but a conservative fixed size keeps packet
     handling uniform with what a UDP-capable client would also tolerate) —
     record the chosen value and why in this item's evidence file.
  2. H.265 (RFC 7798) packetizer, same shape.
  3. AAC (RFC 3640, `mpeg4-generic`) packetizer: wrap each raw access unit with
     the AU-header framing the format requires.
  4. RTP header construction per track: sequence number, timestamp (the frame's
     own PTS scaled to the codec's RTP clock rate — 90kHz video, the codec's
     sample rate for audio), SSRC.

Verify (executed)
  - Round-trip fixtures: feed this item's packetizer output back through issue
    #18's own shipped `H264Depacketizer`/`H265Depacketizer` (an independent,
    already-reviewed correctness oracle, not a new one written for this item) and
    assert the round-tripped NAL sequence matches the original Annex-B input
    byte-for-byte, for single-NAL, FU-A-fragmented, and STAP-A-eligible cases.
  - AAC round-trip: packetize then depacketize (a small depacketizer written for
    this test only, since issue #18's own AAC depacketizer exists but is not
    wired into any shipped consumer — confirm this before assuming it can be
    reused as-is) and assert byte-for-byte equality.
  - `cargo clippy --workspace --all-targets --all-features --locked`, `cargo test
    --workspace`.

Mutation (Significant, oracle not self-evident → required)
  Corrupt one fragmentation boundary (e.g. drop the FU-A end-bit on the last
  fragment) → the round-trip fixture for that case must FAIL with a visibly wrong
  reassembled NAL → restore → PASS. Paste both runs into
  `.agents/issue-12/evidence/X2-mutation.log`.

Scope guard
  Do NOT implement session/protocol state (X1). Do NOT implement the TcpListener/
  fan-out layer (X3). Do NOT add any MoQ-related code (G1). This item is pure,
  synchronous transform code: frames in, RTP payloads out.
```

```
X3  crates/rtsp-restream: async I/O layer, per-connection sessions, StreamProvider
    hosting
    REPO: corvette          MODEL: sonnet   EFFORT: high     PIPELINE: full

Premise
  X1, X2 CLOSED. This is the async runtime layer occupying the same role go2rtc's
  own `pkg/rtsp/server.go` + `conn.go` play (`RESEARCH-rtsp-restream-server.md`
  F-11): a `TcpListener::accept()` loop, one task per inbound RTSP connection,
  driving X1's session state machine over the real socket and X2's packetizer for
  the negotiated stream's `StreamProvider`-supplied frames.

Do
  1. `TcpListener` accept loop; bind address/port is a constructor parameter, not
     hardcoded to 8554 (matching D-3's "configuration point" convention already
     established for the MoQ relay's own exposure).
  2. Per-connection task: read/parse RTSP requests via `rtsp-types`, drive X1's
     state machine, write responses.
  3. On `PLAY`: look up the requested stream name via the caller-supplied
     `StreamProvider`, subscribe to its frames, and run a dedicated write task
     streaming X2-packetized RTP over the negotiated interleaved channel until
     `TEARDOWN` or connection close.
  4. Multiple concurrent sessions against the same stream name are independent —
     this falls out of `StreamProvider::subscribe` being expected to itself be a
     broadcast-shaped fan-out on the caller's side (G1's own job); X3 does not
     implement fan-out itself, it only calls `subscribe` once per inbound session.
  5. Panic/error isolation per inbound connection (INV-5): one session's failure
     never kills the listener or another session.

Verify (executed)
  - Integration test: a fake `StreamProvider` plus a real loopback `TcpListener`.
    Two independent, hand-rolled RTSP clients (built directly against
    `rtsp-types`, not against `corvette-rtsp-client` — using a CLIENT crate to
    test a SERVER would risk a self-referential oracle, the same risk issue #18's
    own M1 item flagged and avoided) both `PLAY` the same stream and both receive
    the same frames independently.
  - Panic-isolation test: one session's handler panics; assert the *other*
    session is unaffected and the listener keeps accepting new connections.
  - `cargo clippy --workspace --all-targets --all-features --locked`, `cargo test
    --workspace`.

Mutation (Critical → required)
  This targets restart-on-panic recovery for the SAME failed session, not
  cross-session isolation — cross-session isolation holds unconditionally
  under Tokio's per-task panic model regardless of supervision, so no
  mutation can meaningfully falsify it, and no evidence here should claim
  otherwise (see issue #18's C1 evidence, `8d47fe6`, for the exact wording
  error this avoids repeating — an earlier draft of this very item's own
  text made that mistake and was corrected before implementation).
  Break this session's restart-on-panic supervision (bare, unsupervised
  spawn for its per-connection task) → inject a panic in that session's
  frame-delivery task → assert only THAT session's output stops (and does
  not resume on its own), while a second, concurrent session against the
  same or a different stream continues completely unaffected throughout →
  restore supervision → assert the panicking session's task restarts and
  its output resumes. If X3's own Do section 3 implementation does not
  give a per-connection task anything meaningful to restart into (e.g. a
  panic there is architecturally equivalent to the client disconnecting,
  with no supervised-restart concept at all), STOP AND ASK rather than
  writing a mutation whose oracle you had to invent — this is a real
  possibility this item's own Do section leaves open (Do step 5 only
  requires isolation, not restart), and the plan does not resolve it for
  you. Paste both runs into `.agents/issue-12/evidence/X3-mutation.log`.

  RESOLVED (orchestrator, 2026-08-26): N/A, no restart-on-panic mutation
  applies. X3's implementer stopped and asked rather than inventing one
  (`.agents/issue-12/evidence/X3-mutation.log`, full reasoning). Confirmed
  correct on independent review: the play task (`server::spawn_play_task`)
  is scoped 1:1 to one inbound client's own TCP socket, not a process-owned
  resource with independent subscribers the way a camera's client task is
  — its `JoinHandle` is deliberately discarded (only its `AbortHandle` is
  kept), so nothing even observes its panic to restart from. This matches
  the architectural precedent this item fills (go2rtc's own
  `pkg/rtsp/server.go`/`conn.go`, `RESEARCH-rtsp-restream-server.md` F-11:
  any session-ending error just ends the connection, no resubscribe-and-
  resume path) and Do step 3's own text ("until TEARDOWN or connection
  close" — not "...or a recoverable internal failure"). Building a same-
  socket resubscribe-and-resume mechanism instead (the only design that
  would give this mutation a real target) would be new, unscoped work
  raising its own unanswered RTP sequence/timestamp-continuity question —
  a future item to propose explicitly if ever wanted, not something X3's
  own Do section asked for. INV-5(a)'s isolation requirement (the part
  that does apply here) was verified directly, not by mutation, per
  INV-5's own text — see the panic-isolation evidence at the end of the
  same log file.

Scope guard
  Do NOT implement protocol logic (X1) or packetization math (X2) here. Do NOT
  implement a Corvette-specific `StreamProvider` (that's G1) — X3 only defines
  and consumes the trait generically, with a fake implementation for its own
  tests.
```

### Phase G — the media-bridge process (D-8/D-9)

```
G1  crates/corvette-media-bridge: wires corvette-rtsp-client to rtsp-restream and
    to a direct MoQ-publish loop, one process, one task per camera per role
    REPO: corvette          MODEL: sonnet   EFFORT: high     PIPELINE: full

Premise
  X1/X2/X3 CLOSED (crates/rtsp-restream complete). `crates/corvette-rtsp-client`
  (issue #18, DONE, all items closed) already ships exactly this public API,
  confirmed by direct source read: `CameraConfig::new(name, host: IpAddr, port,
  path, credentials)` with a `.channel_capacity(usize)` builder override,
  `Client::new(config)` spawns a supervised per-camera task, `Client::subscribe()`
  returns an independent `broadcast::Receiver<Frame>` where `Frame { codec,
  timestamp: u32, payload: bytes::Bytes }` carries Annex-B H.264/H.265 or raw
  non-ADTS AAC. Default broadcast capacity is 64 frames per camera, overridable.
  **A real, currently-shipped gap, not this item's to fix:** `corvette-rtsp-
  client`'s own SDP resolution only ever resolves an `m=video` section
  (`crates/corvette-rtsp-client/src/client/task.rs`'s `build_depacketizer` only
  ever constructs an H.264 or H.265 depacketizer; no audio track is ever
  resolved or depacketized) — no AAC `Frame` is ever actually published by this
  crate today, even though the AAC codec-level API exists on both the client
  side (unit-tested, unused) and this item's own MoQ-publish side. G1's own
  MoQ-publish dispatch should still be written generically over `Frame::codec`
  (so a future audio-SDP-resolution fix in `corvette-rtsp-client` needs no G1
  change), but this item's evidence must record honestly that only video is
  exercised end to end today — do not claim or test audio support that cannot
  currently be exercised.

Do
  1. New binary crate `crates/corvette-media-bridge`, workspace member. This
     supersedes the `crates/corvette-ingest-bridge` name the pre-D-6..D-9 draft
     of this plan used — that crate was never created, so there is nothing to
     migrate.
  2. Config: a list of per-camera fields sufficient to construct a
     `CameraConfig` — read from an env var or a small file, matching issue #18's
     own C1 precedent (a named stub, not a TODO without an owner; Corvette does
     not yet own configuration per `docs/design/architecture.md`'s "Risk
     boundary" section).
  3. For each configured camera: construct one `corvette_rtsp_client::Client`.
     Take two independent subscriptions from it: (a) one adapts
     `corvette_rtsp_client::Frame` into `rtsp_restream`'s own `Frame`/
     `FrameReceiver` shape and implements `StreamProvider` for that camera's
     name — a thin, mechanical adapter with no protocol logic of its own; (b)
     the other drives the MoQ-publish loop directly via
     `moq_mux::codec::h264::{Split, Import}` / `h265::{Split, Import}` /
     `aac::Import` per D-9 — this is the actual former-G1 logic from the
     superseded draft, now free of any TS/HTTP concern, dialing the relay via
     `moq_net` directly (D-5's "small in-repo binary against moq-mux/moq-net
     directly" — unaffected by D-9, only the frame source changed).
  4. Run one `rtsp_restream::X3`-hosted server for the whole process, backed by
     a `StreamProvider` implementation that dispatches by stream name across all
     configured cameras' individual per-camera adapters from step 3(a).
  5. Panic/task isolation (INV-5, DP-3): each camera's MoQ-publish task and its
     StreamProvider-feeding subscription task are independent of each other and
     of every other camera's tasks, using the same supervised-`JoinHandle`
     pattern `corvette-rtsp-client`'s own C1 item already established as this
     project's convention.
  6. Basic structured log lines per camera per role on connect/disconnect/
     reconnect/panic, matching `corvette-rtsp-client`'s own `eprintln!`-based
     convention (no metrics/logging-framework dependency exists in this
     workspace; do not add one here, per that same precedent).

Verify (executed)
  - Integration test: a real, locally-run `moq-relay` (R1) plus issue #18's own
    mock camera server (`corvette-rtsp-client`'s `mock_camera` module — reused
    directly, per playbook §2 rail 4, rather than writing a second one) standing
    in for one camera. Assert (a) a `moq_net` subscriber sees the camera's video
    catalog and receives frames; (b) a hand-rolled RTSP client (matching X3's
    own test convention) can `DESCRIBE`/`SETUP`/`PLAY` the same camera name
    against this item's own hosted listener and receives the same underlying
    video; (c) killing and letting the supervisor restart one camera's
    `Client`/publish tasks does not affect a second, independent camera's
    outputs on either path.
  - `cargo clippy --workspace --all-targets --all-features --locked`, `cargo test
    --workspace`.

Mutation (Critical → required)
  Break one camera's restart-on-panic supervision (bare, unsupervised spawn for
  either its StreamProvider-feeding task or its MoQ-publish task) → inject a
  panic in that task → assert only that camera's affected output (RTSP-restream
  availability or MoQ publish, matching which task was mutated) stops and never
  resumes → restore → assert it resumes after being restarted, with the other
  camera and the other role for the SAME camera unaffected throughout (verified
  directly, not by this mutation, per INV-5(a)'s own distinction). Paste both
  runs into `.agents/issue-12/evidence/G1-mutation.log`.

Scope guard
  Do NOT modify `crates/rtsp-restream` or `crates/corvette-rtsp-client` — both
  adopted as-is. Do NOT add camera discovery from Frigate's `config.yml`. Do NOT
  add audio-track SDP resolution to `corvette-rtsp-client` — a real, named gap
  (see Premise), but a separate future item if the human wants it fixed, not
  this item's job. Do NOT add any Dockerfile/manifest changes (P1/K1).
```

```
G2  crates/corvette-media-bridge: fMP4-over-WebSocket repackager for the grid
    tile (OPEN-1, resolved; DP-4)
    REPO: corvette          MODEL: sonnet   EFFORT: high     PIPELINE: full

Premise
  G1 CLOSED. OPEN-1 is resolved (2026-08-26, see "How this redraft resolves..."
  above): the grid tile connects to a new, small, dedicated fMP4-over-WebSocket
  output, landing inside this same process per DP-4, not as a separate
  component. This item subscribes to the same D-8 broadcast channel G1's own
  MoQ-publish loop already reads from — per camera, a second independent
  subscription, not a fan-out from G1's own task. Structurally comparable to
  X2's RTP packetizer and G1's own MoQ-publish logic: frame stream in
  (Annex-B H.264/H.265 NAL units), different wire format out. No transcoding —
  the existing NAL units are repackaged, not re-encoded.

Do
  1. For each configured camera, take an independent subscription to its
     `corvette_rtsp_client::Frame` broadcast (matching G1's own per-camera
     subscription pattern).
  2. Build a minimal fMP4 muxer: one initialization segment per camera
     (`ftyp`/`moov`, built once from the stream's SPS/PPS/VPS parameter sets,
     re-emitted whenever a parameter-set change is detected) followed by a
     continuous stream of fragmented `moof`/`mdat` boxes, one per access unit
     or small group of access units — video only (G1's own Premise already
     names the audio-track gap; this item inherits it, does not fix it).
  3. Host a WebSocket listener (bind address/port a constructor parameter,
     matching D-3's "configuration point" convention) that, per camera name,
     sends the current initialization segment first on connect, then streams
     `moof`/`mdat` boxes as they are produced — one WebSocket connection per
     viewer, matching go2rtc's own existing MSE-tile transport shape (the
     human's own stated rationale for choosing this option).
  4. Panic/task isolation (INV-5, DP-4): each camera's fMP4-repackaging task is
     independent of its own MoQ-publish task, its own RTSP-restream-feeding
     task, and every other camera's tasks, using the same supervised-
     `JoinHandle` pattern as G1.

Verify (executed)
  - Unit tests: a synthetic Annex-B sequence (including a parameter-set
    change) produces a valid initialization segment and a sequence of
    `moof`/`mdat` boxes that a real MSE `SourceBuffer` accepts without error —
    checked via a headless browser harness (matching U2's own Playwright
    convention) appending this item's own output directly, not only a
    box-structure assertion.
  - Integration test: issue #18's own mock camera server (`corvette-rtsp-
    client`'s `mock_camera` module, reused per playbook §2 rail 4) feeds a
    camera; a WebSocket test client connects, receives the initialization
    segment then a bounded number of `moof`/`mdat` boxes, and a headless
    browser's `<video>`/`SourceBuffer` reaches `HAVE_CURRENT_DATA`.
  - `cargo clippy --workspace --all-targets --all-features --locked`, `cargo
    test --workspace`.

Mutation (Significant, oracle not self-evident → required)
  Break this camera's restart-on-panic supervision (bare, unsupervised spawn
  for the fMP4-repackaging task) → inject a panic in that task → assert only
  this camera's WebSocket output stops and never resumes, while the same
  camera's MoQ-publish output (G1) and the other camera's every output
  continue unaffected → restore → assert this camera's WebSocket output
  resumes after being restarted. Paste both runs into
  `.agents/issue-12/evidence/G2-mutation.log`.

Scope guard
  Do NOT modify `crates/rtsp-restream`, `crates/corvette-rtsp-client`, or G1's
  own MoQ-publish code. Do NOT add audio-track support (see Premise — a
  separate future item). Do NOT add HLS output here (G3). Do NOT add any
  Dockerfile/manifest changes (P1/K1).
```

```
G3  crates/corvette-media-bridge: HLS/LL-HLS packager for the expanded-view
    fallback (OPEN-2, resolved; DP-4)
    REPO: corvette          MODEL: sonnet   EFFORT: high     PIPELINE: full

Premise
  G1 CLOSED. OPEN-2 is resolved (2026-08-26, see "How this redraft resolves..."
  above): a new, small, dedicated HLS/LL-HLS packager, landing inside this
  same process per DP-4, serves N1's `/live/hls/` location and U2's fallback
  step. Like G2, this item subscribes independently to the same D-8 broadcast
  channel — a third independent per-camera subscription, not a fan-out from
  G1's or G2's own tasks. Video only (same inherited audio gap named in G1's
  Premise).

Do
  1. For each configured camera, take an independent subscription to its
     `corvette_rtsp_client::Frame` broadcast.
  2. Build a minimal fMP4-segment-based LL-HLS packager: fragmented CMAF
     segments (reusing the same fMP4-fragmentation logic as G2 where the
     shape is identical — a shared internal helper is fine; a shared public
     module boundary between G2 and G3 is this item's own call to make and
     justify) plus an `m3u8` media playlist that rolls segments per a chosen,
     justified target duration (e.g. 2s, short enough to bound live latency
     without producing excessive segment-request overhead).
  3. Host an HTTP listener (bind address/port a constructor parameter, per
     D-3) serving the media playlist and segments at the path convention N1
     will proxy to.
  4. Panic/task isolation (INV-5, DP-4): each camera's HLS-packaging task is
     independent of its own MoQ-publish task, its own fMP4-repackaging task
     (G2), its own RTSP-restream-feeding task, and every other camera's
     tasks, using the same supervised-`JoinHandle` pattern as G1/G2.

Verify (executed)
  - Unit tests: a synthetic Annex-B sequence produces a valid `m3u8` playlist
    and a sequence of CMAF segments that `ffprobe` (or an equivalent
    real-world HLS consumer, not a hand-rolled parser) accepts without error.
  - Integration test: issue #18's own mock camera server feeds a camera; an
    HTTP test client fetches the playlist, resolves at least two segment
    URLs from it, and fetches those segments successfully; a headless
    browser's `hls.js` (the same library U2 vendors for its own fallback)
    plays the stream to `HAVE_CURRENT_DATA`.
  - `cargo clippy --workspace --all-targets --all-features --locked`, `cargo
    test --workspace`.

Mutation (Significant, oracle not self-evident → required)
  Break this camera's restart-on-panic supervision (bare, unsupervised spawn
  for the HLS-packaging task) → inject a panic in that task → assert only
  this camera's HLS output stops and never resumes, while the same camera's
  MoQ-publish (G1) and fMP4/WebSocket (G2) outputs and the other camera's
  every output continue unaffected → restore → assert this camera's HLS
  output resumes after being restarted. Paste both runs into
  `.agents/issue-12/evidence/G3-mutation.log`.

Scope guard
  Do NOT modify `crates/rtsp-restream`, `crates/corvette-rtsp-client`, or
  G1's/G2's own code beyond an explicitly justified shared fMP4-fragmentation
  helper (see Do step 2). Do NOT add audio-track support (see Premise). Do
  NOT add any Dockerfile/manifest changes (P1/K1). Do NOT touch nginx (N1) —
  this item only serves HTTP internally; N1 is the one that proxies to it.
```

### Phase P — package the relay and the media bridge

```
P1  One Corvette-owned OCI image: adopted moq-relay + corvette-media-bridge (DT-1)
    REPO: corvette          MODEL: sonnet   EFFORT: high     PIPELINE: full
    REVIEW: sonnet, named re-verification duty: confirm no push occurred and both
    binaries are present and runnable in the built layout, independent of the
    implement agent's own log.

Premise
  R1 and G1 CLOSED. VERIFY THE PREMISE FIRST: `command -v docker && docker buildx
  version` outside the devShell (docker/buildx are host tools, not devShell
  tools, per issue #2's own C1 precedent).

Do
  1. Add `docker/Dockerfile.media-bridge`: a multi-stage build. Stage 1 clones/
     vendors R1's pinned `moq-dev/moq` commit and builds `moq-relay`. Stage 2
     builds this workspace's `corvette-media-bridge` release binary. Final stage
     copies both binaries into a minimal runtime image (not `FROM scratch` —
     these are real running processes needing a libc and TLS certificate store
     at minimum; check `frigate-vulkan/docker/` for existing precedent before
     picking a base, per playbook §2 rail 4).
  2. No default `command:` that self-signs a TLS cert (INV-6) — the deployed
     command (K1's manifest) points `moq-relay` at the mounted cert-manager
     Secret's files.
  3. Add `scripts/publish_media_bridge_image.sh`, mirroring `scripts/
     publish_site_image.sh`'s shape: default mode builds a local OCI layout only,
     no registry contact; push mode requires a same-day dated-tag confirmation
     env var, fails closed otherwise, and on a real push records tag+digest+
     commit to `.agents/issue-12/evidence/corvette-media-bridge-digest.txt`.
  4. Add an OCI-layout check asserting both binaries are present and executable,
     and that INV-1's denied literals do not appear anywhere in the Dockerfile
     or the built layout's dependency manifest.

Verify (executed)
  - `./scripts/publish_media_bridge_image.sh` (default mode) builds a local OCI
    layout with zero network calls beyond fetching R1's pinned source.
  - The OCI-layout check passes; both binaries run from the built layout with a
    `--help`/version flag.
  - Push guard: confirmation env var unset → push mode exits non-zero, zero
    network calls.

Mutation (Critical → required)
  Remove the media-bridge `COPY` line → the OCI-layout check must FAIL naming
  the missing binary → restore → PASS. Separately: set the confirmation env var
  to yesterday's date → push mode must still refuse → restore. Do NOT actually
  push in this test. Paste all runs into
  `.agents/issue-12/evidence/P1-mutation.log`.

Scope guard
  Do NOT push to any registry (G-5/G-6). Do NOT write the cluster manifest (K1).
  Do NOT modify `moq-relay`'s own source. Do NOT touch `frigate-vulkan`.
```

### Phase F — stop go2rtc's process from starting (frigate-vulkan)

```
F1  Disable go2rtc's s6-overlay service in frigate-vulkan's own Dockerfile
    REPO: frigate-vulkan    MODEL: sonnet   EFFORT: high     PIPELINE: full

Premise
  `RESEARCH-frigate-ingest-boundary.md` F-6 establishes, at the UPSTREAM Frigate
  source, that go2rtc is an s6-rc `longrun` service the `frigate` service
  declares as a startup dependency (`s6-rc.d/frigate/dependencies.d/go2rtc`
  marker; `s6-rc.d/go2rtc/run` renders `/dev/shm/go2rtc.yaml` then execs the
  go2rtc binary). That research's own RISK note explicitly flags this as
  UNVERIFIED against `frigate-vulkan`'s own Dockerfile layout — a separate
  repository, out of scope for that research pass. **VERIFY THE PREMISE FIRST,
  in full, before writing any change:** read `frigate-vulkan`'s actual
  Dockerfile and rootfs directly and confirm it derives from (or structurally
  mirrors) the same s6-overlay layout, or differs — a from-scratch Vulkan-
  specific rootfs might not use s6-overlay identically. If it differs
  materially from what F-6 describes, STOP-AND-ASK (gate G-8) rather than
  guessing how to disable an unfamiliar startup mechanism.
  **A second, load-bearing premise this item must also verify, not assume:**
  does Frigate's own Python process (`frigate/app.py`'s `init_go2rtc`, which
  only discovers an already-running go2rtc via `psutil` — it does not spawn
  it) fail, degrade, or continue gracefully if go2rtc's process never starts at
  all? `RESEARCH-frigate-ingest-boundary.md` F-3/F-8 suggest go2rtc is unneeded
  when no `live:` config block requests an `mse`/`webrtc` technology (true for
  this deployment, confirmed by the D-6/D-7 fact review's live config read) and
  `birdseye.restream` is off (true, F-9) — but `init_go2rtc`'s own exact
  behavior on a permanently-missing process was never read by any prior
  research pass. Read it directly before this item's Do.

Do
  1. Modify frigate-vulkan's Dockerfile/rootfs so go2rtc's binary is never
     installed or started: remove (or neutralize) the go2rtc s6-rc service
     definition and the `frigate` service's dependency marker on it (or
     frigate-vulkan's own equivalent, per the Premise's verification), so the
     `frigate` s6 service starts without waiting on or requiring go2rtc.
  2. Do not remove the `create_config.py` invocation unless it errors when
     go2rtc's binary is absent (check first) — an unused, never-read
     `/dev/shm/go2rtc.yaml` is harmless; remove it only if it breaks the build
     or start sequence.
  3. Confirm Frigate itself reaches a healthy running state with go2rtc absent,
     in a local container run — not the live deployment (INV-3).

Verify (executed)
  - Build the modified image locally; run it (or the relevant layer) and
    confirm the `frigate` process reaches its normal ready state with no fatal
    error referencing go2rtc.
  - Confirm nothing inside the running container binds port 8554 once modified
    (`ss`/`netstat` inside the container, or equivalent).

Mutation (Critical → required)
  Temporarily reintroduce the go2rtc dependency marker → confirm Frigate's
  startup sequence still waits on/requires it (proving the removed marker was
  actually load-bearing, not inert) → remove again → confirm a clean start.
  Paste both runs into `.agents/issue-12/evidence/F1-mutation.log`.

Scope guard
  Do NOT modify anything under `frigate/` (the Python source) — build/rootfs
  change only. Do NOT push or deploy this image (G-1, G-5, G-6). Do NOT assume
  the live deployment already reflects this change — a human must coordinate
  rolling out this image together with K1's manifest change; landing one
  without the other breaks the pod (go2rtc gone with nothing yet listening on
  8554, or the reverse). Name this sequencing risk explicitly in this item's
  own evidence file for V1/the human to see.
```

### Phase K — cluster exposure (drafted only — see INV-3)

```
K1  Draft the StatefulSet container addition, the RTSP port's move from the
    frigate container to the media-bridge container, the MoQ NodePort UDP
    Service port, and the TLS secret mount (D-3, D-6) — a reviewable file, never
    applied or committed
    REPO: corvette (writes land under .agents/issue-12/evidence/ only)
    MODEL: sonnet   EFFORT: high     PIPELINE: full
    REVIEW: sonnet, named re-verification duty: confirm (a) no
    `~/dockers/kubernetes` working-tree file was touched, (b) no forbidden verb
    from INV-3 appears unmarked anywhere in this item's evidence, (c) the RTSP
    port declaration is moved, not duplicated.

Premise
  P1's image defines the media-bridge container's command(s) and ports.
  `~/dockers/kubernetes/tirnanog/frigate.yaml` is the live StatefulSet + Service
  manifest (read-only source of truth). F1's own local verification (not a live
  cluster change) confirms go2rtc no longer binds port 8554 once its image
  ships. **A specific, citable Kubernetes behavior this item relies on and must
  verify, not assume:** a Service's `targetPort:` resolved by name (as the
  existing `frigate` Service's `rtsp` port already is, per
  `RESEARCH-frigate-ingest-boundary.md` F-7) resolves against the POD's
  aggregate set of named container ports across ALL its containers, not only
  the container the name happened to be declared on first — VERIFY THIS
  against the live manifest's own current structure (does any existing
  Service in this manifest already exercise a named `targetPort` that lives on
  a non-first container in a multi-container pod? if not found as existing
  precedent in this specific manifest, cite Kubernetes' own Service/EndpointSlice
  port-resolution documentation directly rather than assuming) before relying
  on it to mean "the Service's own `rtsp` port entry needs no change, only
  which container declares the `rtsp`-named `containerPort` moves."

Do
  1. Author `.agents/issue-12/evidence/K1-manifest-draft.yaml`: a diff-shaped
     document (not applied anywhere) showing (a) two new containers added to
     the StatefulSet's pod template — `moq-relay` and `corvette-media-bridge` —
     using P1's image referenced by the `PENDING-DIGEST` placeholder (INV-2);
     (b) the `rtsp`-named `containerPort: 8554` declaration REMOVED from the
     `frigate` container's own port list and ADDED to the `corvette-media-
     bridge` container's port list (per the Premise's verified port-resolution
     behavior, the Service's own `rtsp` port entry itself needs no change); (c)
     the relay container's own MoQ port declared as a second `containerPort`;
     (d) a new `Service` port entry, `protocol: UDP`, `nodePort:` a chosen
     number colliding with none of the manifest's existing ports, targeting the
     relay container's port directly (D-3/`RESEARCH-webtransport-ingress.md`
     F-2: not proxied through nginx); (e) a `volumeMount` on the relay
     container for the existing cert-manager Secret, with `moq-relay`'s TLS
     flags pointed at the mounted paths (INV-6).
  2. Write, in the same file's header comment, the D-3 "configuration point"
     note: the same container spec under `hostPort` and a hypothetical
     `LoadBalancer` Service, so NodePort's choice for tirnanog specifically is
     documented as a choice among options.
  3. Add `scripts/check_k1_manifest_draft.sh`: asserts the placeholder-or-digest
     form (INV-2), the chosen UDP port collides with no existing port, the cert
     Secret mount name matches the live manifest's actual name, and the
     `rtsp`-named port appears on exactly one container in the drafted pod
     template (never zero, never two) — the check named in this item's own
     REVIEW duty (c).

Verify (executed)
  - `scripts/check_k1_manifest_draft.sh` passes against the drafted file.
  - `git status` inside `~/dockers/kubernetes` shows no change.
  - `scripts/check_no_mutation.sh` passes over this item's full evidence output.

Mutation (Critical → required)
  Change the chosen UDP port to collide with an existing port → the check must
  FAIL naming the collision → restore → PASS. Duplicate the `rtsp`-named port
  onto both the frigate and media-bridge containers → the check must FAIL
  naming the duplication → restore → PASS. Replace the digest placeholder with
  a fabricated `sha256:...` value → the check must FAIL → restore → PASS. Paste
  all runs into `.agents/issue-12/evidence/K1-mutation.log`.

Scope guard
  Do NOT run any mutating `kubectl` verb anywhere (G-1, G-3, G-4). Do NOT `git
  add`/`git commit`/edit `~/dockers/kubernetes`'s working tree (INV-3, G-1). Do
  NOT choose or fabricate a real digest. If the live manifest's actual cert
  Secret name, or the Premise's Service-port-resolution assumption, does not
  hold as expected, STOP-AND-ASK (G-7) rather than guessing.
```

### Phase N — nginx (`frigate-vulkan`, separate repo)

```
N1  Add the /live/hls/ location, serving G3's HLS/LL-HLS packager (OPEN-2,
    resolved)
    REPO: frigate-vulkan    MODEL: sonnet   EFFORT: high     PIPELINE: full

Premise
  `tests/nginx-parity/vendor/nginx.conf` (confirmed directly, this redraft's own
  drafting) has no `/live/hls/` location today. OPEN-2 is resolved (2026-08-26,
  see "How this redraft resolves..." above): this location proxies to G3, the
  new HLS/LL-HLS packager built inside `corvette-media-bridge` (not to go2rtc,
  which D-6 fully removes, and not to any remux process). G3 CLOSED before
  this item starts. Read G3's own evidence for its exact HTTP path/port
  convention and playlist/segment naming before writing this item's Do.

Do
  1. Add `location /live/hls/ { ... }` in the same style as this file's
     existing `/live/*` blocks, proxying to G3's own HTTP listener (host/port
     per G3's evidence; matching D-3's "configuration point" convention for
     anything not hardcoded).
  2. Update the donor-config drift md5 guard the same way issue #2's D1 built
     it.
  3. Add the new route(s) to `scripts/route_parity.sh`'s table per INV-4.

Verify (executed)
  - The harness's new location returns the content type G3's own contract
    documents (the media playlist and its CMAF segments) and is asserted NOT
    to be the SPA shell.
  - The donor-drift guard passes against the patched config and fails against
    a stale donor copy.
  - `route_parity.sh`'s new row(s) pass in `--mode local`.

Mutation
  Remove the new `location` block → the harness assertion must FAIL; restore →
  PASS. Paste both runs into `.agents/issue-12/evidence/N1-mutation.log`.

Scope guard
  Do NOT touch `/live/mse/*`, `/live/webrtc/*`, `/api/go2rtc/*`, or `/live/
  jsmpeg/` (N2's job, or untouched — `/live/jsmpeg/` proxies to Frigate's own
  `jsmpeg` upstream, confirmed directly against the vendored nginx.conf, NOT to
  go2rtc, so it is unaffected by go2rtc's removal and stays out of scope for
  both N1 and N2). Do NOT modify G3's own server-side contract — if it does
  not support what this item needs, STOP-AND-ASK (G-7) rather than reaching
  into G3's own item to change it.
```

```
N2  Retire every go2rtc-proxied nginx route — unconditional (go2rtc is fully
    gone, not gated on a reading)
    REPO: frigate-vulkan    MODEL: sonnet   EFFORT: medium   PIPELINE: full

Premise
  Confirmed directly against `tests/nginx-parity/vendor/nginx.conf` (this
  redraft's own drafting): exactly five locations proxy to the `go2rtc`
  upstream — `/live/mse/api/ws`, `/live/webrtc/api/ws`, `/live/webrtc/
  webrtc.html`, `/api/go2rtc/api`, and `/api/go2rtc/webrtc` (corrected count
  from the superseded draft's two — that draft only knew about
  the WebRTC-specific pair because it was written under the old BLOCKER-1's
  reading (a), which assumed everything else about go2rtc stayed). Because D-6
  fully replaces go2rtc (not only its live-view role), **every one of these
  five is permanently dead** once F1/K1 land, regardless of how OPEN-1 or
  OPEN-2 resolve — those two open questions are about what (if anything)
  *replaces* `/live/mse/api/ws`'s function and HLS's backend, not about
  whether the OLD go2rtc-backed routes stay. U1/U2 CLOSED, or at minimum
  confirmed to no longer reference these paths, before this item removes them
  (do not remove a route a client still calls).

Do
  1. Remove all five go2rtc-proxied locations named in the Premise from
     frigate-vulkan's donor nginx patch.
  2. OPEN-1 resolved to option (a): add a NEW location replacing
     `/live/mse/api/ws`'s function, proxying to G2's WebSocket endpoint (per
     G2's own evidence for its exact path/port convention) — do not leave
     `/live/mse/api/ws` silently absent; U1's own client code depends on this
     replacement existing.
  3. Update the donor-drift guard for the removed (and any added) hunks, same
     mechanism as N1/issue #2's D1.
  4. Update `scripts/route_parity.sh`'s table: the five removed routes now fall
     through to whatever nginx actually does with no matching location (SPA
     shell or 404 — established by the harness, not assumed).

Verify (executed)
  - Harness: all five named locations no longer proxy to go2rtc; `/live/
    jsmpeg/` is unchanged (a regression test proving this item did not touch
    more than it claims).
  - `route_parity.sh` passes with the updated table.
  - `grep -rn '/live/webrtc/\|/live/mse/api/\|/api/go2rtc/' crates/corvette-ui/src`
    (corvette repo) → no hits, confirming the UI actually stopped depending on
    these routes before this item removes them. NOTE (orchestrator, 2026-08-27):
    narrowed from `/live/mse/` to `/live/mse/api/` — U1 (shipped) introduced a
    deliberate, unrelated NEW route, `/live/mse/ws/<camera>` (G2's own
    replacement path), which shares the `/live/mse/` prefix with the OLD
    `/live/mse/api/ws` this item removes; the broader pattern would wrongly
    flag U1's own intentional code as a leftover reference. Confirmed via
    `grep -rn '/live/webrtc/\|/live/mse/\|/api/go2rtc/' crates/corvette-ui/src`
    that the only matches left are `/live/mse/ws/` (U1's real route) and one
    doc-comment quoting this plan's own text — neither is a stale reference.

Mutation
  Leave one of the five locations in place while removing the other four → the
  "unchanged" regression assertion for `/live/jsmpeg/` must still PASS while
  the four removed ones' assertions FAIL to show they weren't removed, and the
  one left in place must be caught by a dedicated assertion that ALL FIVE are
  gone → fully apply the real change → all pass as intended. Paste runs into
  `.agents/issue-12/evidence/N2-mutation.log`.

Scope guard
  Do NOT act on this item before U1/U2 confirm they no longer reference these
  routes. Do NOT touch `/live/jsmpeg/`. Do NOT remove go2rtc from the
  frigate-vulkan image itself here — that is F1's own item.
```

### Phase U — UI

```
U1  Grid tile: iframe → G2's WebSocket/fMP4 output (OPEN-1, resolved)
    REPO: corvette          MODEL: sonnet   EFFORT: medium-high   PIPELINE: full

Premise
  `crates/corvette-ui/src/dashboard.rs:66-78` currently embeds
  `/live/webrtc/webrtc.html?src={camera}` in an `<iframe>` — confirmed at this
  redraft's own drafting. OPEN-1 is resolved (2026-08-26, see "How this
  redraft resolves..." above): the grid tile connects to G2's new
  fMP4-over-WebSocket output instead. G2 CLOSED before this item starts (this
  item is the client half of G2's contract). Read G2's own evidence for its
  exact WebSocket URL shape, fMP4 initialization-segment framing, and
  per-camera naming convention before writing this item's Do.

Do
  1. Remove the `<iframe>` entirely — no dual-path fallback to it (the Done-
     when clause requires the embedded player page gone, not optional).
  2. Extend `scripts/check_no_go2rtc.sh` per INV-7 with the `/live/webrtc/
     webrtc.html` literal.
  3. Replace the tile's rendering with a native `<video>` element driven by
     `MediaSource`/`SourceBuffer` (mirroring Frigate's own `MsePlayer.tsx`,
     per issue #12's own Scope wording), fed by a WebSocket connection to N2's
     replacement location for that camera, appending each received fMP4 box
     directly to the `SourceBuffer` — no repackaging on the client side.
  4. Handle the WebSocket's own close/error events with a bounded reconnect
     (matching this codebase's existing reconnect-with-backoff convention,
     e.g. `corvette-rtsp-client`'s own `Backoff` shape, reimplemented
     client-side since this is a browser context) rather than leaving the
     tile permanently blank on one dropped connection.

Verify (executed)
  - `cargo clippy --workspace --all-targets --all-features --locked`, `cargo
    test --workspace`, `make check-ui`.
  - `grep -rn 'webrtc.html' crates/corvette-ui/src` → no hits; the extended
    `check_no_go2rtc.sh` passes.
  - Playwright test: the grid tile's `<video>` element reaches
    `readyState >= HAVE_CURRENT_DATA` against G2's real endpoint (not a
    mock), i.e. the tile actually renders a frame.

Mutation
  Reintroduce the iframe rendering path → the "no iframe present" assertion and
  the extended grep check must both FAIL → restore → PASS. Paste both runs
  into `.agents/issue-12/evidence/U1-mutation.log`.

Scope guard
  Do NOT touch the expanded-view interaction (U2). Do NOT touch anything
  under `docker/` or cluster manifests. Do NOT modify G2's own server-side
  contract — if it does not support what this item needs, STOP-AND-ASK (G-7)
  rather than reaching into G2's own item to change it.
```

```
U2  Expanded view: MoQ-first via hang's Web Component, HLS fallback gated on
    OPEN-2
    REPO: corvette          MODEL: sonnet   EFFORT: high     PIPELINE: full

Premise
  No expanded/click-to-enlarge interaction exists today — confirmed at this
  redraft's drafting. R1/G1 CLOSED: the resolved `hang` package and a confirmed
  working local `moq-relay` are in `R1-dependency-pin.md`; G1 is the process
  actually publishing each camera's MoQ track. VERIFY THE PREMISE FIRST: read
  `js/hang`'s own README/source at R1's pinned commit for its Web Component's
  exact tag name, required attributes, and its connection-failure/timeout event
  surface. **A real, currently-shipped gap this item's audio handling must
  account for honestly (see G1's own Premise):** no camera's audio track is
  actually published over MoQ today, since `corvette-rtsp-client` only resolves
  `m=video`. This item's own "UI clarification" design note (carry audio when
  the camera provides it) should be implemented so it degrades to video-only
  cleanly when no audio track is announced — not written as if audio is always
  present, and not blocked on the audio gap being fixed first (that is a
  separate, unscoped future item).
  OPEN-2 is resolved (2026-08-26): this item's HLS-fallback step is now fully
  specifiable against G3/N1's contract (gate G-10 is closed, kept below as a
  historical record).

Do
  1. Add a click/expand interaction on each grid tile that mounts `hang`'s
     custom element, pointed at the relay's public WebTransport URL
     (configurable, matching D-3), with the camera's broadcast name (matching
     G1's own naming).
  2. A bounded timeout (choose and justify a value, e.g. 3-5 seconds), OR the
     Premise-confirmed failure event firing first, tears down the MoQ
     component and mounts an HLS `<video>` element pointed at N1's
     `/live/hls/` location (OPEN-2 resolved to option (a): G3's HLS/LL-HLS
     packager). N1 CLOSED before this item's fallback half starts.
  3. Surface which path is active (MoQ vs. fallback vs. none) in the UI in some
     minimal, testable way.

Verify (executed)
  - Playwright test 1 (does not depend on OPEN-2): relay reachable → the MoQ
    path is attempted first and no fallback element is ever mounted.
  - Playwright test 2 (depends on OPEN-2's answer): relay unreachable → the
    fallback fires within the chosen timeout, against whatever OPEN-2 named.
  - `cargo clippy`, `cargo test`, `make check-ui`.

Mutation
  Disable the fallback branch → test 2 must FAIL; restore → PASS. Paste both
  runs into `.agents/issue-12/evidence/U2-mutation.log`.

Scope guard
  Do NOT touch the grid tile itself (U1). Do NOT modify `crates/rtsp-restream`,
  `corvette-media-bridge`, or any cluster/manifest file. Do NOT implement the
  fallback's own backend-specific wiring before OPEN-2 is answered. If `hang`'s
  Web Component exposes no usable failure/timeout event at all, STOP-AND-ASK
  (G-7) — do not invent a polling-based workaround as a silent substitute.
```

### Phase W — documentation (batched)

```
W1  docs/design/api-contracts.md and architecture.md updates
    REPO: corvette + frigate-vulkan   MODEL: sonnet   EFFORT: medium-high   PIPELINE: batched

Premise
  `docs/design/architecture.md`'s "Media boundary" section already carries an
  "Open" note this redraft resolves into OPEN-1 (see above) — that note itself
  needs replacing once OPEN-1 has an answer, not left in its current
  provisional wording. Every other item this batch depends on (X1-X3, G1, P1,
  F1, K1, N1, N2, U1, U2) must be CLOSED first.

Do
  1. `architecture.md`, "Media boundary": rewrite to state the actual post-#12
     shape, including which OPEN-1 and OPEN-2 answers were chosen, verbatim.
     Record the port-8554 move from the frigate container to the media-bridge
     container as a stated architectural fact, not left implicit.
  2. `api-contracts.md`: add sections recording — the restream server's own
     narrowed protocol contract (5 methods, 1 transport, no auth — matching
     what a future external consumer, e.g. Home Assistant per
     `RESEARCH-frigate-ingest-boundary.md` F-7/DP-3, would need to know); the
     media-bridge's per-camera broadcast/track naming convention; the NodePort
     exposure mechanism and why (D-3, quoted); the `/live/hls/` route's actual
     contract once OPEN-2 resolves; N2's five retired routes explicitly; the
     audio-track gap named in G1/U2's own Premises, recorded as a durable,
     findable fact rather than left only in this plan's evidence.
  3. Link this work from issue #12 per `AGENTS.md`'s requirement.

Verify (executed)
  - `make check-whitespace` clean.
  - A human or reviewer reads both files against what each closed item's
    evidence actually recorded.

Mutation
  N/A — Routine profile, no mutation cycle.

Scope guard
  Do NOT restate anything as settled that OPEN-1/OPEN-2 left open at the time
  this item runs. Do NOT introduce new decisions of record here.
```

### Phase V — cutover evidence and issue hygiene

```
V1  End-to-end evidence bundle against the real deployment, including Frigate's
    own recording pipeline
    REPO: corvette          MODEL: sonnet   EFFORT: high     PIPELINE: full
    REVIEW: sonnet, re-executes the parity diff, the image-ID match, and the
    recording-continuity check itself.

Premise
  This item cannot start until a human has (a) pushed P1's and F1's images and
  (b) applied K1's manifest. Until both are true, this item has nothing to
  verify against. All other items may close independently of this one starting.

Do
  1. Pre-flight: capture the current (pre-cutover) route-parity baseline for
     every `/live/*` route this plan touches, read-only, against the real
     deployment.
  2. Post-cutover: re-run the same table and classify every changed row as
     INTENDED or REGRESSION.
  3. Confirm both new containers run the pushed digests, not merely that the
     manifest names them.
  4. **New in this redraft, and higher-stakes than anything the prior draft's
     V1 checked:** confirm Frigate's own `detect` and `record` roles continue
     to function post-cutover — a new clip is produced and retained for at
     least one configured camera in the period after cutover, read-only via
     `kubectl exec … ls`/`cat` against Frigate's own recordings storage or API,
     not merely that the pod is Running. A route-parity pass alone would not
     catch a break in this path, since none of the `/live/*` routes this plan
     touches exercise it.
  5. File the human-browser-artifact requirement (grid tile plays video; the
     mechanism OPEN-1 chose plus OPEN-2's fallback both work in a real browser)
     explicitly, not agent-provable.

Verify (executed)
  - Pre/post parity diff, image-ID match, and the new recording-continuity
    check, all with their actual output pasted into evidence.

Mutation
  N/A in the classic sense — the pre/post diff and the recording-continuity
  check are themselves the mechanical way to catch an unintended regression.

Scope guard
  Do NOT push, apply, or restart anything. Read-only against the cluster
  throughout. Do NOT re-litigate any earlier item's already-passed review.
```

```
V2  Issue hygiene (orchestrator duty, not a delegated item)
    ORCHESTRATOR — no model/effort/review line.

Updates issue #12 as items close, with named evidence, not assertion. Records
OPEN-1's and OPEN-2's eventual human answers on the issue itself, not buried
only in this plan file. Closes issue #12 only when every Done-when clause
carries named evidence AND the record is honest about which OPEN-1/OPEN-2
answers were chosen — including correcting the Done-when clause itself if
OPEN-2 resolves to dropping the HLS-fallback requirement (option (b)), which
would otherwise leave the issue's own closing criteria referencing something
this plan deliberately did not build. Updates issue #1's roadmap checkbox.
Does not grow issue #12's scope silently — any newly discovered item (e.g. a
new OPEN-1/OPEN-2-driven component) is filed as a NEW issue linked from #1,
never folded into an already-reviewed item in this plan.
```

---

## STOP-AND-ASK gates (§3, §9) — topic-anchored, unconditional

```
G-1  If the work would APPLY A KUBERNETES MANIFEST, or `git add`/`git commit`
     inside `~/dockers/kubernetes` at all → STOP and ask.

G-2  If the work would REPLACE THE RUNNING IMAGE OR EDIT AN IMAGE/DIGEST
     REFERENCE in any deployment manifest → STOP and ask.

G-3  If the work would EDIT CONFIGURATION INSIDE THE POD (kubectl exec with
     anything other than curl/cat/ls, kubectl cp, writing to any container's
     own config on the running pod) → STOP and ask.

G-4  If the work would RESTART, SCALE, DELETE, OR ROLL OUT a workload → STOP
     and ask.

G-5  If the work would PUSH ANYTHING TO ANY REGISTRY → STOP and ask. Building
     locally to an OCI layout is permitted; leaving the machine is not.

G-6  FIRST PUSH TO A NEW REGISTRY REPOSITORY specifically → STOP and ask, even
     with G-5 otherwise satisfied.

G-7  If a DECISION OF RECORD (D-1…D-9, DP-3, DT-1, quoted above) turns out to
     be unimplementable AS STATED → STOP and ask with a re-spec. Do not
     substitute, reinterpret, or "improve" a decision. Named triggers already
     identified: `hang`'s Web Component exposing no usable failure/timeout
     event (U2); go2rtc's HLS surface characterization turning out irrelevant
     once OPEN-2 is answered by a non-go2rtc mechanism (N1 — already assumed,
     not a surprise); `moq-relay` requiring Redis/`moq-api` to run standalone
     (R1); `rtsp-types`/`sdp-types` lacking something X1/X7 assumed they
     provide; a Kubernetes Service's named-`targetPort` resolution not
     matching K1's Premise-stated behavior.

G-8  If the work would reach FRIGATE'S BACKEND (the Python source under
     `frigate/`), the non-UI contents of `/opt/frigate/web`, the media/API
     contracts already recorded in `docs/design/api-contracts.md`, a
     CLIENT-ROUTE RENAME, or file deletion outside the item's own new files →
     STOP and ask. F1's own Dockerfile/rootfs change is the one narrow,
     explicitly-scoped exception this plan authorizes into frigate-vulkan's
     build layer — it still does not authorize touching `frigate/`'s own
     Python source, and F1's own Premise already names a STOP-AND-ASK if the
     actual layout found there differs from what was assumed.

G-9  If the work would `git push`, or commit outside the item's stated scope,
     in EITHER `corvette` or `frigate-vulkan` → STOP and ask.

G-10 CLOSED (2026-08-26) — kept for the historical record, not an active gate.
     OPEN-1 (grid tile MSE source) and OPEN-2 (HLS fallback backend) — named
     at the top of this plan — were unanswered as of this plan's initial
     drafting and blocked specific items: OPEN-1 blocked U1 in full and N2's
     `/live/mse/api/ws`-replacement question; OPEN-2 blocked N1's actual
     content and U2's fallback step. Both are now resolved (OPEN-1 → G2,
     OPEN-2 → G3); every item below carries its real Do. This gate's original
     rule — an implementer handed a still-blocked item STOPS and asks rather
     than guessing — remains true in general and is restated here as a
     pattern, not because either question is still open.
```

---

## What lands where

| Item | `corvette` | `frigate-vulkan` | `~/dockers/kubernetes` | `docs/design/` |
| --- | --- | --- | --- | --- |
| R1 | `Cargo.toml` git-dep pin, evidence file | — | — | — |
| X1 | `crates/rtsp-restream/src/{session,sdp}.rs`-shaped files | — | — | — |
| X2 | `crates/rtsp-restream/src/packetize/**` | — | — | — |
| X3 | `crates/rtsp-restream/src/server/**` | — | — | — |
| G1 | `crates/corvette-media-bridge/**` | — | — | — |
| G2 | `crates/corvette-media-bridge/**` (fMP4/WebSocket module) | — | — | — |
| G3 | `crates/corvette-media-bridge/**` (HLS/LL-HLS module) | — | — | — |
| P1 | `docker/Dockerfile.media-bridge`, `scripts/publish_media_bridge_image.sh`, OCI check | — | — | — |
| F1 | — | Dockerfile/rootfs (go2rtc s6 service removed) | — | — |
| K1 | `.agents/issue-12/evidence/K1-manifest-draft.yaml` (draft only) | — | — (read-only) | — |
| N1 | `route_parity.sh` table rows | nginx patch, drift-guard update | — | — |
| N2 | `route_parity.sh` table rows | nginx patch, drift-guard update | — | — |
| U1 | `crates/corvette-ui/src/dashboard.rs`, `check_no_go2rtc.sh` extension | — | — | — |
| U2 | new UI component(s), `@moq/hang`/hls.js dependency | — | — | — |
| W1 | release notes if any | rollback/pointer docs if needed | — | **`architecture.md`, `api-contracts.md`** |
| V1 | `.agents/issue-12/evidence/**` | — | — (read-only) | — |
| V2 | — | — | — | — (GitHub only) |

Nothing in this plan gives `corvette` ownership of `~/dockers/kubernetes` or of
`frigate-vulkan`'s image build in general — K1 only ever produces a proposal a human
carries over by hand.

---

## Plan-gate summary (what the human is being asked to approve)

- **17 delegated items across 10 phases** (R, X, G, P, F, K, N, U, W, V), plus one
  ongoing orchestrator duty (V2). No item is CLOSED; this is a full redraft, not an
  amendment.
- **The prior draft's BLOCKER-1 is resolved, not carried forward**: go2rtc is fully
  replaced — including Frigate's own `detect`/`record` path, per D-6 — by this plan's
  own Rust components. This redraft does not guess at that; it is what D-6 through D-9
  already decided.
- **Two new, narrower questions this redraft found (not guessed at) and the human has
  now resolved (2026-08-26)**: OPEN-1 (what serves the grid tile's live MSE source —
  resolved to a new fMP4-over-WebSocket repackager, item G2) and OPEN-2 (what serves
  the HLS/LL-HLS fallback — resolved to a new HLS/LL-HLS packager, item G3). Both land
  inside `corvette-media-bridge` alongside G1, per the new DP-4 decision. U1 (grid
  tile) and N1/U2's HLS-fallback step, previously blocked, now have fully specifiable
  Do sections against G2/G3's contracts.
- **One entirely new phase (X, three items)** builds the D-6/D-7 RTSP-restream server
  as its own standalone, reusable crate — the prior draft had no item for this at all.
- **G1 is redesigned from the ground up**: it no longer reads go2rtc's MPEG-TS HTTP
  endpoint (the exact thing D-9 replaced); it now wires issue #18's already-shipped
  `corvette-rtsp-client` to the new restream server and to a direct MoQ-publish loop,
  all in one process (D-8), with a documented, honestly-reported audio gap (no camera's
  audio is actually published today, since the client only resolves video).
- **Two new items (G2, G3) added to phase G**, both landing in the same
  `corvette-media-bridge` process as G1 per DP-4: G2 repackages the shared frame
  stream into fMP4-over-WebSocket for the grid tile; G3 packages it into HLS/LL-HLS
  for the fallback path. Neither is recording-path-critical (UI-only), so both carry a
  Significant, not Critical, risk profile despite sharing G1's process.
- **One new phase (F, one item)**, not present in the prior draft at all: disabling
  go2rtc's own s6-overlay startup dependency in `frigate-vulkan`'s Dockerfile — without
  it, D-6's "fully replace go2rtc" cannot actually take effect, since go2rtc would still
  start and still hold port 8554.
- **K1 is expanded**, not just re-verified: it now also moves the RTSP port declaration
  Frigate's own ffmpeg depends on from the frigate container to the new media-bridge
  container — a real architectural fact (Kubernetes pods share a network namespace, so
  Frigate's own `127.0.0.1:8554` config needs no change) this redraft establishes and
  states plainly rather than leaving implicit.
- **N2 is expanded from two routes to five**, unconditionally (not gated on a
  BLOCKER-1 reading), since go2rtc's full removal makes every one of its proxied routes
  permanently dead regardless of how OPEN-1/OPEN-2 resolve.
- **DT-1's prior topology is confirmed to still hold**, with one amendment: the
  media-bridge container's responsibility grows to include hosting the RTSP-restream
  server, raising the blast radius of a bug there from "live view degrades" to
  "Frigate's own recording breaks" — reflected in every affected item's risk profile.
- **DP-3 is refined**, not replaced: one supervised task per camera *per role* (restream-
  feeding and MoQ-publish are now independent), not one task per camera overall — and
  **DP-4** extends this further to the two new G2/G3 roles.
- One batch (W1, docs). Everything else gets the full per-item implement→review
  pipeline.
- Maximum 2 agents in flight, and only across `corvette`/`frigate-vulkan`.
- Ten topic-anchored STOP gates, including one (G-10) that anchored specifically to
  OPEN-1/OPEN-2 remaining unanswered and is now closed — kept as a historical record.
- Eight invariants, each with a named enforcement mechanism and the item that builds it.
```
