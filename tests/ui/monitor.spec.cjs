const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Issue #20 item M2's own Verify step: `/monitor` is a new, independently
// bookmarkable client route (`crates/corvette-ui/src/monitor.rs`) that fetches
// the camera list on its own, mirroring every other route's convention (D-5,
// `.agents/issue-20/DESIGN-monitor-mode.md`) rather than sharing Dashboard's
// resource. This only exercises the placeholder-tile scaffold this item adds;
// live video, layout, and fullscreen are later items (M3/M4/M5).

const mockCameraConfig = (page, cameras) => {
  return Promise.all([
    page.route("**/api/config", (route) => route.fulfill({ json: { cameras } })),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] })),
  ]);
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
