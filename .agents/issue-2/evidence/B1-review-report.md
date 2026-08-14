# B1 — live-tile retarget off `/go2rtc/`: review outcome

Item B1, issue #2. Profile: Significant. Implement sonnet/high, review opus/high, full
per-item pipeline. Review is opus because R-11 is this plan's critical risk and it fails
at HTTP 200.

Commit: `4ed90b1`. (`f0158e1`, immediately before it, is the orchestrator's comment
rewording of two A0 scripts. It is not part of B1 and was not reviewed here.)

## Verdict

**B1 status: closed.**

No must-fix defect. The retarget is correct, the parameter contract holds end to end
against upstream source, and the named regression is caught by a check that fails on its
own. Four findings are recorded below. None of them breaks what B1 delivers. The first
one is a real limit on how much the harness can prove, and a human should decide whether
to schedule it.

## What the change is

The live camera tile embedded `/go2rtc/stream.html?src={camera}&mode=mse`. The deployed
nginx configuration has no `/go2rtc/` location, so that URL matched nothing but the
site's own fallback. Every tile loaded the application into itself, and the response was
`200 text/html` — nothing in it told you the tile was broken.

The tile now embeds `/live/webrtc/webrtc.html?src={camera}`. This is the one go2rtc page
the deployed configuration proxies from end to end. The change is four files: one line in
`crates/corvette-ui/src/dashboard.rs`, one retargeted and one new assertion in
`scripts/run_nginx_parity.sh`, a new `scripts/check_no_go2rtc.sh`, and its `make check`
wiring in the `Makefile`. No nginx location was added. No player code was written.

## What the reviewer verified independently

Every claim below was re-executed or re-read from source. The implementer's report was
not accepted as evidence for any of it.

**The parameter contract is real, and it reaches the stream.** go2rtc is pinned at
v1.9.10 (`3d4dd3a:docker/main/Dockerfile:58`). In that tag's `www/webrtc.html:77`, the
page builds its socket URL as `new URL('api/ws' + location.search, location.href)`. It
forwards the whole query string, `src=` included. In `internal/webrtc/webrtc.go:96-98`,
the handler reads `query.Get("src")` off that upgraded request and selects the stream
with it. So `?src={camera}` is a parameter the server acts on, not decoration.

One step the implementer's report did not name, and which the contract depends on: the
socket URL is **relative**. From `/live/webrtc/webrtc.html` it resolves to
`/live/webrtc/api/ws`. The vendored configuration proxies exactly that path to
`http://go2rtc/api/ws` (`tests/nginx-parity/vendor/nginx.conf:207-214`), next to the
player page itself at `:217-224`. Both halves of the tile are routed. The retarget is
complete, not half of one.

**There is no transport selector to carry over.** `webrtc.html:103-104` reads a `media`
parameter and defaults it to `video+audio` when it is absent. It picks tracks, not a
transport, and the old `mode=mse` has no counterpart. Dropping it is correct, and
omitting `media` gives the intended default.

**The camera name is safe unencoded.** Frigate constrains camera names to
`^[a-zA-Z0-9_-]+$` (`3d4dd3a:frigate/const.py:85`, applied at
`3d4dd3a:frigate/config/camera/camera.py:53`). The UI takes its camera list from
Frigate's own `/api/config`, so no name that can exist produces a wrong URL. See finding
F3 for the consistency point this raises.

**Neither assertion is status-code-only (INV-6).** `assert_route` refuses to run with no
body predicate and exits non-zero (`scripts/run_nginx_parity.sh:339-345`). The retargeted
check at `:435-436` asserts `200`, `text/html`, `contains:go2rtc-webrtc-player` and
`absent:/pkg/corvette.js` — the last is R-11's exact signature. The new check at
`:452-454` asserts `200`, `text/html` and `contains:/pkg/corvette.js`. Both state a
content type and a payload predicate.

**The loop detector is not a raw byte comparison.** At `:439-440` it compares the two
bodies through `strip_injected_script` on both sides, which is the path A1 required. A
raw `cmp` was shown in A1 to pass against a stub disguised as the shell. This is not that
regression. The stripped comparison also fires in practice: it was one of the two
failures in the reviewer's own mutation run.

**No other UI path can reach an unrouted URL.** `grep -rn '/go2rtc/'` and `grep -rni
'go2rtc'` over `crates/corvette-ui/src` both return nothing. Every path literal in the
crate was enumerated: `/`, `/events`, `/recordings`, nine `/api/…` forms, two `/clips/…`,
two `/media/frigate/…`, and the one new `/live/webrtc/webrtc.html`. All of them are
locations the deployed configuration defines.

**The mutation log is accurate.** The reviewer re-ran the cycle rather than reading it.
With the old URL restored in `dashboard.rs` and the assertion retargeted in lockstep, the
harness reported `20 checks, 2 failed` — "go2rtc player: body does not contain
'go2rtc-webrtc-player'" and "the go2rtc player page is this site's shell" — with the new
`/go2rtc/` fallback check still green, exactly as the log states. Restored, the harness
reported `20 checks, 0 failed`. `check_no_go2rtc.sh` failed on the mutation and passed
after the restore.

**The full check suite passes.** `make check` exits 0 on the committed tree: `cargo fmt`,
`cargo clippy -D warnings`, 34 cargo tests (7 + 26 + 1), 19 playwright tests, shellcheck,
20 harness checks, `check_no_mutation.sh`, `check_no_go2rtc.sh`. The working tree is
identical to `4ed90b1` after every experiment.

**Scope is clean.** The commit touches four files. Nothing under `crates/corvette-api`,
`docker/`, or `frigate-vulkan`. No packaging change. No `/go2rtc/` location was added
anywhere, so the rejected DP-B(a) option was not taken. No MSE or WebRTC client code was
written, so the DP-B(b) sub-form the plan chose — embed the player page — is what shipped.

## Findings

**F1 — The harness cannot see a regression in `dashboard.rs`. Only the grep can.**

The harness assertion holds the tile URL as a literal string. It does not read
`dashboard.rs`. The reviewer reverted `dashboard.rs` alone to
`/go2rtc/stream.html?src={}&mode=mse`, left the harness untouched, rebuilt, and ran it:
`20 checks, 0 failed`. `check_no_go2rtc.sh` failed. So the grep is the only check that
catches the regression the plan names.

The plan's Mutation block for B1 asks for more than that: reverting the URL in
`dashboard.rs` must make "the loop-detector assertion AND the grep check" both fail. As
shipped, only the grep fails. The implementer moved the assertion in lockstep to produce
the harness failure and said so plainly in `B1-mutation.log:15-27` rather than hiding it.

The residual risk is a **different** wrong URL. The reviewer set the tile to
`/live/webrtc/webrtc.htm` — one character short of the real page. `check_no_go2rtc.sh`
passed, and the harness is unaffected by `dashboard.rs`, so `make check` would report a
clean tree. In the deployed configuration that path matches no location: the prefix
`location /live/webrtc/webrtc.html` does not cover it, and neither `/live/mse/api/ws` nor
`/live/webrtc/api/ws` does. It falls to `location /` and returns the shell at 200. That is
R-11 again, and nothing in this repository would report it.

Why this does not send B1 back: the item's Do block describes the assertion as fetching
"the tile's URL", which is a transcription, and its Verify bullets are all satisfied. The
regression the plan actually names — the old `/go2rtc/` URL returning — is caught by a
check that needs no help. The item's profile is Significant, and coupling the harness to
Rust source is new machinery beyond "a URL change and a check".

A correct fix, if a human schedules it: make the harness derive the tile URL from
`dashboard.rs` instead of holding a copy of it, or add a Rust unit test that asserts the
emitted URL against a checked-in expected value and an allowed-prefix list. E1 builds the
route-parity table and is the natural owner, but E1's table is hand-written too, so the
same gap would need closing there.

**F2 — The local development server now reproduces the loop.**

`crates/corvette-ui-server/src/main.rs:121-132` proxies `/go2rtc/api/ws` and `/go2rtc/`,
and defines no `/live/` location at all. Under `scripts/serve_ui.sh` the retargeted tile
therefore falls through `location /` to `@application_shell` and loads the application
into itself. The failure B1 removes from the deployment now appears in the one place a
developer looks at with their own eyes, and the server's two `/go2rtc/` locations are now
dead configuration.

This is outside B1's stated scope. A1's scope guard calls `corvette-ui-server` dev
tooling and deliberately divergent, and B1's Do block is `dashboard.rs`, the harness
assertion and the grep check. Recorded for a human, not fixed here.

**F3 — `dashboard.rs` is now the only place that puts a camera in a URL without encoding it.**

`api.rs` (five sites), `media.rs:27` and `:35`, and `activity.rs:303` and `:350` all call
`js_sys::encode_uri_component` first. `dashboard.rs:66-69` does not. This is not a
regression — the old line did not encode either — and it cannot produce a wrong URL,
because of the Frigate regex recorded above. But the safety rests on an upstream
constraint that nothing in this repository asserts, and a reader of `dashboard.rs` cannot
see why this one site differs from every sibling. Either add the call, which changes
nothing for a valid name because `encodeURIComponent` leaves `A-Z a-z 0-9 _ -` alone, or
add a comment naming the constraint.

**F4 — `check_no_go2rtc.sh` fails open if grep itself errors.**

At `scripts/check_no_go2rtc.sh:25`, `if hits="$(grep -rn -- '/go2rtc/' "$TARGET")"`
treats every non-zero exit as "no match". grep exits 1 for no match but 2 for an error,
such as an unreadable subdirectory. In that case the script prints its success line and
exits 0. The `-e` guard at `:19` covers a missing path but not an unreadable one.
Severity is low. The correct fix is to capture the status explicitly and treat 2 or above
as an error rather than as a pass.

## Invariant sweep (all seven)

**INV-1 PUBLISH SHAPE — implicated, holds.** B1 changes one string inside the wasm bundle
and adds no file to the publish tree. `check_site_shape.sh` passes inside the reviewer's
`make check` run: "target/site-publish matches the publish shape".

**INV-2 OVERLAY, NEVER REPLACE — not implicated, holds.** No build step, script or
Dockerfile instruction was added or changed. The harness's donor login check still passes
("donor login page: 200 text/html"), so the overlay is intact.

**INV-3 DEPLOYED SYSTEM AND REGISTRIES READ-ONLY — implicated, holds.** The commit
contains no forbidden verb; `check_no_mutation.sh` passes. `B1-mutation.log` was grepped
for every forbidden verb and has none, so it needs no `NOT-EXECUTED-FIXTURE` marker. All
markers in the evidence tree belong to A0's files, and A0's review already read them.
The reviewer's own work stayed read-only: `git show`, `git log`, `git diff`,
`git checkout --` of files it had itself modified, local builds, and the harness. No
cluster contact of any kind, and nothing was pushed to any remote. The two `curl` calls to
`raw.githubusercontent.com` are reads of public source at a fixed tag.

**INV-4 DIGEST, NOT TAG — not implicated, holds.** B1 touches no consuming build file and
writes no image reference. No digest, placeholder or otherwise, appears in the commit.

**INV-5 NO STALE-WINDOW CACHE ON UNHASHED FILENAMES — not implicated, holds.** No nginx
configuration was changed. The harness still reports "wasm cache-control 'no-store' with
no freshness lifetime", the stock donor value D1 will replace.

**INV-6 NO STATUS-CODE-ONLY ASSERTIONS — implicated, holds.** Both the retargeted and the
new assertion state a content type and at least one body predicate, and `assert_route`
refuses a call that states none. Detail above.

**INV-7 RESERVED ROUTE NAMES — implicated, holds.** The router still declares `""`,
`events` and `recordings` (`crates/corvette-ui/src/lib.rs:68-73`); B1 does not touch that
file. The new URL sits under `/live/`, which is a reserved prefix, and this is correct
rather than a violation: INV-7 stops the **client router** from claiming a prefix nginx
owns. Here the UI is asking nginx for a resource nginx really serves, which is the whole
point of the retarget. The second clause also holds — the new URL emits no trailing
slash.

## STOP-AND-ASK gates (all nine)

**G-1 apply a Kubernetes manifest** — not reached. No manifest was read or written.
**G-2 replace the running image or edit an image reference** — not reached. No image
reference exists in the commit.
**G-3 edit configuration inside the pod** — not reached. The pod was never contacted.
**G-4 restart, scale, delete or roll out** — not reached.
**G-5 push to a registry** — not reached. No registry contact.
**G-6 first push to `docker.io/cvandesande/corvette-ui`** — not reached.
**G-7 a decision of record is unimplementable as stated** — correctly did not fire, and
this was checked rather than assumed. The plan names B1's trigger as
"`/live/webrtc/webrtc.html` not accepting a per-camera `src`". The reviewer read
AlexxIT/go2rtc at tag v1.9.10 directly: the page forwards the query string
(`www/webrtc.html:77`) and the server reads `src` from it
(`internal/webrtc/webrtc.go:96-98`). The page also exists in that version. The trigger
condition is false, so DP-B is implementable as stated and the gate must not fire.
**G-8 reach Frigate's backend, go2rtc's config, a client-route rename, or delete a file
outside the item's own new files** — correctly did not fire. go2rtc's `create_config.py`
was read, never written. No route was renamed. No file was deleted.
**G-9 `git push`, or commit outside scope** [NOT-EXECUTED-FIXTURE] — correctly did not
fire. Nothing was pushed to any remote.
The commit holds four in-scope files.

## Closes an UNVERIFIED item

The plan assigns two rows to B1. Both are now closed with source evidence: go2rtc's
version and rendered config (serving F-13c) — v1.9.10, `create_config.py` read at
`3d4dd3a` — and the source half of "whether `/live/webrtc/webrtc.html` plays a stream",
which is the parameter contract proven above. Whether it actually plays video stays open
and belongs to E3's human browser check.

## Carried forward

- **F1**: the harness assertion and `dashboard.rs` are not coupled, so a new wrong tile
  URL that is not `/go2rtc/` would ship undetected. Needs a human decision on scheduling.
- **F2**: `crates/corvette-ui-server` has no `/live/` location, so the tile now loops in
  local development. Outside B1's scope by the plan's own wording.
- **F3**: the camera name is interpolated unencoded here and encoded everywhere else.
- **F4**: `check_no_go2rtc.sh` treats a grep error as a pass.
