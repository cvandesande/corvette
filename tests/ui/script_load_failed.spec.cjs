const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// The live screen must land on the explicit no-path state when its HLS
// player script fails to load and the browser has no native HLS support.

const JPEG = Buffer.from(
  "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q==",
  "base64",
);

const FRONT_CAMERA = {
  name: "front",
  display_name: "Front",
  order: 0,
  detect_width: 1920,
  detect_height: 1080,
};

test("the hls player script failing to load ends on the no-path state", async ({ page }) => {
  await Promise.all([
    page.route("**/corvette/api/v1/cameras", (route) =>
      route.fulfill({ json: { cameras: [FRONT_CAMERA] } }),
    ),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] })),
    page.route("**/corvette/api/v1/cameras/*/still.jpg*", (route) =>
      route.fulfill({ contentType: "image/jpeg", body: JPEG }),
    ),
    page.route("**/live/hls/**", () => {}),
  ]);
  await page.addInitScript(() => {
    HTMLMediaElement.prototype.canPlayType = () => "";
  });
  await page.route("**/vendor/hls.min.js", (route) => route.abort());
  await page.goto("/");
  await page.getByRole("link", { name: /open front live view/i }).click();

  await expect(page.locator(".live-path-indicator")).toHaveAttribute("data-live-path", "none", {
    timeout: 3_000,
  });
  await expect(page.locator(".live-path-indicator")).toHaveText("Live view unavailable");
});
