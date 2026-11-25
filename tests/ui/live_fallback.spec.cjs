const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// The live screen's tap-bound start: a picture that never starts must offer a
// play button whose single tap rebuilds the player inside the tap, and a
// picture that does start must never show it. The playlist route is held
// unanswered so no picture ever starts on its own.

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

const setup = async (page) => {
  const requests = [];
  page.on("request", (request) => requests.push(request.url()));
  await Promise.all([
    page.route("**/corvette/api/v1/cameras", (route) =>
      route.fulfill({ json: { cameras: [FRONT_CAMERA] } }),
    ),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] })),
    page.route("**/corvette/api/v1/cameras/*/still.jpg*", (route) =>
      route.fulfill({ contentType: "image/jpeg", body: JPEG }),
    ),
    // Never answered, so the HLS picture never starts and the play prompt is what the screen must offer.
    page.route("**/live/hls/**", () => {}),
  ]);
  return requests;
};

const openExpandedView = async (page) => {
  await page.goto("/");
  await page.getByRole("link", { name: /open front live view/i }).click();
};

test("a picture that does not start brings up the play button after the bound", async ({
  page,
}) => {
  test.setTimeout(30_000);
  await setup(page);
  await openExpandedView(page);

  await expect(page.locator(".live-path-indicator")).toHaveAttribute("data-live-path", "hls", {
    timeout: 6_000,
  });
  await expect(page.locator(".live-play-button")).toBeHidden();
  await expect(page.locator(".live-play-button")).toBeVisible({ timeout: 8_000 });
});

test("one tap on the play button rebuilds the player inside the tap", async ({ page }) => {
  test.setTimeout(30_000);
  const requests = await setup(page);
  await page.addInitScript(() => {
    window.__playCalls = [];
    window.__inClick = false;
    document.addEventListener(
      "click",
      () => {
        window.__inClick = true;
        setTimeout(() => {
          window.__inClick = false;
        }, 0);
      },
      true,
    );
    const original = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function (...args) {
      window.__playCalls.push(window.__inClick);
      return original.apply(this, args);
    };
  });
  await openExpandedView(page);

  await expect(page.locator(".live-play-button")).toBeVisible({ timeout: 15_000 });
  const hlsRequestsBefore = requests.filter((url) => url.includes("/live/hls/")).length;
  await page.getByRole("button", { name: "Play live video" }).click();

  await expect(page.locator(".live-play-button")).toBeHidden({ timeout: 1_000 });
  const playCalls = await page.evaluate(() => window.__playCalls);
  expect(playCalls.at(-1)).toBe(true);
  expect(playCalls[0]).toBe(false);
  await expect
    .poll(() => requests.filter((url) => url.includes("/live/hls/")).length)
    .toBeGreaterThan(hlsRequestsBefore);
  await expect(page.locator(".expanded-player video")).toHaveCount(1);
});

test("a picture that starts keeps the play button away", async ({ page }) => {
  test.setTimeout(30_000);
  await setup(page);
  await page.addInitScript(() => {
    HTMLMediaElement.prototype.play = function () {
      this.dispatchEvent(new Event("playing"));
      return Promise.resolve();
    };
  });
  await openExpandedView(page);

  await expect(page.locator(".live-path-indicator")).toHaveAttribute("data-live-path", "hls", {
    timeout: 6_000,
  });
  // The absence of a prompt can only be shown by waiting past the bound.
  await page.waitForTimeout(6_500);
  await expect(page.locator(".live-play-button")).toBeHidden();
});

test("a picture that starts after the bound hides the play button again", async ({ page }) => {
  test.setTimeout(30_000);
  await setup(page);
  await page.addInitScript(() => {
    HTMLMediaElement.prototype.play = function () {
      // play() records a starter instead of starting: the test begins the picture itself.
      window.__startPicture = () => this.dispatchEvent(new Event("playing"));
      return Promise.resolve();
    };
  });
  await openExpandedView(page);

  await expect(page.locator(".live-play-button")).toBeVisible({ timeout: 15_000 });
  await page.evaluate(() => window.__startPicture());
  await expect(page.locator(".live-play-button")).toBeHidden({ timeout: 1_000 });
});
