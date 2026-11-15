const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);
const { refuseMalformedActivityQuery } = require("./activity_query.cjs");
const { refuseMalformedClipLinkQuery } = require("./clip_link_query.cjs");

// A headless browser cannot rotate the phone or grant a real fullscreen, so the orientation
// event, the Fullscreen API reads and the media paused state are replaced at the browser, and
// the page's own rotation wiring runs on top of them untouched.

const JPEG = Buffer.from(
  "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q==",
  "base64",
);

const FRONT_CAMERA = { name: "front", display_name: "Front", order: 0, detect_width: 1920, detect_height: 1080 };

// A relay address that refuses at once, so any MoQ attempt fails fast.
const DEAD_RELAY_URL = "https://127.0.0.1:9/anon";

const installHarness = (page) =>
  page.addInitScript(() => {
    window.__orientation = "portrait-primary";
    window.__windowOrientation = 0;
    window.__playing = true;
    window.__fullscreenMode = "resolve";
    window.__fullscreenRequests = [];
    window.__exitCalls = 0;
    let fullscreenElement = null;
    const setFullscreenElement = (element) => {
      fullscreenElement = element;
      document.dispatchEvent(new Event("fullscreenchange"));
    };
    window.__setFullscreenElement = setFullscreenElement;
    if (screen.orientation) {
      Object.defineProperty(screen.orientation, "type", { configurable: true, get: () => window.__orientation });
    }
    Object.defineProperty(document, "fullscreenElement", { configurable: true, get: () => fullscreenElement });
    Object.defineProperty(document, "fullscreenEnabled", { configurable: true, get: () => true });
    Object.defineProperty(HTMLMediaElement.prototype, "paused", {
      configurable: true,
      get: () => !window.__playing,
    });
    Element.prototype.requestFullscreen = function requestFullscreen() {
      window.__fullscreenRequests.push({
        tagName: this.tagName, className: String(this.className || ""), clipUrl: this.getAttribute("data-clip-url"),
      });
      if (window.__fullscreenMode === "throw") {
        throw new Error("refused at once");
      }
      if (window.__fullscreenMode === "reject") {
        return Promise.reject(new Error("refused"));
      }
      setFullscreenElement(this);
      return Promise.resolve();
    };
    document.exitFullscreen = function exitFullscreen() {
      window.__exitCalls += 1;
      setFullscreenElement(null);
      return Promise.resolve();
    };
  });

const setupLiveView = (page) =>
  Promise.all([
    page.addInitScript((url) => {
      window.__corvetteMoqRelayUrl = url;
    }, DEAD_RELAY_URL),
    page.route("**/corvette/api/v1/cameras", (route) => route.fulfill({ json: { cameras: [FRONT_CAMERA] } })),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] })),
    page.route("**/corvette/api/v1/cameras/*/still.jpg*", (route) =>
      route.fulfill({ contentType: "image/jpeg", body: JPEG })),
    // Never answered, so the live view settles on the bare video it holds from here on.
    page.route("**/live/hls/**", () => {}),
  ]);

const openLiveView = async (page, beforeVisit) => {
  await installHarness(page);
  if (beforeVisit) {
    await beforeVisit();
  }
  await setupLiveView(page);
  await page.goto("/");
  await page.getByRole("link", { name: /open front live view/i }).click();
  await expect(page.locator(".live-path-indicator")).toHaveAttribute("data-live-path", "hls", { timeout: 6_000 });
  await expect(page.locator(".expanded-player video")).toHaveCount(1);
  await expect(page.locator(".expanded-player video")).toBeVisible();
};

const turn = (page, orientation) =>
  page.evaluate((type) => {
    window.__orientation = type;
    screen.orientation.dispatchEvent(new Event("change"));
  }, orientation);

const recordedRequests = (page) => page.evaluate(() => window.__fullscreenRequests);

const activityAnswer = (request, reviews) => {
  const params = new URL(request.url()).searchParams;
  return {
    after: Number(params.get("after")), before: Number(params.get("before")),
    motion_bucket_seconds: 30, reviews, motion: [],
  };
};

const activityReview = (id, bounds, clipUrl) => ({
  id, camera: "front", severity: "alert", start: bounds.start, end: bounds.end,
  objects: ["person"], zones: [], detections: [], sub_labels: [], audio: [],
  media: { thumbnail: null, clip: clipUrl === null ? null : { url: clipUrl } },
});

async function stubAlertModal(page, firstActivityAnswer) {
  const fixture = { bounds: null };
  await page.route("**/corvette/api/v1/cameras", (route) => route.fulfill({ json: { cameras: [] } }));
  await page.route("**/api/review?*", (route) => {
    const before = Number(new URL(route.request().url()).searchParams.get("before"));
    fixture.bounds = { start: before - 60, end: before - 50 };
    return route.fulfill({ json: [{
      id: "alert-review", camera: "front", severity: "alert",
      start_time: fixture.bounds.start, end_time: null,
      thumb_path: "/clips/review/alert.webp",
      data: { objects: ["person"], zones: [], audio: [] },
    }] });
  });
  await page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] }));
  await page.route("**/corvette/api/v1/activity?*", (route) => {
    if (refuseMalformedActivityQuery(route)) {
      return;
    }
    return firstActivityAnswer(route, fixture);
  });
  await page.route("**/clips/review/*.webp", (route) =>
    route.fulfill({ contentType: "image/webp", body: Buffer.alloc(0) }));
  await page.route("**/corvette/api/v1/clip-link?*", (route) => {
    if (refuseMalformedClipLinkQuery(route)) {
      return;
    }
    return route.fulfill({ status: 500, json: { code: "unexpected", message: "no clip link was expected" } });
  });
  await page.route("**/corvette/api/v1/media/**", (route) => {
    if (new URL(route.request().url()).pathname.endsWith("/master.m3u8")) {
      const playlist = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nsegment.ts\n#EXT-X-ENDLIST\n";
      return route.fulfill({ contentType: "application/vnd.apple.mpegurl", body: playlist });
    }
    return route.fulfill({ status: 404 });
  });
}

const openAlertWithLink = async (page) => {
  await installHarness(page);
  let link = null;
  await stubAlertModal(page, (route, modalFixture) => {
    const start = modalFixture.bounds.start;
    link = `/corvette/api/v1/media/front/start/${start}/end/${start + 90}/master.m3u8`;
    return route.fulfill({ json: activityAnswer(route.request(), [
      activityReview("alert-review", { ...modalFixture.bounds, end: null }, link),
    ]) });
  });
  await page.goto("/");
  await page.locator(".event-card").getByRole("button").click();
  const playback = page.getByRole("dialog", { name: "Selected event playback" });
  await expect(playback).toBeVisible();
  await expect(playback.locator("video")).toHaveAttribute("data-clip-url", link);
  return link;
};

test("a sideways turn takes the playing live view fullscreen", async ({ page }) => {
  test.setTimeout(30_000);
  await openLiveView(page);
  expect(await recordedRequests(page)).toHaveLength(0);
  await turn(page, "landscape-primary");
  const requests = await recordedRequests(page);
  expect(requests).toHaveLength(1);
  expect(requests[0].className).toContain("expanded-player");
  await expect(page.locator(".rotation-fullscreen-button")).toHaveCount(0);
});

test("a refused request shows one Full screen button above the modal, and one tap asks again", async ({ page }) => {
  test.setTimeout(30_000);
  await openLiveView(page);
  await page.evaluate(() => { window.__fullscreenMode = "reject"; });
  await turn(page, "landscape-primary");
  expect(await recordedRequests(page)).toHaveLength(1);
  const button = page.locator(".rotation-fullscreen-button");
  await expect(button).toHaveCount(1);
  await expect(button).toBeVisible();
  await expect(button).toHaveText("Full screen");
  const stackedAboveModal = await page.evaluate(() => {
    const element = document.querySelector(".rotation-fullscreen-button");
    const box = element.getBoundingClientRect();
    return document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2) === element;
  });
  expect(stackedAboveModal).toBe(true);
  await page.evaluate(() => { window.__fullscreenMode = "resolve"; });
  await button.click();
  const requests = await recordedRequests(page);
  expect(requests).toHaveLength(2);
  expect(requests[1].className).toContain("expanded-player");
  await expect(button).toHaveCount(0);
});

test("a request that throws at once shows the button too", async ({ page }) => {
  test.setTimeout(30_000);
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error));
  await openLiveView(page);
  await page.evaluate(() => { window.__fullscreenMode = "throw"; });
  await turn(page, "landscape-primary");
  await expect(page.locator(".rotation-fullscreen-button")).toHaveCount(1);
  await expect(page.locator(".rotation-fullscreen-button")).toBeVisible();
  expect(await recordedRequests(page)).toHaveLength(1);
  expect(pageErrors).toHaveLength(0);
});

test("without a fullscreen request method the turn does nothing", async ({ page }) => {
  test.setTimeout(30_000);
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error));
  await openLiveView(page, () => page.addInitScript(() => {
    delete Element.prototype.requestFullscreen;
  }));
  await turn(page, "landscape-primary");
  expect(await recordedRequests(page)).toHaveLength(0);
  await expect(page.locator(".rotation-fullscreen-button")).toHaveCount(0);
  expect(pageErrors).toHaveLength(0);
});

test("without fullscreen enabled the turn does nothing", async ({ page }) => {
  test.setTimeout(30_000);
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error));
  await openLiveView(page, () => page.addInitScript(() => {
    Object.defineProperty(document, "fullscreenEnabled", { configurable: true, get: () => false });
  }));
  await turn(page, "landscape-primary");
  expect(await recordedRequests(page)).toHaveLength(0);
  await expect(page.locator(".rotation-fullscreen-button")).toHaveCount(0);
  expect(pageErrors).toHaveLength(0);
});

test("turning upright leaves the fullscreen the page entered", async ({ page }) => {
  test.setTimeout(30_000);
  await openLiveView(page);
  await turn(page, "landscape-primary");
  expect(await recordedRequests(page)).toHaveLength(1);
  await turn(page, "portrait-primary");
  expect(await page.evaluate(() => window.__exitCalls)).toBe(1);
  expect(await recordedRequests(page)).toHaveLength(1);
});

test("turning upright leaves a fullscreen the page did not enter alone", async ({ page }) => {
  test.setTimeout(30_000);
  await openLiveView(page);
  await page.evaluate(() => window.__setFullscreenElement(document.querySelector(".expanded-player")));
  await turn(page, "portrait-secondary");
  expect(await page.evaluate(() => window.__exitCalls)).toBe(0);
  expect(await recordedRequests(page)).toHaveLength(0);
});

test("a paused player gets no request", async ({ page }) => {
  const link = await openAlertWithLink(page);
  await page.evaluate(() => { window.__playing = false; });
  await turn(page, "landscape-primary");
  expect(await recordedRequests(page)).toHaveLength(0);
  await page.evaluate(() => { window.__playing = true; });
  await turn(page, "portrait-primary");
  await turn(page, "landscape-primary");
  const requests = await recordedRequests(page);
  expect(requests).toHaveLength(1);
  expect(requests[0].tagName).toBe("VIDEO");
  expect(requests[0].clipUrl).toBe(link);
});

test("only a marked player is asked", async ({ page }) => {
  await openAlertWithLink(page);
  await turn(page, "landscape-primary");
  const firstRequests = await recordedRequests(page);
  expect(firstRequests).toHaveLength(1);
  expect(firstRequests[0].tagName).toBe("VIDEO");
  await turn(page, "portrait-primary");
  await page.evaluate(() => document.querySelector("video.recording-player").removeAttribute("data-rotate-fullscreen"));
  await turn(page, "landscape-primary");
  expect(await recordedRequests(page)).toHaveLength(1);
});

test("without screen.orientation the window's own event drives the page", async ({ page }) => {
  test.setTimeout(30_000);
  await openLiveView(page, () => page.addInitScript(() => {
    Object.defineProperty(screen, "orientation", { configurable: true, get: () => undefined });
    Object.defineProperty(window, "orientation", { configurable: true, get: () => window.__windowOrientation });
  }));
  await page.evaluate(() => {
    window.__windowOrientation = 90;
    window.dispatchEvent(new Event("orientationchange"));
  });
  const requests = await recordedRequests(page);
  expect(requests).toHaveLength(1);
  expect(requests[0].className).toContain("expanded-player");
});
