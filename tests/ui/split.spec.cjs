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

// Civil-date arithmetic, done in UTC where no daylight-saving shift can occur.
const consecutiveDates = (lastDate, count) =>
  Array.from({ length: count }, (_, index) => {
    const date = new Date(`${lastDate}T00:00:00Z`);
    date.setUTCDate(date.getUTCDate() - (count - 1 - index));
    return date.toISOString().slice(0, 10);
  });

const calendarDayLabel = (date) =>
  `${new Intl.DateTimeFormat("en-IE", { timeZone: "UTC" }).format(
    new Date(`${date}T00:00:00Z`),
  )}, recordings available`;

test.describe("daylight-saving date conversion", () => {
  test.use({ timezoneId: "America/New_York" });

  const transitions = [
    { day: "23-hour day", now: "2026-03-09T00:30:00-04:00", lastDate: "2026-03-09" },
    { day: "25-hour day", now: "2026-11-01T23:30:00-05:00", lastDate: "2026-11-01" },
  ];

  for (const { day, now, lastDate } of transitions) {
    test(`event calendar covers every local date across a ${day}`, async ({ page }) => {
      const dates = consecutiveDates(lastDate, 21);
      await page.clock.setFixedTime(new Date(now));
      await page.route("**/api/recordings/summary?*", (route) =>
        route.fulfill({ json: Object.fromEntries(dates.map((date) => [date, true])) }),
      );
      await page.route("**/api/review?*", (route) => route.fulfill({ json: [] }));
      await page.route("**/api/review/activity/motion?*", (route) =>
        route.fulfill({ json: [] }),
      );

      await page.goto("/events");
      const eventDays = page.getByRole("group", { name: "Event days" }).getByRole("button");
      await expect(eventDays).toHaveCount(21);
      expect(
        await eventDays.evaluateAll((buttons) =>
          buttons.map((button) => button.getAttribute("aria-label")),
        ),
      ).toEqual(dates.map(calendarDayLabel));
    });
  }
});

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
  let alertClipPath;
  let mergedMotionPreviewPath;
  await page.route("**/api/config", (route) => route.fulfill({ json: { cameras: {} } }));
  await page.route("**/api/review?*", (route) => {
    const url = new URL(route.request().url());
    const after = Number(url.searchParams.get("after"));
    const before = Number(url.searchParams.get("before"));
    expect(before - after).toBeCloseTo(6 * 60 * 60, 0);
    alertClipPath = `/api/front/start/${before - 60}/end/${before - 50}/clip.mp4`;
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
      ],
    });
  });
  await page.route("**/api/review/activity/motion?*", (route) => {
    const before = Number(new URL(route.request().url()).searchParams.get("before"));
    mergedMotionPreviewPath =
      `/api/back/start/${before - 180}/end/${before - 120}/preview.gif`;
    return route.fulfill({
      json: [
        { start_time: before - 180, motion: 42, camera: "back" },
        { start_time: before - 150, motion: 21, camera: "back" },
        { start_time: before - 120, motion: 0, camera: "" },
        { start_time: before - 60, motion: 12, camera: "back" },
      ],
    });
  });
  await page.route("**/clips/review/*.webp", (route) =>
    route.fulfill({ contentType: "image/webp", body: Buffer.alloc(0) }),
  );
  await page.route("**/api/*/start/*/end/*/preview.gif", (route) =>
    route.fulfill({ contentType: "image/gif", body: Buffer.alloc(0) }),
  );
  await page.route("**/api/front/start/*/end/*/clip.mp4", (route) =>
    route.fulfill({ contentType: "video/mp4", body: Buffer.alloc(0) }),
  );

  await page.goto("/");
  const cards = page.locator(".event-card");
  await expect(cards).toHaveCount(3);
  await page.getByRole("button", { name: "Motion", exact: true }).click();
  await expect(cards).toHaveCount(2);
  await expect(cards).toContainText(["Motion", "Motion"]);
  await expect(cards.nth(1).locator("img")).toHaveAttribute(
    "src",
    mergedMotionPreviewPath,
  );
  await page.getByRole("button", { name: "Alerts", exact: true }).click();
  await expect(cards).toHaveCount(1);
  await expect(cards).toContainText("person");
  await cards.getByRole("button").click();
  await expect(page.getByRole("dialog", { name: "Selected event playback" })).toBeVisible();
  await expect(page.locator(".playback-modal")).toHaveCSS("position", "fixed");
  await expect(page.locator(".review-playback video")).toHaveAttribute(
    "src",
    alertClipPath,
  );
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Selected event playback" })).toHaveCount(0);
  await page.getByRole("button", { name: "Detections", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "No events in the last 6 hours" }),
  ).toBeVisible();
});

test("events route browses a selected day and filters severity", async ({ page }) => {
  let alertClipPath;
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
    alertClipPath = `/api/front/start/${after + 60}/end/${after + 70}/clip.mp4`;
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
  await page.route("**/api/review/activity/motion?*", (route) => {
    const after = Number(new URL(route.request().url()).searchParams.get("after"));
    return route.fulfill({
      json: [{ start_time: after + 180, motion: 25, camera: "front" }],
    });
  });
  await page.route("**/clips/review/*.webp", (route) =>
    route.fulfill({ contentType: "image/webp", body: Buffer.alloc(0) }),
  );
  await page.route("**/api/*/start/*/end/*/preview.gif", (route) =>
    route.fulfill({ contentType: "image/gif", body: Buffer.alloc(0) }),
  );
  await page.route("**/api/front/start/*/end/*/clip.mp4", (route) =>
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
  await expect(cards).toHaveCount(3);
  await cards.filter({ hasText: "person" }).getByRole("button").click();
  await expect(page.getByRole("dialog", { name: "Selected event playback" })).toBeVisible();
  await expect(page.locator(".review-playback video")).toHaveAttribute(
    "src",
    alertClipPath,
  );
  await page.getByRole("button", { name: "Close" }).click();
  await page.getByRole("button", { name: "Detections", exact: true }).click();
  await expect(cards).toHaveCount(1);
  await expect(cards).toContainText("cat");
  await page.getByRole("button", { name: "Motion", exact: true }).click();
  await expect(cards).toHaveCount(1);
  await expect(cards).toContainText("Motion");
});
