const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

const isRecordingPayload = (request) =>
  new URL(request.url()).pathname.includes("split___recordings_route_view_");

const recentRecordingDays = (count) =>
  Object.fromEntries(
    Array.from({ length: count }, (_, offset) => {
      const date = new Date();
      date.setDate(date.getDate() - offset);
      return [date.toISOString().slice(0, 10), true];
    }),
  );

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

test("recording timeline exposes availability and review activity", async ({ page }) => {
  const now = Date.now() / 1000;
  const recordingDays = recentRecordingDays(21);
  await page.addInitScript(() => {
    HTMLMediaElement.prototype.play = function play() {
      Object.defineProperty(this, "paused", { configurable: true, get: () => false });
      this.dispatchEvent(new Event("play"));
      return Promise.resolve();
    };
  });
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
        { start_time: now - 1800, end_time: now - 1800, severity: "detection" },
        { start_time: now - 200, end_time: now - 190, severity: "alert" },
      ],
    }),
  );
  await page.route("**/api/recordings/summary?*", (route) =>
    route.fulfill({ json: recordingDays }),
  );
  await page.route("**/api/front/recordings?*", (route) =>
    route.fulfill({
      json: [
        { start_time: now - 510, end_time: now - 480, motion: 1 },
        { start_time: now - 210, end_time: now - 180, motion: null },
      ],
    }),
  );
  await page.route("**/api/preview/front/start/*/end/*", (route) =>
    route.fulfill({
      json: [
        {
          camera: "front",
          src: "/clips/previews/front/hour.mp4",
          type: "video/mp4",
          start: now - 3600,
          end: now,
        },
      ],
    }),
  );
  await page.route("**/clips/previews/front/hour.mp4", (route) =>
    route.fulfill({ contentType: "video/mp4", body: Buffer.alloc(0) }),
  );

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();
  await expect(
    page.getByRole("region", { name: "Recording timeline" }),
  ).toBeVisible();
  const availableSpans = page.locator(".timeline-availability");
  await expect(availableSpans).toHaveCount(2);
  const activitySpans = page.locator(".timeline-activity");
  await expect(activitySpans).toHaveCount(3);
  const motionRecording = page.locator(".timeline-motion-recording");
  await expect(motionRecording).toHaveCount(1);
  await expect(page.locator(".timeline-activity.activity-detection.timeline-point"))
    .toHaveCount(1);
  await expect(page.locator(".timeline-player")).toHaveAttribute(
    "src",
    "/clips/previews/front/hour.mp4",
  );

  const playhead = page.getByRole("slider", { name: "Recording playhead" });
  await expect
    .poll(() => playhead.inputValue().then(Number))
    .toBeCloseTo(now - 510, 0);
  await motionRecording.click();
  await expect
    .poll(() => playhead.inputValue().then(Number))
    .toBeCloseTo(now - 510, 0);
  await page.locator(".timeline-player").dispatchEvent("ended");
  await expect
    .poll(() => playhead.inputValue().then(Number))
    .toBeCloseTo(now - 200, 0);
  await page.locator(".timeline-player").dispatchEvent("ended");
  await expect
    .poll(() => playhead.inputValue().then(Number))
    .toBeCloseTo(now - 190, 0);
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible();
  await playhead.evaluate((input, value) => {
    input.value = value;
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }, String(now - 300));
  await expect
    .poll(() => playhead.inputValue().then(Number))
    .toBeCloseTo(now - 210, 0);
  await page.getByRole("button", { name: "Play", exact: true }).click();
  await expect(page.locator(".timeline-player")).toHaveAttribute(
    "src",
    new RegExp(`/api/front/start/${now - 210}/end/${now - 180}/clip\\.mp4`),
  );

  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  const scrollBeforeDrag = await page.evaluate(() => window.scrollY);
  const sliderBounds = await playhead.boundingBox();
  expect(sliderBounds).not.toBeNull();
  await page.mouse.move(sliderBounds.x + sliderBounds.width * 0.25, sliderBounds.y + 5);
  await page.mouse.down();
  await page.mouse.move(sliderBounds.x + sliderBounds.width * 0.75, sliderBounds.y + 5);
  await page.mouse.up();
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBeCloseTo(scrollBeforeDrag, 0);

  await page.setViewportSize({ width: 390, height: 844 });
  const recordingDayButtons = page
    .getByRole("group", { name: "Recent days" })
    .getByRole("button", { disabled: false });
  await recordingDayButtons.nth(0).click();
  await recordingDayButtons.nth(1).click();
  await expect(page.locator(".calendar-grid button[aria-pressed='true']")).toHaveCount(2);

});

test("recent events are filtered review activity from the last six hours", async ({ page }) => {
  await page.route("**/api/config", (route) => route.fulfill({ json: { cameras: {} } }));
  await page.route("**/api/review?*", (route) => {
    const url = new URL(route.request().url());
    const after = Number(url.searchParams.get("after"));
    const before = Number(url.searchParams.get("before"));
    expect(before - after).toBeCloseTo(6 * 60 * 60, 0);
    return route.fulfill({
      json: [
        {
          id: "alert-review",
          camera: "front",
          start_time: before - 60,
          end_time: before - 50,
          severity: "alert",
          thumb_path: "/media/frigate/clips/review/alert.webp",
          data: { objects: ["person"], zones: ["drive"], audio: [] },
        },
        {
          id: "motion-review",
          camera: "back",
          start_time: before - 120,
          end_time: before - 110,
          severity: "significant_motion",
          thumb_path: "/media/frigate/clips/review/motion.webp",
          data: { objects: [], zones: [], audio: [] },
        },
      ],
    });
  });
  await page.route("**/clips/review/*.webp", (route) =>
    route.fulfill({ contentType: "image/webp", body: Buffer.alloc(0) }),
  );

  await page.goto("/");
  const cards = page.locator(".event-card");
  await expect(cards).toHaveCount(2);
  await page.getByRole("button", { name: "Motion", exact: true }).click();
  await expect(cards).toHaveCount(1);
  await expect(cards).toContainText("Motion");
  await page.getByRole("button", { name: "Alerts", exact: true }).click();
  await expect(cards).toHaveCount(1);
  await expect(cards).toContainText("person");
  await page.getByRole("button", { name: "Detections", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "No events in the last 6 hours" }),
  ).toBeVisible();
});

test("events route browses a selected day and filters severity", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.route("**/api/recordings/summary?*", (route) =>
    route.fulfill({ json: recentRecordingDays(20) }),
  );
  await page.route("**/api/review?*", (route) => {
    const url = new URL(route.request().url());
    const after = Number(url.searchParams.get("after"));
    const before = Number(url.searchParams.get("before"));
    const rangeDuration = before - after;
    if (rangeDuration < 2 * 24 * 60 * 60) {
      expect(rangeDuration).toBeGreaterThanOrEqual(23 * 60 * 60);
      expect(rangeDuration).toBeLessThanOrEqual(25 * 60 * 60);
    }
    return route.fulfill({
      json: [
        {
          id: "alert-review",
          camera: "front",
          start_time: after + 60,
          end_time: after + 70,
          severity: "alert",
          thumb_path: "/media/frigate/clips/review/alert.webp",
          data: { objects: ["person"], zones: ["drive"], audio: [] },
        },
        {
          id: "detection-review",
          camera: "back",
          start_time: after + 120,
          end_time: after + 130,
          severity: "detection",
          thumb_path: "/media/frigate/clips/review/detection.webp",
          data: { objects: ["cat"], zones: [], audio: [] },
        },
      ],
    });
  });
  await page.route("**/clips/review/*.webp", (route) =>
    route.fulfill({ contentType: "image/webp", body: Buffer.alloc(0) }),
  );
  await page.route("**/api/review/*/clip.mp4", (route) =>
    route.fulfill({ contentType: "video/mp4", body: Buffer.alloc(0) }),
  );

  await page.goto("/events");
  await expect(page.getByRole("heading", { name: "Events", exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "Events" })).toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(page.getByRole("group", { name: "Event days" }).getByRole("button"))
    .toHaveCount(21);
  await expect(
    page.getByRole("group", { name: "Event days" }).getByRole("button", { disabled: true }),
  ).toHaveCount(1);
  await expect(
    page.getByRole("group", { name: "Event days" }).locator(".activity-alert"),
  ).toHaveCount(1);
  const eventDays = page.getByRole("group", { name: "Event days" }).getByRole("button");
  await eventDays.nth(1).click();
  await eventDays.nth(2).click();
  await expect(page.locator(".calendar-grid button[aria-pressed='true']")).toHaveCount(2);
  const cards = page.locator(".event-card");
  await expect(cards).toHaveCount(2);
  await cards.nth(0).getByRole("button").click();
  await expect(page.getByRole("region", { name: "Selected event playback" })).toBeVisible();
  await expect(page.locator(".review-playback video")).toHaveAttribute(
    "src",
    "/api/review/alert-review/clip.mp4",
  );
  await page.getByRole("button", { name: "Close" }).click();
  await page.getByRole("button", { name: "Detections", exact: true }).click();
  await expect(cards).toHaveCount(1);
  await expect(cards).toContainText("cat");
});
