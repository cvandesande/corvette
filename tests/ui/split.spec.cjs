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

// A 1x1 PNG. An <img> fires `error` on an empty body, so any test that asserts
// an image was kept rather than replaced has to serve bytes that decode.
const DECODABLE_IMAGE = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

// Answers the recording-availability preflight with the whole requested range
// retained, so a review's clip URL is reached rather than reported as expired.
const retainRequestedRange = (route) => {
  const params = new URL(route.request().url()).searchParams;
  return route.fulfill({
    json: [
      {
        start_time: Number(params.get("after")),
        end_time: Number(params.get("before")),
        motion: 1,
      },
    ],
  });
};

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

test.describe("single-day request bounds", () => {
  test.use({ timezoneId: "America/New_York" });

  // Frigate matches reviews with `start_time < before`, so a day's upper bound is
  // exclusive and belongs at the *next* day's local midnight. Browsing 31 July
  // therefore asks Frigate for everything before 1 August 00:00; an end bound of
  // 23:59 on the day itself would silently drop its final minute.
  test("a chosen day is requested from its local midnight to the next", async ({ page }) => {
    const julyThirtyFirstStart = 1785470400;
    const augustFirstStart = 1785556800;
    const reviewBounds = [];
    const motionBounds = [];
    const boundsOf = (request) => {
      const params = new URL(request.url()).searchParams;
      return [Number(params.get("after")), Number(params.get("before"))];
    };

    await page.clock.setFixedTime(new Date("2026-08-03T12:00:00-04:00"));
    await page.route("**/api/recordings/summary?*", (route) =>
      route.fulfill({
        json: Object.fromEntries(
          consecutiveDates("2026-08-03", 21).map((date) => [date, true]),
        ),
      }),
    );
    await page.route("**/api/review?*", (route) => {
      reviewBounds.push(boundsOf(route.request()));
      return route.fulfill({ json: [] });
    });
    await page.route("**/api/review/activity/motion?*", (route) => {
      motionBounds.push(boundsOf(route.request()));
      return route.fulfill({ json: [] });
    });

    await page.goto("/events");
    const eventDays = page.getByRole("group", { name: "Event days" }).getByRole("button");
    await expect(eventDays).toHaveCount(21);
    await expect(eventDays.nth(17)).toHaveAccessibleName(calendarDayLabel("2026-07-31"));
    await eventDays.nth(17).click();

    await expect
      .poll(() => reviewBounds)
      .toContainEqual([julyThirtyFirstStart, augustFirstStart]);
    await expect
      .poll(() => motionBounds)
      .toContainEqual([julyThirtyFirstStart, augustFirstStart]);
  });
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

test.describe("recording timeline", () => {
  // One camera with two retained spans in the last hour: an earlier one that
  // recorded motion, and a later one that did not. Two reviews sit inside the
  // window, the first of them instantaneous.
  let now;

  test.beforeEach(async ({ page }) => {
    now = Date.now() / 1000;
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
      route.fulfill({ json: recentRecordingDays(21) }),
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
    await expect(page.getByRole("region", { name: "Recording timeline" })).toBeVisible();
  });

  test("marks each retained span, motion span and review severity", async ({ page }) => {
    await expect(page.locator(".timeline-availability")).toHaveCount(2);
    await expect(page.locator(".timeline-activity")).toHaveCount(3);
    await expect(page.locator(".timeline-motion-recording")).toHaveCount(1);
    await expect(
      page.locator(".timeline-activity.activity-detection.timeline-point"),
    ).toHaveCount(1);
  });

  test("scrubs against Frigate's low-resolution preview video", async ({ page }) => {
    await expect(page.locator(".timeline-player")).toHaveAttribute(
      "src",
      "/clips/previews/front/hour.mp4",
    );
  });

  test("advances through each later activity and stops after the last", async ({ page }) => {
    const playhead = page.getByRole("slider", { name: "Recording playhead" });
    await expect.poll(() => playhead.inputValue().then(Number)).toBeCloseTo(now - 510, 0);

    await page.locator(".timeline-motion-recording").click();
    await expect.poll(() => playhead.inputValue().then(Number)).toBeCloseTo(now - 510, 0);

    await page.locator(".timeline-player").dispatchEvent("ended");
    await expect.poll(() => playhead.inputValue().then(Number)).toBeCloseTo(now - 200, 0);

    await page.locator(".timeline-player").dispatchEvent("ended");
    await expect.poll(() => playhead.inputValue().then(Number)).toBeCloseTo(now - 190, 0);
    await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible();
  });

  test("snaps a playhead time in a gap to the nearest retained span", async ({ page }) => {
    const playhead = page.getByRole("slider", { name: "Recording playhead" });
    await playhead.evaluate((input, value) => {
      input.value = value;
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }, String(now - 300));

    await expect.poll(() => playhead.inputValue().then(Number)).toBeCloseTo(now - 210, 0);
  });

  test("plays the full-resolution clip once playback starts", async ({ page }) => {
    const playhead = page.getByRole("slider", { name: "Recording playhead" });
    await playhead.evaluate((input, value) => {
      input.value = value;
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }, String(now - 300));
    await expect.poll(() => playhead.inputValue().then(Number)).toBeCloseTo(now - 210, 0);

    await page.getByRole("button", { name: "Play", exact: true }).click();
    await expect(page.locator(".timeline-player")).toHaveAttribute(
      "src",
      new RegExp(`/api/front/start/${now - 210}/end/${now - 180}/clip\\.mp4`),
    );
  });

  // The playhead's own bounds are the window the track is showing, so their
  // span is what zooming changes.
  const visibleSeconds = async (page) => {
    const playhead = page.locator(".timeline-track input");
    return (
      Number(await playhead.getAttribute("max")) - Number(await playhead.getAttribute("min"))
    );
  };

  test("ctrl and the wheel zoom the window, stopping at the whole selection", async ({
    page,
  }) => {
    const wholeSelection = await visibleSeconds(page);
    expect(wholeSelection).toBeCloseTo(3600, 0);

    await page.locator(".timeline-track").hover();
    await page.keyboard.down("Control");
    await page.mouse.wheel(0, -500);
    await page.keyboard.up("Control");
    await expect.poll(() => visibleSeconds(page)).toBeLessThan(wholeSelection / 2);

    await page.keyboard.down("Control");
    await page.mouse.wheel(0, 5000);
    await page.keyboard.up("Control");
    await expect.poll(() => visibleSeconds(page)).toBeCloseTo(wholeSelection, 0);
  });

  test("pinching the track widens the spans inside it", async ({ page }) => {
    const track = page.locator(".timeline-track");
    const motionWidth = () =>
      page
        .locator(".timeline-motion-recording")
        .evaluate((span) => Number.parseFloat(span.style.width));
    const box = await track.boundingBox();
    const acrossTrack = (fraction) => box.x + box.width * fraction;

    // A 30-second span of a one-hour selection, before zooming: under a
    // percent of the track, which is a handful of pixels to aim at.
    const beforePinch = await motionWidth();
    expect(beforePinch).toBeCloseTo((30 / 3600) * 100, 2);

    await track.dispatchEvent("pointerdown", { pointerId: 1, clientX: acrossTrack(0.8) });
    await track.dispatchEvent("pointerdown", { pointerId: 2, clientX: acrossTrack(0.9) });
    await track.dispatchEvent("pointermove", { pointerId: 2, clientX: acrossTrack(0.98) });

    await expect.poll(motionWidth).toBeGreaterThan(beforePinch);
    await expect.poll(() => visibleSeconds(page)).toBeLessThan(3600);
  });

  test("dragging the playhead does not scroll the page", async ({ page }) => {
    const playhead = page.getByRole("slider", { name: "Recording playhead" });
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    const scrollBeforeDrag = await page.evaluate(() => window.scrollY);
    const sliderBounds = await playhead.boundingBox();
    expect(sliderBounds).not.toBeNull();

    await page.mouse.move(sliderBounds.x + sliderBounds.width * 0.25, sliderBounds.y + 5);
    await page.mouse.down();
    await page.mouse.move(sliderBounds.x + sliderBounds.width * 0.75, sliderBounds.y + 5);
    await page.mouse.up();

    await expect.poll(() => page.evaluate(() => window.scrollY)).toBeCloseTo(scrollBeforeDrag, 0);
  });

  test("tapping two days on a phone selects a calendar range", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    const recordingDayButtons = page
      .getByRole("group", { name: "Recent days" })
      .getByRole("button", { disabled: false });

    await recordingDayButtons.nth(0).click();
    await recordingDayButtons.nth(1).click();

    await expect(page.locator(".calendar-grid button[aria-pressed='true']")).toHaveCount(2);
  });

  test("a cleared start time is reported instead of loading a range", async ({ page }) => {
    await page.getByRole("textbox", { name: "From" }).fill("");
    await page
      .getByRole("group", { name: "Recent days" })
      .getByRole("button", { disabled: false })
      .nth(0)
      .click();

    await expect(page.getByRole("alert")).toContainText(
      "Enter both times as 24-hour HH:MM before choosing a date.",
    );
  });
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
  await page.route("**/api/front/recordings?*", retainRequestedRange);
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

test("event cards show the detection snapshot, falling back to the review crop", async ({
  page,
}) => {
  await page.route("**/api/config", (route) => route.fulfill({ json: { cameras: {} } }));
  await page.route("**/api/review?*", (route) => {
    const before = Number(new URL(route.request().url()).searchParams.get("before"));
    return route.fulfill({
      json: [
        {
          id: "kept-snapshot",
          camera: "front",
          start_time: before - 60,
          end_time: before - 50,
          severity: "alert",
          thumb_path: "/media/frigate/clips/review/kept.webp",
          data: { objects: ["person"], zones: [], audio: [], detections: ["event-kept"] },
        },
        {
          id: "expired-snapshot",
          camera: "front",
          start_time: before - 120,
          end_time: before - 110,
          severity: "alert",
          thumb_path: "/media/frigate/clips/review/expired.webp",
          data: { objects: ["car"], zones: [], audio: [], detections: ["event-expired"] },
        },
        {
          id: "audio-only",
          camera: "front",
          start_time: before - 180,
          end_time: before - 170,
          severity: "detection",
          thumb_path: "/media/frigate/clips/review/audio.webp",
          data: { objects: [], zones: [], audio: ["speech"], detections: [] },
        },
      ],
    });
  });
  await page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] }));
  await page.route("**/api/events/event-kept/snapshot.jpg*", (route) =>
    route.fulfill({ contentType: "image/png", body: DECODABLE_IMAGE }),
  );
  // Frigate expires a snapshot on its own retention, so the event outlives it.
  await page.route("**/api/events/event-expired/snapshot.jpg*", (route) =>
    route.fulfill({
      status: 404,
      json: { success: false, message: "Snapshot not available" },
    }),
  );
  await page.route("**/clips/review/*.webp", (route) =>
    route.fulfill({ contentType: "image/png", body: DECODABLE_IMAGE }),
  );

  await page.goto("/");
  const images = page.locator(".event-card img");
  await expect(images).toHaveCount(3);

  await expect(images.nth(0)).toHaveAttribute(
    "src",
    "/api/events/event-kept/snapshot.jpg?crop=1&height=360&quality=80",
  );
  // The expired snapshot and the review with no tracked object both land on
  // the review's own crop rather than showing a broken image.
  await expect(images.nth(1)).toHaveAttribute("src", "/clips/review/expired.webp");
  await expect(images.nth(2)).toHaveAttribute("src", "/clips/review/audio.webp");
});

test("a review whose footage expired reports the absence, not a media error", async ({
  page,
}) => {
  await page.route("**/api/config", (route) => route.fulfill({ json: { cameras: {} } }));
  await page.route("**/api/review?*", (route) => {
    const before = Number(new URL(route.request().url()).searchParams.get("before"));
    return route.fulfill({
      json: [
        {
          id: "expired-review",
          camera: "front",
          start_time: before - 60,
          end_time: before - 50,
          severity: "alert",
          thumb_path: "/media/frigate/clips/review/alert.webp",
          data: { objects: ["person"], zones: [], audio: [] },
        },
      ],
    });
  });
  await page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] }));
  // The review outlived its recording: Frigate answers a clip request for this
  // range with a JSON 400 that a <video> element would call an unplayable file.
  await page.route("**/api/front/recordings?*", (route) => route.fulfill({ json: [] }));
  await page.route("**/api/front/start/*/end/*/clip.mp4", (route) =>
    route.fulfill({
      status: 400,
      json: { success: false, message: "No recordings found for the specified time range" },
    }),
  );
  await page.route("**/clips/review/*.webp", (route) =>
    route.fulfill({ contentType: "image/webp", body: Buffer.alloc(0) }),
  );

  await page.goto("/");
  await page.locator(".event-card").getByRole("button").click();
  const playback = page.getByRole("dialog", { name: "Selected event playback" });
  await expect(playback).toBeVisible();
  await expect(playback.getByRole("alert")).toHaveText(
    "Frigate has no retained recording for this activity.",
  );
  await expect(playback.locator("video")).toHaveCount(0);
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
  await page.route("**/api/front/recordings?*", retainRequestedRange);
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
