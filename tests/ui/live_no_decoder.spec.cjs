const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// The MoQ player decodes with WebCodecs VideoDecoder; a browser without it
// must skip the MoQ attempt at once and mount the HLS live mode instead.

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

// A relay address that refuses at once, so any MoQ attempt fails fast.
const DEAD_RELAY_URL = "https://127.0.0.1:9/anon";

const setup = async (page) => {
  const requests = [];
  page.on("request", (request) => requests.push(request.url()));
  await Promise.all([
    page.route("**/corvette/api/v1/cameras", (route) =>
      route.fulfill({ json: { cameras: [FRONT_CAMERA] } }),
    ),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/*/latest.jpg*", (route) =>
      route.fulfill({ contentType: "image/jpeg", body: JPEG }),
    ),
    // Never answered, so the live-path label stays on its value.
    page.route("**/live/hls/**", () => {}),
  ]);
  return requests;
};

test("a browser without a video decoder goes straight to the HLS live mode", async ({ page }) => {
  await page.addInitScript((url) => {
    window.__corvetteMoqRelayUrl = url;
    delete window.VideoDecoder;
  }, DEAD_RELAY_URL);
  const requests = await setup(page);
  await page.goto("/");

  expect(await page.evaluate(() => typeof VideoDecoder)).toBe("undefined");

  await page.getByRole("button", { name: /expand front to full view/i }).click();

  // Well under the 4 000 ms MoQ timeout, so only skipping MoQ passes.
  await expect(page.locator(".live-path-indicator")).toHaveAttribute("data-live-path", "hls", {
    timeout: 2_000,
  });
  await expect(page.locator(".expanded-player moq-watch")).toHaveCount(0);
  expect(requests.filter((url) => url.includes("moq-watch"))).toEqual([]);
});

test("a browser with a video decoder still tries the MoQ player first", async ({ page }) => {
  await page.addInitScript((url) => {
    window.__corvetteMoqRelayUrl = url;
  }, DEAD_RELAY_URL);
  const requests = await setup(page);
  await page.goto("/");

  expect(await page.evaluate(() => typeof VideoDecoder)).toBe("function");

  await page.getByRole("button", { name: /expand front to full view/i }).click();

  await expect
    .poll(() => requests.some((url) => url.includes("/vendor/moq-watch.bundle.js")), {
      timeout: 5_000,
    })
    .toBe(true);
  await expect
    .poll(() => page.locator(".expanded-player moq-watch").count(), { timeout: 5_000 })
    .toBe(1);
});
