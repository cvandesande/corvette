const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Issue #22 item R2's own Verify step: "All cameras" is now `/recordings`'
// default camera selection (D-5), and choosing a preset in that mode packs
// one placeholder tile per camera into `.all-cameras-grid` via
// `monitor_layout::pack_tiles`, re-packing on resize (D-2(b)). Per-tile
// video, the shared scrub position, and the no-footage placeholder are later
// items (R3/R4) -- this file only exercises the grid shell R2 adds.

const mockCameraConfig = (page, cameras) => {
  return Promise.all([
    page.route("**/api/config", (route) => route.fulfill({ json: { cameras } })),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/recordings/summary?*", (route) => route.fulfill({ json: {} })),
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
      await expect(tiles.nth(index)).toHaveText(`Camera ${index}`);
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

test("All mode never requests a per-camera recordings endpoint", async ({ page }) => {
  const recordingsRequests = [];
  page.on("request", (request) => {
    const { pathname } = new URL(request.url());
    if (/^\/api\/[^/]+\/recordings$/.test(pathname)) {
      recordingsRequests.push(pathname);
    }
  });

  await mockCameraConfig(page, nCameras(3));
  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();
  await expect(page.locator(".all-cameras-tile")).toHaveCount(3);

  // R2 renders placeholder tiles only -- no per-tile fetch exists yet (R3),
  // so this is stricter than invariant 5 requires today: zero requests, not
  // merely none naming a nonexistent "all" camera. The `/api/all/recordings`
  // guard this item's own Do-3 step adds (`recordings.rs`) is what a
  // regression here would actually be catching.
  expect(recordingsRequests).toEqual([]);
});
