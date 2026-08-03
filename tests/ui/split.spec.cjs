const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

const isRecordingPayload = (request) =>
  new URL(request.url()).pathname.includes("split___recordings_route_view_");

test("recording payload loads only after recordings navigation", async ({ page }) => {
  const recordingPayloads = [];
  page.on("request", (request) => {
    if (isRecordingPayload(request)) recordingPayloads.push(request.url());
  });

  await page.goto("/");
  await expect(page.getByRole("link", { name: "Recordings" })).toBeVisible();
  expect(recordingPayloads).toEqual([]);

  await page.getByRole("link", { name: "Recordings" }).click();
  await expect(page).toHaveURL(/\/recordings$/);
  await expect.poll(() => recordingPayloads.length).toBe(1);
});

test("recordings route supports direct navigation", async ({ page }) => {
  const response = await page.goto("/recordings");

  expect(response?.status()).toBe(200);
  await expect(page.getByRole("link", { name: "Recordings" })).toBeVisible();
});

test("recording filters update the visible recordings", async ({ page }) => {
  const now = Date.now() / 1000;
  await page.route("**/api/config", (route) =>
    route.fulfill({
      json: {
        cameras: {
          front: { enabled: true, friendly_name: "Front", ui: { order: 0 } },
        },
      },
    }),
  );
  await page.route("**/api/events?*", (route) => route.fulfill({ json: [] }));
  await page.route("**/api/review?*", (route) =>
    route.fulfill({
      json: [
        { start_time: now - 500, end_time: now - 490, severity: "detection" },
        { start_time: now - 200, end_time: now - 190, severity: "alert" },
      ],
    }),
  );
  await page.route("**/api/recordings/summary?*", (route) =>
    route.fulfill({ json: {} }),
  );
  await page.route("**/api/front/recordings?*", (route) =>
    route.fulfill({
      json: [
        { start_time: now - 510, end_time: now - 480, motion: 1 },
        { start_time: now - 210, end_time: now - 180, motion: null },
      ],
    }),
  );

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();
  const recordings = page.locator(".recording-list article");
  await expect(recordings).toHaveCount(2);

  await page.getByRole("button", { name: "Detection" }).click();
  await expect(recordings).toHaveCount(1);
  await page.getByRole("button", { name: "All" }).click();
  await expect(recordings).toHaveCount(2);
});
