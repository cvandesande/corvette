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
playable footage. The server issues the link for such a range through the
range-link route, `GET /corvette/api/v1/clip-link`.

## Ranges use half-open overlap

Every activity query uses half-open `[after, before)` bounds. A review belongs to
the range when it overlaps it: it starts before `before`, and it is ongoing, ends
after `after`, or starts at or after `after`. The last clause keeps a zero-length
review that sits exactly on a boundary in the later range. A review that crosses a
boundary appears in both adjacent ranges with the same `id`, and clients
deduplicate by `id`. A motion bucket belongs to the range that contains its start
(see "Motion aggregation stays server-side"), so each bucket lies in exactly one of
two adjacent ranges. Adjacent ranges therefore cover the timeline without dropping
a record at a boundary.

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
the authoritative rows; Rust selects the greatest motion count and collects the
participating camera set for each bucket, streaming the ordered rows and holding one
open bucket at a time.

Buckets are 30 s wide and start at multiples of 30 s since the epoch. A recording
row with a positive motion count belongs to the one bucket that contains its
`start_time`. A bucket is returned when its start lies in `[after, before)`, and its
level and cameras come from all of its rows, including rows that start at or after
`before` in a bucket the range only partly covers. The query therefore selects rows
whose `start_time` lies in `[ceil30(after), ceil30(before))`, where `ceil30` rounds
up to a multiple of 30. A bucket has the same value however the timeline is split.

The level is the raw motion count, not normalized. Frigate rescales each hour of buckets
(120 at its default 30 s scale) to 0..100 on its own, so its values are not comparable
across runs or across adjacent queries. Clients scale the raw count for display over the
span they draw.

The API returns compact activity buckets rather than raw recording telemetry.
Sending storage-shaped rows would increase transfer size, repeat aggregation in
every open browser, and expose an implementation detail instead of the activity
contract clients need.

## nginx-vod mapping remains available

nginx uses `vod_mode mapped` with `vod_upstream_location /api`. It asks the API
for a JSON mapping from a playback request to files on disk, then reads those
files itself. Any service taking ownership of `/api` must continue answering the
mapping request until the playback architecture changes.

The playlist route contract "A recorded range plays through one playlist
route" is served by an nginx-vod location of its own, and the mapping request
that location makes must reach the same lookup, so the lookup keeps answering
for as long as the playlist route exists.

**Current backing**: nginx hands that mapping request to the backing's
existing lookup route through an internal location that sits outside the
`/corvette/api/v1/media/` prefix.

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

The UI test in `crates/corvette-ui/src/lib.rs` enforces this contract from two
hand-maintained mirrors. `RESERVED_NGINX_PREFIXES` mirrors the current
backing's own nginx locations; its reference copy is the vendored file
`tests/nginx-parity/vendor/nginx.conf`, a byte-for-byte upstream copy that is
never edited (per its `PROVENANCE`). Each entry keeps the trailing slash nginx
matches it with — all carry one except `/ws` and `/auth`, which nginx matches
without. `CLIENT_ROUTES` mirrors the `<Route>` declarations the two `App` cfg
branches share; a test has no way to introspect those declarations. A new
client route goes into `CLIENT_ROUTES`, and a new backing-location prefix goes
into `RESERVED_NGINX_PREFIXES`; the reserved list gains no `/corvette/` entry,
because Corvette's own API lives under `/corvette/api/v1/` and no client
route claims it.

The `recordings` exception stays declared until a deliberate client-route
rename resolves the collision. The test fails when a declared collision stops
matching the record, so the exception cannot go stale silently.

Every on-screen navigation href comes from `shell::NAVIGATION`, which a
source-text grep for `href="..."` literals cannot see; the test inspects that
array instead. A self-route href there ends without a trailing slash because
one would send a click to nginx's resource for that prefix instead of the SPA;
`"/"` and hash-fragment links are exempt — neither is a narrower path a
trailing slash could misroute.

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
(RFC 3640, `mpeg4-generic`) already is that raw access-unit shape.

Today the crate publishes video only. Its SDP resolution
(`session::sdp::resolve_video_track`) locates only the SDP's `m=video`
section, so no audio track is ever resolved against a live camera session
and no `AacDepacketizer` is constructed from one — even though the AAC
depacketizer itself exists and is unit-tested against synthetic RTP
payloads. A camera's audio, if it has any, is not read today.

Corvette's contract: every frame `crates/corvette-rtsp-client` publishes is
either Annex-B-framed H.264/H.265 or a raw, non-ADTS AAC access unit — never
MPEG-TS, fMP4, or any other container — so a downstream consumer can consume
each frame directly, with no demuxing step in between. The
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

## The fMP4-over-WebSocket transport carries no per-viewer timestamp epoch

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

Corvette's contract: a consumer of this transport (the browser check named above is its only
consumer today) sets `sourceBuffer.mode = "sequence"` before appending
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

## The restream feed resolves each camera's parameter sets once, from its own frames

`crates/corvette-media-bridge`'s restream provider adapts `corvette-rtsp-client`'s frame
stream to the RTSP-restream server's frame and stream-provider shapes, dispatching by
stream name across every configured camera's own adapter. The parameter-set scan is the
one piece of plumbing this pairing needs: the restream server needs a camera's parameter
sets up front to answer `DESCRIBE`, and the `Client` type this module uses exposes only
the frame stream; the SDP-resolved track description that also carries them is a separate
public type in the client crate's `session` module, which this module does not use, so
the parameter sets are scanned out of the frame stream itself. The scan
itself lives in the bridge's `parameter_sets` module, shared with the fMP4 muxer.

The provider's own policy on top of that scan is to resolve once: the first complete set
for a codec (SPS/PPS, or VPS/SPS/PPS for H.265) resolves the camera's `StreamInfo`, and
it is never updated again even if the underlying bytes later change; until a camera has
resolved, `DESCRIBE` reports it as no such stream. An AAC track never resolves here even
after the upstream audio gap is fixed: an AAC stream's configuration is an
`AudioSpecificConfig`, not an in-band parameter-set NAL, so the audio branch is a
permanent no-op -- written generically over the frame's own codec so the upstream fix
needs no change in this module.

The camera set is fixed at startup: every configured camera is registered before any feed
task spawns, and the registry is never mutated afterwards, so only each camera's own
state needs synchronization. A duplicate registration is a caller bug, not a runtime
condition to recover from. The internal channel carrying adapted frames from a camera's
feed task to the restream server is buffered at 64 frames, matching the documented
default capacity of `corvette_rtsp_client::CameraConfig`.

The restream server hands frames to viewers through a synchronous receiver, so the
bridge's receiver drives its async channel with `block_on` -- sound because the server
calls it from a blocking play-task context. A subscriber that lags behind skips forward:
the server's per-connection play task has no resync concept, so a lag is treated like a
bad packetize rather than a stream end. When the feed task ends, the receiver reports no
further frames, matching `FrameReceiver::recv`'s contract; a send with no current PLAY
session is ignored, the same no-listeners convention `corvette-rtsp-client`'s own
per-camera task uses. Each feed task runs under the bridge's supervisor until its
camera's client is dropped -- which happens only during process shutdown, so the
supervisor's restart loop spinning on a closed channel from then on is acceptable.

## The bridge's integration tests feed a synthetic camera alongside a real RTSP dial

The `corvette-media-bridge` integration tests (`tests/g2_integration.rs`,
`tests/g3_integration.rs`) each drive one live packager from two cameras: a
mock RTSP camera server (`corvette_rtsp_client::mock_camera`) dialed through
a real `corvette-rtsp-client`, and a synthetic camera whose feed publishes
onto the same broadcast-channel shape a real camera client's subscription
exposes, so the packagers consume it exactly as they consume a dialed camera.

The mock camera fabricates RTP payloads with no real SPS/PPS: it was built
for the depacketizer's NAL/FU-A framing, not for a decodable stream, so that
camera's initialization segment never resolves. The leg earns its place for
the real-RTSP path it exercises -- handshake, reconnect and task wiring --
and for the graceful degradation of its frame-error handling: a keyframe
without a matching SPS degrades without a panic. The synthetic camera
supplies a real, valid H.264 SPS/PPS/IDR triple, so the served outputs
resolve end to end.

Stopping a camera's supervised tasks only requests cancellation; the tests
pause after the stop until the runtime has released the aborted tasks' state
before asserting that the other camera's outputs continue.

## The HLS packager is tested over real HTTP and through a real external parser

`tests/g3_integration.rs` fetches the packager's playlist and segments over real HTTP;
`tests/g3_ffprobe.rs` hands the same bytes to `ffprobe`, a real, independently authored
CMAF/HLS parser, so acceptance rests on a parser this repository does not author. The
packager path needs no external binary, so both tests run under `cargo test`.

Both tests issue raw `TcpStream` GET requests and read each response to its end: the
workspace has no HTTP client crate, and the packager answers every response with
`Connection: close`, so the end of input is the end of it.

Both feed one synthetic camera through the broadcast-channel shape described in "The
bridge's integration tests feed a synthetic camera alongside a real RTSP dial". The HTTP
test pairs it with a mock RTSP camera dialed by a real client; the
mock camera's frames carry no real parameter sets, so its initialization segment never
resolves, yet its playlist is still served: a registered camera's playlist always exists,
so before resolution it reads as a valid empty live document (200, no map line, no
segments) rather than an error -- browser-based validation established the empty document
as the choice that avoids a player-visible race. Stopping that camera's client and task
leaves the synthetic camera's served output unchanged; the test checks that directly.

The `ffprobe` test feeds its SPS/PPS/IDR triple byte-for-byte from
`examples/g2_browser_fixture.rs`, whose own doc records its provenance: a real encoder's
single-frame baseline output. The shorter fixture the other bridge tests share is valid
enough for container-level parsing but carries no encoded macroblock data; `ffprobe`
attempts to decode a frame while probing stream parameters and rejected it ("non-existing
PPS referenced", "sps_id out of range").

The fetched playlist and segments are written as real files in one directory because
`ffprobe`'s HLS demuxer resolves relative segment URIs against the directory holding the
`.m3u8`. The served playlist is live and carries no `#EXT-X-ENDLIST`; the tag is appended
to the on-disk copy alone, because the demuxer treats its absence as a stream still being
appended to and would wait for a playlist reload a static copy never provides -- what the
server serves is unchanged. The run is bounded at 30 seconds so a hung parse fails loudly
instead of blocking the suite.

`ffprobe` reaches the test through the `flake.nix` devShell (`pkgs.ffmpeg`); when missing,
the test fails with a message naming `nix develop`. The stream assertion checks two
independent substrings of its csv output rather than one joined line, so it does not
depend on that output's field order.

## One camera name identifies its stream across every live-view transport

`crates/corvette-media-bridge` names a configured camera's `CameraSpec::name` field as that
camera's identity on every transport it serves, so a single string is enough for a client to find
one camera across all of them:

- **RTSP-restream** (`restream_provider::MultiCameraProvider::register`): the stream name in
  `DESCRIBE`/`SETUP`'s request URI (`rtsp://<host>:8554/<name>`).
- **fMP4-over-WebSocket** (`ws_repackager`): one WebSocket connection per viewer, at path
  `/<name>` on the repackager's own listener.
- **HLS/LL-HLS** (`hls`): `GET /<name>/playlist.m3u8`, `GET /<name>/init.mp4`, `GET
  /<name>/segment-<sequence>.m4s` on the packager's own listener.

None of these three listeners hard-codes its own bind address or port — each takes one as a
constructor parameter — so the actual deployed ports are a provisioning concern, not a fact
fixed by this crate.

## The media bridge's serve future never returns, and a listener panic reaches its caller

`crates/corvette-media-bridge` hosts three whole-process listeners, and all three of their `serve` methods return `-> !` (`rtsp_restream::RtspServer`, `ws_repackager::Fmp4WsServer`, `hls::HlsServer`), so the listener future `start` hands its caller never returns on success. A listener panic reaches that caller as an unwinding panic: the RTSP listener is awaited directly, while the fMP4-WebSocket and HLS listeners' join arms `resume_unwind` whatever their spawned tasks return, since a `-> !` task can only ever end by panic; the other spawned listener task keeps running until that unwind ends the process.

## A Kubernetes Service resolves a named target port across the pod's containers

A Kubernetes Service's named `targetPort` resolves against the pod's aggregate named container
ports across every container in the pod, not only the container the name was originally
declared on (Kubernetes' own `k8s.io/endpointslice` `FindPort`, read directly). This is what
lets the `rtsp`-named `containerPort: 8554` be served by the `corvette-media-bridge` container
while the `frigate` Service's own `rtsp` port entry keeps its name unchanged.

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
  `/live/mse/api/ws`.

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

## The HLS packager serves a sliding-window CMAF playlist

The HLS packager (`crates/corvette-media-bridge`, `hls`) cuts each camera's fragmented-MP4
stream into CMAF media segments of a 2-second target duration. The duration trades live-view
latency against request volume: shorter segments keep a viewer closer to live, and each
segment boundary costs the viewer one more segment request, so the chosen target keeps
segment-request overhead to about one request every two seconds per viewer. The 2-second
target sits inside the 1 to 6 second range real HLS deployments commonly use; Apple's HLS
authoring guidance recommends around 6 seconds for broad compatibility, while a LAN-local
expanded-view fallback favors the low end of that range for lower glass-to-glass latency.

A segment is cut only on a keyframe, so every segment after the first is independently
decodable from its own first sample. The very first segment of a camera begins with whatever
access unit the fragmenter emits first; a real camera conventionally emits its parameter sets
immediately followed by an IDR, so that first segment is expected, not guaranteed, to start
at a keyframe.

Each playlist is a sliding window of the 6 most recent segments (12 seconds at the chosen
target duration): a live playlist is not an ever-growing manifest, so the window bounds the
packager's per-camera memory while covering the few-segment buffer a live player keeps.

The packager's own listener serves these per-camera paths behind the `/live/hls/` prefix that
the nginx proxy strips, so the packager itself never sees that prefix; the paths are listed
under "One camera name identifies its stream across every live-view transport".

The playlist is standard segmented HLS: no partial segments, no preload hints, no blocking
playlist reload. Every response carries `Connection: close`, so each playlist or segment
poll costs a fresh TCP handshake — one request per connection keeps the packager minimal,
and a player's periodic polling simply opens the next connection.

A camera's served state — the resolved initialization segment, the live segment window and
the next sequence number — is a plain mutex-guarded snapshot rather than the
`watch`/`broadcast` pair `ws_repackager` uses: an HTTP client pulls a snapshot on its own
schedule instead of being pushed updates over a held-open connection, so there is no
"no receivers yet" race to solve, and every write simply replaces the snapshot.

Every accepted connection carries a five-second bound covering reading its request and
preparing its answer; a buffered response is written inside that same bound, and a streamed
body continues under the per-write bound its route states. Without it, a peer that opens a
connection and never finishes sending
would leave the read loop parked forever, leaking one task and one file descriptor per
connection — a live deployment showed roughly seventy such stuck connections after less
than an hour. Five seconds is a generous bound because nginx is this listener's only real
client, one LAN-local hop away, and every request resolves in microseconds once its bytes
arrive: a few in-memory lookups and no I/O.

Every response carries `Access-Control-Allow-Origin: *` unconditionally. The listener serves
no per-viewer state and no credentialed request, so an unrestricted origin costs nothing and
lets a browser-hosted player on another origin fetch the playlist and segments; behind the
nginx proxy every real request is same-origin, so the header is inert in a real deployment.

The listener bounds what one connection may make it read. A request header block is capped
at 8192 bytes so that one misbehaving connection cannot grow the read buffer without bound.
The header-line cap is 64: nginx forwards every header the browser sent plus its own, so a
real request carries two to three times the headers a bare client sends; the earlier limit
of 16 was too low for real browser traffic, and 64 leaves headroom.

## The fMP4 muxer packages camera video into browser-feedable fragments

`crates/corvette-media-bridge`'s `fmp4` module is the muxer shared by the fMP4-over-WebSocket transport
and the HLS packager: one initialization segment per camera (`ftyp`/`moov`, rebuilt whenever the camera's
own parameter sets genuinely change, so a viewer that connects after a resolution change receives the
current segment and a camera's periodic in-stream re-announcement of unchanged sets changes nothing)
plus one `moof`/`mdat` fragment per access unit.

Samples are written in AVCC (length-prefixed, four-byte) framing, matching the `avcC`/`hvcC` boxes' own
`lengthSizeMinusOne = 3` declaration, not the wire's Annex-B framing. The camera's frame stream carries
one NAL per frame, so an H.264 coded slice (non-IDR 1, IDR 5) is a complete video access unit; for
H.265 the coded slice segment types 0-31 are VCL and the IRAP keyframe range is 16-23 (RFC 7798 /
ITU-T H.265 Table 7-1). Parameter-set NALs feed the initialization segment instead, and the remaining
non-VCL NALs a real stream may carry (SEI, AUD, filler) hold no sample data a decoder needs and are
dropped -- the shared contract is one fragment per access unit, not full NAL-unit fidelity.

The very first sample of a fragment stream carries a default duration -- 30 fps at the 90 kHz video
clock -- because no interval has been measured before a second access unit arrives; the second fragment
replaces it with the real measured interval, and a browser's `SourceBuffer` accepts that approximate
first-sample duration, so no viewer rejects a first fragment over it.

Each fragment carries the `moof`/`mdat` bytes plus two facts that plain bytes do not expose without
re-parsing: whether it holds a keyframe (sync) sample, and its own sample duration in the same RTP
tick units as the frame timestamps. The HLS packager uses both facts to cut a segment on a keyframe,
once a segment has grown long enough, without re-parsing the `moof` it was just handed.

`trun`'s `data_offset` is relative to the first byte of the containing `moof` box (the
`default-base-is-moof` flag), so `moof`'s own final size must be known before the field can be patched
in: the builder builds `moof` once to learn that size, sets `data_offset` to it plus `mdat`'s eight-byte
header, and patches the field at its fixed byte offset -- the boxes and headers preceding it sum to 84
bytes (moof header 8, `mfhd` 16, `traf` header 8, `tfhd` 16, `tfdt` 20, `trun` header 8, `trun`
flags 4, sample count 4).

The initialization segment declares a fixed placeholder width and height (1920x1080): they affect
rendering geometry only -- a real `SourceBuffer` accepts the segment regardless -- while the true
dimensions would mean parsing the SPS's exp-Golomb-coded fields for a property no consumer asserts on.
Sample entries share the 78-byte `VisualSampleEntry` header of ISO/IEC 14496-12 section 12.1.3.

The `ftyp` brands advertise what the writer emits: `iso5` as the major brand signals
`default-base-is-moof` support; its compatible-brand list names `iso5`, `iso6`, `isom` and `mp41`,
with `isom` and `mp41` listed as widely, conservatively supported brands.

The `avcC` record copies `AVCProfileIndication`, `profile_compatibility` and `AVCLevelIndication`
starting at index 1 of the stored SPS bytes: the stored SPS includes its own one-byte NAL header
(RFC 6184 section 1.3) and those three fields begin immediately after it.

The `hvcC` record (ISO/IEC 14496-15 section 8.3.3.1) takes real values only for the
`profile_tier_level` fields that sit at fixed bit offsets in the SPS -- general profile, compatibility
flags, constraint indicators and level, the first 12 bytes of `profile_tier_level` per ITU-T H.265
section 7.3.3, reachable without exp-Golomb decoding -- and gives every other field a conservative
common default; an SPS too short to hold these fields yields all-zero bytes, defensive only, never
expected from a real camera. That yields a structurally valid record; browser-verified evidence covers
the H.264 path, and no fixture verifies `hvcC` against a real H.265 decoder.

## The camera-list contract carries Frigate's own detect resolution, unscaled

`corvette_api::Camera` (`crates/corvette-api/src/lib.rs`) carries `width`/`height` fields sourced
from each camera's `detect.width`/`detect.height` in Frigate's own `/api/config` response, verbatim
and unscaled. Frigate v0.17.2 (the tag this deployment pins, `frigate-vulkan/docker/Dockerfile.py313:18`)
models these as `Optional[int]` on `DetectConfig` (`frigate/config/camera/detect.py`) — either can be
absent from raw user configuration — but `frigate/config/config.py:530-552` resolves any unset value
from the camera's own stream before the config is ever served, so a running deployment's
`config_obj.model_dump(...)` (`frigate/api/app.py:115-116`) always serializes concrete integers for an
enabled camera. Confirmed directly against the real deployed Frigate (namespace `icams`, tirnanog
cluster): `/api/config` returns `detect.width`/`detect.height` as populated integers for both enabled
cameras (`front`, `back`: 704x576), and this same response parses cleanly through
`corvette_api::FrigateConfig`/`Camera`.

Corvette's contract: `Camera.width`/`height` are Frigate's own `detect.width`/`detect.height`
verbatim — no client-side derivation, scaling, or defaulting when the upstream value is present. A
consumer needing a camera's aspect ratio computes it from these two fields directly rather than
inferring it from stream probing, a hard-coded default, or any other source.

## Corvette's own API lives under `/corvette/api/v1/`

Corvette's routes live under `/corvette/api/v1/`. Frigate keeps serving everything under
its own `/api/` unchanged, and the two route trees are separate. Camera names appear as
segments inside Frigate's `/api/` routes, so a Corvette segment placed there could collide
with a camera; a separate top-level prefix cannot. One nginx block,
`location ^~ /corvette/api/`, covers every current and future Corvette route, so adding a
route needs no nginx change. The `^~` modifier ranks it ahead of Frigate's unanchored
`/api/` image regex, so `/corvette/api/v1/x.jpg` reaches Corvette and gets its `404` body
(confirmed against the deployed pod).

nginx sends every request to the prefix through an identity check before forwarding it —
a subrequest to Frigate's `/auth` today — and sets `Remote-User` and `Remote-Role` from
that response, so a browser cannot supply them. On the authenticated port (8971) the
check requires a login; on Frigate's internal port (5000) it grants every caller the
`admin` role, so that port must stay inside the cluster. nginx forwards `GET` and
`HEAD` and answers any other method with `403`; the service's parser accepts nothing but
`GET` and answers `400` to anything else, so a `HEAD` request receives `400`. The
service has no TCP listener: it binds a Unix socket file in a volume shared only with the
`frigate` container, where nginx runs, which is what keeps the identity headers
unspoofable from outside that container.

Identity is required before routing: a request that reaches the service with a missing or
empty `Remote-User` gets `401` whatever the path, so an unauthenticated caller learns
nothing about which routes exist.

The service bounds the size of a request header block, so one connection cannot grow its
buffer without bound, and bounds the whole life of a connection, so a peer that stalls
cannot leak a task and a file descriptor. It stops on SIGTERM, the container stop signal,
or on Ctrl-C at a terminal. The configuration fetch is bounded in time, so a stalled
upstream cannot hold a request open, and in size, so an oversized response is a broken
upstream and not a configuration worth parsing.

Every response the service sends — success and error alike — is JSON with
`Content-Type: application/json`, with two exceptions: the still route's `200` is the
camera's picture, `Content-Type: image/jpeg`, while every error the still route sends is
JSON like every other route's; the errors the configuration routes answer are
`application/problem+json`, the RFC 9457 problem details the configuration routes
define. Every response carries `Cache-Control: no-store` and
closes the connection. Responses nginx generates itself — `401` from a failed identity check, `403`
for a refused method, `502` when the service is unreachable — carry nginx's own HTML
body instead. The other service errors share one shape, `{"code": ..., "message": ...}`,
where `code` is a stable machine value to branch on and `message` describes the failure:

- `bad_request` (`400`): the request could not be parsed; `message` names the reason,
  including a malformed camera segment or `height` on the still route.
- `unauthorized` (`401`): no or empty `Remote-User`.
- `not_found` (`404`): a path under the prefix that names no route, the range-link route
  answering for a camera that does not exist, that the caller's role may not see, or that
  has no recorded footage overlapping the requested range, or the still route answering
  for a camera that does not exist or that the caller's role may not see.
- `config_unavailable` (`503`): the cameras, activity, range-link, still, status, objects or detection status route
  has no usable Frigate configuration.
- `database_unavailable` (`503`): the activity or range-link route could not read Frigate's
  database. The underlying error is logged, not returned.
- `recording_unavailable` (`503`): the recordings route or the events routes could not read
  Corvette's recording index, because the file is absent, unreadable or corrupt. No answer or log line carries
  the underlying error or names the index path. A read-only mount of the index cannot be
  served between recorder runs, after the recorder closes it cleanly, until the recorder
  reopens it; that case answers this code. The status route answers the same failure
  inside its `recording` field instead.
- `range_unsatisfiable` (`416`): the playlist route could not satisfy the request's
  `Range` header on a segment; the answer names the total length and nothing else.
- `no_picture` (`404`): the still route found a camera the caller may see that is switched
  off.
- `picture_unavailable` (`503`): the still route could not get a usable picture from the
  backing; the reason is logged, not returned.
- `status_unavailable` (`503`): the status route could not get a usable stats answer from
  the backing; the reason is logged, not returned.
- `detection_unavailable` (`503`): the objects, detection status or configuration routes
  could not get a usable answer from the detection service; the reason is logged, not
  returned.
- `unhealthy` (`503`): the health route found failing parts; `message` names each one.

The detection configuration routes answer six more codes, in their problem-details
bodies:

- `forbidden` (`403`): a `PUT` from a role that may not write.
- `stale_revision` (`412`): `If-Match` does not match the current revision.
- `revision_required` (`428`): a `PUT` arrived without `If-Match`.
- `invalid_config` (`422`): the submitted value failed validation.
- `source_not_allowed` (`422`): a camera in the submitted body carries a `source` table; stream
  details arrive only by editing the file, and the refusal never echoes a value.
- `apply_incomplete` (`500`): the file write failed and nothing changed.

`GET /corvette/api/v1/health` answers `200` `{"database":"ok","config":"ok"}` when both
probes pass, and `503` `unhealthy` naming each failing part otherwise. The database probe
opens Frigate's SQLite database strictly read-only — the service never writes it — and
consists of one short read proving a Frigate table is queryable.

`GET /corvette/api/v1/cameras` answers `200` `{"cameras":[...]}`, one entry per camera the
caller's role may see, cameras that are off included, in dashboard order: `ui.order`, then
camera name. Each entry carries `name`, `display_name`, `order`, `detect_width`,
`detect_height` and `enabled` (`true` when the camera is switched on, `false` when it is
switched off by choice); for a consumer, an entry that lacks `enabled` reads as `true`.
The list is filtered by role, so a role limited to one camera receives only that camera's
entry; Frigate's `/api/config` returns every camera to every user, which is the reason
the route exists. The role rule: a role absent from the configured `auth.roles` —
including the empty role of a request that carries no `Remote-Role` — sees no cameras,
and a configuration with no `auth.roles` at all grants nothing; a role with an empty
camera list sees every camera that exists; otherwise the role sees exactly the listed
cameras that exist — off cameras kept — and a listed name that no camera has is dropped.
**Current backing**: `enabled` is the camera's runtime `enabled` flag in the backing's
configuration copy, the flag the backing's enable/disable command sets. The route
answers `503` `config_unavailable` when no Frigate configuration copy younger than 10 s
can be served.

`GET /corvette/api/v1/activity?after=&before=&cameras=&include=` answers the reviews and
motion of a time range for the cameras the caller's role may see:

- `after` and `before` are required Unix seconds, decimal fractions allowed, finite and not
  negative, with `after < before` and a range of at most 31 days (2678400 s).
- `cameras` is an optional comma-separated list of camera names. Absent, it means every
  camera the role may see, the set the cameras route lists, off included: a camera that
  is off now returns the reviews and motion recorded for it while it was on. Present,
  it narrows that set. A name that is unknown or not allowed for the role is dropped
  without an error, so the route reveals nothing about cameras the caller cannot see.
  **Current backing**: the backing selects its database rows by camera name, so the
  rows of a camera that is off are returned like any other camera's.
- `include` is an optional comma-separated subset of `reviews,motion`; absent means both,
  and an omitted kind is returned as an empty array.
- Names and values are percent-decoded (`+` stays `+`). An unknown, repeated, or
  value-less parameter, an empty segment (`&&` or a trailing `&`), a malformed escape or one
  that decodes to invalid UTF-8, or a value outside these rules answers `400` `bad_request`
  with a message naming the reason. An empty `cameras` value names no camera and answers
  empty arrays.

Decision of record (DP-3, `.agents/issue-12/DESIGN-main-page.md`, human-approved
2026-10-17), quoted verbatim: "yes, keep those events visible".

The query is checked before the configuration is fetched, so a malformed request answers
`400` even while Frigate's configuration is unavailable. When the role's cameras leave
nothing to read, the route answers `200` with empty arrays without opening the database.
Otherwise it reads the requested kinds in one short read-only transaction and closes the
database before it checks media files.

The response is
`{"after", "before", "motion_bucket_seconds": 30, "reviews": [...], "motion": [...]}`:

- `after` and `before` echo the request.
- Each review carries `id`, `camera`, `severity`, `start`, `end` (`null` while the review is
  ongoing), the sorted lists `objects`, `zones`, `detections` (event IDs), `sub_labels`,
  and `audio` from Frigate's review data, and `media: {"thumbnail", "clip"}`. Reviews are
  ordered by `start`, then `id`. Labels pass through verbatim, including Frigate's
  `-verified` suffix. Per-user review status is not returned: it belongs to a user
  identity Corvette does not have until issue #30.
- Each motion entry is `{"start", "level", "cameras"}`, one per 30 s bucket with motion,
  ordered by `start`, with `cameras` sorted; buckets without motion are omitted.

Media links follow "Media availability is explicit". Snapshots belong to detections, not to
reviews, so a review's `media` carries only its thumbnail and its clip. Each link is
`{"url": ...}` when the media is available now and `null` otherwise:

- `thumbnail` is present when Frigate's stored thumbnail path lies under `/media/frigate/`,
  contains no `..` segment, does not begin with `.`, and names a file that exists. The
  service checks the file under its media root (`CORVETTE_API_MEDIA_ROOT`, default
  `/media/frigate`), which must be Frigate's media volume, mounted read-only, so a thumbnail
  deleted outside Frigate is reported unavailable. The URL is the stored path with the
  `/media/frigate` prefix removed, each segment percent-encoded, which nginx serves under
  `/clips/` (see "Thumbnails under `/clips/` carry no usable content type").
- `clip` is present when the review has ended and recorded footage of its camera overlaps
  `[start, end)`. A review with no end has `clip` when its start is before the moment of
  the request and recorded footage of its camera overlaps `[start, moment of the
  request)`, and the link's range ends at the moment of the request, which differs from
  request to request. The URL is the address of "A recorded range plays through one
  playlist route",
  `/corvette/api/v1/media/{camera}/start/{start}/end/{end}/master.m3u8`, with the camera
  percent-encoded and the review's start and end padded as that section states (the moment
  of the request in place of the end for a review with no end). The answers that route gives
  for a range it cannot serve are those of that
  section. **Current backing**: a recording is a row of the backing's database, and a
  recording file deleted outside Frigate while its row remains is not detected, as
  Frigate does not detect it.

Which store answers the route is chosen at startup by `CORVETTE_API_ACTIVITY_BACKING`:
`frigate`, the default, reads Frigate's database as described above; `corvette` reads
Corvette's own recording index, through the same reads as `/corvette/api/v1/events`; any
other value is a configuration error and the server does not start.

Under the `corvette` backing each review is the form the events list route gives: `id` (a
number), `camera`, `severity`, `start`, `end`, and `media` carrying only `clip`. It has no
`thumbnail`, and no `objects`, `zones`, `detections`, `sub_labels` or `audio` list:
Corvette builds no review thumbnail and keeps no per-review summary lists, so a consumer
must treat `thumbnail` as absent, not as a deletion event. Its severities are `alert` and
`detection`, the severities the index holds. Camera selection, ordering and window overlap
are as above; `media.clip` is computed from the index's pieces with the same half-open
overlap and the same padding.

The `corvette` motion source is Corvette's own detection state: a second counts when the
detection motion stage triggered in it, and the index holds one tick per camera per
second. Buckets use the same 30 s arithmetic and bounds as above; a bucket's `level` is
the largest number of motion seconds any one camera holds in that bucket, and `cameras`
lists each camera with a motion second in the bucket, sorted. Because this is a measured
source and Frigate's `recordings.motion` counts are the backing's, the same interval can
answer different `level`s from the two backings, and ticks exist only where Corvette's
detection ran.

An unreadable index answers `503` `recording_unavailable`, the events routes' code; the
Frigate backing answers `503` `database_unavailable` when its store cannot be read.

When Corvette owns login (issue #30), and later its own camera configuration, these
response shapes stay the same; only their sources change.

## The page's request helpers report failures as text and treat two absences as empty results

Every request through the page's request helpers reports failure as a `String`
naming the path and what went wrong, for display to the reader. Exactly two
absences are reported as an empty result rather than as an error, and only
these two: a camera that has not been chosen yet, and a preview range the
backing answers `404` for because it has no preview videos. Every other
unsuccessful status is an error.

## A recorded range plays through one playlist route

Corvette serves a recorded camera range to a browser as one playlist route,
so every browser can play a clip, scrub it, and start the picture before the
whole clip has loaded.

The address is
`GET /corvette/api/v1/media/<camera>/start/<start>/end/<end>/master.m3u8`.
`<camera>` is a percent-encoded path segment; `<start>` and `<end>` are Unix
seconds, decimals allowed. Clients do not build this address: they take it
from a link the server issued, either a review's `media.clip` or the range-link
route's answer, as "Media availability is explicit" requires.

Every clip link carries four seconds of lead-in before its start, with the
start clamped at zero, and its end is four seconds after a stored end but
never later than the moment of the request. A review still in progress
therefore ends at the moment of the request. A clip is offered only when the
range without the padding has a recording overlapping it, so the padding
never creates a link on its own. The figure matches the review padding of
the current backing's own interface, where it is a client-side constant
(`REVIEW_PADDING`, `web/src/types/review.ts:71` at v0.17.2).

The playlist's variant, index, init and segment URIs are relative and
resolve under the route's own directory (`…/start/<start>/end/<end>/`), so
one nginx location covers the whole directory.

The playlist's media type is `application/vnd.apple.mpegurl`. Segments are
byte-range capable: a request with a `Range` header on a segment answers
`206` with `Content-Range`.

The route accepts `GET` only. A request that carries no signed-in identity
is refused by the identity check with its `401` or `403`; a method other
than `GET` is refused with `403`.

A range with no recording retained, and an inverted range (`start > end`),
each answer `404` with code `not_found`, in the JSON error shape
`{"code": ..., "message": ...}` this file defines for service errors
above, where the code is the stable value. This `404` is the one answer
nginx itself generates in this JSON shape. An empty range (`start == end`)
is served like any other range: it answers `200` with a playlist when a
recording holds that instant, and answers like a range with no recording
(the `404` above) when none does. A range longer than 31 days
(2678400 s), the bound the activity route places on its own window, answers
the same `404`, and so does a range whose index rows number more than
25000; that answer arrives before any stored piece file is read.

A clip starts at or before the requested second. **Current backing**: the
backing pulls the start back to the previous keyframe of the first
recording, or to the start of that recording when no keyframe can be read.

A clip carries the audio its recording holds; audio presence follows the
camera's recording settings and the contract promises nothing more.

Every recording row that overlaps the requested range is served.
**Current backing**: the backing skips a recording whose resulting duration
is under 100 ms, or 600 s or more, where the resulting duration is the
recording's duration trimmed to the requested range and pulled back to the
previous keyframe; a range for which every recording is skipped answers
like a range with no recording. Whether the deployed recordings ever reach
600 s is unverified.

## The playlist route answers four files under the range's directory

The playlist address names a directory. Four playlist files resolve under it
with relative URIs and no leading slash: `master.m3u8`, `index.m3u8`,
`init-<run>.mp4` and `seg-<n>.m4s`; `download.mp4` resolves in the same
directory and is specified by "A recorded range downloads as one stitched
MP4". `<run>` numbers a run of stored pieces
that share one set of stream parameters, and `<n>` numbers a piece of the
requested range; both count from 0 and are written as plain decimal
integers. A `<run>` or `<n>` in any other spelling, and any range with no
overlapping piece, answer `404` `not_found`; a file name that is not one of
these five is no route at all.

The playlists carry media type `application/vnd.apple.mpegurl`; init and
segment files carry `video/mp4`. Every answer under the directory carries
`Cache-Control: no-store`, because retention can delete pieces at any
moment.

Pieces are ordered by their start and numbered `seg-0`, `seg-1`, … over the
half-open range, so one request answers the same names for as long as the
index holds the same pieces. A piece begins a discontinuity when it starts
more than one second after the previous piece ends, or when its stream
parameters differ from the previous piece's; the parameters are read from
the piece's own stored init prefix.

`master.m3u8` names the single variant `index.m3u8` and carries an
`#EXT-X-STREAM-INF` line whose `BANDWIDTH` is the range's stored byte total
multiplied by eight and divided by its stored duration total, zero when the
stored duration total is zero. `index.m3u8` is the media playlist:
`#EXTM3U`, `#EXT-X-VERSION:7`, `#EXT-X-PLAYLIST-TYPE:VOD`,
`#EXT-X-INDEPENDENT-SEGMENTS`, and an `#EXT-X-TARGETDURATION` set to the
ceiling of the longest `#EXTINF` it carries; `#EXT-X-ENDLIST` ends it, and
no other tags appear. Each piece contributes one `#EXTINF:<seconds>,` line
carrying its stored duration to three decimals, followed by its
`seg-<n>.m4s` line.

The first segment of the playlist, and the first segment of every
discontinuity, is preceded by an `#EXT-X-MAP:URI="init-<run>.mp4"` line
naming that run's init prefix and by an `#EXT-X-PROGRAM-DATE-TIME` in UTC
ISO-8601 with milliseconds carrying the wall-clock start of that piece. A
discontinuity carries one `#EXT-X-DISCONTINUITY` line before that map line.

After each discontinuity the media timeline restarts at zero: a piece's
fragments are shifted onto a time base that begins at its sequence's start,
so stored pieces stay standalone files while one playlist carries one
continuous local time through every run and gap. The stored duration of a
piece truncates its media ticks to a whole millisecond, so two consecutive
pieces placed end to end by their rounded durations overlap by less than a
millisecond at their junction; that overlap keeps every served decode time
monotonic.

`seg-<n>.m4s` answers the piece's fragments with their decode times shifted
by the piece's place on the timeline; the init prefix is not part of the
answer, the playlist points to it separately. A `Range` request naming one
single byte range answers `206` with `Content-Range`; a single range that
starts at or past the end of the answer answers `416` with code
`range_unsatisfiable`; a multi-range request answers `200` with the whole
body; a single range whose first position lies past the last position it
asks for, a `bytes=-0` suffix, and a malformed or overflowing range
specification likewise answer `200` with the whole body; a zero-length
answer has no satisfiable suffix range and answers `416`; without `Range`
the answer is `200` with `Accept-Ranges: bytes`.
**Current backing**: the backing's vod routes answer these same four roles
from files it builds per request.

A piece row whose file is absent or unreadable, or whose stored path is not
exactly `recordings/<camera>/<date>/<hour>/<name>.mp4` naming the row's own
camera, answers `503` `recording_unavailable` for that request and logs one
fixed-word line naming the camera and the piece's `seq`; the answer and the
log line carry no path and no system error text. Stored bytes that cannot be
read as a piece answer `503` `recording_unavailable` with a different
fixed-word log line, and an init prefix claiming more than one mebibyte of
file answers the same way without being read whole. A piece file longer
than 64 MiB is refused the same way before it is read.

Because the recorder is the only writer under the storage root, the path
check treats the tree as the recorder writes it, and a symbolic link placed
under the root is followed like any other piece file. Segment numbers are
positions in the current plan: when retention deletes a piece between a
playlist request and a later segment request, the numbers after it shift
onto the following pieces; a clip is requested promptly after its playlist,
and a short plan keeps that window small.

## A recorded range downloads as one stitched MP4

A person sometimes wants a recording as a single file rather than a stream:
`GET /corvette/api/v1/media/<camera>/start/<start>/end/<end>/download.mp4`.
The address takes the same `<camera>` percent-encoding, the same Unix-second
`<start>` and `<end>`, and the same bounds as the playlist route: an inverted
range, a range longer than 31 days, and a range whose index rows number more
than 25000 each answer `404` `not_found` before any stored piece is read, a
camera the caller's role may not see, a range with no retained recording, and
a camera name carrying bytes outside `A-Za-z0-9_-` answer the same `404`
`not_found` before the name reaches a header or a log line, and an unreadable
index answers `503` `recording_unavailable`.
**Current backing**: this replaces the backing's `clip.mp4` and export
download routes.

The answer is `200` with media type `application/mp4`, `Cache-Control:
no-store`, and `Content-Disposition: attachment; filename="<camera>-<start>-<end>.mp4"`
carrying the validated camera name and the requested Unix seconds floored to
whole seconds. The body is the init prefix of the first piece of the run,
then every piece's rebased fragments in time order — one MP4 on one continuous
time base, assembled by the same serve-time rebase the playlist route uses.

The file covers one run of identical stream parameters, and gaps inside that
run collapse: a run holding a pause in the recording answers as one
continuous clip with no time-base jump, its duration the sum of the pieces it
holds. When the range reaches a stream-parameter change, the answer holds the
first run and carries `X-Corvette-Next-Start: <value>`, where `<value>` is
Unix seconds with three decimals, the greater of the next run's first piece
start and the last piece's end inside this answer. The client asks the next
part at the same address with `start=<value>` and the original `end`; that
answer holds the next run, starts its own time base, and carries the header
again only when a further run follows. Every piece of the range arrives in
exactly one of the parts. A range holding a single run answers without the
header.

The body streams as chunked bytes with no `Content-Length` and no `Accept-Ranges`;
the server sends each piece as it reads it. A `Range` header on a download is
ignored: the answer is always `200` with the whole file. Byte ranges belong to
the playlist route's segments.

The shared five-second request deadline bounds reading the request and
preparing the answer up to its first body byte; a download that has begun is
never expired by that deadline. Each body write — the head, every chunk, the
terminating chunk — must complete within 15 seconds of starting, and every
completed write resets that bound; a download has no total time limit, so a
slow reader receives the whole file at its own pace. Buffered answers keep the
deadline over their whole write.

The failures the playlist routes state for pieces apply before the first body
byte: a piece file that is absent or unreadable, stored bytes that do not
parse, a stored path outside the layout the recorder writes, or a piece file
longer than 64 MiB answer `503` `recording_unavailable` with the fixed-word
log line naming the camera and the piece's `seq`. Once the headers are sent,
a piece that turns unreadable or malformed cannot change the status: the
server closes the stream without the terminating zero-length chunk, so the
client sees a failed download rather than a complete file cut short, and logs
the same fixed-word line naming the camera and the piece's `seq`. A body write
that has not completed within its 15 seconds — a client that stopped reading —
ends the stream the same way, and the server logs one fixed-word line
`recording-download-stalled` naming the camera alone.

## The server issues a link for any recorded range

A client sometimes holds a time range with no review behind it: the range of a motion entry
in the activity answer, or a range a person selects on the timeline. This `clip-link` route
answers the link that plays a camera's recorded footage for such a range, and the client
never builds the playlist address itself.

`GET /corvette/api/v1/clip-link?camera=&start=&end=` answers `{"url": "<address>"}`:

- `camera` is required and names one camera.
- `start` and `end` are required Unix seconds, decimal fractions allowed, finite and not
  negative, with `start < end` and a range of at most 31 days (2678400 s), the bounds of
  the activity route's `after` and `before`.
- Names and values are percent-decoded (`+` stays `+`). A missing required parameter, an
  unknown, repeated or value-less parameter, an empty segment (`&&` or a trailing `&`), a
  malformed escape or one that decodes to invalid UTF-8, or a value outside these rules
  answers `400` `bad_request` with a message naming the reason.

The query is checked before the configuration is fetched, so a malformed request answers
`400` even while the configuration is unavailable.

The answer is `200` with `{"url": "<address>"}` when the caller's role may see the camera
and recorded footage of the camera overlaps `[start, end)` under "Ranges use half-open
overlap". The address is the one "A recorded range plays through one playlist route"
defines, with the camera percent-encoded and the requested range padded as that section
states, and it has the same shape as the `url` of a review's `media.clip`.

A camera that does not exist, a camera the caller's role may not see, and a camera with no
recorded footage overlapping the range each answer `404` `not_found` with the same `code`
and `message`, so the route reveals nothing about cameras the caller cannot see. The route
answers `503` `config_unavailable` under the condition the cameras route states for it: no
usable configuration copy can be served. It answers `503` `database_unavailable` when the
recordings cannot be read; the underlying error is logged, not returned.

A link states availability at the moment of the request. Retention can remove footage
afterwards, and the playlist route's own answers for a range it cannot serve then apply to
the link.

**Current backing**: recorded footage is what the rows of the backing's recordings table
describe; a recording file deleted outside Frigate while its row remains is not detected,
as Frigate does not detect it.

The route accepts `GET` only, with the identity rules "Corvette's own API lives under
`/corvette/api/v1/`" states for every route.

## Recorded coverage answers as merged ranges

A timeline and a player need to know where footage exists before addressing it.
This route answers one camera's recorded coverage of a requested range as merged
continuous ranges, read from Corvette's recording index (`index.sqlite`, the
media bridge's file, opened strictly read-only through `CORVETTE_API_INDEX_PATH`,
default `/media/corvette/index.sqlite`).

`GET /corvette/api/v1/recordings?camera=&start=&end=` answers
`{"ranges":[{"camera","start","end"}, ...]}`:

- `camera` is required and names one camera.
- `start` and `end` are required Unix seconds, decimal fractions allowed, finite
  and not negative, with `start < end` and a range of at most 31 days (2678400 s),
  the bounds of the activity route's `after` and `before`.
- Names and values are percent-decoded (`+` stays `+`). A missing required
  parameter, an unknown, repeated or value-less parameter, an empty segment (`&&`
  or a trailing `&`), a malformed escape or one that decodes to invalid UTF-8, or
  a value outside these rules answers `400` `bad_request` with a message naming
  the reason.

The query is checked before the configuration is fetched, so a malformed request
answers `400` even while the configuration is unavailable.

A stored piece belongs to the range when it overlaps it: it starts before `end`
and ends after `start`, the half-open rule of "Ranges use half-open overlap". The
answer carries Unix seconds with millisecond precision, because the index stores
milliseconds, and every returned range is clipped to the requested bounds.
Ranges are ordered by `start`. Pieces that touch at their ends or overlap merge
into one range — adjacent pieces are continuous coverage — and a gap of any
positive length separates ranges. Each entry names the requested camera; an
empty array answers when the camera is allowed and no piece overlaps the range.

A camera that does not exist or a camera the caller's role may not see answers
`404` `not_found` exactly as the range-link route answers for such a camera, so
the route reveals nothing about cameras the caller cannot see. The route answers
`503` `config_unavailable` under the condition the cameras route states for it,
and `503` `recording_unavailable` when the recording index is absent, unreadable
or corrupt; no answer or log line carries the underlying error or names the
index path. Request bounds are read on a one-millisecond grid, so a bound is
rounded to the nearest millisecond before the half-open comparison.

**Current backing**: recorded coverage today answers from the backing's
recordings summary routes; this contract replaces them.

The route accepts `GET` only, with the identity rules "Corvette's own API lives
under `/corvette/api/v1/`" states for every route.

## Reviews list through `/corvette/api/v1/events`

Two routes together answer Corvette's own review timeline: the list answers
which reviews happened in a window, and the detail answers one review with the
events inside it. Frigate's `/api/events` set is the current backing these
routes replace; the response shapes stand on their own.

An event is one camera track that moved. Its first-motion time is the first
frame whose box centre lies more than 0.04 of the frame away from the track's
first centre; a track that never moved opens no event. A review is a camera's
time range holding one or more overlapping events; it ends after a per-severity
inactivity cutoff, 40 s for `alert` and 30 s for `detection`, and its severity
is `alert` when any event it holds carries one of the camera's alert labels,
`detection` otherwise. A review may upgrade to `alert` and never downgrades.

`GET /corvette/api/v1/events?after=&before=&cameras=&severity=` answers
`{"reviews":[{"id","camera","severity","start","end","media.clip"}, ...]}`:

- `after` and `before` are required Unix seconds under the activity route's
  rules: decimal fractions allowed, finite and not negative, `after < before`,
  and at most 31 days apart.
- `cameras` is an optional comma-separated list of camera names, narrowed and
  dropped exactly as the activity route narrows its camera set: a name that is
  unknown or not allowed for the role is dropped without an error, and an empty
  `cameras` value names no camera and answers an empty list.
- `severity` is an optional comma-separated subset of `alert,detection`; absent
  means both, and any other value answers `400` `bad_request`. Parameter
  validation follows the activity route: an unknown, repeated or value-less
  parameter, an empty segment (`&&` or a trailing `&`), a malformed escape or
  one that decodes to invalid UTF-8 answers `400` with a message naming the
  reason, checked before the configuration is fetched.
- `id` identifies a review in Corvette's index and is stable for one review.
  `start` and `end` are Unix seconds with millisecond precision, `end` null
  while the review is still open.
- Reviews are ordered by `start`, then `id`, the order the activity route
  gives its reviews, so one client reading both answers sees one order.

A review belongs to the window when it overlaps it, exactly as "Ranges use
half-open overlap" states: it starts before `before`, and it is ongoing, ends
after `after`, or starts at or after `after`. A review crossing a window edge
appears once in that window; two adjacent windows list an exactly-touching pair
with one review each, and a review crossing the shared boundary keeps its `id`
in both so clients deduplicate by `id`.

`media.clip` follows "Media availability is explicit": it is present as
`{"url": ...}` when recorded footage of the review's camera overlaps the
review's half-open `[start, end)` — an open review uses the moment of the
request in place of its end, and the link's range ends there — and `null`
otherwise. The answer is computed from the recording index at query time, never
from a stored flag, so footage deleted after the review was recorded shows no
clip. The URL is the playlist address "A recorded range plays through one
playlist route" defines, with the four-second padding that section states.

The camera set follows the role rule "Corvette's own API lives under
`/corvette/api/v1/`" states for the cameras route; a role that may see no
cameras answers `200` with an empty list without reading the index. An
unreadable index answers `503` `recording_unavailable` on the one condition the
recordings route states: the file is absent, unreadable or corrupt. The index
is opened read-only and never written.

## A review answers with its events

`GET /corvette/api/v1/events/<id>` answers the review's own fields — `id`,
`camera`, `severity`, `start`, `end`, `media` as the list route states them —
plus `"events": [{"label","box","zones","first_motion"}, ...]`, the events on
the review's camera whose active span overlaps the review's half-open window,
ordered by their start then their id:

- `label` is the detection class of the track.
- `box` is the track's latest bounding box in detect-frame coordinates.
- `zones` lists the zones the track entered.
- `first_motion` is the event's first-motion time, null when it was never set.

Both spans are half-open, and an event is a member when its active span
overlaps the review's window: `event.start < review_end` and
`event_end > review_start`. An event's end is its stored end, or its
last-motion time while it is open; if that end equals its start — one frame —
the span counts as `[start, start + 1 ms)`, so a single-frame event inside the
window still belongs. A review's end is its stored end, or its last-activity
time while it is open, so an open review's window runs to its last activity.
An event ending exactly at the review's start does not overlap and is absent;
an event active across two adjacent reviews answers under both. Membership is
computed from the times at query time, never from a stored link between an
event and a review, so neither answer can go stale as a review extends.

`<id>` is a decimal integer and the route takes no query parameters; any
parameter answers `400` `bad_request`. An unknown `id` answers `404` `not_found`
with the message `No such review.`; a review on a camera the caller's role may
not see answers the same, so the route reveals nothing about cameras the caller
cannot see. An unreadable index answers `503` `recording_unavailable` like the
list route.

## The server serves a camera's current picture

A client shows a camera's picture — a tile, a preview — without building a backing
address: the route fetches the picture and the server checks the role.

`GET /corvette/api/v1/cameras/<name>/still.jpg?height=N` answers the camera's current
picture, with `<name>` the camera's name percent-encoded (`+` stays `+`):

- `height` is optional and defaults to 270; it is a whole number from 1 to 576, written
  with ASCII digits only.

The camera segment and the query are checked before the configuration is fetched: a
malformed `height`, a repeated, unknown or value-less parameter, an empty query segment
(`&&` or a trailing `&`) or a malformed name escape answers `400` `bad_request` with a
message naming the reason, so such a request answers `400` while the configuration is
unavailable.

The answer is `200` with `Content-Type: image/jpeg`, `Cache-Control: no-store` and
`Connection: close`; the body is the camera's current picture `height` pixels tall, its
width following the camera's aspect ratio.

A camera that does not exist and a camera the caller's role may not see each answer `404`
`not_found` with the same `code` and `message` ("No such camera."), so the route reveals
nothing about cameras the caller cannot see. A camera that is switched off answers `404`
`no_picture` ("The camera is off and has no picture.") and the server asks the backing for
nothing. No usable configuration copy answers `503` `config_unavailable` under the
condition the cameras route states for it. A backing answer that is not a usable picture
answers `503` `picture_unavailable` ("The camera's picture could not be fetched."); the
backing's own status and body are never returned, and the reason is logged.

A usable picture is a `200` whose whole response is at most 1 MiB, carries no transfer or
content encoding, has a body matching its `Content-Length`, begins with the three JPEG
marker bytes `FF D8 FF`, and arrives within the fetch's 2-second bound.

**Current backing**: the picture is Frigate's `latest.jpg` for the camera, requested
through Frigate's local API port at the requested height. A camera whose frames have
stopped makes Frigate answer `200` with its own substituted error image, so the route
forwards that image as a picture; the answer carries no frame time, so a client cannot
tell from this route how old a picture is. `height=0` makes Frigate answer the picture
at its detect-stream size, so the contract enforces its own 1-576 range.

The route accepts `GET` only, with the identity rules "Corvette's own API lives under
`/corvette/api/v1/`" states for every route.

## The server reports each camera's state

The main page shows which cameras are live, which are off by choice, which have no
picture and which are stale. The route classifies every camera server-side and answers
the four states as one Corvette contract.

`GET /corvette/api/v1/cameras/status` answers the state of the caller's cameras. The
route takes no parameters and ignores the query string.

The answer is `200` `{"cameras":[{"name","state","since","recording"}, ...]}`: one entry per camera
the caller's role may see, cameras that are switched off included, in the order the
cameras route lists them; a role that sees no camera gets `{"cameras":[]}`. `state` is
one of four values:

- `live`: the camera is switched on and frames are arriving.
- `stale`: the camera is switched on, the backing's stats show frames arriving, and a
  run of the server's own failed picture fetches for it, with no delivered picture
  between them, began over 60 seconds ago and has its latest failure at most 60 seconds
  ago. A delivered picture ends `stale`.
- `off_by_choice`: the camera is switched off in the configuration; a neutral state, not
  a fault.
- `no_picture`: the camera is switched on and no frames are arriving, which includes a
  camera the backing's stats do not list.

`since` is a JSON number of Unix seconds at which this service first observed the camera
in its current state. A camera turning `stale` or returning to `live` gets a new `since`.
The service keeps these times in process memory, so a restart resets every `since` to
the first observation after the restart.

`recording` is `true` when Corvette's recording index holds a piece of that camera that
ends within three piece lengths of the moment of the request, the piece length being the
value the index records for the camera. It answers `false` for every camera when the index
cannot be read, and the service logs one line when the index first becomes unreadable and
one line when it becomes readable again; the rest of the answer stands either way. An
intentionally switched-off recorder and a broken recorder both read `false`, since the
field reads the index alone.

No usable configuration copy answers `503` `config_unavailable` under the condition the
cameras route states for it, and the server asks the backing for no stats in that case.
Otherwise the server reads the backing's stats once per request, and a stats answer
that is not usable answers `503` `status_unavailable` ("The camera status could not be
read."); the backing's own status and body never appear in any answer, and the reason
is logged. A camera that is switched off is classified from the configuration alone.
A usable stats answer is a plain `200` whose whole response is at most 4 MiB, arrives
within the fetch's 2-second bound, and carries a body that is a JSON object whose
`cameras` member is an object in which every camera has a numeric `camera_fps`.

**Current backing**: the state comes from Frigate v0.17.2 (deployed `0.17.2-3d4dd3a`),
from the configuration's `enabled` flag and from the backing's `GET /api/stats`
(`cameras.<name>.camera_fps`). The stats list every configured camera, switched-off
cameras included. The served answer is the stats emitter's last snapshot, appended
every 15 seconds, so a change shows up to 15 seconds late. Frigate zeroes `camera_fps`
only after about 20 seconds without frames plus its watchdog cycle (10 seconds by
default), so `no_picture` can appear up to about 45 seconds after frames stop, computed
from Frigate's defaults. A camera switched off at runtime keeps its last non-zero frame
rate in the stats — the deployed `living` camera shows 5.0 there while its configuration
says `enabled: false` — so the route reads `enabled` and ignores the stats for an off
camera. The stats body is about 14 KiB and the route reads it on every request. The
evidence for `stale` is the outcome of the still route's fetches
(`GET /api/<camera>/latest.jpg` on the backing), recorded in process memory, so a
camera nobody fetches a picture for is never `stale` and a restart forgets the records.

The route accepts `GET` only, with the identity rules "Corvette's own API lives under
`/corvette/api/v1/`" states for every route.

## No camera's audio track is published on any live-view transport yet

The "RTSP camera frames carry Annex-B video, not a container format" contract above already
records the root cause: `corvette-rtsp-client`'s SDP resolution locates only a camera's `m=video`
section, so no audio `Frame` is ever produced. That gap propagates through every consumer built on
top of it, each confirmed directly rather than assumed:

- The media bridge's fMP4-over-WebSocket repackager (G2) and the HLS/LL-HLS packager (G3) both
  inherit the same video-only scope from the same upstream cause.

Corvette's contract: every live-view transport this document describes — RTSP-restream, the
fMP4/WebSocket transport, and HLS/LL-HLS — carries a configured camera's video unconditionally, and
its audio only once `corvette-rtsp-client` resolves an SDP audio section, a separate, unscoped
future item. No component in this list needs to change shape when that fix lands: each already
dispatches on a frame's own codec generically.

## The review-segment contract carries Frigate's own camera attribution

Frigate v0.17.2's `/api/review` handler selects `ReviewSegment.camera` alongside every other
returned column (`frigate/api/review.py:120`), so every row in the response already names the
camera it belongs to. `corvette_api::ReviewSegment` (`crates/corvette-api/src/lib.rs`) did not
declare a `camera` field, so `serde` silently dropped the column on deserialization.

Corvette's contract: `ReviewSegment.camera` is Frigate's own `camera` column verbatim, carried the
same way `ReviewEvent.camera` and `PreviewClip.camera` already are, so a future consumer needing
per-camera review attribution has the data without a wire-format change.

## Overlapping detections are suppressed on corner coordinates read as position and size

Frigate `v0.17.2`'s `__post_process_nms_yolo` (`frigate/util/model.py`, lines 187 to 226) turns
each candidate from centre-and-size into corners `(x1, y1, x2, y2)` (lines 200 to 206) and hands
that array to `cv2.dnn.NMSBoxes` (line 209). OpenCV's rectangles carry a position and a size
(`Rect_` fields `x`, `y`, `width`, `height`), so the suppression compares the rectangles
`[x1, x1 + x2] x [y1, y1 + y2]`. The width and height of a compared rectangle are the far
corner's coordinates, so a box far from the top left corner of the frame is compared as a much
larger rectangle than it is, and which detections survive depends on where in the frame they lie.

Two 40 by 40 boxes centred at `(100, 100)` and `(130, 100)` overlap by about 14% (`400 / 2800`)
as drawn, while the reading compares `(80, 80, 120, 120)` against `(110, 80, 150, 120)` and
measures exactly 50% (`10800 / 21600`), above the 0.4 suppression threshold, so Frigate returns
one row for the pair.

Corvette's contract: `corvette_yolo::decode` builds the suppression rectangles from the corner
coordinates `(x1, y1, x2, y2)` read as position and size, so Corvette and the current backing
return the same rows for the same model output; the test
`decode_feeds_the_suppression_corner_coordinates_as_frigate_does` and the fixture case
`quirk_nms_suppression` hold the behaviour. Suppressing on the real boxes returns different
rows, so Corvette makes that change only as its own separate, measured step after Frigate's
detector is retired, and a human decides when.

## The detection service answers the current tracked objects of one camera

`GET /corvette/api/v1/cameras/{camera}/objects` answers one camera's tracked objects as they
stand when the request is read: one JSON entry per live tracked object, in tracker order. The
object fields are the same values the replay record file writes per frame, so one parser reads
both.

The answer is `200` `{"objects":[...]}`, each object carrying:

- `id`: the tracker's per-object id printed as a decimal string (`ObjectView::id`, a `u64` at
  `crates/corvette-track/src/object_tracker.rs:74`). Clients treat it as opaque.
- `label`: the label the detector reported.
- `score`: the median of the object's recent score history: `Scores` computes it on update
  (`crates/corvette-track/src/score.rs`: `median` at line 11, stored at line 44, read by
  `computed` at line 58). The history holds the last ten scores. A frame on which the
  tracker holds the object but no detection matched it adds 0.0, so the median decays while
  an object goes unseen; Frigate v0.17.2 does the same (`frigate/track/tracked_object.py`
  `update`, which appends 0.0 when the object's frame time differs from the current frame's).
  The tracker's own view reports only the latest raw score, so the detection service keeps
  the history itself.
- `confirmed`: the sticky flag in the same `Scores` (`score.rs:49`, read by `confirmed` at
  line 68): it turns true the first time the median reaches the threshold and stays true for
  the object's life.
- `box`: `[x1, y1, x2, y2]` in 0-to-1 coordinates from the top left, so a client needs no
  detect size to place the box on a differently sized picture.
- `first_seen` and `last_seen`: Unix seconds as JSON numbers, the frames the object was first
  and most recently seen.
- `zones_now` and `zones_entered`: arrays of zone names. `zones_entered` only grows during the
  object's life; `zones_now` is this frame's set, as the zone contract below defines both.
- `state`: `stationary` when the object's motionless count after the frame
  (`ObjectView::motionless`, `crates/corvette-track/src/object_tracker.rs:86`) is at or above
  the camera's stationary threshold, otherwise `moving`, as the stationary contract below
  defines.

A camera that does not exist and a camera the caller's role may not see each answer `404`
`not_found` with the same `code` and `message`, as the still route does.

No usable configuration copy answers `503` `config_unavailable` under the condition the
cameras route states for it. When the API server cannot get a usable answer from the
detection service's local socket, the route answers `503` `detection_unavailable` ("The
detection service could not be reached."); the detection service's own answer never appears
in any response, and the reason is logged.

Detection off for a known camera answers `200` with `{"objects":[]}`; the detection status
route names the reason. The route accepts `GET` only, with the identity rules "Corvette's own API lives under
`/corvette/api/v1/`" states for every route.

## The detection service reports its own state

`GET /corvette/api/v1/detection/status` answers what the detection pipeline is doing for each
camera the caller's role may see, so a client can tell a quiet scene from a dead pipeline.
Per camera the entry carries:

- `state`: one of `running` (frames arriving and inferred), `waiting_for_frames` (detection
  on with no frames arriving), `inference_unavailable` (frames arriving while the worker
  cannot infer), `off` (detection switched off for the camera) or `failed` (the camera's
  pipeline met a fatal error and the service is restarting that camera's pipeline; the
  entry shows `failed` through the back-off wait and returns to `running` when a fresh
  pipeline runs).
- `fps_in` and `fps_processed`: the frames per second arriving and the frames per second
  processed;
- `frames_dropped`: frames a newer one replaced before the pipeline took them;
- `runner_restarts`: the service's count of restarts of this camera's pipeline, kept in
  the service across those restarts;
- `inferences_run` and `frames_without_region`: the region tensors the pipeline handed to
  inference, one request each (a frame whose inference fails counts all its tensors), and the
  frames that built no region to infer on, both counted since
  the camera's pipeline last started, so a quiet scene (frames arriving, no region, no
  inference) reads differently from a stalled worker (inferences sent, failures rising);
- `worker_state` and `worker_restarts`: the worker's state and its restart count;
- `worker_ms` and `roundtrip_ms`: the worker's own inference time beside the request's
  round-trip time, both in milliseconds, so slow inference and slow transport read
  differently; `lock_wait_ms`: the time a request waited for the shared worker link,
  measured beside `roundtrip_ms` and excluded from it;
- `running_revision` (PENDING until the service builds it): the revision the running
  configuration loaded at start; a client compares it with the read route's `ETag` to see
  whether a saved change still waits on a restart.

The numbers `fps_in`, `fps_processed`, `frames_dropped`, `runner_restarts`,
`inferences_run`, `frames_without_region`, `worker_restarts`, `worker_ms`, `roundtrip_ms` and `lock_wait_ms` are each subject to the
corpus replay run.

The detection service serves both routes on a local socket and the API server relays them,
so the API server stays the single public door (decided 2026-11-08). Both routes are
specified and not yet served; their fields are final only after the corpus replay run
amends them.

No usable configuration copy answers `503` `config_unavailable` under the condition the
cameras route states for it. When the API server cannot get a usable answer from the
detection service's local socket, the route answers `503` `detection_unavailable` ("The
detection service could not be reached."); the detection service's own answer never appears
in any response, and the reason is logged.

The route accepts `GET` only, with the identity rules "Corvette's own API lives under
`/corvette/api/v1/`" states for every route.

### The detection service logs each camera's lifecycle

The service writes one log line per camera state change, so `kubectl logs` shows when a
camera connected, stopped delivering frames or recovered without polling the status route.
Each line reads `unit="<camera>" event=<event> detail="<detail>"`, and a line holds the
camera name, fixed words and integers:

- `connected`: the first frame arrived (`first_frame`), or frames returned after a gap
  (`down_ms=<N>`);
- `source-gap`: frames stopped for `no_frames_for_ms=<frame timeout>`, logged once at the
  start of the gap;
- `inference-unavailable` and `inference-restored`: the worker stopped and resumed
  answering;
- `runner-failed` (`reason=pipeline-error`) and `runner-restart`
  (`restarts=<n> delay_ms=<ms> reason=build-failed|pipeline-error`): the camera's pipeline
  failed and the service restarts it after the back-off delay.

Alongside the state changes the service logs one line per confirmed object:
`object-confirmed` (`id=<n> label=<word> score_pct=<0-100> zones=<count>`) on the frame
where an object first reaches confirmation, and `object-ended` (`id=<n> label=<word>
seconds=<lifetime>`) when an object that had confirmed retires. These lines carry the
tracker's numeric id and the label in written form — letters, digits, `_` and `-`, or the
word `other` — so they add only fixed words and integers to a line.

No line carries a stream address, a user name, a password or the text of an error; the
camera's decoder error text never reaches the log. A camera pipeline restart builds a fresh
runner, so the first frame after a restart logs `connected` with `first_frame` and no
downtime figure; `runner-failed` and `runner-restart` precede it.

Finding: the line writer does not escape the camera name, so a name holding a quote or a
newline could forge a log line. The detection configuration check now rejects a camera
name outside ASCII letters, digits, `_` and `-` (rule text `name may hold only letters,
digits, _ and -`, which never echoes the name). The media bridge's own configuration and
the request path it logs as a unit are not covered by this rule; that is a separate
contract decision.

## The detection configuration is read, validated and applied through the API

Three routes read, dry-run and change the saved detection configuration. The API server
relays all three to the detection service over its local socket, and the detection
service validates and writes the file; an applied configuration takes effect at the next
service restart; the file lives with the service that loads it, and Corvette's process is
its only writer (issue 32, decided 2026-11-19).

`GET /corvette/api/v1/detection/config` answers the saved configuration as JSON in the
loader's value form — the shape `Config::to_value` produces
(`crates/corvette-detect/src/config.rs:144`): top-level `worker_socket`, `labels`,
`deadline_ms` and a `cameras` array. The answer is the normalized value: defaults the
file omits appear filled in (`confirm_threshold` 0.7, `pad_pixels` 16, `config.rs:381,290`).
The response carries the configuration's revision in an `ETag` header and follows the
shared response rules "Corvette's own API lives under `/corvette/api/v1/`" states: JSON,
`Cache-Control: no-store`.

`POST /corvette/api/v1/detection/config/validate` takes the same value form as its body
and runs the loader's complete rule set against it — `Config::from_value`, every rule
behind one entry point (`config.rs:87-90`) — while changing nothing: no file write, no
pipeline touch. Success answers `200`
`{"valid":true,"restart_required":[<restart-required paths>]}` — every changed path in
the proposal — every change takes effect only after a restart — naming each added,
removed or changed camera as `cameras[<name>]` and each changed top-level field as its
key; the array is empty when the proposal changes none — and
failure answers `422` `invalid_config` with every violation listed together (the error
body below).

`PUT /corvette/api/v1/detection/config` replaces the configuration as a whole: the body
is the entire desired value form and the request carries, in `If-Match`, the revision it
was based on. Success answers `200`
`{"revision":"<new>","restart_required":[<changed paths>]}`: every change is
restart-required, and `restart_required` names every changed path, each added, removed
or changed camera as `cameras[<name>]`; the body's `revision` is the token for the next
`If-Match`. The apply `200` carries the new
revision in its body only and sends no `ETag` header: RFC 9110 §9.3.4 forbids a
validator in a successful `PUT` response unless the saved representation is identical
to the content received, and the saved form is normalized. Re-applying identical
content against
the current revision changes nothing and answers that same revision. A retry whose first
attempt already succeeded carries the pre-apply revision and answers `412` like any
stale write; the `412` body carries the current configuration value in its `config`
member and the current revision in its `revision` member, so a client compares `config`
— once its omitted defaulted fields are filled in — against the normalized form the
read route returns, recognizes its own completed write, and retries with `revision`
without a second request.

The revision is one strong token for the saved file: `sha256:<hex>` over the saved file
bytes, quoted in `ETag` and `If-Match` exactly as RFC 9110 defines for lost-update
prevention. A `PUT` whose `If-Match` does not match answers `412` with nothing written
and nothing applied; a `PUT` without `If-Match` answers `428`, because unconditional
writes are never applied — which is what RFC 6585 added 428 for. `If-Match` carries one
strong revision tag; `*` and a list of tags never match, so they answer `412`, and a weak
tag (`W/"…"`) never matches. The file is written through a temporary file in the same
directory, synced, then renamed over the original; the containing directory is not synced,
so a `200` survives a process crash but a power loss in the instant after it can restore
the previous file. The next read then shows the previous revision, and a client that
retries with its saved revision is answered by the same rules.

`GET /corvette/api/v1/detection/config/schema` answers the schema document, so editors
get completion and hover against the live deployment; one more `GET` under the existing
prefix needs no nginx change. The served copy must stay the tested copy: the loader does
not read the schema at runtime and a test checks agreement
(`crates/corvette-detect/tests/config.rs:495-498`). The schema declares its dialect at
JSON Schema 2020-12 (`docs/design/detection-config.schema.json:2`) and gains a stable
`$id` equal to this serving URL, keeping `additionalProperties: false`, which is the
right closure while the schema is one plain object.

## The configuration routes answer with these codes and one error body

- `200` — read succeeded; validate passed; the apply wrote the saved file; the applied
  configuration takes effect at the next restart of the detection service.
- `400` `bad_request` — the body is missing, not JSON, not a JSON object, or over the
  service's body-size bound, 256 KiB; the bound exists so a stalled or oversized writer
  cannot grow the service unboundedly, and is sized for a hand-authored detection
  configuration (tens of cameras).
- `401` `unauthorized` — no or empty `Remote-User`, exactly as every route today.
- `403` `forbidden` — the `PUT` configuration route and the caller's role may not write.
- `412` `stale_revision` — `If-Match` does not match the current revision; nothing was
  written and nothing was applied; the body carries the current configuration value in
  `config` and the current revision in `revision`.
- `422` `invalid_config` — the value failed validation; the error body below.
- `422` `source_not_allowed` — a camera in the body carries a `source` table; answered before
  validation, nothing was written, and the body echoes no submitted value.
- `428` `revision_required` — `PUT` arrived without `If-Match`, answered after
  validation in the apply order below.
- `503` `detection_unavailable` — the detection service could not be reached over its
  local socket; the same condition and code the two detection routes already define
  ("The detection service could not be reached.").
- `500` `apply_incomplete` — the file write failed and nothing took effect: the saved
  file and the running configuration stand unchanged — a write failure changes
  nothing — and `500` `apply_incomplete` names the file that did not land.

Errors are RFC 9457 problem details, `application/problem+json`, carrying `type`,
`title`, `status`, `detail` and `instance`, plus two extension members: the stable
string in `code` (`invalid_config`, `stale_revision`, …) and, on validation failures,
`errors`. One validation failure answers, as an example:

`{"type":"<problem type>","title":"<problem title>","status":422,`
`"detail":"2 configuration errors","code":"invalid_config","errors":[`
`{"path":"cameras[1].fps","rule":"expected an integer, found a string","found":"a"},`
`{"path":"cameras[0].area.polygon","rule":"expected at least 3 points, found 2","found":2}]}`

`path` is the loader's own path style — dotted keys, bracketed indices, built by `dotted`
and `indexed` (`config.rs:514-530`) — so `cameras[1].fps` names the same place in the
file, the value form and the schema. `rule` is the loader's one-line message
(`unknown key`, `missing`, `duplicate camera name "front"`). `found` carries the value
found where a scalar one exists, and `allowed` carries a closed set or range where one
applies (fps bounds, threshold bounds, known key names). A file that does not parse to
TOML at all answers one entry with an empty `path` and the loader's syntax message,
which already names line and column (`toml line 4 column 9: ...`, `config.rs:166-175`).
Validation reports every violation found in the document, collected in one pass, so a
multi-typo fix takes one round trip; the loader returns on the first rule violation today
(`config.rs:87-89`), so accumulating them is a loader change. Field-locating diagnostics
serve both audiences a change can come from: a person fixes a form and a model fixes its
next attempt from the same field.

The pre-routing identity answer keeps the shared `{"code","message"}` shape: its body
must be identical for every path, so an unauthenticated caller learns nothing about
which routes exist (this file, "Corvette's own API lives under `/corvette/api/v1/`");
problem details appear only on responses a route reaches. The two existing detection
routes answer `{"code","message"}` today; moving them to problem details is a later
step of its own, and until then their `503` `detection_unavailable` keeps that body.

## An apply runs in one numbered order and takes effect at the next service restart

Every configuration change takes effect at the next service restart (human decision
2026-11-20); an apply touches no running pipeline.

Restart-required is the whole configuration: the three top-level fields and every field
of every camera entry — added, removed or changed — take effect at the next service
restart. The response shape is stable across changes to the set, and a restart stays
out of band: an operator or their platform performs it, never a route.

One apply runs this order. The API server performs step 1 and relays; the detection
service performs steps 3 to 7:

1. authenticate — the `Remote-User` rule "Corvette's own API lives under
   `/corvette/api/v1/`" states; a role not allowed to write answers `403` here;
2. the API server relays the request to the detection service over its socket;
3. parse and validate the body with the same rule set the dry run uses;
4. take the service's single apply lock;
5. check the revision against the saved file; a missing `If-Match` answers `428` here;
6. diff the proposal against the saved value: the changed paths answer as `restart_required` and as the audit entry's summary;
7. write the file last.

Concurrent applies serialise on the lock at step 4: the second reaches step 5 only after
the first finishes and then sees the first's new revision, which closes the
check-versus-write race. An apply that refuses writes nothing and touches no running
pipeline; the new configuration takes effect only as a whole, at the next service
restart.

An apply never touches a running pipeline: each camera keeps its pipeline until the
service restarts, so no apply leaves a camera without detection. A camera's pipeline is built fresh at service
start; that pipeline start restarts the camera's tracking and object ids begin anew.
Adding, removing or changing a camera answers `restart_required` naming `cameras[<name>]`;
the frame-delivery registry is fixed at startup
(`crates/corvette-media-bridge/src/restream_provider.rs:39-40`) and is built by the media
bridge: `MultiCameraProvider::register` fixes the camera set at startup
(`crates/corvette-media-bridge/src/restream_provider.rs:37-47`), so a camera added to the
file is served frames only once the media bridge also restarts; a detection-service
restart alone leaves it outside the registry, the media bridge and the detection service
being separate processes (`docs/design/detection.md:139-148`,
`docs/design/architecture.md:131`); whether anything restarts the media bridge together
with the detection service is UNVERIFIED; the operator action that brings a camera change
into effect is a restart of the pod, which restarts every container together.

ASSUMPTION (to confirm): the saved file declares TOML 1.1.0 as its version. The write
edits the file in place with `toml_edit`, so comments, key order and spacing outside
the changed values survive; a comment attached to a value the change itself rewrites
is not promised, because assigning a value replaces that value's decoration. The
whole-camera rewrite keeps the comments around it, proven in
`crates/corvette-detect/tests/config_file.rs` (`rewriting_a_camera_keeps_comments_elsewhere`).
The writer (`render`) emits exactly one
trailing newline and a defined key order, so a save whose content has not changed keeps
one revision; plain `toml_edit` output does not hold a missing trailing newline or
dotted-key order stable. The current `toml_edit` release reported upstream
(`0.25.19+spec-1.1.0`) is UNVERIFIED as current; the lock is settled: `Cargo.lock:3404`
pins `toml_edit 0.25.13+spec-1.1.0`, which already parses and writes to the 1.1.0 spec.

A file-write failure changes nothing: the saved file and the running configuration stand
unchanged, the answer is `500` `apply_incomplete` naming that the file did not land, and
the service logs it. Because the revision is the file's hash, the saved file and its
revision stand unchanged and the next apply at the current revision passes step 5. The
running configuration is what the service loaded at start; a successful apply makes the
saved file newer than it by design, and the next service restart loads that file — a
saved change is never dropped.

A successful apply records one structured audit entry in the service's log: who
(`Remote-User`), when, the previous and new revision, and a summary of the changed
paths — nothing secret, which the configuration format guarantees by construction. An
API audit-log precedent for mutating APIs backs this entry; that precedent's citation
404'd when the research read it — UNVERIFIED.

## Configuration reads carry no address or credential, and writes need the admin role

The identity rules "Corvette's own API lives under `/corvette/api/v1/`" states stand
unchanged: nginx runs an identity check (a subrequest to Frigate's `/auth` today) and
sets `Remote-User` and `Remote-Role` from its response, so a browser cannot supply
them; a request reaching the service with a missing or empty `Remote-User` gets `401`
whatever the path. The service itself performs no authentication beyond that check —
the identity arrives on headers from nginx, unspoofable from outside the `frigate`
container because the service binds a Unix socket file in a volume shared only with
that container, where nginx runs — and `Remote-Role` today only filters which cameras
read routes show (`crates/corvette-api-server/src/lib.rs:514-522`).

A write demands more than a read: a `PUT` whose `Remote-Role` is not `admin` answers
`403` `forbidden`. The `admin` string is the role value the current backing's identity
check writes into `Remote-Role`; Corvette's own role names are the human's to set. Two
changes belong to this contract's implementation: nginx's `location ^~ /corvette/api/`
block forwards `PUT` and `POST` (it answers `403` to other methods today), and the
service's request parser accepts them with a bounded body (it accepts `GET` only today,
`crates/corvette-api-server/src/lib.rs:411`). The proxy also strips any client-supplied `Remote-User` or `Remote-Role`
before setting them — a requirement of this contract; UNVERIFIED whether the deployed
nginx already does, settled by reading the `proxy_set_header` lines of the
`/corvette/api/` location in the deployed configuration.

The current backing shows why the write permission is explicit: its config-write
endpoint was once reachable without authentication (GHSL-2023-198) and its raw-config
read leaked secrets to low-privilege readers (CVE-2026-33469) — advisory- and
doc-sourced, UNVERIFIED against the deployed revision. It saves whole-file YAML on
every UI write with no revision guard between two writers (`PUT /api/config/set`; the
whole-file and no-guard details are UNVERIFIED against the deployed revision), and it
can restart broadly on a saved change while flagging restart-required fields in its
editor (docs-sourced; the one-camera-change-restarts-the-system pain is its issue
#16492 — UNVERIFIED against the deployed revision). Corvette's detection adopts the
restart-on-camera-change model by decision (2026-11-20), at the detection service's own
restart.

No response ever carries a stream address or a credential. The configuration format has
no field for one and none is ever added, in the file or in any API or MCP result
(issue 32). A camera entry names its source by the camera name alone — the base of the
restream provider's stream name `<name>_sub` — so a
configuration that is correct never contains a place an address or password could sit,
and the read response, the validate body and every tool result share that one shape.
The detection service dials the sub-stream `rtsp://<restream-address>/<name>_sub` with
the camera's `name`; the address comes from the service flag `--restream-address
HOST:PORT`. Where stream addresses live — the delivery layer's own configuration — is
outside this contract.

## The MCP server administers the detection configuration

The MCP server is a separate process over the public API (issue 32); it authenticates
to `/corvette/api/v1/` like any other client, and that it holds no privileged path into
the detection service is a reading of the issue's words "over the public API", not a
recorded decision. It connects over stdio, launched as a local subprocess by the MCP
client, and reads a service-account credential from the environment; the
stdio-credentials guidance is quoted directly from the MCP authorization page (primary).
The audience this transport suits — one operator and one MCP client — is UNVERIFIED
(no record found). OAuth 2.1, RFC 8707 resource indicators and RFC 9728 are quoted
from the MCP authorization text, primary; the PKCE and `401` plus `WWW-Authenticate`
resource-metadata discovery that a later HTTP transport would use come from secondary
sources and stand as direction, not settled text. The spec revision relied on is the
current 2026-07-28.

It exposes the four intent-level tools issue 32 named; a fifth is not justified, because
the four close the read-validate-apply-check loop as far as the saved file reaches:
`read_detection_config` reads the saved configuration and its revision,
`validate_detection_config` validates the proposal, `apply_detection_config` writes the
file, then a further `read_detection_config` reads the configuration back to confirm the
saved revision; `read_detection_status` answers detection state only — the status
route's `running_revision` member tells a client whether the running configuration is
the saved one — and every applied change takes effect at the next service restart.
Endpoint-mirroring is the documented anti-pattern (relayed through secondary sources). Each tool's `inputSchema`
and `outputSchema` are JSON Schema documents derived from the route bodies above:

- `read_detection_config` — input `{}`; output `{config, revision}`: the read route's
  body plus the revision it carries. Annotated read-only.
- `validate_detection_config` — input `{config}`; output `{valid, restart_required}` on
  success, or the full `errors` list as a tool-execution error on failure. Annotated
  read-only.
- `apply_detection_config` — input `{config, revision}`; the server places the revision
  on `If-Match` and puts the configuration in the body, the route's whole value form;
  output is the apply response `{revision, restart_required}` as the `PUT` route defines
  it: `restart_required` names every changed path, and the change takes effect at the next
  service restart. The one tool that changes state. Annotated `destructiveHint` and
  `idempotentHint`, and the server requests a form-mode elicitation confirmation before
  each apply; elicitation is a client capability, so the request reaches a human when
  the client implements it. Annotations are hints and clients must treat them as
  untrusted, so enforcement stays in the API's own authentication and admin-role rule.
- `read_detection_status` — input `{}`; output is the detection status route's body, so
  the model can tell a quiet scene from a dead pipeline before proposing a change.
  Annotated read-only. Its route, `GET /corvette/api/v1/detection/status`, is specified
  and not yet served; the tool ships with it.

The server is read-only by default (issue 32): the advertised tool list omits
`apply_detection_config`, and the server's documentation names how writes are turned on.
A validation failure returns as a tool-execution error whose content is the `errors`
list — the spec assigns execution errors exactly that role: actionable feedback a model
can use to self-correct — while malformed requests stay protocol errors. Human
confirmation before `apply_detection_config` sits with the client, where the spec places
it; the server's elicitation request sits on top of that.

The server has no tool that yields stream addresses or credentials — the contract gives
it no shape that contains one — and tool results carry JSON values, never stream bytes
or pictures, and no tool returns a stream URI: stream addressing belongs to the
delivery contract. No normative prohibition on embedding streams was found in the spec
— that rule is marked UNVERIFIED — so "no stream in any tool result, ever" stands as
this contract's own rule.

Implementation status: the three configuration routes, the schema route and the MCP
server are specified and not yet served.

## A tracked object is stationary when its motionless count reaches the threshold

The tracker in `crates/corvette-track` follows five tracking rules; the first, second and
fourth part from Frigate `v0.17.2`. Each is held by the named test:

- **One stationary comparator.** Frigate compares in three places: the tracker decides
  with `motionless_count >= threshold` before counting the frame
  (`frigate/track/norfair_tracker.py:456-459`), the frame loop that builds the stationary
  set uses `>=` (`frigate/video.py:874-875`), and the object's own flag `is_stationary` uses
  a strict `>` (`frigate/track/tracked_object.py:432-434`). Corvette uses `>=` only:
  `motionless >= after` at `crates/corvette-track/src/stationary.rs:221` ports the
  tracker's comparison, and every later stationary decision applies the same `>=` to the
  count after the frame. The test `stationary_threshold_is_crossed_on_the_exact_frame`
  (`crates/corvette-track/tests/object_tracker.rs:1511`) holds the motionless count on each
  frame around the threshold.
- **The median of an even count averages.** Frigate's `median_of_boxes` returns the entry at
  index `int(len / 2)` of the area-sorted list (`frigate/util/object.py:355-358`), the
  upper-middle box when the count is even. Corvette's `median_box` averages the two middle
  boxes edge by edge for an even count (`crates/corvette-track/src/stationary.rs:110`, the
  even branch at lines 129-135). `tools/goldens/track_stationary.py` runs Frigate's own
  stationary code with `median_of_boxes` replaced by the ordinary median (header lines 5-16),
  and `fixture_matches_the_real_library` (`crates/corvette-track/tests/stationary.rs:947`)
  replays the golden fixture bit for bit.
- **The expiry cap is per label and expires only capped labels**, as in Frigate's
  `is_expired` (`frigate/track/norfair_tracker.py:427-443`). `max_frames` is an optional
  per-label cap with an optional default (`max_frames_default: Option<u64>` and
  `max_frames_overrides` at `crates/corvette-track/src/object_tracker.rs:27-28`, resolved by
  `max_frames_for` at line 461); an object retires once `motionless - after > max_frames`
  (`crates/corvette-track/src/stationary.rs:250-251`). The tests
  `moving_sibling_survives_the_cap_retirement` and `object_without_a_cap_never_retires`
  (`crates/corvette-track/tests/object_tracker.rs:1527`, `:1541`) hold both halves.
- **Retiring an object removes one track.** Frigate's `deregister`
  (`frigate/track/norfair_tracker.py:300-320`) rewrites the tracked-object list of the
  tracker that serves the label (the label's own tracker, or the shared default tracker for
  a label without one, `norfair_tracker.py:248-259`) when the label has a `max_frames`
  value, keeping only entries that are not the retiring one and carry a negative hit
  counter (line 317), so every other live track on that tracker disappears in the same
  call. Corvette's `Tracker::remove`
  (`crates/corvette-track/src/tracker.rs:118`) removes the track carrying the requested id and
  leaves every other track alone, same-label live tracks included:
  `removes_only_the_named_track` and `removal_keeps_other_live_tracks_of_the_same_label`
  (`crates/corvette-track/tests/remove.rs:60`, `:141`) hold it.
- **One id counter, shared**, as norfair 2.3's class-level counter is (`tracker.py:389`,
  `:427-430`; Frigate pins `norfair == 2.3.*`). The per-object id counter is a single
  `IdSource` (`crates/corvette-track/src/track.rs:35`) that the object loop hands to every
  per-label
  tracker and to the fallback tracker (`crates/corvette-track/src/object_tracker.rs:114`,
  used at lines 252 and 259), so ids stay unique and never repeat while one source lives.
  `ids_count_per_source` (`crates/corvette-track/tests/track.rs:310`) holds the per-source
  counting; `two_same_label_objects_registered_in_one_frame_get_different_ids`
  (`crates/corvette-track/tests/object_tracker.rs:1629`) holds fresh ids for new objects.

## A zone counts the consecutive frames an object's foot is inside

Frigate `v0.17.2` decides zone membership in the zone section of `TrackedObject.update`
(`frigate/track/tracked_object.py:183-298`). The tested point is the box's bottom centre —
centroid x and the box's lower edge (`tracked_object.py:185`) — and inside or on the edge
counts as inside (`cv2.pointPolygonTest(...) >= 0`, line 198). Each frame's score is the
stored presence plus one (line 195); an inside frame stores it while it is below the zone's
inertia (default 3, `frigate/config/camera/zone.py:29-33`; line 287), so the stored presence
stops at inertia minus one once the object enters, and the outside branch counts it down
only while the score is below inertia (lines 290-291). After the first entry the presence
never decays, so a re-entry after a long absence waits no frames.

Corvette's contract: `ZoneSet::observe` decides membership from the box's bottom centre
(`crates/corvette-track/src/zones.rs:42`); any frame outside the polygon resets the counter to
zero, and entry needs `inertia` consecutive inside frames (lines 55-58). `entered` only grows
— a zone name is added once and never removed (lines 60-61) — and `current` is rebuilt fresh
every frame (line 48). Loitering, per-zone object filters and speed zones sit outside this
contract, and the cameras' live zone configuration uses none of them. Polygons arrive as
relative 0-to-1 vertices and are converted to pixels with truncation
(`polygon_from_relative` at `zones.rs:77`, truncation at line 98).

The golden `tools/goldens/track_zones.py` runs Frigate's own zone section unchanged apart from
the single change, `self.zone_presence[name] = zone_score - 1` replaced by
`self.zone_presence[name] = 0` (lines 289-291; header lines 3-18), so a zone counts
consecutive inside frames and any outside frame resets. The tests
`fixture_rows_replay_frame_by_frame` and `hand_written_scenarios_replay`
(`crates/corvette-track/tests/zones.rs:412`, `:453`) replay
`crates/corvette-track/tests/fixtures/zones_cases.txt`, whose header records a floor of 360
on-edge cases, which holds the inside-or-on-the-edge rule.

## The replay record file is JSON lines, one frame per line

The detection service records, per camera segment, a file that the offline replay reads
back to reconstruct the tracked-object stream (decided 2026-11-17). Schema version 1.

Line 1 is a header object: `{"schema": 1, "camera": <name>, "segment": <segment file name>,
"frame_width": <int>, "frame_height": <int>, "detect_fps": <number>}`. Every later line is
one frame: `{"frame_time": <seconds as number>, "objects": [{"id": <string>, "label":
<string>, "score": <number>, "confirmed": <bool>, "box": [x1, y1, x2, y2] (0-to-1
coordinates), "first_seen": <seconds as number>, "last_seen": <seconds as number>,
"zones_now": [<string>], "zones_entered": [<string>], "state": "moving"|"stationary"}],
"ended": [<id string>]}`. `objects` lists the camera's tracked objects as of that frame
with exactly the object fields of the objects route, so one parser reads both; `ended` lists
the ids that retired on that frame. `box` divides each pixel edge by `frame_width` or
`frame_height` from the header.

Frame times derive from the segment name and the frame index, so two
runs over the same segment produce byte-identical files. Keys appear in the order shown,
numbers use the shortest decimal form that round-trips, lines end with a single `\n`, and the
file has no trailing blank line.
