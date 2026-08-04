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
