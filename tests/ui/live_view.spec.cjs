const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// The dashboard tile shows a still picture refreshed in place, not an embedded
// player page: this keeps the standing "no iframe, no live video element in
// the grid" check, which held when the tile drove a `MediaSource` over a
// WebSocket and still holds now that it shows stills.

const JPEG = Buffer.from(
  "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q==",
  "base64",
);

test("the grid tile renders no iframe and no live video element", async ({ page }) => {
  await page.route("**/corvette/api/v1/cameras", (route) =>
    route.fulfill({
      json: {
        cameras: [
          {
            name: "front",
            display_name: "Front",
            order: 0,
            detect_width: 1920,
            detect_height: 1080,
          },
        ],
      },
    }),
  );
  await page.route("**/api/review?*", (route) => route.fulfill({ json: [] }));
  await page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] }));
  await page.route("**/api/*/latest.jpg*", (route) =>
    route.fulfill({ contentType: "image/jpeg", body: JPEG }),
  );

  await page.goto("/");

  await expect(page.locator(".camera-grid img.still-tile-image")).toHaveCount(1);
  await expect(page.locator(".camera-grid video")).toHaveCount(0);
  await expect(page.locator("iframe")).toHaveCount(0);
});
