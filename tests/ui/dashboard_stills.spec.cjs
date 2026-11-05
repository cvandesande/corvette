const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Pins the dashboard tile's still-image behavior: each tile shows the latest
// picture of its camera from the signed-in image route, refreshes it every 30
// seconds while the page is visible and no expanded view is open, labels the
// picture by its own load time, and reports failure as `No picture`.

// A real, decodable 1x1 JPEG, held as base64 so no binary file enters the
// repository: a stub that merely returns bytes is not enough, because the tile
// only swaps a picture in once the browser has actually loaded it.
const JPEG = Buffer.from(
  "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q==",
  "base64",
);

// A decodable 2x2 JPEG, a different size from the first picture, so a swapped
// refresh shows as a new natural width on the displayed image.
const JPEG2 = Buffer.from(
  "/9j/4AAQSkZJRgABAgAAAQABAAD//gAQTGF2YzYyLjI4LjEwMgD/2wBDAAgEBAQEBAUFBQUFBQYGBgYGBgYGBgYGBgYHBwcICAgHBwcGBgcHCAgICAkJCQgICAgJCQoKCgwMCwsODg4RERT/xABLAAEBAAAAAAAAAAAAAAAAAAAABQEBAAAAAAAAAAAAAAAAAAAABhABAAAAAAAAAAAAAAAAAAAAABEBAAAAAAAAAAAAAAAAAAAAAP/AABEIAAIAAgMBIgACEQADEQD/2gAMAwEAAhEDEQA/AIIB2OP/2Q==",
  "base64",
);

// The exact address a front-camera tile must ask for: height is its only query
// parameter, because the contract refuses any other one.
const FRONT_STILL_URL = "/corvette/api/v1/cameras/front/still.jpg?height=270";
const GARAGE_STILL_URL = "/corvette/api/v1/cameras/garage/still.jpg?height=270";

const FRONT_CAMERA = {
  name: "front",
  display_name: "Front",
  order: 0,
  detect_width: 1920,
  detect_height: 1080,
};

const GARAGE_CAMERA = {
  name: "garage",
  display_name: "Garage",
  order: 1,
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
    route.fulfill({
      contentType: "image/jpeg",
      headers: { "cache-control": "no-store" },
      body: JPEG,
    }),
  );

const recordStillRequests = (page, requests) => {
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.endsWith("/still.jpg")) {
      requests.push(`${url.pathname}${url.search}`);
    }
  });
};

const requestsForCameraRefresh = (requests, camera) =>
  requests.filter((path) => path.startsWith(`/corvette/api/v1/cameras/${camera}/`)).length;

const stillTile = (page, index) => page.locator(".camera-card").nth(index).locator(".still-tile");
const stillImage = (page, index) => stillTile(page, index).locator("img.still-tile-image");

test("each tile shows a sized still picture and no live video", async ({ page }) => {
  const allRequests = [];
  page.on("request", (request) => allRequests.push(new URL(request.url()).pathname));

  await stubDashboardApis(page, [FRONT_CAMERA]);
  await stubStills(page);
  await page.goto("/");

  await expect(stillImage(page, 0)).toHaveCount(1);
  await expect(stillImage(page, 0)).toHaveAttribute("src", FRONT_STILL_URL);
  await expect(page.locator(".camera-card video")).toHaveCount(0);
  expect(
    allRequests.filter((pathname) => pathname.startsWith("/live/mse/ws/")),
    "the dashboard grid must make no media-bridge WebSocket request",
  ).toEqual([]);
});

test("each tile requests a fresh picture every 30 seconds", async ({ page }) => {
  await page.clock.install();
  const requests = [];
  recordStillRequests(page, requests);

  await stubDashboardApis(page, [FRONT_CAMERA, GARAGE_CAMERA]);
  await stubStills(page);
  await page.goto("/");

  await expect(stillImage(page, 0)).toHaveAttribute("src", FRONT_STILL_URL);
  await expect(stillImage(page, 1)).toHaveAttribute("src", GARAGE_STILL_URL);

  await page.clock.fastForward(30_000);

  await expect
    .poll(() => requestsForCameraRefresh(requests, "front"))
    .toBeGreaterThanOrEqual(2);
  await expect
    .poll(() => requestsForCameraRefresh(requests, "garage"))
    .toBeGreaterThanOrEqual(2);

  // The refresh re-reads the identical address: no counter, no other parameter.
  const frontRequests = requests.filter((path) =>
    path.startsWith("/corvette/api/v1/cameras/front/"),
  );
  expect(frontRequests[0]).toBe(FRONT_STILL_URL);
  expect(frontRequests[1]).toBe(FRONT_STILL_URL);

  // A refresh that loaded restarts the tile's own age, so the label is back to
  // "just now" even though thirty seconds passed.
  await expect(stillTile(page, 0).locator(".still-tile-age")).toHaveText("just now");
  await expect(stillImage(page, 0)).toHaveAttribute("src", FRONT_STILL_URL);
  await expect(stillImage(page, 1)).toHaveAttribute("src", GARAGE_STILL_URL);
});

test("a refreshed picture replaces the displayed one", async ({ page }) => {
  await page.clock.install();
  // Each refresh cycle fetches the identical address twice, once to preload and
  // once when the visible image takes the src, so the stub switches on the
  // clock moving rather than on a request count.
  let refreshed = false;

  await stubDashboardApis(page, [FRONT_CAMERA]);
  await page.route("**/corvette/api/v1/cameras/*/still.jpg*", (route) =>
    route.fulfill({
      contentType: "image/jpeg",
      headers: { "cache-control": "no-store" },
      body: refreshed ? JPEG2 : JPEG,
    }),
  );

  await page.goto("/");
  await expect(stillImage(page, 0)).toHaveAttribute("src", FRONT_STILL_URL);
  await expect
    .poll(() => stillImage(page, 0).evaluate((image) => image.naturalWidth))
    .toBe(1);

  refreshed = true;
  await page.clock.fastForward(30_000);

  await expect
    .poll(() => stillImage(page, 0).evaluate((image) => image.naturalWidth))
    .toBe(2);
  await expect(stillImage(page, 0)).toHaveAttribute("src", FRONT_STILL_URL);
});

test("a hidden page skips picture refreshes and a visible page refreshes at once", async ({
  page,
}) => {
  await page.clock.install();
  const requests = [];
  recordStillRequests(page, requests);

  await stubDashboardApis(page, [FRONT_CAMERA, GARAGE_CAMERA]);
  await stubStills(page);
  await page.goto("/");

  await expect(stillImage(page, 0)).toHaveAttribute("src", FRONT_STILL_URL);
  await expect(stillImage(page, 1)).toHaveAttribute("src", GARAGE_STILL_URL);
  const baseline = requests.length;

  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", { configurable: true, get: () => true });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.clock.fastForward(30_000);
  expect(requests.length).toBe(baseline);

  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", { configurable: true, get: () => false });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect
    .poll(() => requestsForCameraRefresh(requests, "front"))
    .toBeGreaterThanOrEqual(2);
  await expect
    .poll(() => requestsForCameraRefresh(requests, "garage"))
    .toBeGreaterThanOrEqual(2);
});

test("an expanded view pauses picture refreshes and closing it resumes them", async ({ page }) => {
  await page.clock.install();
  const requests = [];
  recordStillRequests(page, requests);

  await stubDashboardApis(page, [FRONT_CAMERA]);
  await stubStills(page);
  await page.goto("/");

  await expect(stillImage(page, 0)).toHaveAttribute("src", FRONT_STILL_URL);
  const baseline = requests.length;

  await page.getByRole("button", { name: /expand front to full view/i }).click();
  await expect(page.locator(".expanded-view")).toHaveCount(1);

  await page.clock.fastForward(60_000);
  expect(requests.length).toBe(baseline);

  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.locator(".expanded-view")).toHaveCount(0);

  await expect.poll(() => requests.length, { timeout: 5_000 }).toBeGreaterThan(baseline);
});

test("a camera whose first load fails shows no picture and an old picture turns stale", async ({
  page,
}) => {
  await page.clock.install();

  await stubDashboardApis(page, [
    { ...FRONT_CAMERA, name: "broken", display_name: "Broken" },
    { ...GARAGE_CAMERA, name: "flaky", display_name: "Flaky" },
  ]);
  await page.route("**/corvette/api/v1/cameras/broken/still.jpg*", (route) => route.abort());
  let flakyLoads = 0;
  await page.route("**/corvette/api/v1/cameras/flaky/still.jpg*", (route) => {
    flakyLoads += 1;
    if (flakyLoads === 1) {
      return route.fulfill({ contentType: "image/jpeg", body: JPEG });
    }
    return route.abort();
  });

  await page.goto("/");
  await expect(stillImage(page, 1)).toHaveAttribute(
    "src",
    "/corvette/api/v1/cameras/flaky/still.jpg?height=270",
  );
  await expect(stillTile(page, 1)).toHaveClass(/still-tile-showing/);

  await page.clock.fastForward(60_000);

  await expect(stillTile(page, 0)).toHaveClass(/still-tile-no-picture/);
  await expect(stillTile(page, 0)).toContainText("No picture");

  await expect(stillTile(page, 1)).toHaveClass(/still-tile-stale/);
  await expect(stillTile(page, 1).locator(".still-tile-age")).toHaveText("no update for 1 min");
  await expect(stillImage(page, 1)).toHaveAttribute(
    "src",
    "/corvette/api/v1/cameras/flaky/still.jpg?height=270",
  );
});

test("the age label grows while refreshes fail", async ({ page }) => {
  await page.clock.install();

  await stubDashboardApis(page, [FRONT_CAMERA]);
  let loads = 0;
  await page.route("**/corvette/api/v1/cameras/*/still.jpg*", (route) => {
    loads += 1;
    if (loads === 1) {
      return route.fulfill({
        contentType: "image/jpeg",
        headers: { "cache-control": "no-store" },
        body: JPEG,
      });
    }
    return route.abort();
  });

  await page.goto("/");
  await expect(stillTile(page, 0).locator(".still-tile-age")).toHaveText("just now");

  await page.clock.fastForward(30_000);
  await expect(stillTile(page, 0).locator(".still-tile-age")).toHaveText("30 s ago");
});
