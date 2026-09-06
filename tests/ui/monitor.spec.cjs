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

// Issue #21 item T3's own Verify step: D-1's decision ("Direct Navigation
// Mode only") named the in-product communication step pointing users at
// tv-bro's long-press menu and its Direct Navigation Mode toggle as its own
// remaining cost. A later issue #21 follow-up shrank the one-time gesture
// prompt (`EnterFullscreenButton`, `crates/corvette-ui/src/monitor.rs`) from
// a full-viewport modal to a small button, carrying this hint as a `title`
// for a mouse-hovering viewer. A `title` alone is Chromium-hover-only, never
// shown on keyboard/programmatic focus, so a sighted TV-remote/D-pad
// viewer -- exactly who this hint is for -- could never see it that way; a
// follow-up review caught this, so the hint also renders as a real, visible
// sibling element gated on the button's own `:focus-visible` state
// (`styles.css`'s `.monitor-enter-fullscreen:focus-visible +
// .monitor-enter-fullscreen-hint`), not just present-but-hidden in the DOM.
test("the fullscreen-entry button names tv-bro's Direct Navigation Mode, reachable by both mouse and keyboard/D-pad focus", async ({ page }) => {
  await mockCameraConfig(page, nCameras(1));

  await page.goto("/monitor");

  const button = page.getByRole("button", { name: "Enter fullscreen" });
  const hint = page.locator(".monitor-enter-fullscreen-hint");

  // Mouse-hover path: still carried as `title`.
  await expect(button).toHaveAttribute("title", /Direct Navigation Mode/);
  await expect(hint).toContainText("Direct Navigation Mode");

  // Keyboard/D-pad path: `EnterFullscreenButton` auto-focuses on mount, so
  // the hint is already visible immediately after load, with nothing else
  // done -- the exact moment a TV remote's D-pad would first reach it.
  await expect(hint, "visible while the button has focus, right after auto-focus on mount").toBeVisible();

  // Moving focus elsewhere hides it again -- it does not permanently occupy
  // screen space once the viewer has moved on.
  await page.locator(".monitor-tile").first().focus();
  await expect(hint, "hidden once focus leaves the button").not.toBeVisible();

  // Tabbing back re-focuses the button (nothing else on this bare wall is
  // reachable before it in tab order) and the hint reappears.
  await button.focus();
  await expect(hint, "visible again once the button regains focus").toBeVisible();
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

// Issue #20 item M5's own Verify step: click/select-to-fullscreen per tile,
// gated on the one-time gesture-to-arm prompt (D-1/D-4,
// `.agents/issue-20/DESIGN-monitor-mode.md`).
//
// Real Fullscreen API activation is not exercised here: headless Chromium
// has no real screen, and this project's own harness runs
// `browserName: "chromium", headless: true` (`playwright.config.cjs`) with
// no guarantee a synthetic Playwright click's transient activation carries
// through to a real `requestFullscreen()` grant in that mode. Mocked at the
// DOM level instead -- the same approach `expanded_view.spec.cjs`'s own
// `patchWebTransportCertificateHashes` uses for a real browser API this
// harness cannot exercise for real -- so these tests assert against
// `monitor.rs`'s actual call into `Element.requestFullscreen()`, not a
// screenshot or a real rendered fullscreen state. The mock also reproduces
// the one piece of behavior `monitor.rs` deliberately leaves to the browser
// itself: Escape exiting an active Fullscreen-API session. `monitor.rs`
// never calls `exitFullscreen()` (that is a real, spec-guaranteed browser
// default `monitor.rs` relies on rather than reimplements); this mock has
// to model that default for the "Escape returns to the wall" test below to
// mean anything in a harness with no real Fullscreen API session to exit.
const installFullscreenMock = (page) => {
  return page.addInitScript(() => {
    window.__fullscreenRequests = [];
    let fullscreenElement = null;
    Object.defineProperty(document, "fullscreenElement", {
      configurable: true,
      get: () => fullscreenElement,
    });
    Element.prototype.requestFullscreen = function mockRequestFullscreen() {
      window.__fullscreenRequests.push(this);
      fullscreenElement = this;
      document.dispatchEvent(new Event("fullscreenchange"));
      return Promise.resolve();
    };
    document.exitFullscreen = function mockExitFullscreen() {
      fullscreenElement = null;
      document.dispatchEvent(new Event("fullscreenchange"));
      return Promise.resolve();
    };
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && fullscreenElement) {
        document.exitFullscreen();
      }
    });
  });
};

const dismissFullscreenPrompt = (page) => page.getByRole("button", { name: "Enter fullscreen" }).click();

// Marks a DOM node with a property no re-render could preserve -- a fresh
// node from a re-mount would not carry this forward, unlike an in-place
// attribute/state change on the same node. Used to confirm a tile's own
// live-session player element is untouched by entering/exiting fullscreen.
const markPlayerIdentity = (page) =>
  page.evaluate(() => {
    document.querySelector(".monitor-tile-player").__identityToken = "original-player-node";
  });

const playerIdentityPreserved = (page) =>
  page.evaluate(
    () => document.querySelector(".monitor-tile-player")?.__identityToken === "original-player-node",
  );

test.describe("item M5: click/select-to-fullscreen per tile", () => {
  test("a tile click before the one-time prompt is dismissed does not attempt requestFullscreen", async ({ page }) => {
    await installFullscreenMock(page);
    await mockCameraConfig(page, nCameras(2));

    await page.goto("/monitor");
    // `dispatchEvent` fires the `click` directly on the tile's own element
    // (bypassing Playwright's coordinate-based hit test) to confirm the
    // underlying `armed` gate in `monitor.rs` itself -- not incidental
    // stacking with the fullscreen-entry button -- is what prevents this.
    await page.locator(".monitor-tile").first().dispatchEvent("click");

    expect(await page.evaluate(() => window.__fullscreenRequests.length)).toBe(0);
  });

  test("dismissing the prompt (Enter) then clicking a tile fullscreens that tile's own element, and only that one", async ({ page }) => {
    await installFullscreenMock(page);
    await mockCameraConfig(page, nCameras(3));

    await page.goto("/monitor");
    await page.getByRole("button", { name: "Enter fullscreen" }).focus();
    await page.keyboard.press("Enter");

    const tiles = page.locator(".monitor-tile");
    await tiles.nth(1).click();

    expect(await page.evaluate(() => window.__fullscreenRequests.length), "requestFullscreen invoked exactly once").toBe(
      1,
    );

    const fullscreenedCorrectTile = await page.evaluate(
      (tileIndex) => window.__fullscreenRequests[0] === document.querySelectorAll(".monitor-tile")[tileIndex],
      1,
    );
    expect(
      fullscreenedCorrectTile,
      "requestFullscreen was called on tile 1's own element, not a different tile or the wall",
    ).toBe(true);
  });

  test("entering fullscreen does not remount or reconnect the tile's live session", async ({ page }) => {
    await installFullscreenMock(page);
    await mockCameraConfig(page, nCameras(1));

    await page.goto("/monitor");
    await dismissFullscreenPrompt(page);
    await markPlayerIdentity(page);

    await page.locator(".monitor-tile").first().click();

    expect(await page.evaluate(() => window.__fullscreenRequests.length)).toBe(1);
    expect(
      await playerIdentityPreserved(page),
      "the tile's live-session player element was not replaced when it entered fullscreen",
    ).toBe(true);
  });

  test("Escape exits fullscreen back to the wall, with the tile's own session still running", async ({ page }) => {
    await installFullscreenMock(page);
    await mockCameraConfig(page, nCameras(1));

    await page.goto("/monitor");
    await dismissFullscreenPrompt(page);
    await markPlayerIdentity(page);

    await page.locator(".monitor-tile").first().click();
    expect(await page.evaluate(() => document.fullscreenElement !== null)).toBe(true);

    await page.keyboard.press("Escape");

    expect(await page.evaluate(() => document.fullscreenElement)).toBeNull();
    expect(
      await playerIdentityPreserved(page),
      "the tile's live-session player element survived the fullscreen exit unchanged",
    ).toBe(true);
  });

  test("the one-time fullscreen-entry button does not reappear once dismissed", async ({ page }) => {
    await installFullscreenMock(page);
    await mockCameraConfig(page, nCameras(1));

    await page.goto("/monitor");
    await dismissFullscreenPrompt(page);
    await page.locator(".monitor-tile").first().click();
    await page.keyboard.press("Escape");

    await expect(page.getByRole("button", { name: "Enter fullscreen" })).toHaveCount(0);
  });
});

// A defensive on-page fullscreen-exit control (D-4,
// `.agents/issue-20/DESIGN-monitor-mode.md`), independent of whatever a
// given platform's remote/Back-button pathway does or does not forward as
// an Escape keypress. `monitor.rs`'s own button is always mounted and gated
// purely by `styles.css`'s `.monitor-tile:fullscreen` selector -- there is
// no Rust-side `fullscreenchange` tracking to mock, so unlike the M5 tests
// above, `installFullscreenMock`'s fully synthetic `requestFullscreen`
// cannot exercise it: that mock never touches the real Fullscreen API, so
// the real `:fullscreen` pseudo-class this button's CSS depends on would
// never actually match under it. These tests enter fullscreen for real
// instead, via a real click on the tile (the same shipped interaction the
// M5 tests exercise through `monitor.rs`'s own `enter_fullscreen` closure,
// just left un-mocked here) -- confirmed directly against this project's
// own `playwright.config.cjs` (`headless: true`, chromium) by running that
// exact click-then-`:fullscreen` sequence 27 times (9 fresh runs, then a
// `--repeat-each=3` pass) with no failures, so a real Playwright click does
// carry enough transient user-activation here. The earlier attempt at this
// (calling `Element.requestFullscreen()` directly from `page.evaluate`)
// checked `:fullscreen` immediately after the call instead of awaiting the
// async transition, which raced it; these tests wait for it properly with
// `page.waitForFunction`, the same pattern the exit-button test below
// already uses for the exit transition.
test.describe("on-page fullscreen-exit control", () => {
  test("the exit control is a real DOM node but not visible or interactable before the tile is fullscreened", async ({ page }) => {
    await mockCameraConfig(page, nCameras(1));

    await page.goto("/monitor");
    await dismissFullscreenPrompt(page);

    const exitButton = page.locator(".monitor-tile-exit-fullscreen");
    await expect(exitButton, "the button is mounted unconditionally, not added only once fullscreen").toHaveCount(1);
    await expect(exitButton, "hidden (and thus non-interactive) while the tile is not fullscreen").toBeHidden();
  });

  test("the exit control becomes visible once the tile is actually fullscreen, and clicking it exits fullscreen without re-entering it", async ({ page }) => {
    await mockCameraConfig(page, nCameras(1));

    await page.goto("/monitor");
    await dismissFullscreenPrompt(page);
    await markPlayerIdentity(page);

    const wallPlacement = await page.locator(".monitor-tile").first().boundingBox();

    // Spies on the real `requestFullscreen`, still calling through to it, so
    // a click on the exit control re-triggering the tile's own
    // `enter_fullscreen` (a real defect: `on:click`'s `stop_propagation`
    // would not be doing its job) shows up as a second call, not just a
    // silent no-op.
    await page.evaluate(() => {
      const original = Element.prototype.requestFullscreen;
      window.__realRequestFullscreenCalls = 0;
      Element.prototype.requestFullscreen = function spy(...args) {
        window.__realRequestFullscreenCalls += 1;
        return original.apply(this, args);
      };
    });
    // The actual shipped interaction: a real click on the tile, routed
    // through `monitor.rs`'s own `enter_fullscreen` closure, not a direct
    // `requestFullscreen()` call from the test.
    await page.locator(".monitor-tile").first().click();
    await page.waitForFunction(() => document.querySelector(".monitor-tile")?.matches(":fullscreen"));
    expect(await page.evaluate(() => document.querySelector(".monitor-tile").matches(":fullscreen"))).toBe(true);

    const exitButton = page.getByRole("button", { name: "Exit fullscreen" });
    await expect(exitButton, "visible and reachable the instant the tile is actually fullscreen").toBeVisible();

    await exitButton.click();
    // `Document.exitFullscreen()` settles asynchronously, same as entry
    // above -- wait for the real `fullscreenchange` transition to actually
    // complete rather than asserting immediately after the click.
    await page.waitForFunction(() => document.fullscreenElement === null);

    expect(await page.evaluate(() => document.fullscreenElement), "the exit control's click handler called exitFullscreen").toBeNull();
    expect(
      await page.evaluate(() => window.__realRequestFullscreenCalls),
      "the click did not also bubble into the tile's own on:click and re-enter fullscreen",
    ).toBe(1);
    expect(
      await playerIdentityPreserved(page),
      "the tile's live-session player element survived the fullscreen exit unchanged",
    ).toBe(true);

    const restoredPlacement = await page.locator(".monitor-tile").first().boundingBox();
    expect(restoredPlacement, "the tile returned to its own packed wall placement, not still full-viewport").toEqual(wallPlacement);
  });

  test("pressing Enter on the focused exit control also exits fullscreen, without re-entering it", async ({ page }) => {
    await mockCameraConfig(page, nCameras(1));

    await page.goto("/monitor");
    await dismissFullscreenPrompt(page);

    await page.evaluate(() => {
      const original = Element.prototype.requestFullscreen;
      window.__realRequestFullscreenCalls = 0;
      Element.prototype.requestFullscreen = function spy(...args) {
        window.__realRequestFullscreenCalls += 1;
        return original.apply(this, args);
      };
    });
    // The actual shipped interaction: a real click on the tile, routed
    // through `monitor.rs`'s own `enter_fullscreen` closure.
    await page.locator(".monitor-tile").first().click();
    await page.waitForFunction(() => document.querySelector(".monitor-tile")?.matches(":fullscreen"));

    const exitButton = page.getByRole("button", { name: "Exit fullscreen" });
    await exitButton.focus();
    await page.keyboard.press("Enter");
    await page.waitForFunction(() => document.fullscreenElement === null);

    expect(await page.evaluate(() => document.fullscreenElement), "Enter on the exit control called exitFullscreen").toBeNull();
    expect(
      await page.evaluate(() => window.__realRequestFullscreenCalls),
      "Enter's keydown did not also bubble into the tile's own on:keydown and re-enter fullscreen",
    ).toBe(1);
  });
});
