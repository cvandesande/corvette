const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// styles.css harmonizes every element's keyboard focus indicator to this one
// outline (see the pre-existing `.camera-card-link:focus-visible` and its
// siblings). This spec proves each control that did not yet use it -- some
// showed no outline at all, others a mismatched color or width -- now does,
// reached by a real keyboard Tab rather than read off the stylesheet source.
const expectHarmonizedOutline = async (locator) => {
  await expect(locator).toHaveCSS("outline-color", "rgb(105, 209, 186)"); // #69d1ba
  await expect(locator).toHaveCSS("outline-style", "solid");
  await expect(locator).toHaveCSS("outline-width", "2px");
  await expect(locator).toHaveCSS("outline-offset", "-2px");
};

// Presses Tab from `anchor` until `target` is the focused element, asserting
// it lands there within `maxTabs` presses. Focusing `anchor` first (a plain
// DOM `.focus()`, not a click) only sets where Tab starts from; the target
// itself always gains focus through a genuine keyboard Tab key event, which
// is what makes the browser mark it `:focus-visible`.
const tabToFocus = async (page, anchor, target, maxTabs = 1) => {
  await anchor.focus();
  for (let pressed = 0; pressed < maxTabs; pressed += 1) {
    await page.keyboard.press("Tab");
    if (await target.evaluate((element) => element === document.activeElement)) {
      await expect(target).toBeFocused();
      return;
    }
  }
  throw new Error(`Tab did not reach the target within ${maxTabs} press(es)`);
};

const recentRecordingDays = (count) =>
  Object.fromEntries(
    Array.from({ length: count }, (_, offset) => {
      const date = new Date();
      date.setDate(date.getDate() - offset);
      return [date.toISOString().slice(0, 10), true];
    }),
  );

test.describe("site chrome", () => {
  test("primary navigation link shows the harmonized outline on keyboard focus", async ({
    page,
  }) => {
    await page.goto("/");
    const wordmark = page.getByRole("link", { name: "Corvette home" });
    const firstNavLink = page.getByRole("link", { name: "Live" });

    await tabToFocus(page, wordmark, firstNavLink);
    await expectHarmonizedOutline(firstNavLink);
  });
});

test.describe("recent-activity filters", () => {
  test("activity filter button shows the harmonized outline on keyboard focus", async ({
    page,
  }) => {
    await page.goto("/");
    const lastNavLink = page.getByRole("link", { name: "Monitor" });
    const allFilter = page.getByRole("button", { name: "All", exact: true });

    await tabToFocus(page, lastNavLink, allFilter);
    await expectHarmonizedOutline(allFilter);
  });

  test("compact event list's show-more toggle shows the harmonized outline on keyboard focus", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    const reviews = Array.from({ length: 5 }, (_, index) => ({
      id: `review-${index}`,
      camera: "front",
      start_time: 1_700_000_000 - index * 60,
      end_time: 1_700_000_000 - index * 60 + 10,
      severity: "alert",
      thumb_path: "/media/frigate/clips/review/thumb.webp",
      data: {},
    }));
    await page.route("**/api/review?*", (route) => route.fulfill({ json: reviews }));
    await page.route("**/api/review/activity/motion?*", (route) =>
      route.fulfill({ json: [] }),
    );

    await page.goto("/");
    const lastVisibleEventCard = page.locator(".event-card-link:visible").last();
    const showMoreToggle = page.locator(".event-toggle");
    await expect(showMoreToggle).toBeVisible();

    await tabToFocus(page, lastVisibleEventCard, showMoreToggle);
    await expectHarmonizedOutline(showMoreToggle);
  });
});

test.describe("recording browser controls", () => {
  test.beforeEach(async ({ page }) => {
    await page.route("**/api/config", (route) =>
      route.fulfill({
        json: {
          cameras: {
            front: {
              enabled: true,
              friendly_name: "Front",
              ui: { order: 0 },
              detect: { width: 1920, height: 1080 },
            },
          },
        },
      }),
    );
    await page.route("**/api/events?*", (route) => route.fulfill({ json: [] }));
    await page.route("**/api/review?*", (route) => route.fulfill({ json: [] }));
    await page.route("**/api/review/activity/motion?*", (route) =>
      route.fulfill({ json: [] }),
    );
    await page.route("**/api/recordings/summary?*", (route) =>
      route.fulfill({ json: recentRecordingDays(21) }),
    );

    await page.goto("/recordings");
    // The calendar's day buttons render disabled until `recording_days`
    // resolves; waiting for the first one to enable avoids a race between
    // that fetch and the Tab presses below.
    await expect(page.locator(".calendar-grid button").first()).toBeEnabled();
  });

  test("recording preset button shows the harmonized outline on keyboard focus", async ({
    page,
  }) => {
    const lastNavLink = page.getByRole("link", { name: "Monitor" });
    const todayPreset = page.getByRole("button", { name: "Today", exact: true });

    await tabToFocus(page, lastNavLink, todayPreset);
    await expectHarmonizedOutline(todayPreset);
  });

  test("recording calendar's camera select shows the harmonized outline on keyboard focus", async ({
    page,
  }) => {
    const lastPreset = page.getByRole("button", { name: "Last 7 days", exact: true });
    const cameraSelect = page.locator(".recording-calendar select");

    await tabToFocus(page, lastPreset, cameraSelect);
    await expectHarmonizedOutline(cameraSelect);
  });

  test("recording time input shows the harmonized outline on keyboard focus", async ({
    page,
  }) => {
    const cameraSelect = page.locator(".recording-calendar select");
    const fromInput = page.getByLabel("From", { exact: true });

    await tabToFocus(page, cameraSelect, fromInput);
    await expectHarmonizedOutline(fromInput);
  });

  test("calendar day button shows the harmonized outline on keyboard focus", async ({
    page,
  }) => {
    const toInput = page.getByLabel("To", { exact: true });
    const firstDay = page.locator(".calendar-grid button").first();

    // A time input's hour/minute/period segments are each their own Tab
    // stop before Tab leaves the control, so reaching the next element
    // takes more than one press.
    await tabToFocus(page, toInput, firstDay, 5);
    await expectHarmonizedOutline(firstDay);
  });
});

test.describe("recording timeline", () => {
  // One camera with a retained motion span and two reviews (one an
  // instantaneous point, one a range), mirroring the fixture the timeline's
  // own rendering tests already use.
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
            front: {
              enabled: true,
              friendly_name: "Front",
              ui: { order: 0 },
              detect: { width: 1920, height: 1080 },
            },
          },
        },
      }),
    );
    await page.route("**/api/events?*", (route) => route.fulfill({ json: [] }));
    await page.route("**/api/review?*", (route) =>
      route.fulfill({
        json: [
          { camera: "front", start_time: now - 1800, end_time: now - 1800, severity: "detection" },
          { camera: "front", start_time: now - 200, end_time: now - 190, severity: "alert" },
        ],
      }),
    );
    await page.route("**/api/recordings/summary?*", (route) =>
      route.fulfill({ json: recentRecordingDays(21) }),
    );
    await page.route("**/api/front/recordings?*", (route) =>
      route.fulfill({
        json: [{ start_time: now - 510, end_time: now - 480, motion: 1 }],
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
    await expect(page.locator(".calendar-grid button").first()).toBeEnabled();
    // Recordings default to the all-cameras view (issue #22, D-5); select the
    // single camera these tests target so "Last hour" lands on the
    // single-camera RecordingTimeline rather than AllCamerasPlayback.
    await page.getByRole("combobox", { name: "Camera" }).selectOption("front");
    await page.getByRole("button", { name: "Last hour" }).click();
    await expect(page.getByRole("region", { name: "Recording timeline" })).toBeVisible();
  });

  test("timeline availability span shows the harmonized outline on keyboard focus", async ({
    page,
  }) => {
    const lastDay = page.locator(".calendar-grid button").last();
    const firstAvailability = page.locator(".timeline-availability").first();

    await tabToFocus(page, lastDay, firstAvailability);
    await expectHarmonizedOutline(firstAvailability);
  });

  test("timeline activity span shows the harmonized outline on keyboard focus", async ({
    page,
  }) => {
    const detectionPoint = page.locator(
      ".timeline-activity.activity-detection.timeline-point",
    );
    const alertSpan = page.locator(".timeline-activity.activity-alert");

    await tabToFocus(page, detectionPoint, alertSpan);
    await expectHarmonizedOutline(alertSpan);
  });

  test("timeline player control button shows the harmonized outline on keyboard focus", async ({
    page,
  }) => {
    const playhead = page.getByRole("slider", { name: "Recording playhead" });
    const playButton = page.locator(".timeline-player-controls button");

    await tabToFocus(page, playhead, playButton);
    await expectHarmonizedOutline(playButton);
  });
});

test.describe("selected-event playback", () => {
  test("recording playback's close button shows the harmonized outline on keyboard focus", async ({
    page,
  }) => {
    await page.route("**/api/config", (route) =>
      route.fulfill({
        json: {
          cameras: {
            front: {
              enabled: true,
              friendly_name: "Front",
              ui: { order: 0 },
              detect: { width: 1920, height: 1080 },
            },
          },
        },
      }),
    );
    await page.route("**/api/events?*", (route) =>
      route.fulfill({
        json: [
          {
            id: "event-1",
            camera: "front",
            label: "person",
            sub_label: null,
            start_time: 1_700_000_000,
            end_time: 1_700_000_010,
            zones: [],
            has_clip: true,
          },
        ],
      }),
    );
    await page.route("**/api/review?*", (route) => route.fulfill({ json: [] }));
    await page.route("**/api/review/activity/motion?*", (route) =>
      route.fulfill({ json: [] }),
    );
    await page.route("**/api/recordings/summary?*", (route) =>
      route.fulfill({ json: recentRecordingDays(21) }),
    );

    await page.goto("/recordings?event=event-1");
    await expect(page.locator(".calendar-grid button").first()).toBeEnabled();
    const lastDay = page.locator(".calendar-grid button").last();
    const closeButton = page.getByRole("button", { name: "Close", exact: true });

    await tabToFocus(page, lastDay, closeButton);
    await expectHarmonizedOutline(closeButton);
  });
});
