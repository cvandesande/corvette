const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// The expanded (click-to-enlarge) live view mounts its HLS player directly:
// the live-path indicator settles on "Live" and a `<video>` player appears,
// with no relay or real HLS backend involved. The playlist route is held
// unanswered so playback never races a fixture behind the assertions.

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

test("the expanded view mounts its HLS player directly and the indicator reads Live", async ({
  page,
}) => {
  test.setTimeout(30_000);
  await Promise.all([
    page.route("**/corvette/api/v1/cameras", (route) =>
      route.fulfill({ json: { cameras: [FRONT_CAMERA] } }),
    ),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] })),
    page.route("**/corvette/api/v1/cameras/*/still.jpg*", (route) =>
      route.fulfill({ contentType: "image/jpeg", body: JPEG }),
    ),
    // Never answered, so the path label stays on its value and no backend playback starts.
    page.route("**/live/hls/**", () => {}),
  ]);

  await page.goto("/");
  await page.getByRole("link", { name: /open front live view/i }).click();

  await expect(page.locator(".live-path-indicator")).toHaveAttribute("data-live-path", "hls", {
    timeout: 6_000,
  });
  await expect(page.locator(".live-path-indicator")).toHaveText("Live");
  await expect(page.locator(".expanded-player video")).toHaveCount(1);
});
