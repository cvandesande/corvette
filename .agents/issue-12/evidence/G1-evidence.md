# G1 — `crates/corvette-media-bridge`: wires `corvette-rtsp-client` to `rtsp-restream` and to a direct MoQ-publish loop

Item G1, `.agents/issue-12/PLAN-live-view.md`. All commands below were run for real against a
real, locally-built `moq-relay` binary (pinned to the exact commit R1's own evidence pins,
`7b73c43381a7f9c309e3045a8f0f858aa32ef48c`); output is pasted verbatim (only truncated where
noted), not paraphrased. Base commit (parent of this item's own commit):
`d4941fa2d26f547af79ddcf7dbef4b52989d9a5c`.

## 1. Module layout

```
crates/corvette-media-bridge/
  Cargo.toml
  src/
    main.rs              -- entrypoint: load config, start(), drive the RTSP listener
    lib.rs                -- start(): wires every configured camera to both roles + hosts
                              the one whole-process rtsp_restream::RtspServer
    config.rs              -- Config/CameraSpec/MoqConfig + load_from_env (Do step 2)
    restream_provider.rs    -- role (a): StreamProvider dispatch + Frame adapter (Do step 3a/4)
    moq_publish.rs           -- role (b): MoQ-publish loop, dials the relay directly (Do step 3b)
    rtp_clock.rs              -- shared RTP-timestamp -> elapsed-Duration conversion
    supervise.rs               -- restart-on-panic convention (INV-5(b), DP-3)
  tests/
    g1_integration.rs           -- real moq-relay + real mock camera, ignored by default
```

## 2. Camera configuration: how it's read, and why this shape

A single env var, `CORVETTE_MEDIA_BRIDGE_CONFIG`, names a small JSON file
(`config::load_from_env` / `config::load_from_path`). The file carries the whole-process RTSP
bind address, the MoQ relay to dial (`relay_url`, `tls_disable_verify`), and a list of cameras
(`name`, `host`, `port`, `path`, `username`, `password`, optional `channel_capacity`) --
exactly the fields `corvette_rtsp_client::client::CameraConfig::new` needs, per issue #18's
already-shipped public API (confirmed by direct source read of
`crates/corvette-rtsp-client/src/client.rs`).

This matches issue #18's own C1 precedent, quoted directly from its own plan entry: "accept a
config value the same stub-shaped way ... and name it as a known stub in a code comment, not a
TODO without an owner." `docs/design/architecture.md`'s "Risk boundary" section is explicit that
Corvette does not yet own configuration generally, so this item does not attempt a general
solution -- it reads a JSON file via `serde_json` (already a workspace dependency, used
identically by `corvette-api`), rejects a config that names the same camera twice (since this
crate names both a camera's RTSP-restream stream and its MoQ broadcast path after that one
name, a collision would make one camera's traffic indistinguishable from another's), and does
not attempt camera discovery from Frigate's own `config.yml` (out of scope per the plan's Scope
guard). `config.rs`'s own unit tests (`parses_a_minimal_config`,
`rejects_duplicate_camera_names`) exercise both paths.

## 3. Verify — executed against a REAL, locally-started `moq-relay` (not a mock)

### 3.1 Building the pinned `moq-relay`, in an isolated scratch checkout (same commit R1 pinned)

```
$ git clone https://github.com/moq-dev/moq.git repo && cd repo
$ git checkout 7b73c43381a7f9c309e3045a8f0f858aa32ef48c
$ git rev-parse HEAD
7b73c43381a7f9c309e3045a8f0f858aa32ef48c
$ nix develop /home/cvandesande/github/corvette --command cargo build --release --bin moq-relay
   ...
    Finished `release` profile [optimized] target(s) in 56.95s
$ ls -la target/release/moq-relay
-rwxr-xr-x ... 42.7M ... target/release/moq-relay
$ ./target/release/moq-relay --version
moq-relay 0.14.11-7b73c433
```

The scratch checkout lives outside this repo's working tree (under this session's own
scratchpad directory), matching R1's own precedent, and is not part of this item's diff.

### 3.2 The integration test itself (`tests/g1_integration.rs`)

Reused directly, per the plan's own instruction: issue #18's own
`corvette_rtsp_client::mock_camera::MockCamera` test double, unmodified. **A real, confirmed
limitation of that reused test double, stated honestly rather than papered over**: direct source
read of `crates/corvette-rtsp-client/src/mock_camera/{rtp.rs,rtsp_message.rs}` confirms it
fabricates RTP payloads carrying a bare NAL header byte (`0x65`, IDR) followed by 8 bytes of an
arbitrary frame counter -- never a real SPS/PPS, and its SDP `DESCRIBE` response declares no
`sprop-parameter-sets` either. It was built to test issue #18's own depacketizer's NAL/FU-A
framing, not to produce decodable video; this is the first item asking it to also resolve a real
catalog or DESCRIBE with real parameter sets, and it cannot.

Given that, the test wires **two** cameras through this crate's own real, unmodified
`restream_provider`/`moq_publish` code:

- **`cam-mock`**: a real `corvette_rtsp_client::client::Client` dialing a real `MockCamera`
  instance. Proves the real RTSP dial/depacketize/reconnect/task-wiring path holds end to end,
  and that this crate's own per-frame error handling (a "keyframe" with no SPS) degrades
  gracefully -- logs and drops, never panics -- rather than crashing the publish task.
- **`cam-synth`**: a second, independent `corvette_rtsp_client::depacketize::Frame` broadcast,
  fed directly (no RTSP dial at all) with a real, valid H.264 SPS/PPS/IDR triple -- the exact
  fixture bytes `moq-mux`'s own `codec::h264::import::tests::avc3_self_initializes_from_first_keyframe`
  unit test uses, read directly from the pinned commit's source, not invented. This is **not** a
  second RTSP mock server (no RTSP protocol code is duplicated); it drives this crate's own
  `restream_provider::run_restream_feed`/`moq_publish::run_publish_loop` functions directly, the
  exact same functions `lib.rs::start` wires a real `Client`'s subscriptions to. This mirrors two
  precedents already accepted in this same plan: X2's "a small depacketizer written for this test
  only, since [the existing one] is not wired into any shipped consumer," and R1's own synthetic
  Annex-B SPS/PPS/IDR harness proving the codec-level `Split`/`Import` API.

Both cameras run through one `MultiCameraProvider`, one whole-process `rtsp_restream::RtspServer`,
and dial the same real local `moq-relay`.

### 3.3 Full run, `--ignored --nocapture`, against the real relay (log level raised to `debug` and
the relay's own stdout/stderr set to inherit for this one capture, to show its own log lines
alongside the client side; reverted to the committed test's quieter `info`/piped defaults
immediately after)

```
$ MOQ_RELAY_BIN=<pinned moq-relay binary> cargo test -p corvette-media-bridge --test g1_integration \
    --features corvette-rtsp-client/mock-camera -- --ignored --nocapture --test-threads=1

running 1 test
test g1_end_to_end_against_a_real_relay_and_mock_camera ...
2026-08-26T12:31:51.650278Z  INFO moq_relay::cluster: cluster initialized origin_id=1559783776242224 configured=false
2026-08-26T12:31:51.651062Z  INFO moq_relay::relay: listening addr=127.0.0.1:34097
2026-08-26T12:31:51.651081Z  INFO moq_relay::cluster: no cluster peers configured; running standalone
corvette-rtsp-client camera="cam-mock" event=connect detail="codec=H264 clock_rate=90000"
2026-08-26T12:31:51.952214Z DEBUG moq_native::quinn: accepting host= ip=127.0.0.1:50799 alpn=h3
2026-08-26T12:31:51.953402Z DEBUG web_transport_quinn::connect: received CONNECT request request=ConnectRequest { url: Url { ..., host: Some(Ipv4(127.0.0.1)), port: Some(34097), path: "/anon", ... }, protocols: ["moq-lite-05", "moq-lite-04", ...], ... }
2026-08-26T12:31:51.953417Z DEBUG web_transport_quinn::connect: sending CONNECT response response=ConnectResponse { status: 200, protocol: Some("moq-lite-05") }
corvette-media-bridge unit="cam-mock" event=moq-publish-connect detail="relay=https://127.0.0.1:34097/anon"
corvette-media-bridge unit="cam-mock" event=moq-publish-frame-error detail="splitting an access unit: h264: NAL unit is too short"
2026-08-26T12:31:51.954521Z  INFO conn{id=0}: moq_relay::connection: session accepted transport=quic role=Some(Publisher) tier= root=anon publish= subscribe=
2026-08-26T12:31:51.954577Z  INFO conn{id=0}: moq_relay::connection: negotiated version=moq-lite-05 transport=quic
corvette-media-bridge unit="cam-synth" event=moq-publish-connect detail="relay=https://127.0.0.1:34097/anon"
2026-08-26T12:31:51.954891Z  INFO conn{id=1}: moq_relay::connection: session accepted transport=quic role=Some(Publisher) tier= root=anon publish= subscribe=
2026-08-26T12:31:51.954915Z  INFO conn{id=1}: moq_relay::connection: negotiated version=moq-lite-05 transport=quic
2026-08-26T12:31:51.955277Z DEBUG moq_net::lite::subscriber: announce broadcast=anon/cam-mock hops=1
2026-08-26T12:31:51.955558Z DEBUG moq_net::lite::subscriber: announce broadcast=anon/cam-synth hops=1
corvette-media-bridge unit="cam-mock" event=moq-publish-frame-error detail="splitting an access unit: h264: NAL unit is too short"
verify(b): RTSP DESCRIBE/SETUP/PLAY against cam-synth received the real SPS/PPS/IDR byte-for-byte
2026-08-26T12:31:52.021462Z  INFO conn{id=2}: moq_relay::connection: session accepted transport=quic role=Some(Subscriber) tier= root=anon publish= subscribe=
2026-08-26T12:31:52.021499Z  INFO conn{id=2}: moq_relay::connection: negotiated version=moq-lite-05 transport=quic
2026-08-26T12:31:52.021542Z DEBUG moq_net::lite::publisher: announce broadcast=anon/cam-mock
2026-08-26T12:31:52.021547Z DEBUG moq_net::lite::publisher: announce broadcast=anon/cam-synth
2026-08-26T12:31:52.022274Z DEBUG moq_net::lite::publisher: track info requested broadcast=anon/cam-synth track=catalog.json
2026-08-26T12:31:52.023156Z  INFO moq_net::lite::publisher: subscribed started id=0 broadcast=anon/cam-synth track=catalog.json
2026-08-26T12:31:52.023180Z  INFO moq_net::lite::subscriber: subscribe started id=0 broadcast=anon/cam-synth track=catalog.json
2026-08-26T12:31:52.023468Z DEBUG moq_net::lite::publisher: serving group subscribe=0 track=catalog.json sequence=1
2026-08-26T12:31:52.023615Z DEBUG moq_net::lite::publisher: finished group sequence=1
verify(a): moq_net subscriber received a 219-byte cam-synth catalog frame
2026-08-26T12:31:52.023857Z DEBUG moq_net::lite::publisher: track info requested broadcast=anon/cam-synth track=video
2026-08-26T12:31:52.024372Z  INFO moq_net::lite::publisher: subscribed started id=1 broadcast=anon/cam-synth track=video
2026-08-26T12:31:52.024381Z  INFO moq_net::lite::subscriber: subscribe started id=1 broadcast=anon/cam-synth track=video
2026-08-26T12:31:52.024638Z DEBUG moq_net::lite::publisher: serving group subscribe=1 track=video sequence=2
verify(a): moq_net subscriber received a 46-byte cam-synth video frame
corvette-media-bridge unit="cam-mock" event=restream-feed-disconnect detail="camera client dropped"
2026-08-26T12:31:52.025253Z  WARN moq_relay::relay: connection closed err=transport: webtransport error: closed: code=0 reason=dropped
2026-08-26T12:31:52.270225Z  INFO moq_net::lite::publisher: subscribed started id=2 broadcast=anon/cam-synth track=video
2026-08-26T12:31:52.270265Z  INFO moq_net::lite::subscriber: subscribe started id=2 broadcast=anon/cam-synth track=video
2026-08-26T12:31:52.270816Z DEBUG moq_net::lite::publisher: serving group subscribe=2 track=video sequence=14
verify(c): cam-synth's RTSP and MoQ outputs both survived cam-mock's Client/tasks being killed
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.65s
```

The relay's own log lines confirm, independently of the test's own assertions: both broadcasts
really were announced to the relay (`announce broadcast=anon/cam-mock`, `announce
broadcast=anon/cam-synth`), a real subscriber session really was negotiated
(`role=Some(Subscriber)`, `negotiated version=moq-lite-05`), the catalog and video tracks were
really requested and served (`track info requested ... track=catalog.json`, `serving group
... track=catalog.json sequence=1`; the same for `track=video`), `cam-mock`'s connection really
closed when its `Client` was dropped (`connection closed err=... reason=dropped`), and a **third**,
independent subscribe against `cam-synth`'s video track (`subscribed started id=2 ...
sequence=14`) succeeded well after that -- sequence 14 versus sequence 2 earlier confirms
`cam-synth` kept advancing and producing frames the whole time, unaffected by `cam-mock`'s own
teardown.

Re-run three times after reverting the temporary debug-logging/inherit-stdio change back to the
committed test's quieter defaults (`--log-level info`, piped stdout/stderr) to confirm this was
not a one-off:

```
$ MOQ_RELAY_BIN=<pinned moq-relay binary> cargo test -p corvette-media-bridge --test g1_integration \
    --features corvette-rtsp-client/mock-camera -- --ignored --nocapture --test-threads=1
...
verify(b): RTSP DESCRIBE/SETUP/PLAY against cam-synth received the real SPS/PPS/IDR byte-for-byte
verify(a): moq_net subscriber received a 219-byte cam-synth catalog frame
verify(a): moq_net subscriber received a 46-byte cam-synth video frame
corvette-media-bridge unit="cam-mock" event=restream-feed-disconnect detail="camera client dropped"
verify(c): cam-synth's RTSP and MoQ outputs both survived cam-mock's Client/tasks being killed
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.64s
```

(Run twice more with identical `ok` results; timings ranged 0.62-0.65s across all runs, no
flakiness observed.)

**A real regression this item's own re-verification caught and fixed before landing**: an earlier
refactor (extracting the verify(a)/verify(b) blocks into named helper functions, done only to
satisfy `clippy::too_many_lines`) accidentally let the subscriber's own `moq_net::Session` handle
drop at the end of its helper function -- per `moq_net`'s own documented contract ("the transport
still closes when the last Session clone drops"), this closed the subscriber's QUIC connection
early and broke verify(c)'s later re-read with `Error::Dropped`. Caught by re-running the real
test (not by inspection), fixed by returning the session handle alongside the broadcast consumer
and holding it for the test's full duration, and reconfirmed with three clean re-runs above.

### 3.4 Interpreting `cam-mock`'s own errors

`cam-mock`'s repeated `moq-publish-frame-error` lines ("NAL unit is too short", "not
initialized", "SPS NAL too short") are `MockCamera`'s fabricated, non-decodable payload being
correctly rejected by `moq_mux`'s own `Split`/`Import` layer -- not a bug in this item. The
important property they demonstrate is negative: the publish task never panics over them, matches
this crate's own documented per-frame error-tolerance design (log and drop, keep running), and
`cam-mock`'s own broadcast is still announced to the relay (confirmed by the relay's own
`announce broadcast=anon/cam-mock` log line above) even though its video track never resolves a
usable catalog.

### 3.5 `cargo clippy`/`cargo test`/`cargo fmt`

```
$ cargo clippy --workspace --all-targets --all-features --locked
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 14.70s
(0 warnings, 0 errors)

$ cargo test --workspace
(every one of 25 test binaries in the workspace reports "test result: ok", 0 failed)

$ cargo fmt --check -p corvette-media-bridge
(clean, no diff)
```

One pre-existing, unrelated flake was observed and confirmed NOT caused by this item:
`corvette-rtsp-client`'s own `tests/smoke.rs::disconnect_closes_the_socket_immediately_unlike_the_silent_stall`
failed once under the full `cargo test --workspace`'s parallel load (`Elapsed(())`, a timing
assertion) but passes cleanly every time when run in isolation
(`cargo test -p corvette-rtsp-client --test smoke disconnect_closes_the_socket_immediately_unlike_the_silent_stall`).
This crate is untouched by G1 (scope guard forbids editing it); the flake is a pre-existing
timing sensitivity under system load, not a regression this item introduced.

## 4. The audio-track gap — stated honestly, per this item's own Premise

`corvette-rtsp-client`'s own SDP resolution (`crates/corvette-rtsp-client/src/client/task.rs`'s
`build_depacketizer`) only ever resolves an `m=video` section; no audio track is ever
depacketized or published as a `Frame`, and this item does not fix that (explicitly out of scope
per the plan's Scope guard). Both of this item's own roles are written generically over
`corvette_rtsp_client::depacketize::Codec` (`restream_provider::ParameterSetCache::observe`,
`moq_publish::new_track`), so a future fix to that upstream gap needs no change here -- but as of
this commit, **no AAC data has ever been exercised end to end**, including in this item's own
integration test (`cam-mock`/`cam-synth` are both H.264-only). `moq_publish.rs`'s own `new_track`
returns `FrameError::AacUnavailable` rather than fabricating an `AudioConfig` if a `Codec::Aac`
frame were ever to arrive, since `aac::Import::new` requires a resolved sample rate/channel
count/`AudioSpecificConfig` this crate has no source for without that upstream fix.

## 5. Mutation

See `.agents/issue-12/evidence/G1-mutation.log` for the full FAIL-then-PASS cycle (INV-5(b):
restart-on-panic recovery for one camera's MoQ-publish task, with the sibling camera's own
recovery-independent liveness confirmed directly throughout, matching INV-5(a)'s own
unconditional-under-Tokio framing).
