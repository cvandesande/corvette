const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Issue #12 item U1's own real-endpoint Verify step: the grid tile's own
// production client (`crates/corvette-ui/src/live_view.rs`) drives a real
// MSE `SourceBuffer` to `HAVE_CURRENT_DATA` against G2's REAL
// fMP4-over-WebSocket output -- the same
// `crates/corvette-media-bridge/examples/g2_browser_fixture.rs` process G2's
// own browser check (`crates/corvette-media-bridge/tests/browser/
// g2_fmp4_ws.spec.cjs`) drives with a hand-rolled page script, exercised
// here instead through the actual shipped UI.
//
// `U1_LIVE_VIEW_WS_ORIGIN` and `U1_LIVE_VIEW_CAMERA_NAME` are read from the
// environment: `scripts/run_u1_live_view_browser_check.sh` starts the
// fixture on an ephemeral port and passes its origin and camera name in
// here, mirroring `run_g2_fmp4_ws_browser_check.sh`'s own shape. When unset
// -- e.g. a plain `make check-ui` run, which starts no fixture -- this test
// is skipped rather than failed; the dedicated script above is what
// actually exercises it against a real process.
const wsOrigin = process.env.U1_LIVE_VIEW_WS_ORIGIN;
const cameraName = process.env.U1_LIVE_VIEW_CAMERA_NAME;

// Issue #12 item U1's own mutation oracle (INV-7): unlike the real-endpoint
// test below, this one needs no live `corvette-media-bridge` fixture -- only
// a mocked camera list -- so it always runs as part of a plain `make
// check-ui`, giving the iframe-removal mutation cycle a fast, deterministic
// assertion to falsify and restore.
test("the grid tile renders no iframe, with or without a live media-bridge endpoint", async ({
  page,
}) => {
  await page.route("**/api/config", (route) =>
    route.fulfill({
      json: { cameras: { front: { enabled: true, friendly_name: "Front", ui: { order: 0 } } } },
    }),
  );
  await page.route("**/api/review?*", (route) => route.fulfill({ json: [] }));
  await page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] }));

  await page.goto("/");

  await expect(page.locator(".camera-grid video")).toHaveCount(1);
  await expect(page.locator("iframe")).toHaveCount(0);
});

test("the grid tile's video element reaches HAVE_CURRENT_DATA against G2's real endpoint", async ({
  page,
}) => {
  test.skip(
    !wsOrigin,
    "U1_LIVE_VIEW_WS_ORIGIN is not set -- run via scripts/run_u1_live_view_browser_check.sh",
  );

  const consoleErrors = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      consoleErrors.push(message.text());
    }
  });

  // Runs before any page script, including the WASM app's own `mount()` --
  // this is what lets `live_view::websocket_url` dial the real fixture
  // directly instead of this page's own origin (there is no nginx proxy in
  // this harness; issue #12 item N2 is what adds that in production). See
  // `live_view.rs`'s own doc for why this hook exists only for tests.
  await page.addInitScript((origin) => {
    window.__corvetteMediaBridgeWsOrigin = origin;
  }, wsOrigin);

  await page.route("**/api/config", (route) =>
    route.fulfill({
      json: {
        cameras: {
          [cameraName]: { enabled: true, friendly_name: "Browser check", ui: { order: 0 } },
        },
      },
    }),
  );
  await page.route("**/api/review?*", (route) => route.fulfill({ json: [] }));
  await page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] }));

  await page.goto("/");

  const video = page.locator(".camera-grid video");
  await expect(video).toHaveCount(1);
  // The iframe this item removed is gone from the whole page, not only the
  // grid tile -- the strongest form of this item's own "no dual-path
  // fallback" requirement a browser-level check can make.
  await expect(page.locator("iframe")).toHaveCount(0);

  await expect
    .poll(() => video.evaluate((element) => element.readyState), { timeout: 15_000 })
    .toBeGreaterThanOrEqual(2);

  // Two known-benign lines this harness itself produces, unrelated to this
  // item's own correctness: no favicon is served here, and `cargo-leptos`'s
  // release bundle still wires its dev-mode hot-reload WebSocket client,
  // which has nothing to connect to outside `cargo leptos watch`.
  const unexpectedErrors = consoleErrors.filter(
    (line) => !line.includes("favicon") && !line.includes("live_reload"),
  );
  expect(
    unexpectedErrors,
    "video reached HAVE_CURRENT_DATA, but the page logged unexpected console errors",
  ).toEqual([]);
});
