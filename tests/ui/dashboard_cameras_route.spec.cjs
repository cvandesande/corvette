const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Issue #4 item A9's own Verify step: the UI's camera list comes from
// Corvette's `/corvette/api/v1/cameras` route -- the order and errors it
// answers with, with no request to Frigate's old camera source.
const mockCameraList = (page, cameras) => {
  return Promise.all([
    page.route("**/corvette/api/v1/cameras", (route) => route.fulfill({ json: { cameras } })),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/events?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/recordings/summary?*", (route) => route.fulfill({ json: {} })),
    page.route("**/api/preview/all/start/*/end/*", (route) => route.fulfill({ json: [] })),
  ]);
};

const FRONT_CAMERA = {
  name: "front",
  display_name: "Front",
  order: 0,
  detect_width: 1920,
  detect_height: 1080,
};

test("the dashboard shows cameras in the order the camera list route returns them", async ({
  page,
}) => {
  // Neither ordering a UI could infer on its own matches the shown order:
  // names sort to alpha/mid/zeta, display names to Alpha/Mike/Zulu cam.
  await mockCameraList(page, [
    { name: "zeta", display_name: "Zulu cam", order: 30, detect_width: 1920, detect_height: 1080 },
    { name: "alpha", display_name: "Mike cam", order: 10, detect_width: 1280, detect_height: 720 },
    { name: "mid", display_name: "Alpha cam", order: 20, detect_width: 1280, detect_height: 720 },
  ]);

  await page.goto("/");

  await expect(page.locator(".camera-card")).toHaveCount(3);
  await expect(page.locator(".camera-card h2")).toHaveText(["Zulu cam", "Mike cam", "Alpha cam"]);
});

test("no page asks Frigate's /api/config for its camera list", async ({ page }) => {
  const requestPaths = [];
  page.on("request", (request) => requestPaths.push(new URL(request.url()).pathname));
  await mockCameraList(page, [FRONT_CAMERA]);

  // The WASM app issues its camera fetch shortly after `load`, so each page
  // is awaited up to that request; otherwise a navigation would cancel the
  // previous page mid-boot and the assertion below would judge nothing.
  for (const path of ["/", "/recordings", "/monitor"]) {
    const camerasRequested = page.waitForRequest("**/corvette/api/v1/cameras", { timeout: 10_000 });
    await page.goto(path);
    await camerasRequested;
  }

  expect(requestPaths, `pages requested: ${requestPaths.join(", ")}`).not.toContain("/api/config");
});

test("the dashboard reports unavailable cameras when the camera route answers 503", async ({
  page,
}) => {
  await page.route("**/corvette/api/v1/cameras", (route) =>
    route.fulfill({
      status: 503,
      json: {
        code: "config_unavailable",
        message: "Frigate's configuration could not be read.",
      },
    }),
  );

  await page.goto("/");

  // Scoped to the live section: the events section shows its own status while
  // its (mocked-away) requests settle.
  await expect(page.locator("#live").getByRole("status")).toContainText("Cameras unavailable");
});
