# API and media contracts

These contracts define the server behavior Corvette's UI and retained nginx
components may depend on. Frigate incompatibilities and implementation work are
tracked in the relevant GitHub issue; this document records the intended
Corvette behavior.

## Media availability is explicit

A review record can outlive its thumbnail, snapshot, or recording, and every
asset can have a different retention policy. Corvette reports thumbnail,
snapshot, and playable-clip availability as independent current capabilities.
An available clip includes its playback URL.

Clients do not synthesize media URLs from review identifiers or timestamps,
infer video availability from the presence of a review, or discover
availability by assigning a URL and interpreting a failed media load.

Frigate demonstrates why this is necessary: a deployment can retain and play a
review's recording by camera and timestamp while rejecting the same review at
`/api/review/{id}/clip.mp4`. Newer Frigate source adds that route, making its
presence version-dependent as well as retention-dependent.

## Activity is one explicit resource

Review decisions and sampled motion are different source records. Frigate's
`/api/review` classifies alerts and detections, while
`/api/review/activity/motion` samples significant motion from recording
metadata. Corvette exposes a unified activity model while preserving whether an
entry is a review decision or a sampled motion interval.

A motion timestamp identifies the start of a sampling bucket, not an exact
retained frame. Media for motion activity is addressed as a range because a
point timestamp can fall in a recording gap even when its bucket contains
playable footage.

## Ranges use half-open overlap

Every activity query uses half-open `[after, before)` bounds and selects records
that overlap the range. Adjacent ranges therefore partition the timeline without
dropping records that cross a boundary.

This deliberately differs from Frigate's two activity endpoints:

- `/api/review` uses overlap: `start_time < before` and an absent `end_time` or
  `end_time > after`.
- `/api/review/activity/motion` uses strict containment:
  `start_time > after` and `end_time < before`.

Strict containment drops a recording row that straddles midnight from both
adjacent day queries. Corvette does not preserve that behavior.

Default ranges are resolved for each request. Frigate's
`/api/{camera}/recordings` declares timestamp expressions as Python default
arguments, freezing them when the module is imported. Corvette either computes
a default at request time or rejects an unbounded query.

## Motion aggregation stays server-side

Frigate loads matching recording rows into pandas, resamples them into buckets,
and normalizes each hour. Corvette replaces that per-request dataframe with an
ordered database query and a bounded-memory reduction. SQL filters and orders
the authoritative rows; Rust selects the signed value with the greatest
absolute magnitude and collects the participating camera set for each bucket.

The API returns compact activity buckets rather than raw recording telemetry.
Sending storage-shaped rows would increase transfer size, repeat aggregation in
every open browser, and expose an implementation detail instead of the activity
contract clients need.

## nginx-vod mapping remains available

nginx uses `vod_mode mapped` with `vod_upstream_location /api`. It asks the API
for a JSON mapping from a playback request to files on disk, then reads those
files itself. Any service taking ownership of `/api` must continue answering the
mapping request until the playback architecture changes.

## Media paths remain stable during migration

nginx serves `clips/`, `recordings/`, and `exports/` with
`root /media/frigate`. Corvette's writer preserves that layout until the nginx
configuration and writer migrate together.

go2rtc reads `/dev/shm/go2rtc.yaml`, not Frigate's `config.yml`. The Rust NVR
renders that file directly from its configuration.

`/tmp/cache/birdseye` is required only if Corvette deliberately keeps birdseye.
It is not part of the recording contract.

## Thumbnails under `/clips/` carry no usable content type

`location /clips/` declares its own `types { video/mp4 mp4; image/jpeg jpg; }`
block. An nginx `types` block inside a location *replaces* the inherited MIME map
rather than extending it, so any extension absent from those two entries falls to
`default_type`. Review thumbnails are `.webp`, so they are served as
`application/octet-stream`.

This is invisible today because `<img>` sniffs the bytes and ignores the declared
type. It breaks the moment a client needs the real type — `fetch` plus
`createImageBitmap`, a `<picture>` element keyed on `type`, or a Content-Security
Policy that discriminates by media type.

Corvette's contract: media responses state their actual type. A location that
narrows the MIME map must enumerate every extension it serves, or extend the map
instead of replacing it. When Corvette owns this route, `.webp` is served as
`image/webp`.

Note that Corvette's local development proxy diverges here in a second, unrelated
way: its `types` block declares no image entries at all. Neither environment is a
guide to the other for content types.

## `/recordings/` is a different resource from `/recordings`

The deployed nginx has, verified: "`location /recordings/` (:164) serves
`/media/frigate/recordings` as a JSON autoindex. A trailing slash on the Leptos
`recordings` route hits the filesystem, not the SPA. Verified: `/recordings` →
200 `text/html`, `/recordings/` → 200 `application/json`." The two URLs are
not variants of the same resource; one is the SPA shell and the other is a
directory listing.

Corvette's UI must never emit the trailing-slash form on its own route links.
This is one instance of a general contract, not a special case: "Same
collision class for `/exports/`, `/clips/`, `/stream/`, `/vod/`, `/cache/`,
`/ws`, `/live/*`, `/api/*`, `/assets/`, `/fonts/`, `/locales/`. These names
are reserved and must never become client-side routes."

## The deployed go2rtc surface is one HTML file, not a `/go2rtc/` prefix

There is no `/go2rtc/` location in the deployed nginx config. The only go2rtc
player nginx proxies is a single, self-contained file: `/live/webrtc/webrtc.html`.
`/live/webrtc/video-rtc.js` is not proxied — it falls through to the SPA
fallback rather than returning a go2rtc asset, so there is no drop-in player
bundle to load.

Corvette's live view therefore depends on that one HTML file plus the two
websocket routes proxied directly to go2rtc: `/live/mse/api/ws` and
`/live/webrtc/api/ws`. A client route or link that assumes any other
`/go2rtc/*` or `/live/webrtc/*` asset exists will not resolve against this
deployment.

## RTSP camera sessions require a real keep-alive margin, not a thin one

Two independently deployed Reolink cameras' RTSP servers (LIVE555-based, same
firmware version) each grant a Session with a declared timeout on `SETUP`
(`Session: <id>;timeout=65`). Confirmed directly against both live cameras:
with no further RTSP activity on that session, the RTP stream goes silent at
roughly the declared timeout — frames simply stop arriving, with no TCP close
and no RTSP error to signal it. Confirmed directly, both cameras: sending
`GET_PARAMETER` on that session every 20 seconds (roughly a third of the
declared timeout) keeps the stream flowing indefinitely with zero gaps. This
is not a one-off quirk of a single unit.

The currently-deployed go2rtc (`v1.9.10`, `pkg/rtsp/conn.go`) does send its
own keep-alive (`OPTIONS` on the session), but on a `declared_timeout - 5`
second schedule — only a 5-second margin against this camera's 65-second
deadline. A missed keep-alive triggers go2rtc's reconnect path
(`internal/streams/producer.go`), whose backoff schedule (1s, then 5s, then
10s, then 1-minute tiers) can take on the order of two minutes to land a
working reconnect — consistent with, but not independently reproduced
end-to-end as the cause of, the multi-minute Reolink stalls observed in this
deployment. The camera's silent-timeout behavior and go2rtc's keep-alive/
reconnect code are each confirmed separately from live testing and source;
the two have not been observed failing together in one reproduction.

Corvette's contract: any RTSP client Corvette owns reads each camera's own
declared Session `timeout=` from its `SETUP` response and schedules
keep-alives at a real safety margin against it, not a fixed small offset — a
per-camera value, since different camera firmware may declare different
timeouts and tolerate different margins.

Implemented: `crates/corvette-rtsp-client`'s session layer computes this
schedule per camera from the declared timeout (`keep_alive_interval` in
`src/session/keepalive.rs`), capped so the interval is never closer than half
the declared value — a real margin against every tested case, not the
`declared_timeout - 5` offset go2rtc uses above.

## An RTSP camera's own RTP stream can race its handshake responses

Confirmed directly against one of the same Reolink cameras above: it can
begin pushing RTP frames on the interleaved binary channel before, or
interleaved with, `PLAY`'s own `200 OK` arriving on the same TCP socket — the
handshake response and the camera's first frame race each other on one byte
stream. Every real-camera connection attempt failed immediately until this
was accounted for. Nothing in the RTSP or RTP specifications bars a server
from doing this, and the mock camera this project built to develop against
never exercised it: its own interleaving fixture only ever delayed a
keep-alive's text reply between RTP frames already flowing well after `PLAY`,
never a data frame racing a request/response exchange itself.

Corvette's contract: any RTSP client Corvette owns treats a `$`-prefixed
binary frame arriving while it awaits a response to any request — `DESCRIBE`,
`SETUP`, `PLAY`, a keep-alive, or `TEARDOWN` — as ordinary camera data, never
as an error. `crates/corvette-rtsp-client` buffers such a frame
(`Connection::send_and_receive` in `src/session/mod.rs`) and hands it back,
in arrival order, the first time a caller reads a packet
(`PlayingSession::next_packet`) — no frame a camera sends is ever discarded
just because it arrived early.

## RTSP camera frames carry Annex-B video, not a container format

A container was on the table for the RTP-to-MoQ path: `moq_mux::container::
ts::Import` already demuxes the same MPEG-TS byte stream go2rtc's HTTP
endpoint used to serve, so routing this crate's output through MPEG-TS first
would have reused an existing demuxer. That demuxer turned out to be a thin
wrapper over `moq_mux::codec::h264`/`h265`/`aac`'s own `Split`/`Import`
functions — the same codec-level entry points a caller can reach directly
once RTP is depacketized. Re-packaging depacketized RTP into MPEG-TS only to
immediately demux it back out would cost a container write and a container
parse on every frame for no benefit.

`crates/corvette-rtsp-client`'s RTP depacketizer (`src/depacketize`)
reassembles H.264/H.265 into Annex-B: every emitted NAL unit is prefixed with
the start code `00 00 00 01`, with parameter sets carried in-band from the
SDP's `sprop-parameter-sets` (H.264) or `sprop-vps`/`sprop-sps`/`sprop-pps`
(H.265) rather than out-of-band. This costs only a fixed 4-byte prefix write
per NAL, not a conversion — an RTP H.264/H.265 payload is already
NAL-unit-oriented, so nothing about the source data has to change shape. The
crate's AAC depacketizer (`src/depacketize`'s `aac` module) extracts each
access unit at the length its RTP payload's own AU-header declares and emits
it unmodified, with no ADTS header added, since RTP's AAC payload format
(RFC 3640, `mpeg4-generic`) already matches the raw shape `moq_mux::codec::
aac::Import` expects.

Today the crate publishes video only. Its SDP resolution
(`session::sdp::resolve_video_track`) locates only the SDP's `m=video`
section, so no audio track is ever resolved against a live camera session
and no `AacDepacketizer` is constructed from one — even though the AAC
depacketizer itself exists and is unit-tested against synthetic RTP
payloads. A camera's audio, if it has any, is not read today.

Corvette's contract: every frame `crates/corvette-rtsp-client` publishes is
either Annex-B-framed H.264/H.265 or a raw, non-ADTS AAC access unit — never
MPEG-TS, fMP4, or any other container — so a downstream consumer calls
`moq_mux::codec::*::Import` directly with no demuxing step in between. The
contract governs the shape of any frame this crate publishes; it does not by
itself mean audio is currently published, since no audio track is resolved
yet.

## A camera's own RTP stream occasionally drops or reorders a fragment

Confirmed directly against one of the same Reolink cameras above, over a
150-second run: roughly every 10-20 seconds, the H.264 depacketizer received
an FU-A continuation packet with no start fragment in progress — a real,
occasional dropped or reordered RTP packet from the camera's own stream, not
a parsing bug (`DepacketizeError::FragmentWithoutStart`,
`crates/corvette-rtsp-client/src/depacketize/h264.rs`). A second, different
real camera (a Dahua-style unit) ran the same 150-second window with zero
such errors, confirming this is camera- and network-dependent, not universal.

Corvette's contract: a single packet that fails to depacketize does not end
the camera's session. `crates/corvette-rtsp-client`'s per-camera task
(`depacketize_packet` in `src/client/task.rs`) logs the error and drops that
one packet, then keeps reading — as long as the underlying connection
(`PlayingSession::next_packet`) keeps delivering data, an isolated malformed
fragment is not treated as a reason to tear down the session and force a full
DESCRIBE/SETUP/PLAY reconnect.

## Dialing the MoQ relay by hostname needs address racing, not the first DNS answer

`crates/corvette-media-bridge`'s own MoQ-publish role (issue #12 G1, D-5) dials the relay
directly against `web_transport_quinn::Client::connect`, not through `moq-native` (kept out per
D-5's "small in-repo binary" framing — `moq-native` wraps multiple QUIC backends and a
Happy-Eyeballs dialer this crate doesn't need). Reading `web-transport-quinn` 0.11.12's own
`Client::connect` (`src/client.rs`) directly: for a domain host it calls `tokio::net::lookup_host`
once and dials only the *first* resolved address — no IPv4/IPv6 racing, unlike `moq-native`'s own
dial (`rs/moq-native/src/quinn.rs`, which races every candidate address it resolves). Dialing
`https://localhost:<port>/...` against a relay bound only to `127.0.0.1` timed out outright in
this item's own integration test until this was diagnosed: the test's own resolver returned an
IPv6 `::1` candidate first, and nothing was listening there.

Corvette's contract: `corvette-media-bridge` dials the relay by a literal IP address (from
`MoqConfig::relay_url`), never a hostname requiring DNS resolution, so this single-candidate
behavior can't pick the wrong address family. A future item (K1) that dials a relay by a real
DNS name (rather than a literal cluster-internal IP) needs to either race candidates itself or
confirm the deployed resolver's answer order matches the relay's actual bound address family —
this crate's own direct `web-transport-quinn` dependency does not do that automatically the way
`moq-native`'s own dial would.

## The fMP4-over-WebSocket grid-tile transport carries no per-viewer timestamp epoch

`crates/corvette-media-bridge`'s fMP4 repackager (issue #12 G2, `fmp4::Fragmenter`) builds one
`moof`/`mdat` fragment per access unit and broadcasts the identical bytes to every WebSocket
viewer of a given camera. Each fragment's `tfdt` (`baseMediaDecodeTime`) is relative to when that
camera's own repackaging task first observed a frame — one shared per-camera epoch, not a
per-connection one — because the fragment bytes themselves are precomputed once and fanned out
unchanged to however many viewers are currently connected; giving each viewer its own zeroed
epoch would mean rewriting each fragment's `tfdt` per connection, which this item's own broadcast
design does not do.

A viewer that connects long after a camera's repackaging task started therefore receives a first
fragment whose `tfdt` is a large, arbitrary offset from zero, not from that viewer's own
connection time. A real MSE `SourceBuffer` in the default `"segments"` append mode positions
buffered data at its own declared timestamps, so a viewer relying on that default would see a
buffered range starting at that same large offset — never covering `currentTime` 0 — and never
reach `HAVE_CURRENT_DATA` (confirmed directly: this item's own headless-browser check,
`crates/corvette-media-bridge/tests/browser/g2_fmp4_ws.spec.cjs`, reproduced exactly this symptom
before switching modes).

Corvette's contract: a consumer of this transport (this item's own browser check today; U1's grid
tile, a later item, in production) sets `sourceBuffer.mode = "sequence"` before appending
anything, so the browser plays appended segments back-to-back on its own timeline starting at 0
and ignores each segment's own absolute `tfdt` — using only each sample's declared duration to
advance. This is a client-side integration requirement this transport's own wire format assumes;
it is not negotiated or advertised anywhere in the protocol itself, so any future consumer of this
same WebSocket endpoint needs to know to set it.

## The RTSP-restream server implements five methods, one transport, no authentication

`crates/rtsp-restream` (issue #12 D-6/D-7) is a standalone, from-scratch RTSP server built to
replace go2rtc's own restream role for Frigate's `detect`/`record` ffmpeg, without adopting a
third-party server or any AGPL-licensed code. Confirmed directly from its own source
(`src/session.rs`, `src/sdp.rs`): its `OPTIONS` response advertises exactly `OPTIONS`,
`DESCRIBE`, `SETUP`, `PLAY`, `TEARDOWN` (`IMPLEMENTED_METHODS`) — go2rtc's own deployed RTSP
server (`v1.9.10`) implements exactly this set despite advertising `PAUSE`/`ANNOUNCE`/`RECORD` in
its own `OPTIONS` response, which it does not actually implement, and Frigate's own ffmpeg preset
never requests any of those three either. `SETUP` accepts only
`RTP/AVP/TCP;unicast;interleaved=<n>-<n+1>` and rejects any UDP transport request with `461
Unsupported Transport` (`requests_tcp_interleaved_transport`) — go2rtc's own server rejects UDP
the same way. No request carries or is checked against any credential; nothing this crate has
needed to interoperate with today sends one.

Corvette's contract: a client dials a camera by name at `rtsp://<host>:<port>/<camera>`, receives
that camera's SDP from `DESCRIBE` (H.264/H.265 video today; see "No camera's audio track is
published on any live-view transport yet" below), and must request `RTP/AVP/TCP` in `SETUP` — no
other transport or method is honored. A future external consumer of this same restream port
beyond Frigate's own ffmpeg needs exactly this surface and nothing more: five methods,
TCP-interleaved only, no authentication. `RESEARCH-frigate-ingest-boundary.md` F-7 records that
go2rtc's own restream documentation names Home Assistant as a precedent for this kind of consumer,
and that this deployment's `frigate` Service already exposes the port outside the pod today —
though DP-3 in the same document leaves open whether anything currently dials it.

## One camera name identifies its stream across every live-view transport

`crates/corvette-media-bridge` names a configured camera's `CameraSpec::name` field as that
camera's identity on every transport it serves, so a single string is enough for a client to find
one camera across all of them:

- **RTSP-restream** (`restream_provider::MultiCameraProvider::register`): the stream name in
  `DESCRIBE`/`SETUP`'s request URI (`rtsp://<host>:8554/<name>`).
- **MoQ** (`moq_publish::run_publish_once`): `origin.create_broadcast(name, ...)` publishes the
  camera under a broadcast named `<name>`, relative to the relay connect URL's own root path
  (e.g. `https://<relay>:<port>/anon/<name>`, confirmed in both `crates/corvette-media-bridge`'s
  own integration test and `crates/corvette-ui/src/expanded_view.rs`'s `hang` Web Component
  attributes: `url` is the relay connect URL, `name` is the bare camera name). Each broadcast
  carries a `catalog.json` track (from `moq_mux::catalog::Producer`) and a `video` track
  (`moq_broadcast.create_track("video", ...)`) — no `audio` track is created today (see the
  audio-gap section below).
- **fMP4-over-WebSocket** (`ws_repackager`): one WebSocket connection per viewer, at path
  `/<name>` on the repackager's own listener.
- **HLS/LL-HLS** (`hls`): `GET /<name>/playlist.m3u8`, `GET /<name>/init.mp4`, `GET
  /<name>/segment-<sequence>.m4s` on the packager's own listener.

None of these four listeners hard-codes its own bind address or port — each takes one as a
constructor parameter, matching D-3's "configuration point" convention (below) — so the actual
deployed ports are a provisioning concern (K1), not a fact fixed by this crate.

## The MoQ relay is exposed through a NodePort Service, one of three supported mechanisms

Decision of record (D-3, `.agents/issue-12/DESIGN-live-view.md`, human-approved 2026-08-26),
quoted verbatim:

> All three exposure mechanisms (hostPort, NodePort, LoadBalancer) are supported as
> implementation options; NodePort is chosen for this deployment (human judgment: "more secure
> than hostPort"). The exposure mechanism is a configuration point, not hard-coded.

K1's drafted manifest change (`.agents/issue-12/evidence/K1-manifest-draft.yaml`, reviewed, not
yet applied to the live deployment) adds a dedicated `frigate-moq` Service, `type: NodePort`, one
UDP port (`nodePort: 30443`, chosen inside the apiserver's default `30000-32767` range and
colliding with no other port the manifest declares), targeting the new `moq-relay` container's own
`moq` containerPort directly — not proxied through nginx, since nginx does not speak
QUIC/WebTransport. The same container spec works unchanged under `hostPort` or `LoadBalancer`;
only the container's own `ports` entry and whether/how a Service exists would differ (K1's own
evidence spells out both alternatives in full).

The same drafted change moves the `rtsp`-named `containerPort: 8554` off the `frigate` container
onto the new `corvette-media-bridge` container, with no edit to the `frigate` Service's own `rtsp`
port entry: a Kubernetes Service's named `targetPort` resolves against the pod's aggregate named
container ports across every container it has, not only the container the name was originally
declared on (`k8s.io/endpointslice`'s own `FindPort`, read directly and cited in K1's evidence).

Corvette's contract: whichever exposure mechanism a deployment picks, the relay's own connect URL
(the `url` attribute `hang`'s Web Component receives) is the one piece of configuration a client
needs to change — nothing about the naming convention above depends on which mechanism exposes it.

## `/live/hls/` and `/live/mse/ws/` are the two Rust-native live-view routes; five go2rtc routes are retired

`frigate-vulkan`'s nginx proxies two live-view routes to `corvette-media-bridge`, and no longer
proxies five others anywhere:

- **`/live/hls/`** (issue #12 N1, resolving the grid-tile-fallback question left open by the prior
  draft) proxies to G3's HLS/LL-HLS packager. Confirmed serving the expected content types end to
  end (`.agents/issue-12/evidence/N1-mutation.log`): `/live/hls/<camera>/playlist.m3u8` →
  `application/vnd.apple.mpegurl`; `/live/hls/<camera>/init.mp4` and
  `/live/hls/<camera>/segment-<n>.m4s` → `video/mp4`. A donor-config drift guard and
  `scripts/route_parity.sh` both fail if this location is ever silently removed, since its absence
  falls through to the SPA shell at the same `200` status the real route also returns — a
  status-only check would not catch that.
- **`/live/mse/ws/<camera>`** (issue #12 N2, resolving the grid-tile-source question left open by
  the prior draft) proxies to G2's fMP4-over-WebSocket repackager, replacing go2rtc's own
  `/live/mse/api/ws`. `crates/corvette-ui/src/live_view.rs`'s `MEDIA_BRIDGE_WS_PATH_PREFIX`
  constant is this exact path.

Five locations that used to proxy to go2rtc are retired unconditionally, since go2rtc is fully
removed from the deployed image rather than kept for any remaining role
(`.agents/issue-12/evidence/N2-mutation.log`): `/live/mse/api/ws` (replaced by
`/live/mse/ws/<camera>` above), `/live/webrtc/api/ws`, `/live/webrtc/webrtc.html`,
`/api/go2rtc/api`, and `/api/go2rtc/webrtc` — the last four retired outright, including the
embedded WebRTC player itself. Confirmed: the three `/live/*` routes among these
five fall through to the SPA shell once removed; the two `/api/go2rtc/*` routes fall through to
Frigate's own broader `/api/` location instead, since no more specific location matches them
anymore. `/live/jsmpeg/` proxies to Frigate's own `jsmpeg` upstream, confirmed not go2rtc, and is
unaffected by any of this.

## No camera's audio track is published on any live-view transport yet

The "RTSP camera frames carry Annex-B video, not a container format" contract above already
records the root cause: `corvette-rtsp-client`'s SDP resolution locates only a camera's `m=video`
section, so no audio `Frame` is ever produced. That gap propagates through every consumer built on
top of it, each confirmed directly rather than assumed:

- `crates/corvette-media-bridge`'s MoQ-publish role (`moq_publish::new_track`) is written
  generically over `corvette_rtsp_client::depacketize::Codec`, but its `Codec::Aac` arm can only
  return `FrameError::AacUnavailable` — `aac::Import::new` needs a resolved sample rate, channel
  count, and `AudioSpecificConfig` this crate has no source for without the upstream fix. No AAC
  frame has ever been exercised end to end, including in this crate's own integration test.
- The expanded view's UI (`crates/corvette-ui/src/expanded_view.rs`) is written to degrade to
  video-only cleanly when a broadcast announces no audio track, by design, rather than assuming
  one is always present — but has nothing to degrade from today, since no broadcast ever announces
  one.
- The grid tile's fMP4-over-WebSocket repackager (G2) and the HLS/LL-HLS packager (G3) both
  inherit the same video-only scope from the same upstream cause.

Corvette's contract: every live-view transport this document describes — RTSP-restream, MoQ, the
fMP4/WebSocket grid tile, and HLS/LL-HLS — carries a configured camera's video unconditionally, and
its audio only once `corvette-rtsp-client` resolves an SDP audio section, a separate, unscoped
future item. No component in this list needs to change shape when that fix lands: each already
dispatches on a frame's own codec generically.
