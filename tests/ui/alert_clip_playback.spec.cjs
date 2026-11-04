const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);
const { refuseMalformedActivityQuery } = require("./activity_query.cjs");

// The alert dialog's "no clip" outcomes, an alert in progress that plays its link, and the player's failure state: each exact message, a visible Retry, and one Retry tap that re-asks the activity route or rebuilds the player.

const activityAnswer = (request, reviews) => {
  const params = new URL(request.url()).searchParams;
  return {
    after: Number(params.get("after")),
    before: Number(params.get("before")),
    motion_bucket_seconds: 30,
    reviews,
    motion: [],
  };
};

const activityReview = (id, bounds, clipUrl) => ({
  id,
  camera: "front",
  severity: "alert",
  start: bounds.start,
  end: bounds.end,
  objects: ["person"],
  zones: [],
  detections: [],
  sub_labels: [],
  audio: [],
  media: { thumbnail: null, clip: clipUrl === null ? null : { url: clipUrl } },
});

async function stubAlertModal(page, firstActivityAnswer, card) {
  const activityRequests = [];
  const playlistRequests = [];
  const media = { failing: false };
  const fixture = { bounds: null, playlist: null };
  await page.route("**/corvette/api/v1/cameras", (route) =>
    route.fulfill({ json: { cameras: [] } }),
  );
  await page.route("**/api/review?*", (route) => {
    const before = Number(new URL(route.request().url()).searchParams.get("before"));
    fixture.bounds = { start: before - 60, end: before - 50 };
    fixture.playlist =
      `/corvette/api/v1/media/front/start/${before - 60}/end/${before - 50}/master.m3u8`;
    return route.fulfill({
      json: [
        {
          id: "alert-review",
          camera: "front",
          severity: "alert",
          start_time: fixture.bounds.start,
          end_time: card.openReview ? null : fixture.bounds.end,
          thumb_path: "/clips/review/alert.webp",
          data: { objects: ["person"], zones: [], audio: [] },
        },
      ],
    });
  });
  await page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] }));
  await page.route("**/corvette/api/v1/activity?*", (route) => {
    activityRequests.push(route.request().url());
    if (refuseMalformedActivityQuery(route)) {
      return;
    }
    if (activityRequests.length === 1) {
      return firstActivityAnswer(route, fixture);
    }
    return route.fulfill({
      json: activityAnswer(route.request(), [
        activityReview("alert-review", fixture.bounds, fixture.playlist),
      ]),
    });
  });
  await page.route("**/clips/review/*.webp", (route) =>
    route.fulfill({ contentType: "image/webp", body: Buffer.alloc(0) }),
  );
  await page.route("**/corvette/api/v1/media/**", (route) => {
    const requestUrl = route.request().url();
    if (requestUrl.endsWith("/master.m3u8")) {
      playlistRequests.push(requestUrl);
      if (media.failing) {
        return route.fulfill({ status: 404 });
      }
      return route.fulfill({
        contentType: "application/vnd.apple.mpegurl",
        body: "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nsegment.ts\n#EXT-X-ENDLIST\n",
      });
    }
    return route.fulfill({ status: 404 });
  });
  return { activityRequests, fixture, media, playlistRequests };
}

async function assertUnavailabilityThenRetry(page, activityRequests, fixture, playlistRequests, message) {
  await page.goto("/");
  await page.locator(".event-card").getByRole("button").click();
  const playback = page.getByRole("dialog", { name: "Selected event playback" });
  await expect(playback).toBeVisible();
  await expect(playback.getByRole("alert")).toHaveText(message);
  const retry = playback.getByRole("button", { name: "Retry" });
  await expect(retry).toBeVisible();
  const video = playback.locator("video");
  await expect(video).toHaveCount(0);
  await expect.poll(() => activityRequests.length).toBe(1);
  const activityUrl = new URL(activityRequests[0]);
  expect(activityUrl.pathname).toBe("/corvette/api/v1/activity");
  expect(activityUrl.searchParams.get("cameras")).toBe("front");
  expect(activityUrl.searchParams.get("include")).toBe("reviews");
  await retry.click();
  await expect.poll(() => activityRequests.length).toBe(2);
  await expect(video).toHaveAttribute("data-clip-url", fixture.playlist);
  await expect.poll(() =>
    playlistRequests.some((request) => {
      const recorded = new URL(request);
      return `${recorded.pathname}${recorded.search}` === fixture.playlist;
    }),
  ).toBe(true);
  await expect(retry).toHaveCount(0);
  await expect(playback.getByText(message)).toHaveCount(0);
}

test("an activity answer without the review says the server did not return the alert", async ({ page }) => {
  const stubbed = await stubAlertModal(
    page,
    (route, fixture) =>
      route.fulfill({
        json: activityAnswer(route.request(), [
          activityReview("another-review", fixture.bounds, fixture.playlist),
        ]),
      }),
    { openReview: false },
  );
  await assertUnavailabilityThenRetry(page, stubbed.activityRequests, stubbed.fixture, stubbed.playlistRequests,
    "The server did not return this alert.");
});

test("an alert in progress with no recording yet says so and offers Retry", async ({ page }) => {
  const testStartSeconds = Date.now() / 1000;
  const stubbed = await stubAlertModal(
    page,
    (route, fixture) =>
      route.fulfill({
        json: activityAnswer(route.request(), [
          activityReview("alert-review", { ...fixture.bounds, end: null }, null),
        ]),
      }),
    { openReview: true },
  );
  await assertUnavailabilityThenRetry(page, stubbed.activityRequests, stubbed.fixture, stubbed.playlistRequests,
    "This alert is still in progress and has no recording to play yet.");
  const activityUrl = new URL(stubbed.activityRequests[0]);
  expect(Number(activityUrl.searchParams.get("after"))).toBe(stubbed.fixture.bounds.start);
  const windowEnd = Number(activityUrl.searchParams.get("before"));
  expect(windowEnd).toBeGreaterThanOrEqual(testStartSeconds);
  expect(windowEnd).toBeLessThanOrEqual(Date.now() / 1000);
});

test("an alert still in progress with a link plays that link", async ({ page }) => {
  let link = null;
  const stubbed = await stubAlertModal(
    page,
    (route, fixture) => {
      link =
        `/corvette/api/v1/media/front/start/${fixture.bounds.start}/end/${fixture.bounds.start + 90}/master.m3u8`;
      return route.fulfill({
        json: activityAnswer(route.request(), [
          activityReview("alert-review", { ...fixture.bounds, end: null }, link),
        ]),
      });
    },
    { openReview: true },
  );
  await page.goto("/");
  await page.locator(".event-card").getByRole("button").click();
  const playback = page.getByRole("dialog", { name: "Selected event playback" });
  await expect(playback).toBeVisible();
  const video = playback.locator("video");
  await expect(video).toHaveAttribute("data-clip-url", link);
  await expect.poll(() =>
    stubbed.playlistRequests.some((request) => {
      const recorded = new URL(request);
      return `${recorded.pathname}${recorded.search}` === link;
    }),
  ).toBe(true);
  await expect(playback.getByRole("alert")).toHaveCount(0);
  await expect(playback.getByRole("button", { name: "Retry" })).toHaveCount(0);
  expect(stubbed.activityRequests.length).toBe(1);
});

test("an activity answer of 503 says the clip could not be loaded", async ({ page }) => {
  const stubbed = await stubAlertModal(
    page,
    (route) => route.fulfill({ status: 503, json: { error: "unavailable" } }),
    { openReview: false },
  );
  await assertUnavailabilityThenRetry(page, stubbed.activityRequests, stubbed.fixture, stubbed.playlistRequests,
    "Could not load this alert's clip.");
});

test("an aborted activity request says the clip could not be loaded", async ({ page }) => {
  const stubbed = await stubAlertModal(page, (route) => route.abort(), { openReview: false });
  await assertUnavailabilityThenRetry(page, stubbed.activityRequests, stubbed.fixture, stubbed.playlistRequests,
    "Could not load this alert's clip.");
});

test("a clip the player cannot play says so, and Retry builds a new player for the same link", async ({ page }) => {
  const stubbed = await stubAlertModal(
    page,
    (route, fixture) =>
      route.fulfill({
        json: activityAnswer(route.request(), [
          activityReview("alert-review", fixture.bounds, fixture.playlist),
        ]),
      }),
    { openReview: false },
  );
  stubbed.media.failing = true;
  await page.goto("/");
  await page.locator(".event-card").getByRole("button").click();
  const playback = page.getByRole("dialog", { name: "Selected event playback" });
  const failure = playback.getByText("This clip did not play in this browser.");
  await expect(failure).toBeVisible();
  const retry = playback.getByRole("button", { name: "Retry" });
  await expect(retry).toBeVisible();
  const video = playback.locator("video");
  await expect(video).toHaveAttribute("data-clip-url", stubbed.fixture.playlist);
  await expect(playback.getByRole("alert")).toHaveText("This clip did not play in this browser.");
  const requestsBeforeRetry = stubbed.playlistRequests.length;
  await video.evaluate((element) => {
    element.firstPlayer = true;
  });
  stubbed.media.failing = false;
  await retry.click();
  await expect(failure).toHaveCount(0);
  await expect(retry).toHaveCount(0);
  await expect.poll(() => stubbed.playlistRequests.length).toBeGreaterThan(requestsBeforeRetry);
  await expect.poll(() => video.evaluate((element) => element.firstPlayer === true)).toBe(false);
  expect(stubbed.activityRequests.length).toBe(1);
});

test("a browser with no native HLS shows the same failure after hls.js fails", async ({ page }) => {
  const stubbed = await stubAlertModal(
    page,
    (route, fixture) =>
      route.fulfill({
        json: activityAnswer(route.request(), [
          activityReview("alert-review", fixture.bounds, fixture.playlist),
        ]),
      }),
    { openReview: false },
  );
  stubbed.media.failing = true;
  // Models a browser that cannot play HLS itself; the code checks the capability, never a browser name.
  await page.addInitScript(() => {
    HTMLMediaElement.prototype.canPlayType = () => "";
  });
  await page.goto("/");
  await page.locator(".event-card").getByRole("button").click();
  const playback = page.getByRole("dialog", { name: "Selected event playback" });
  await expect(playback.getByText("This clip did not play in this browser.")).toBeVisible();
  await expect(playback.getByRole("button", { name: "Retry" })).toBeVisible();
  expect(stubbed.playlistRequests.length).toBe(1);
});
