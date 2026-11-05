const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// /camera/<name> shows only that camera's live screen at full size, loads no
// grid, and returns to the dashboard when the live screen closes.

// A real, decodable 1x1 JPEG, so a stray picture request would succeed visibly.
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

const BACK_CAMERA = {
  name: "back",
  display_name: "Back",
  order: 10,
  detect_width: 1280,
  detect_height: 720,
};

const LIVING_CAMERA = {
  name: "living",
  display_name: "Living",
  order: 20,
  detect_width: 1280,
  detect_height: 720,
  enabled: false,
};

// A relay address that refuses at once, so the live screen falls back at once
// or stays on connecting; no case here depends on which.
const DEAD_RELAY_URL = "https://127.0.0.1:9/anon";

const setup = async (page, cameras) => {
  const requests = [];
  page.on("request", (request) => requests.push(request.url()));
  await page.addInitScript((url) => {
    window.__corvetteMoqRelayUrl = url;
  }, DEAD_RELAY_URL);
  await Promise.all([
    page.route("**/corvette/api/v1/cameras", (route) => route.fulfill({ json: { cameras } })),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] })),
    page.route("**/corvette/api/v1/cameras/*/still.jpg*", (route) =>
      route.fulfill({ contentType: "image/jpeg", body: JPEG }),
    ),
    // Never answered, so the live screen settles on neither a picture nor an error.
    page.route("**/live/hls/**", () => {}),
  ]);
  return requests;
};

test("a camera page address opens that camera's live screen", async ({ page }) => {
  const requests = await setup(page, [FRONT_CAMERA, BACK_CAMERA]);
  await page.goto("/camera/front");

  await expect(
    page.getByRole("dialog", { name: /Front live video expanded live view/i }),
  ).toBeVisible();
  await expect(page.locator(".live-path-indicator")).toHaveCount(1);
  await expect(page.locator(".camera-grid")).toHaveCount(0);
  expect(requests.some((url) => url.includes("still.jpg") || url.includes("latest.jpg"))).toBe(
    false,
  );
  expect(
    requests.some(
      (url) => url.includes("/back/") && (url.includes("still") || url.includes("latest")),
    ),
  ).toBe(false);
});

test("closing the camera page returns to the dashboard", async ({ page }) => {
  await setup(page, [FRONT_CAMERA, BACK_CAMERA]);
  await page.goto("/camera/front");

  await page.getByRole("button", { name: "Close" }).click();
  await expect(page).toHaveURL("/");
  await expect(page.locator(".camera-grid")).toBeVisible();
});

test("an unknown camera says so and links home", async ({ page }) => {
  await setup(page, [FRONT_CAMERA, BACK_CAMERA]);
  await page.goto("/camera/nope");

  await expect(page.locator(".empty-state h2")).toHaveText("Camera not found");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("link", { name: "All cameras" })).toHaveAttribute("href", "/");
});

test("an off camera says it is off", async ({ page }) => {
  await setup(page, [LIVING_CAMERA]);
  await page.goto("/camera/living");

  await expect(page.locator(".empty-state h2")).toHaveText("Living is off");
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("a camera list that cannot be fetched says so", async ({ page }) => {
  await page.addInitScript((url) => {
    window.__corvetteMoqRelayUrl = url;
  }, DEAD_RELAY_URL);
  await page.route("**/corvette/api/v1/cameras", (route) => route.fulfill({ status: 500 }));
  await page.goto("/camera/front");

  await expect(page.locator(".empty-state h2")).toHaveText("Camera unavailable");
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("a dashboard tile opens its camera's page", async ({ page }) => {
  await setup(page, [FRONT_CAMERA, BACK_CAMERA]);
  await page.goto("/");

  await expect(page.getByRole("link", { name: /open front live view/i })).toHaveAttribute(
    "href",
    "/camera/front",
  );
  // The separate "Own page" link is gone: the tile itself is the only link.
  await expect(page.getByRole("link", { name: /own page/i })).toHaveCount(0);

  await page.evaluate(() => {
    window.__spaMarker = 1;
  });
  await page.getByRole("link", { name: /open front live view/i }).click();
  await expect(
    page.getByRole("dialog", { name: /Front live video expanded live view/i }),
  ).toBeVisible();
  expect(new URL(page.url()).pathname).toBe("/camera/front");
  await expect(page.locator(".camera-grid")).toHaveCount(0);
  // Surviving the tap proves the page opened by client-side navigation.
  expect(await page.evaluate(() => window.__spaMarker)).toBe(1);
});
