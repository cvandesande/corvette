# Deployed-parity nginx harness

Serves the publish tree through Frigate's own nginx configuration, offline, and
asserts what the deployment would return for it.

Run it with `scripts/run_nginx_parity.sh`, or `make check-nginx-parity`, from
inside `nix develop`. It needs the publish tree that `scripts/build_site.sh`
stages at `target/site-publish`.

## Why it exists

The UI is served by nginx configuration that Corvette does not own, whose
failure modes return HTTP 200:

- a client route the fallback never sees, because a filesystem location claims
  its trailing-slash form;
- a page that loads this site's own shell where an external player was meant to
  be, so an iframe nests the app inside itself;
- a missing bundle file answered with the shell instead of a 404, which a wasm
  or module loader reports as a parse error.

None of these is visible to a status-code check, and none of them reproduces
against the local dev server: `crates/corvette-ui-server` writes its own small
nginx configuration and differs from the deployment on the fallback filename,
the `$uri.html` step, cache headers, the MIME table, `sub_filter`, and every
proxied route. This harness is the only place those behaviours can be measured
without touching the deployment.

Every route assertion states a content type and a body predicate as well as a
status; the runner refuses to execute a route check that states no body
predicate.

## What it runs

`vendor/` holds Frigate v0.17.2's nginx configuration, byte for byte; see
`vendor/PROVENANCE` for the revision, the source paths and the md5s.
`offline.patch` carries the only edits, each justified on its hunk header:
unprivileged process settings, writable pid/log/cache paths, upstream
addresses, the two filesystem roots, and the removal of the directives that
belong to nginx modules this nginx does not have.

`fixtures/web/` stands in for the donor's `/opt/frigate/web` — a React-style
`index.html`, `login.html`, `assets/`, `fonts/`, `locales/`, `robots.txt` and
`notifications-worker.js`. The publish tree is laid *over* it, exactly as the
shipped image builds that directory, so the checks can tell an overlay from a
replacement: the site's `index.html` displaces the donor's, and every other
donor file must still be served afterwards.

`fixtures/stub/upstreams.conf` runs a second nginx as the two upstreams the
static path needs: Frigate's `/auth`, so `auth_request` resolves, and go2rtc's
player page. Every other path on those upstreams answers 404 with a body naming
the stub.

## What it cannot prove

- **Nothing about `/vod/`.** The flake's nginx has neither nginx-vod-module nor
  nginx-secure-token-module, so those directives will not parse and the patch
  removes them along with the location they configure.
- **Nothing about the proxied routes** (`/api/*`, `/clips/`, `/stream/`,
  `/exports/`, `/ws`, `/live/*` other than the player page). Their upstreams are
  stubs. The runner refuses an assertion under those prefixes rather than
  letting a fixture's answer be read as the deployment's.
- **Nothing about authentication.** The stub answers every `/auth` subrequest
  202. Frigate's real answer depends on the port the request arrived on and on
  session state, neither of which is modelled here.
- **Nothing about the deployment's own state** — its media tree, its cameras,
  or its configuration. The media root is a fixture.

## Testing a proposed configuration change

Two options exist so a change to the shipped nginx configuration can be proven
here before it is built into an image:

    scripts/run_nginx_parity.sh \
      --extra-patch <patch against the vendored config> \
      --expect tests/nginx-parity/expectations/<profile>

`--extra-patch` is applied after `offline.patch`. The expectations file states
what a missing file under `/pkg/` returns and what `Cache-Control` `/pkg/`
carries; `expectations/donor-config` records what the unmodified configuration
does today, and `expectations/pkg-location` states what a configuration that
gives `/pkg/` its own location must do. Running the second against the
unmodified configuration fails on exactly those two rows, which is what makes
the parameter worth having.

## Ports

18971 (external analogue), 15000 (internal analogue), 15001 (Frigate stub),
11984 (go2rtc stub). The runner refuses to start if any of them is in use, so
it can never report on a server it did not start.
