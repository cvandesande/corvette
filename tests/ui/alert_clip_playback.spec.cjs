const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// The alert dialog's four "no clip" outcomes: each exact message, a visible Retry, and one Retry tap that re-asks the activity route and renders a changed answer.

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
  await page.route("**/corvette/api/v1/media/**", (route) =>
    route.fulfill({ contentType: "application/vnd.apple.mpegurl", body: "" }),
  );
  return { activityRequests, fixture };
}

async function assertUnavailabilityThenRetry(page, activityRequests, fixture, message) {
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
  await expect(video).toHaveAttribute("src", fixture.playlist);
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
  await assertUnavailabilityThenRetry(page, stubbed.activityRequests, stubbed.fixture,
    "The server did not return this alert.");
});

test("a review with no end says the alert is still in progress", async ({ page }) => {
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
  await assertUnavailabilityThenRetry(page, stubbed.activityRequests, stubbed.fixture,
    "This alert is still in progress.");
  const activityUrl = new URL(stubbed.activityRequests[0]);
  expect(Number(activityUrl.searchParams.get("after"))).toBe(stubbed.fixture.bounds.start);
  const windowEnd = Number(activityUrl.searchParams.get("before"));
  expect(windowEnd).toBeGreaterThanOrEqual(testStartSeconds);
  expect(windowEnd).toBeLessThanOrEqual(Date.now() / 1000);
});

test("an activity answer of 503 says the clip could not be loaded", async ({ page }) => {
  const stubbed = await stubAlertModal(
    page,
    (route) => route.fulfill({ status: 503, json: { error: "unavailable" } }),
    { openReview: false },
  );
  await assertUnavailabilityThenRetry(page, stubbed.activityRequests, stubbed.fixture,
    "Could not load this alert's clip.");
});

test("an aborted activity request says the clip could not be loaded", async ({ page }) => {
  const stubbed = await stubAlertModal(page, (route) => route.abort(), { openReview: false });
  await assertUnavailabilityThenRetry(page, stubbed.activityRequests, stubbed.fixture,
    "Could not load this alert's clip.");
});
