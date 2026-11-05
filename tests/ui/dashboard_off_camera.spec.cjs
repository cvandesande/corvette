const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// A camera the cameras route lists with `enabled: false` keeps its place in the
// grid, shows a plain off line instead of a picture, asks for no still image,
// and opens nothing when tapped.

// A real, decodable 1x1 JPEG, so the on cameras' tiles load a picture normally.
const JPEG = Buffer.from(
  "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q==",
  "base64",
);

// The list order matches neither alphabetical order: names sort to
// alpha/living/zeta and display names to Alpha cam/Living/Zulu cam, so an off
// middle camera sitting in the middle proves the grid keeps route order.
const ZETA_CAMERA = {
  name: "zeta",
  display_name: "Zulu cam",
  order: 30,
  detect_width: 1920,
  detect_height: 1080,
};

const LIVING_CAMERA = {
  name: "living",
  display_name: "Living",
  order: 20,
  detect_width: 1280,
  detect_height: 720,
  enabled: false,
};

const ALPHA_CAMERA = {
  name: "alpha",
  display_name: "Alpha cam",
  order: 10,
  detect_width: 1280,
  detect_height: 720,
};

const stubDashboardApis = (page, cameras) =>
  Promise.all([
    page.route("**/corvette/api/v1/cameras", (route) => route.fulfill({ json: { cameras } })),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] })),
  ]);

const stubStills = (page) =>
  page.route("**/corvette/api/v1/cameras/*/still.jpg*", (route) =>
    route.fulfill({ contentType: "image/jpeg", body: JPEG }),
  );

const recordStillRequests = (page, requests) => {
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.endsWith("/still.jpg")) {
      requests.push(url.pathname);
    }
  });
};

const stillRequestsFor = (requests, camera) =>
  requests.filter((pathname) => pathname === `/corvette/api/v1/cameras/${camera}/still.jpg`).length;

test("an off camera keeps its grid slot and shows an off line", async ({ page }) => {
  await stubDashboardApis(page, [ZETA_CAMERA, LIVING_CAMERA, ALPHA_CAMERA]);
  await stubStills(page);
  await page.goto("/");

  await expect(page.locator(".camera-card h2")).toHaveText(["Zulu cam", "Living", "Alpha cam"]);

  const offLine = page.locator(".camera-card-off");
  await expect(offLine).toHaveText("Living: off");
  await expect(offLine).toHaveCSS("color", "rgb(215, 226, 224)");
});

test("an off camera asks for no picture while the on cameras do", async ({ page }) => {
  const stillRequests = [];
  recordStillRequests(page, stillRequests);

  await stubDashboardApis(page, [ZETA_CAMERA, LIVING_CAMERA, ALPHA_CAMERA]);
  await stubStills(page);
  await page.goto("/");

  await expect.poll(() => stillRequestsFor(stillRequests, "zeta")).toBeGreaterThan(0);
  await expect.poll(() => stillRequestsFor(stillRequests, "alpha")).toBeGreaterThan(0);
  expect(stillRequestsFor(stillRequests, "living")).toBe(0);
});

test("tapping an off camera opens nothing while live tiles open their page", async ({ page }) => {
  await stubDashboardApis(page, [ZETA_CAMERA, LIVING_CAMERA, ALPHA_CAMERA]);
  await stubStills(page);
  await page.goto("/");

  // The off camera renders no link, so only the two live tiles are links.
  await expect(page.locator("a.camera-card-link")).toHaveCount(2);

  await page.locator(".camera-card").nth(1).click();
  expect(new URL(page.url()).pathname).toBe("/");
  await expect(page.locator(".expanded-view")).toHaveCount(0);

  await page.getByRole("link", { name: /open zulu cam live view/i }).click();
  expect(new URL(page.url()).pathname).toBe("/camera/zeta");
  await expect(
    page.getByRole("dialog", { name: /Zulu cam live video expanded live view/i }),
  ).toBeVisible();
});
