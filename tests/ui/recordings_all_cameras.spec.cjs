const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Issue #22 item R2's own Verify step: "All cameras" is now `/recordings`'
// default camera selection (D-5), and choosing a preset in that mode packs
// one placeholder tile per camera into `.all-cameras-grid` via
// `monitor_layout::pack_tiles`, re-packing on resize (D-2(b)).
//
// Item R3 replaces those placeholders with real per-tile playback: one
// shared scrub position drives N independent `TimelinePlayer` instances
// (D-1(c)), each with its own "not yet settled" indicator and no cross-tile
// barrier (D-1's rejected option (b)). The no-footage placeholder is a later
// item (R4).

const mockCameraConfig = (page, cameras) => {
  return Promise.all([
    page.route("**/api/config", (route) => route.fulfill({ json: { cameras } })),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/recordings/summary?*", (route) => route.fulfill({ json: {} })),
    // "All" mode's own combined preview fetch (`camera == "all"`, F-13):
    // every test below that loads a range triggers this regardless of
    // whether it cares about previews, since R3's `AllCamerasPlayback` now
    // gates its whole grid on this resolving alongside `cameras`.
    page.route("**/api/preview/all/start/*/end/*", (route) => route.fulfill({ json: [] })),
  ]);
};

// Builds N enabled cameras named "camera-0".."camera-{n-1}", matching
// `monitor.spec.cjs`'s own helper shape.
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

// A short, fixed cycle of aspect ratios (16:9, 4:3, portrait 9:16), matching
// `monitor.spec.cjs`'s own helper shape, so a generated fixture actually
// exercises `pack_tiles`'s aspect-ratio-aware placement rather than every
// tile sharing one shape.
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

const rectsOverlap = (a, b) => {
  const epsilon = 0.5;
  return (
    a.x + epsilon < b.x + b.width &&
    b.x + epsilon < a.x + a.width &&
    a.y + epsilon < b.y + b.height &&
    b.y + epsilon < a.y + a.height
  );
};

// Mirrors `monitor_layout.rs`'s own bounds/no-overlap test oracle, asserted
// here against real rendered DOM boxes (`.all-cameras-tile`) rather than
// `pack_tiles` called directly.
const assertTilesPackWithinGrid = (tileBoxes, gridBox) => {
  const epsilon = 0.5;
  for (const [index, box] of tileBoxes.entries()) {
    expect(box.width, `tile ${index} has a non-zero width`).toBeGreaterThan(0);
    expect(box.height, `tile ${index} has a non-zero height`).toBeGreaterThan(0);
    expect(box.x, `tile ${index}'s left edge is within the grid`).toBeGreaterThanOrEqual(
      gridBox.x - epsilon,
    );
    expect(box.y, `tile ${index}'s top edge is within the grid`).toBeGreaterThanOrEqual(
      gridBox.y - epsilon,
    );
    expect(
      box.x + box.width,
      `tile ${index}'s right edge is within the grid`,
    ).toBeLessThanOrEqual(gridBox.x + gridBox.width + epsilon);
    expect(
      box.y + box.height,
      `tile ${index}'s bottom edge is within the grid`,
    ).toBeLessThanOrEqual(gridBox.y + gridBox.height + epsilon);
  }
  for (let i = 0; i < tileBoxes.length; i += 1) {
    for (let j = i + 1; j < tileBoxes.length; j += 1) {
      expect(
        rectsOverlap(tileBoxes[i], tileBoxes[j]),
        `tiles ${i} and ${j} should not overlap`,
      ).toBe(false);
    }
  }
};

const tileBoxes = (page) =>
  page.locator(".all-cameras-tile").evaluateAll((elements) =>
    elements.map((element) => {
      const rect = element.getBoundingClientRect();
      return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
    }),
  );

test("recordings defaults to All cameras with no explicit selection", async ({ page }) => {
  await mockCameraConfig(page, nCameras(2));

  await page.goto("/recordings");

  await expect(page.getByLabel("Camera")).toHaveValue("all");
  await expect(page.locator('option[value="all"]')).toHaveText("All cameras");
});

for (const cameraCount of [2, 4, 5]) {
  test(`choosing a preset in All mode packs ${cameraCount} placeholder tiles with no overlap`, async ({
    page,
  }) => {
    await mockCameraConfig(page, nMixedAspectCameras(cameraCount));

    await page.goto("/recordings");
    await page.getByRole("button", { name: "Last hour" }).click();

    const tiles = page.locator(".all-cameras-tile");
    await expect(tiles).toHaveCount(cameraCount);
    for (let index = 0; index < cameraCount; index += 1) {
      // `toContainText`, not an exact match: since R3, each tile also
      // renders its own `TimelinePlayer` (a play/pause button and a
      // `<video>` fallback-text node) alongside the camera name heading this
      // assertion cares about.
      await expect(tiles.nth(index)).toContainText(`Camera ${index}`);
    }

    await page.waitForFunction(
      (expectedCount) =>
        [...document.querySelectorAll(".all-cameras-tile")].every(
          (tile) => tile.getBoundingClientRect().width > 0,
        ) && document.querySelectorAll(".all-cameras-tile").length === expectedCount,
      cameraCount,
    );

    const gridBox = await page.locator(".all-cameras-grid").boundingBox();
    assertTilesPackWithinGrid(await tileBoxes(page), gridBox);
  });
}

test("resizing the viewport after tiles render re-packs them to the new grid box", async ({
  page,
}) => {
  const cameraCount = 3;
  await page.setViewportSize({ width: 1280, height: 900 });
  await mockCameraConfig(page, nMixedAspectCameras(cameraCount));

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();
  await expect(page.locator(".all-cameras-tile")).toHaveCount(cameraCount);

  const boxesBeforeResize = await tileBoxes(page);
  const gridBoxBeforeResize = await page.locator(".all-cameras-grid").boundingBox();
  assertTilesPackWithinGrid(boxesBeforeResize, gridBoxBeforeResize);

  // `.all-cameras-grid` is sized in `vh` (`styles.css`), so a taller viewport
  // gives it a real, different pixel box -- this is `pack_tiles`'s resize
  // path (D-2(b)), never exercised anywhere else in this codebase.
  await page.setViewportSize({ width: 1280, height: 1500 });

  await expect
    .poll(async () => (await page.locator(".all-cameras-grid").boundingBox()).height)
    .toBeGreaterThan(gridBoxBeforeResize.height);
  await expect
    .poll(async () => (await tileBoxes(page))[0].height)
    .not.toBeCloseTo(boxesBeforeResize[0].height, 0);

  const gridBoxAfterResize = await page.locator(".all-cameras-grid").boundingBox();
  assertTilesPackWithinGrid(await tileBoxes(page), gridBoxAfterResize);
});

test("All mode fetches each real camera's own recordings and never a synthetic \"all\" camera", async ({
  page,
}) => {
  const recordingsRequests = [];
  page.on("request", (request) => {
    const { pathname } = new URL(request.url());
    if (/^\/api\/[^/]+\/recordings$/.test(pathname)) {
      recordingsRequests.push(pathname);
    }
  });

  await mockCameraConfig(page, nCameras(3));
  await page.route("**/api/camera-*/recordings?*", (route) => route.fulfill({ json: [] }));
  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();
  await expect(page.locator(".all-cameras-tile")).toHaveCount(3);

  // Since R3, each tile owns its own `fetch_recording_segments` call against
  // its own real camera name (invariant 5) -- this is the item that first
  // makes this request happen at all; R2 only ever asserted its absence.
  await expect.poll(() => recordingsRequests.length).toBeGreaterThanOrEqual(3);
  expect(recordingsRequests).not.toContain("/api/all/recordings");
  expect(new Set(recordingsRequests)).toEqual(
    new Set(["/api/camera-0/recordings", "/api/camera-1/recordings", "/api/camera-2/recordings"]),
  );
});

test("moving the shared scrub updates each tile independently of the others' clip boundaries", async ({
  page,
}) => {
  const now = Date.now() / 1000;
  await mockCameraConfig(page, nCameras(2));
  // camera-0 has a real gap between two retained clips at `now - 1800`..
  // `now - 1700`; camera-1's own single clip spans the whole loaded range
  // with no boundary to cross.
  await page.route("**/api/camera-0/recordings?*", (route) =>
    route.fulfill({
      json: [
        { start_time: now - 3500, end_time: now - 1800, motion: null },
        { start_time: now - 1700, end_time: now - 100, motion: null },
      ],
    }),
  );
  await page.route("**/api/camera-1/recordings?*", (route) =>
    route.fulfill({ json: [{ start_time: now - 3500, end_time: now - 100, motion: null }] }),
  );

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();
  await expect(page.locator(".all-cameras-tile video")).toHaveCount(2);

  const tileVideo = (camera) =>
    page.locator(`.all-cameras-tile[data-camera="${camera}"] .timeline-player`);
  const scrubTo = (time) =>
    page.getByRole("slider", { name: "All-cameras playhead" }).evaluate((input, value) => {
      input.value = value;
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }, String(time));

  await scrubTo(now - 3000);
  await expect(tileVideo("camera-0")).toHaveAttribute(
    "src",
    `/api/camera-0/start/${now - 3500}/end/${now - 1800}/clip.mp4`,
  );
  await expect(tileVideo("camera-1")).toHaveAttribute(
    "src",
    `/api/camera-1/start/${now - 3500}/end/${now - 100}/clip.mp4`,
  );

  await scrubTo(now - 1600);
  await expect(tileVideo("camera-0")).toHaveAttribute(
    "src",
    `/api/camera-0/start/${now - 1700}/end/${now - 100}/clip.mp4`,
  );
  // camera-1's own single continuous clip never had a boundary to cross, so
  // its source stays exactly what it was -- proving the shared scrub drives
  // each tile independently rather than through one shared player.
  await expect(tileVideo("camera-1")).toHaveAttribute(
    "src",
    `/api/camera-1/start/${now - 3500}/end/${now - 100}/clip.mp4`,
  );
});

test("the \"not yet settled\" indicator is independent per tile", async ({ page }) => {
  await mockCameraConfig(page, nCameras(2));
  // A segment clamped to the whole loaded range on both cameras, so each
  // tile's initial source starts exactly at the shared scrub position (the
  // loaded range's own start) -- its `requested_offset` is exactly `0`,
  // which is also a real, un-stubbed `<video>`'s own default `currentTime`
  // before any media loads.
  await page.route("**/api/camera-0/recordings?*", (route) =>
    route.fulfill({ json: [{ start_time: 0, end_time: 9_999_999_999, motion: null }] }),
  );
  await page.route("**/api/camera-1/recordings?*", (route) =>
    route.fulfill({ json: [{ start_time: 0, end_time: 9_999_999_999, motion: null }] }),
  );

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();
  await expect(page.locator(".all-cameras-tile video")).toHaveCount(2);

  const unsettledIndicator = (camera) =>
    page.locator(`.all-cameras-tile[data-camera="${camera}"] .all-cameras-tile-unsettled`);

  // Neither tile's own <video> ever actually loads real media under this
  // mock, so both tiles start -- and, absent any confirmation, stay -- "not
  // yet settled" (D-1(c)'s own honest-signal requirement).
  await expect(unsettledIndicator("camera-0")).toBeVisible();
  await expect(unsettledIndicator("camera-1")).toBeVisible();

  // Confirms only camera-0's own landing, exactly as a real `seeked`/
  // `loadedmetadata` event would once this tile's source finishes loading.
  await page
    .locator('.all-cameras-tile[data-camera="camera-0"] video')
    .dispatchEvent("loadedmetadata");

  await expect(unsettledIndicator("camera-0")).toHaveCount(0);
  // camera-1's own tile never received a confirming event and is still
  // reporting unsettled -- no cross-tile barrier (D-1's rejected option (b)).
  await expect(unsettledIndicator("camera-1")).toBeVisible();
});
