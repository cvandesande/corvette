# R1 — Pin moq-dev/moq's commit; verify moq-relay AND the codec-level Split/Import API build standalone

Item R1, `.agents/issue-12/PLAN-live-view.md`. All commands below were run for real; output is
pasted verbatim (only truncated where noted), not paraphrased.

## 1. Premise check: is the commit `RESEARCH-d4-ingest-source.md` read still the right one to pin?

`RESEARCH-d4-ingest-source.md` F-1/F-2 read moq-dev/moq source at
`7b73c43381a7f9c309e3045a8f0f858aa32ef48c` (2026-08-16) to confirm
`moq_mux::codec::h264/h265/aac`'s `Split`/`Import` API shape. This item must decide whether to pin
that exact commit or move to moq-dev/moq's actual current HEAD, per the plan's premise
instruction.

Current HEAD, fetched live:

```
$ gh api repos/moq-dev/moq/commits/main --jq '{sha: .sha, date: .commit.committer.date, message: .commit.message}'
{"date":"2026-08-26T02:55:34Z","message":"feat(relay): make the auth API the source of truth for live sessions (#3041)\n\nCo-authored-by: David von Wrangel <wrangelvid@gmail.com>\nCo-authored-by: Claude Opus 5 <noreply@anthropic.com>","sha":"704734774d7cd3a26bb381a5edffb3a207774dc8"}
```

The pinned candidate, confirmed still on `main`'s history:

```
$ gh api repos/moq-dev/moq/commits/7b73c43381a7f9c309e3045a8f0f858aa32ef48c --jq '{sha: .sha, date: .commit.committer.date, message: .commit.message}'
{"date":"2026-08-16T22:27:00Z","message":"test(ts): synthesise an EIT so the import path's SI routing is testable (#2828)\n\nCo-authored-by: Claude <noreply@anthropic.com>\nCo-authored-by: Cursor <cursoragent@cursor.com>\nCo-authored-by: Luke Curley <kixelated@gmail.com>\nCo-authored-by: GPT-5 <noreply@openai.com>","sha":"7b73c43381a7f9c309e3045a8f0f858aa32ef48c"}
```

License of the upstream repo (relevant to INV-1's AGPL prohibition):

```
$ gh api repos/moq-dev/moq --jq '.license'
{"key":"apache-2.0","name":"Apache License 2.0","node_id":"MDc6TGljZW5zZTI=","spdx_id":"Apache-2.0","url":"https://api.github.com/licenses/apache-2.0"}
```

`moq-net`/`moq-mux`/`hang`'s own `Cargo.toml` each declare `license = "MIT OR Apache-2.0"`, and
the repo root carries both `LICENSE-APACHE` and `LICENSE-MIT`. Neither the pinned commit nor HEAD
is AGPL; INV-1 is satisfied either way.

Diff size between the pinned commit and HEAD:

```
$ gh api "repos/moq-dev/moq/compare/7b73c43381a7f9c309e3045a8f0f858aa32ef48c...704734774d7cd3a26bb381a5edffb3a207774dc8" --jq '{ahead_by: .ahead_by, behind_by: .behind_by, total_commits: (.commits|length)}'
{"ahead_by":65,"behind_by":0,"total_commits":65}
```

65 commits over 10 days. Filtering that diff to the files this item's own codec-level API check
depends on:

```
$ gh api ".../compare/7b73c433.../704734774..." --jq '.files[] | select(.filename | test("moq-mux|codec")) | .filename'
rs/moq-mux/src/codec/aac/import.rs
rs/moq-mux/src/codec/h264/import.rs
rs/moq-mux/src/codec/h264/split.rs
rs/moq-mux/src/codec/h265/import.rs
rs/moq-mux/src/codec/h265/split.rs
... (35 more files under rs/moq-mux/src, including catalog/, container/, import/)
```

The exact files this item's `Import`/`Split` calls depend on all changed. The actual patch (not
just the file list) for the three that matter most:

```diff
--- a/rs/moq-mux/src/codec/aac/import.rs (at 7b73c43)
+++ b/rs/moq-mux/src/codec/aac/import.rs (at 7047347)
@@ -30,12 +30,18 @@ impl<E: CatalogExt> Import<E> {
 		tracing::debug!(name = ?track.name(), ?config, "starting track");
 		// Advertise this rendition's timeline before publishing (the generic set() no longer does).
 		config.timeline = Some(reserved.producer().timeline(track.name())?.section());
-		let mut rendition = reserved.audio(track.name());
+		// The caller's config names the container; the writer is built from that same value so the
+		// wire cannot disagree with what the rendition advertises.
+		let wire = crate::catalog::hang::Container::try_from(&config.container)?;
+		let name = track.name().to_string();
+		// Build the writer before advertising the rendition: it is fallible (its timeline track can
+		// collide), and a rendition published for a track we then fail to produce would be
+		// advertised to consumers but never served.
+		let media = reserved.producer().media_producer(track, wire)?;
+		let mut rendition = reserved.audio(name);
 		rendition.set(config);
 		Ok(Self {
-			track: reserved
-				.producer()
-				.media_producer(track, crate::catalog::hang::Container::Legacy)?,
+			track: media,
 			rendition,
 		})
 	}

--- a/rs/moq-mux/src/codec/h264/import.rs (at 7b73c43)
+++ b/rs/moq-mux/src/codec/h264/import.rs (at 7047347)
@@ -48,12 +48,13 @@ impl<E: CatalogExt> Import<E> {
 		hint: crate::catalog::VideoHint,
 	) -> crate::Result<Self> {
 		let rendition = reserved.video(track.name());
+		// The hint names the container; the writer is built from that same value so the wire
+		// cannot disagree with what the rendition advertises.
+		let wire = crate::catalog::hang::Container::try_from(&hint.container)?;
 		let catalog = crate::codec::video::Catalog::new(&reserved, track.name(), hint)?;
 		let mut import = Self {
 			avc1: false,
-			track: reserved
-				.producer()
-				.media_producer(track, crate::catalog::hang::Container::Legacy)?,
+			track: reserved.producer().media_producer(track, wire)?,
 			rendition,
 			catalog,
 			last_sps: None,
@@ -257,7 +258,6 @@ fn config_from_avcc(avcc_bytes: &[u8]) -> Result<hang::catalog::VideoConfig> {
 	config.coded_width = avcc.coded_width;
 	config.coded_height = avcc.coded_height;
 	config.description = Some(Bytes::copy_from_slice(avcc_bytes));
-	config.container = hang::catalog::Container::Legacy;
 	Ok(config)
 }

--- a/rs/moq-mux/src/codec/h264/split.rs (at 7b73c43)
+++ b/rs/moq-mux/src/codec/h264/split.rs (at 7047347)
@@ -38,7 +38,7 @@ pub struct Split {
 	pps: Vec<Bytes>,
-	zero: Option<tokio::time::Instant>,
+	zero: Option<web_async::time::Instant>,
 	pending: Vec<crate::container::Frame>,
 }
```

(h265's `import.rs`/`split.rs` carry the mirror-image of the same two changes; omitted for
brevity.)

**Decision: pin `7b73c43381a7f9c309e3045a8f0f858aa32ef48c`, not HEAD.**

Reasoning: the diff between the researched commit and current HEAD is not incidental churn
elsewhere in the monorepo — it rewrites exactly the `Import::new` constructors this item
verifies, changing how they resolve the wire container (from a hardcoded `Legacy` value to one
derived from the caller's `VideoConfig`/`AudioConfig.container` field via a new fallible
`Container::try_from`). That is an API-shape change, not a pure refactor: a caller built against
the pinned commit's behavior (container always `Legacy`, unconditionally) would need to know to
set `.container` explicitly against HEAD, and `media_producer` becomes fallible in a new way
(`Container::try_from` can itself error) at a point it wasn't before. `RESEARCH-d4-ingest-source.md`
read source at the older commit; nobody in this issue has read source at HEAD. Moving the pin to
HEAD without that reading would mean shipping D-9's decision against unread source, exactly the
gap R1 exists to close for the commit that *was* read. AGENTS.md's own instruction ("Establish
upstream API contracts ... by reading Frigate's [and by the same logic, any upstream's] own
source at the revision the deployment is running") and the pipeline playbook's premise-before-work
rail both point the same way: pin what was actually read, and treat a fresh HEAD move as a new
research question for a future item, not something R1 quietly absorbs.

This is also consistent with D-9's own citation of F-1/F-2 as the (source-reading, not
build-verified) basis for "no MPEG-TS anywhere" — the whole point of this item is to close that
verification gap *for the commit the decision was actually made against*.

## 2. Pin added to the workspace `Cargo.toml`

`/home/cvandesande/github/corvette/Cargo.toml` gained a `[workspace.dependencies]` table (none
existed before):

```toml
[workspace.dependencies]
moq-net = { git = "https://github.com/moq-dev/moq", rev = "7b73c43381a7f9c309e3045a8f0f858aa32ef48c", package = "moq-net" }
moq-mux = { git = "https://github.com/moq-dev/moq", rev = "7b73c43381a7f9c309e3045a8f0f858aa32ef48c", package = "moq-mux" }
hang = { git = "https://github.com/moq-dev/moq", rev = "7b73c43381a7f9c309e3045a8f0f858aa32ef48c", package = "hang" }
```

No workspace member consumes these yet (that starts at X1-X3/G1, out of R1's scope), so
`Cargo.lock` is untouched by this item — nothing pulls the git dependency into the resolved graph
until something declares `{ workspace = true }` for it.

**This was verified, not assumed.** Since an unreferenced `[workspace.dependencies]` entry is
never fetched or resolved by Cargo, adding the stanza alone does not prove the pin is even valid
(right repo, right rev, right package name inside a 28-crate monorepo, resolvable via `package =
"..."` with no `path` given). To prove it resolves, a temporary crate
(`crates/_r1-probe`, added to the workspace `members` list only for this check) declared
`moq-net = { workspace = true }` / `moq-mux = { workspace = true }` / `hang = { workspace = true }`
and was built:

```
$ cargo build -p _r1-probe
...
   Compiling moq-flate v0.1.1 (https://github.com/moq-dev/moq?rev=7b73c43381a7f9c309e3045a8f0f858aa32ef48c#7b73c433)
...
   Compiling moq-net v0.2.12 (https://github.com/moq-dev/moq?rev=7b73c43381a7f9c309e3045a8f0f858aa32ef48c#7b73c433)
   Compiling hang v0.20.5 (https://github.com/moq-dev/moq?rev=7b73c43381a7f9c309e3045a8f0f858aa32ef48c#7b73c433)
   Compiling moq-json v0.3.3 (https://github.com/moq-dev/moq?rev=7b73c43381a7f9c309e3045a8f0f858aa32ef48c#7b73c433)
   Compiling moq-loc v0.2.1 (https://github.com/moq-dev/moq?rev=7b73c43381a7f9c309e3045a8f0f858aa32ef48c#7b73c433)
   Compiling moq-mux v0.9.7 (https://github.com/moq-dev/moq?rev=7b73c43381a7f9c309e3045a8f0f858aa32ef48c#7b73c433)
   Compiling _r1-probe v0.1.0 (/home/cvandesande/github/corvette/crates/_r1-probe)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 14.00s
```

`cargo build --workspace` and `cargo clippy --workspace --all-targets` both stayed clean with the
probe present (no new warnings from `moq-net`/`moq-mux`/`hang` under this workspace's
`[workspace.lints]`, since third-party git dependencies aren't linted under a consumer's own
`[workspace.lints]` regardless — the check that matters is that adding them doesn't break
*corvette's own* crates, which it didn't). `crates/_r1-probe` and its temporary `members` entry
were then deleted; `git diff Cargo.lock` afterward is empty, confirming the probe left no residue.

Final `cargo build --workspace` in this repo, with only the `Cargo.toml` pin remaining (no
consumer, no probe):

```
$ cargo build --workspace
warning: Git tree '/home/cvandesande/github/corvette' is dirty
   Compiling corvette-ui v0.1.0 (/home/cvandesande/github/corvette/crates/corvette-ui)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.05s
$ echo $?
0
```

```
$ cargo clippy --workspace --all-targets
warning: Git tree '/home/cvandesande/github/corvette' is dirty
    Checking corvette-api v0.1.0 (/home/cvandesande/github/corvette/crates/corvette-api)
    Checking corvette-ui-server v0.1.0 (/home/cvandesande/github/corvette/crates/corvette-ui-server)
   Compiling ncnn-sys v0.1.0 (/home/cvandesande/github/corvette/crates/ncnn-sys)
    Checking corvette-rtsp-client v0.1.0 (/home/cvandesande/github/corvette/crates/corvette-rtsp-client)
    Checking corvette-ui v0.1.0 (/home/cvandesande/github/corvette/crates/corvette-ui)
    Checking ncnn-spike v0.1.0 (/home/cvandesande/github/corvette/crates/ncnn-spike)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.10s
```

Zero warnings, zero errors, under `[workspace.lints.clippy]`'s `pedantic`/`nursery`/`cargo` deny
groups plus the project's own `undocumented_unsafe_blocks`/`disallowed_methods` denies.

(The "Git tree is dirty" line is `nix develop`'s own flake warning about this session's
uncommitted evidence files; unrelated to the pin.)

## 3. `moq-relay` builds and runs standalone in an isolated scratch checkout

Scratch checkout (NOT under this repo's working tree): a fresh clone of `moq-dev/moq` at
`/tmp/.../scratchpad/r1-moq/repo`, checked out to the pinned commit:

```
$ git clone https://github.com/moq-dev/moq.git repo
$ git checkout 7b73c43381a7f9c309e3045a8f0f858aa32ef48c
$ git rev-parse HEAD
7b73c43381a7f9c309e3045a8f0f858aa32ef48c
```

Toolchain: this repo's own `nix develop` shell (rustc 1.97.1), which exceeds moq-relay's own
`rust-toolchain.toml` floor (1.95.0) and its per-crate MSRV bump (`moq-relay`'s own `1.95`).

```
$ cargo build --release --bin moq-relay
warning: Git tree '/home/cvandesande/github/corvette' is dirty
    Finished `release` profile [optimized] target(s) in 0.21s
$ echo $?
0
$ ls -la target/release/moq-relay
-rwxr-xr-x 2 cvandesande users 44751240 Aug 26 08:35 target/release/moq-relay
```

(First build from clean took ~1m04s compiling the full dependency graph; the run above is a
no-op rebuild from the same checkout, still exit 0.)

No `Cargo.toml`, no config file, no environment variables — only CLI flags, per the item's Do
step 2. No Redis appears anywhere in the dependency tree or CLI surface; no `--auth-api`/
`--auth-key-dir` (the moq-api-equivalent auth backends) were configured.

### QUIC connection accepted from a second local invocation

Relay A (server), bound on a local port, self-signed cert generated on the fly, public
(unauthenticated) access for testing:

```
$ ./target/release/moq-relay --server-bind 127.0.0.1:44333 --tls-generate localhost --auth-public / --log-level debug
INFO moq_relay::cluster: cluster initialized origin_id=8956765813067974 configured=false
INFO moq_relay::relay: listening addr=127.0.0.1:44333
INFO moq_relay::cluster: no cluster peers configured; running standalone
```

Relay B (a second, independent local invocation of the same binary), dialing Relay A as a real
QUIC peer via `--cluster-connect` (moq-relay's own peer-dial mechanism, wired through
`moq_native::Client`/Quinn — this is the actual codepath `moq-relay` uses to open an outbound
QUIC/WebTransport session, distinct from the unused-by-relay `--client-connect` library flag):

```
$ ./target/release/moq-relay --cluster-connect "https://localhost:44333/anon" --client-tls-disable-verify --tls-generate localhost --server-bind 127.0.0.1:44444 --auth-public / --log-level debug
WARN moq_native::tls: TLS server certificate verification is disabled; A man-in-the-middle attack is possible.
INFO moq_relay::cluster: cluster initialized origin_id=113232335304783 configured=false
INFO moq_relay::relay: listening addr=127.0.0.1:44444
INFO remote{remote=https://localhost:44333/anon}:conn{id=0}: moq_relay::cluster: dialing cluster peer url=https://localhost:44333/anon
DEBUG remote{...}:conn{id=0}: moq_native::quinn: connecting url=https://localhost:44333/anon
DEBUG remote{...}:conn{id=0}: moq_native::failover: dialing addr=[::1]:44333 index=0
DEBUG remote{...}:conn{id=0}: moq_native::websocket: QUIC not yet connected, attempting WebSocket fallback ...
WARN remote{...}:conn{id=0}: moq_native::websocket: WebSocket connection failed err=failed to connect WebSocket
DEBUG remote{...}:conn{id=0}: moq_native::failover: dialing addr=[::ffff:127.0.0.1]:44333 index=1
DEBUG remote{...}:conn{id=0}: moq_native::failover: connected addr=[::ffff:127.0.0.1]:44333 index=1
DEBUG remote{...}:conn{id=0 id=140735206686960}: web_transport_quinn::connect: sending CONNECT request ...
DEBUG remote{...}:conn{id=0 id=140735206686960}: web_transport_quinn::connect: received CONNECT response response=ConnectResponse { status: 200, protocol: Some("moq-lite-05") }
INFO remote{...}:conn{id=0 id=140735206686960}: moq_native::client: connected version=moq-lite-05
```

Relay A's own log, captured concurrently, shows it accepting that exact connection:

```
DEBUG moq_native::quinn: accepting host=localhost ip=127.0.0.1:51996 alpn=h3
DEBUG moq_native::quinn: accepted host=localhost ip=127.0.0.1:51996 alpn=h3
DEBUG web_transport_quinn::connect: received CONNECT request request=ConnectRequest { url: Url { ..., host: Some(Domain("localhost")), port: Some(44333), path: "/anon", ... } }
DEBUG web_transport_quinn::connect: sending CONNECT response response=ConnectResponse { status: 200, protocol: Some("moq-lite-05") }
INFO conn{id=0}: moq_relay::connection: session accepted transport=quic role=None tier= root=anon publish= subscribe=
INFO conn{id=0}: moq_relay::connection: negotiated version=moq-lite-05 transport=quic
```

Both the dialing side ("connected version=moq-lite-05") and the listening side ("session
accepted transport=quic" / "negotiated version=moq-lite-05 transport=quic") confirm a real QUIC
handshake completed end to end between two independently-started local processes, using nothing
but CLI flags, no Redis, no `moq-api`/`--auth-api`.

Both relay processes were terminated after this check (`pkill -f moq-relay`); confirmed no
`moq-relay` process remained running afterward.

## 4. Codec-level `Split`/`Import` API: builds and runs against synthetic input, no MPEG-TS

Wrote a disposable throwaway crate, `rs/throwaway-r1`, temporarily added as a workspace member of
the *scratch checkout's own* `Cargo.toml` (not this repo's), with path dependencies on the
scratch checkout's `rs/moq-mux`, `rs/moq-net`, `rs/hang` at the pinned commit:

```toml
[dependencies]
moq-mux = { path = "../moq-mux" }
moq-net = { path = "../moq-net" }
hang = { path = "../hang" }
bytes = "1"
tokio = { version = "1.48", features = ["full"] }
```

The program (full source kept only in the scratch checkout, deleted with it — reproduced here for
the record):

- `run_h264()`: builds a synthetic Annex-B access unit (SPS + PPS + IDR slice, each with a 4-byte
  start code — the same known-valid SPS/PPS/IDR triple moq-mux's own
  `avc3_self_initializes_from_first_keyframe` test uses), feeds it through
  `moq_mux::codec::h264::Split::new()` → `.decode()`/`.flush()`, then
  `moq_mux::codec::h264::Import::new(track, reserved, Default::default())` → `.decode(frames)`,
  and asserts the resulting catalog rendition resolved the correct H.264 profile/level from the
  synthetic SPS.
- `run_aac()`: builds a synthetic raw (non-ADTS) AAC access unit (`[0xAB; 32]`, opaque —
  `aac::Import::decode` treats it as an already-depacketized packet, exactly the shape issue #18's
  shipped depacketizer produces per D-9), and calls
  `moq_mux::codec::aac::Import::new(track, reserved, config.into())` → `.decode(...)` →
  `.finish()`.
- Both paths use `moq_net::broadcast::Info::new().produce()` + `moq_mux::catalog::Producer::new()`
  to build the minimal `Reserved`/track-producer scaffolding `Import::new` requires — the same
  setup pattern moq-mux's own unit tests for these exact constructors use
  (`rs/moq-mux/src/codec/h264/import.rs::tests::setup`,
  `rs/moq-mux/src/import/track.rs::tests::new_broadcast`).
- No MPEG-TS, no `moq_mux::container`, anywhere in the program.

Build:

```
$ cargo build -p throwaway-r1
   Compiling moq-net v0.2.12 (/tmp/.../scratchpad/r1-moq/repo/rs/moq-net)
   Compiling hang v0.20.5 (/tmp/.../scratchpad/r1-moq/repo/rs/hang)
   Compiling moq-mux v0.9.7 (/tmp/.../scratchpad/r1-moq/repo/rs/moq-mux)
   Compiling throwaway-r1 v0.1.0 (/tmp/.../scratchpad/r1-moq/repo/rs/throwaway-r1)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 11.06s
```

Run:

```
$ cargo run -p throwaway-r1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.12s
     Running `target/debug/throwaway-r1`
h264: Split::new/decode/flush + Import::new/decode all ran; resolved profile=0x42 level=0x1f
aac: Import::new/decode/finish all ran; resolved codec=mp4a.40.2 sample_rate=48000 channels=2
R1 codec-level API check: PASS (no panic, both codecs produced a non-error result)
$ echo $?
0
```

Exit 0, no panic, both codec paths produced a non-error result: `h264::Split`/`h264::Import`
resolved the correct profile (`0x42`) and level (`0x1f`) from the synthetic SPS, and
`aac::Import` published a rendition with the correct sample rate (48000) and channel count (2)
from the synthetic raw AAC access unit. This closes the specific gap the Premise names: F-1/F-2
were confirmed by reading source, not by an independent executable build; this is that
independent executable build, at the exact commit this item pins.

The scratch checkout (including `rs/throwaway-r1` and the `Cargo.toml` member-list edit) is
disposable and was not committed anywhere; it lives only under
`/tmp/claude-1000/-home-cvandesande-github-corvette/7778fb74-5a60-4066-a534-c5a17a01dad5/scratchpad/r1-moq/`,
outside this repo's working tree, and is not part of this item's diff.

## 5. `js/hang`'s Web Component package (for U2)

Literal `js/hang` (`/js/hang/package.json` in the moq-dev/moq monorepo):

```json
{
  "name": "@moq/hang",
  "version": "0.4.0",
  "description": "WebCodecs-based media format for MoQ",
  "license": "(MIT OR Apache-2.0)"
}
```

**Finding for U2, stated plainly so it isn't lost:** `@moq/hang` is the underlying WebCodecs/
catalog/container library — it defines no `customElements.define(...)` anywhere under
`js/hang/src`. The actual Web Components live in two sibling packages that depend on `@moq/hang`:

- `js/watch/package.json` → `@moq/watch` v0.5.0, registers `customElements.define("moq-watch",
  MoqWatch)` (`js/watch/src/element.ts:518`) and a companion `<moq-watch-ui>`
  (`js/watch/src/ui/element.ts:238`). This is the playback/viewer element — the one a live-view
  grid tile (U2's actual target) would vendor.
- `js/publish/package.json` → `@moq/publish` v0.4.4, registers `customElements.define("moq-publish",
  MoqPublish)` (`js/publish/src/element.ts:463`) — a publisher-side element, not relevant to U2.

Both declare `@moq/hang: "workspace:^"` as a dependency. So: if U2 vendors literally what the
plan's own R1 text names (`js/hang`), that's `@moq/hang` v0.4.0 with no custom element to mount;
if U2's actual need is a droppable `<moq-watch>` tag (which the live-view grid-tile use case
implies), the package to vendor is `@moq/watch` v0.5.0, which pulls `@moq/hang` in as its own
dependency. Recording both so U2 doesn't have to re-derive this.

## Summary

- **Pinned commit:** `7b73c43381a7f9c309e3045a8f0f858aa32ef48c` (2026-08-16), chosen over
  moq-dev/moq's current HEAD (`704734774d7cd3a26bb381a5edffb3a207774dc8`, 2026-08-26, 65 commits
  ahead) because the diff between them rewrites the exact `Import`/`Split` API surface this item
  verifies, and nobody has read source at HEAD.
- **`cargo build --workspace`** (this repo, with the pin added, no consumer yet): exit 0, clean
  under `[workspace.lints]`; `cargo clippy --workspace --all-targets` likewise clean.
- **`moq-relay` standalone**: built release in the scratch checkout, exit 0; two independently
  started local processes completed a real QUIC handshake (`session accepted transport=quic`,
  `negotiated version=moq-lite-05` on both ends) with no Redis, no `moq-api`/`--auth-api`, CLI
  flags only.
- **Codec-level API**: `h264::Split`/`h264::Import`/`aac::Import` all built and ran to completion
  against synthetic Annex-B and raw-AAC input, no MPEG-TS, exit 0, no panic.
- **`js/hang` for U2**: `@moq/hang` v0.4.0 (no Web Component); the actual custom element is
  `@moq/watch` v0.5.0 (`<moq-watch>`), which depends on `@moq/hang`.
- Mutation: N/A per the plan (Significant item, self-evident oracle) — not fabricated.
