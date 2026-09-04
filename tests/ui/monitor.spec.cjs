const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Issue #20 item M2's own Verify step: `/monitor` is a new, independently
// bookmarkable client route (`crates/corvette-ui/src/monitor.rs`) that fetches
// the camera list on its own, mirroring every other route's convention (D-5,
// `.agents/issue-20/DESIGN-monitor-mode.md`) rather than sharing Dashboard's
// resource. This only exercises the placeholder-tile scaffold that item adds.
//
// Item M3 (below) wires each tile to its own instance of `expanded_view.rs`'s
// own `ExpandedSession`/`LivePath` (mirrored via a visibility change, not
// reimplemented -- plan invariant 2). Layout and fullscreen are later items
// (M4/M5).

const mockCameraConfig = (page, cameras) => {
  return Promise.all([
    page.route("**/api/config", (route) => route.fulfill({ json: { cameras } })),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] })),
  ]);
};

// Builds N enabled cameras named "camera-0".."camera-{n-1}", in dashboard
// order -- enough to exercise M3's per-tile staggering without hand-writing
// a fixture per test.
const nCameras = (count) => {
  const cameras = {};
  for (let index = 0; index < count; index += 1) {
    cameras[`camera-${index}`] = {
      enabled: true,
      friendly_name: `Camera ${index}`,
      ui: { order: index },
      detect: { width: 1920, height: 1080 },
    };
  }
  return cameras;
};

// A short, fixed cycle of aspect ratios (16:9, 4:3, portrait 9:16) so a
// generated fixture actually exercises item M4's aspect-ratio-aware
// packing (`monitor_layout::pack_tiles`) rather than every tile sharing one
// shape, matching the resolution fields M1's contract carries.
const MIXED_ASPECT_RATIOS = [
  { width: 1920, height: 1080 },
  { width: 1280, height: 960 },
  { width: 1080, height: 1920 },
];

const nMixedAspectCameras = (count) => {
  const cameras = {};
  for (let index = 0; index < count; index += 1) {
    const { width, height } = MIXED_ASPECT_RATIOS[index % MIXED_ASPECT_RATIOS.length];
    cameras[`camera-${index}`] = {
      enabled: true,
      friendly_name: `Camera ${index}`,
      ui: { order: index },
      detect: { width, height },
    };
  }
  return cameras;
};

// Installed before navigation so it is in place before any tile's reactive
// `data-live-path` attribute is ever written. Records, once per tile, the
// moment (`performance.now()`) its `.monitor-tile-player` first reaches the
// "hls" state -- the exact instant `expanded_view.rs`'s own
// `ExpandedSession::start` runs on that tile (with
// `window.__corvetteMoqRelayUrl` unset, `mount_moq` falls straight through to
// `mount_hls_fallback` synchronously, before any script load or network I/O),
// so this is a direct, jitter-free measurement of M3's own per-tile stagger
// -- not a proxy inferred from network or script-load timing.
const installLivePathLog = (page) => {
  return page.addInitScript(() => {
    window.__monitorHlsLog = [];
    const observeHlsTransitions = () => {
      const observer = new MutationObserver((mutations) => {
        for (const mutation of mutations) {
          if (mutation.type !== "attributes" || mutation.attributeName !== "data-live-path") {
            continue;
          }
          const tilePlayer = mutation.target;
          if (tilePlayer.getAttribute("data-live-path") !== "hls" || tilePlayer.dataset.hlsLogged) {
            continue;
          }
          tilePlayer.dataset.hlsLogged = "true";
          const camera = tilePlayer.closest(".monitor-tile")?.getAttribute("data-camera");
          window.__monitorHlsLog.push({ camera, at: performance.now() });
        }
      });
      observer.observe(document.body, { attributes: true, attributeFilter: ["data-live-path"], subtree: true });
    };
    if (document.body) {
      observeHlsTransitions();
    } else {
      document.addEventListener("DOMContentLoaded", observeHlsTransitions);
    }
  });
};

test("visiting /monitor renders one placeholder tile per enabled camera", async ({ page }) => {
  await mockCameraConfig(page, {
    "front-door": {
      enabled: true,
      friendly_name: "Front door",
      ui: { order: 0 },
      detect: { width: 1920, height: 1080 },
    },
    "back-yard": {
      enabled: true,
      friendly_name: "Back yard",
      ui: { order: 1 },
      detect: { width: 1280, height: 720 },
    },
    disabled: {
      enabled: false,
      friendly_name: "Disabled camera",
      ui: { order: 2 },
      detect: { width: 1280, height: 720 },
    },
  });

  await page.goto("/monitor");

  const tiles = page.locator(".monitor-tile");
  await expect(tiles).toHaveCount(2);
  await expect(tiles.nth(0)).toHaveText("Front door");
  await expect(tiles.nth(1)).toHaveText("Back yard");
});

test("visiting /monitor with no cameras configured shows the empty state, not a blank page", async ({ page }) => {
  await mockCameraConfig(page, {});

  await page.goto("/monitor");

  await expect(page.getByRole("status")).toContainText("No cameras configured");
  await expect(page.locator(".monitor-tile")).toHaveCount(0);
});

test("visiting /monitor when the camera fetch errors shows the error state, not a blank page", async ({ page }) => {
  await page.route("**/api/config", (route) => route.fulfill({ status: 500, body: "boom" }));

  await page.goto("/monitor");

  await expect(page.getByRole("status")).toContainText("Cameras unavailable");
  await expect(page.locator(".monitor-tile")).toHaveCount(0);
});

// Issue #20 item M3's own Verify step. `window.__corvetteMoqRelayUrl` is left
// unset throughout this file, forcing every tile's `mount_moq` straight into
// the HLS fallback per `expanded_view.rs`'s own documented behavior --
// matching `expanded_view.spec.cjs`'s identical precedent for exercising this
// path deterministically, without a real relay.
const CAMERA_COUNT = 3;

test("every tile independently reaches the HLS fallback with no cross-tile interference", async ({ page }) => {
  await installLivePathLog(page);
  await mockCameraConfig(page, nCameras(CAMERA_COUNT));

  await page.goto("/monitor");

  await page.waitForFunction(
    (expectedCount) => window.__monitorHlsLog.length >= expectedCount,
    CAMERA_COUNT,
  );

  const log = await page.evaluate(() => window.__monitorHlsLog);
  const loggedCameras = log.map((entry) => entry.camera).sort();
  expect(loggedCameras, "every tile reached the HLS fallback exactly once, none skipped or duplicated").toEqual(
    ["camera-0", "camera-1", "camera-2"],
  );

  // Each tile's own `.monitor-tile-player` took the HLS path independently --
  // asserted directly against the DOM (not just the log above) so a bug that
  // let one tile's session overwrite or clear another's state would still be
  // caught. This test harness has no real HLS origin behind
  // `/live/hls/<camera>/playlist.m3u8`, so `create_hls_video`'s own fatal-
  // error handling can carry a tile on to `LivePath::None` shortly after
  // (`try_native_hls_or_give_up`'s native-HLS probe also fails in
  // Chromium) -- both "hls" and "none" are this scenario's legitimate rest
  // states, per this item's own Verify step; the mutation log above is what
  // actually confirms every tile reached the HLS fallback itself.
  const players = page.locator(".monitor-tile-player");
  await expect(players).toHaveCount(CAMERA_COUNT);
  for (let index = 0; index < CAMERA_COUNT; index += 1) {
    await expect(players.nth(index)).toHaveAttribute("data-live-path", /^(hls|none)$/);
  }
});

test("tiles' initial dial is staggered rather than simultaneous", async ({ page }) => {
  await installLivePathLog(page);
  await mockCameraConfig(page, nCameras(CAMERA_COUNT));

  await page.goto("/monitor");

  await page.waitForFunction(
    (expectedCount) => window.__monitorHlsLog.length >= expectedCount,
    CAMERA_COUNT,
  );

  const log = await page.evaluate(() => window.__monitorHlsLog);
  const arrivalTimesMs = log.map((entry) => entry.at).sort((left, right) => left - right);
  expect(arrivalTimesMs, "one HLS-fallback arrival per tile").toHaveLength(CAMERA_COUNT);

  // `monitor.rs`'s own `STAGGER_MS` is 200; this asserts each consecutive
  // tile's dial trails the previous one by at least half that -- comfortably
  // above ordinary scheduler jitter, but low enough that it fails cleanly
  // (rather than flaking) when the stagger is disabled (`STAGGER_MS = 0`,
  // this item's own required mutation check).
  const MINIMUM_EXPECTED_GAP_MS = 100;
  for (let index = 1; index < arrivalTimesMs.length; index += 1) {
    const gapMs = arrivalTimesMs[index] - arrivalTimesMs[index - 1];
    expect(gapMs, `tile ${index}'s dial should trail tile ${index - 1}'s by roughly STAGGER_MS, not fire simultaneously`).toBeGreaterThanOrEqual(
      MINIMUM_EXPECTED_GAP_MS,
    );
  }
});

// Issue #20 item M4's own Verify step. `monitor_layout::pack_tiles`
// (`crates/corvette-ui/src/monitor_layout.rs`) is unit-tested directly
// against its own bounds/overlap/proportional-width oracle; these exercise
// the real `/monitor` route end to end, confirming `MonitorGrid` actually
// applies the computed layout to the DOM and that a mixed-aspect-ratio
// camera set fills the real browser viewport with no scrollbar, for every
// camera count this item's own Verify step names (2, 4, 5).
for (const cameraCount of [2, 4, 5]) {
  test(`/monitor with ${cameraCount} mixed-aspect cameras fills the viewport with no scrollbar`, async ({ page }) => {
    await mockCameraConfig(page, nMixedAspectCameras(cameraCount));

    await page.goto("/monitor");

    const tiles = page.locator(".monitor-tile");
    await expect(tiles).toHaveCount(cameraCount);

    // `MonitorGrid`'s layout effect sets every tile's placement the moment
    // its container mounts, independent of any tile's own MoQ/HLS session
    // (D-6) -- wait for that placement rather than for any player state.
    await page.waitForFunction(
      (expectedCount) =>
        document.querySelectorAll(".monitor-tile").length === expectedCount &&
        [...document.querySelectorAll(".monitor-tile")].every(
          (tile) => tile.getBoundingClientRect().width > 0 && tile.getBoundingClientRect().height > 0,
        ),
      cameraCount,
    );

    const [scrollSize, viewportSize, tileBoxes] = await Promise.all([
      page.evaluate(() => ({
        width: document.documentElement.scrollWidth,
        height: document.documentElement.scrollHeight,
      })),
      page.evaluate(() => ({ width: window.innerWidth, height: window.innerHeight })),
      tiles.evaluateAll((elements) =>
        elements.map((element) => {
          const rect = element.getBoundingClientRect();
          return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
        }),
      ),
    ]);

    // No scrollbar: the document's own scrollable extent never exceeds the
    // viewport that's actually visible, regardless of camera count.
    expect(scrollSize.width, "no horizontal scrollbar").toBeLessThanOrEqual(viewportSize.width);
    expect(scrollSize.height, "no vertical scrollbar").toBeLessThanOrEqual(viewportSize.height);

    for (const [index, box] of tileBoxes.entries()) {
      expect(box.width, `tile ${index} has a non-zero width`).toBeGreaterThan(0);
      expect(box.height, `tile ${index} has a non-zero height`).toBeGreaterThan(0);
      expect(box.x, `tile ${index}'s left edge is within the viewport`).toBeGreaterThanOrEqual(0);
      expect(box.y, `tile ${index}'s top edge is within the viewport`).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width, `tile ${index}'s right edge is within the viewport`).toBeLessThanOrEqual(
        viewportSize.width + 1,
      );
      expect(box.y + box.height, `tile ${index}'s bottom edge is within the viewport`).toBeLessThanOrEqual(
        viewportSize.height + 1,
      );
    }
  });
}
