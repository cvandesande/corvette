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
