const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// The recordings page offers Corvette's stitched download only after a head-only probe of the download route answers 200 application/mp4; a next-start header chains further parts, every other answer shows no control and no error text.

const cameraAnswer = {
  cameras: [
    {
      name: "front",
      display_name: "Front",
      order: 1,
      detect_width: 640,
      detect_height: 480,
    },
  ],
};

async function stubRecordingsPage(page, download) {
  const probeRequests = [];
  const probeMethods = [];
  await page.route("**/corvette/api/v1/cameras", (route) =>
    route.fulfill({ json: cameraAnswer }),
  );
  await page.route("**/api/events?*", (route) => route.fulfill({ json: [] }));
  await page.route("**/api/review?*", (route) => route.fulfill({ json: [] }));
  await page.route("**/api/recordings/summary?*", (route) => route.fulfill({ json: {} }));
  await page.route("**/api/front/recordings?*", (route) => {
    const params = new URL(route.request().url()).searchParams;
    return route.fulfill({
      json: [
        {
          start_time: Number(params.get("after")),
          end_time: Number(params.get("before")),
          motion: 0,
        },
      ],
    });
  });
  await page.route("**/api/preview/**", (route) => route.fulfill({ json: [] }));
  await page.route("**/api/*/recordings/*/snapshot.jpg*", (route) =>
    route.fulfill({ contentType: "image/jpeg", body: Buffer.alloc(0) }),
  );
  await page.route("**/api/*/start/*/end/*/clip.mp4", (route) =>
    route.fulfill({ contentType: "video/mp4", body: Buffer.alloc(0) }),
  );
  await page.route("**/corvette/api/v1/media/**", (route) => {
    probeRequests.push(route.request().url());
    probeMethods.push(route.request().method());
    return download(route, route.request().url(), probeRequests.length);
  });
  return { probeRequests, probeMethods };
}

async function selectFrontAndLoadLastTenMinutes(page) {
  await page.goto("/recordings");
  await page.selectOption("select", "front");
  await page.getByRole("button", { name: "Last 10 minutes" }).click();
}

const offerGroup = (page) => page.getByRole("group", { name: "Download the recording" });

const offerLinks = (page) => offerGroup(page).getByRole("link");

const mp4Answer = (route, headers = {}) =>
  route.fulfill({
    contentType: "application/mp4",
    headers: { "Cache-Control": "no-store", ...headers },
    body: Buffer.alloc(8),
  });

const startOf = (url) => url.split("/start/")[1].split("/end/")[0];

test("an mp4 head offers one download control with the probe's own address", async ({ page }) => {
  const stubbed = await stubRecordingsPage(page, (route) =>
    route.fulfill({ contentType: "application/mp4", body: Buffer.alloc(8) }),
  );
  await selectFrontAndLoadLastTenMinutes(page);
  const control = page.getByRole("link", { name: "Download video" });
  await expect(control).toBeVisible();
  await expect(offerLinks(page)).toHaveCount(1);
  await expect(control).toHaveAttribute("download", "");
  await expect.poll(() => stubbed.probeRequests.length).toBe(1);
  await expect(control).toHaveAttribute(
    "href",
    new URL(stubbed.probeRequests[0]).pathname,
  );
  expect(new URL(stubbed.probeRequests[0]).pathname).toMatch(
    /^\/corvette\/api\/v1\/media\/front\/start\/\d+\/end\/\d+\/download\.mp4$/,
  );
});

test("a next-start header chains a second part with its own start and the same end", async ({ page }) => {
  const stubbed = await stubRecordingsPage(page, (route, url) => {
    if (url.includes("/start/1008.500/")) {
      return route.fulfill({ contentType: "application/mp4", body: Buffer.alloc(8) });
    }
    return route.fulfill({
      contentType: "application/mp4",
      headers: { "X-Corvette-Next-Start": "1008.500" },
      body: Buffer.alloc(8),
    });
  });
  await selectFrontAndLoadLastTenMinutes(page);
  await expect(offerLinks(page)).toHaveCount(2);
  await expect(page.getByRole("link", { name: "Download video" })).toBeVisible();
  const second = page.getByRole("link", { name: "Download part 2" });
  await expect(second).toBeVisible();
  const secondHref = await second.getAttribute("href");
  expect(secondHref).toMatch(
    /^\/corvette\/api\/v1\/media\/front\/start\/1008\.500\/end\/\d+\/download\.mp4$/,
  );
  const firstHref = await page
    .getByRole("link", { name: "Download video" })
    .getAttribute("href");
  const endOf = (href) => href.split("/end/")[1].split("/download.mp4")[0];
  expect(endOf(secondHref)).toBe(endOf(firstHref));
  await expect(
    page.getByText("The recording changed camera settings partway, so it comes in parts."),
  ).toBeVisible();
  expect(stubbed.probeMethods).toEqual(["GET", "GET"]);
});

test("a four-oh-four head offers nothing and says nothing", async ({ page }) => {
  await stubRecordingsPage(page, (route) => route.fulfill({ status: 404 }));
  await selectFrontAndLoadLastTenMinutes(page);
  await expect(offerGroup(page)).toHaveCount(0);
  await expect(page.getByText(/download/i)).toHaveCount(0);
});

test("a two-hundred html head offers nothing and says nothing", async ({ page }) => {
  await stubRecordingsPage(page, (route) =>
    route.fulfill({ contentType: "text/html", body: "<!doctype html><html><body>index</body></html>" }),
  );
  await selectFrontAndLoadLastTenMinutes(page);
  await expect(offerGroup(page)).toHaveCount(0);
  await expect(page.getByText(/download/i)).toHaveCount(0);
});

test("a stale answer from a slow first probe never shows after the selection moved on", async ({ page }) => {
  let held = null;
  const stubbed = await stubRecordingsPage(page, (route) => {
    if (held === null) {
      held = route;
      return;
    }
    return route.fulfill({ contentType: "application/mp4", body: Buffer.alloc(8) });
  });
  await selectFrontAndLoadLastTenMinutes(page);
  await expect.poll(() => stubbed.probeRequests.length).toBe(1);
  const stalePathname = new URL(stubbed.probeRequests[0]).pathname;
  await page.getByRole("button", { name: "Last hour" }).click();
  await held.fulfill({
    contentType: "application/mp4",
    headers: { "X-Corvette-Next-Start": "9.000" },
    body: Buffer.alloc(8),
  });
  await expect(offerLinks(page)).toHaveCount(1);
  await expect
    .poll(async () => (await offerLinks(page).first().getAttribute("href")) ?? "")
    .toContain("/start/");
  const hrefs = await offerLinks(page).evaluateAll((links) =>
    links.map((link) => link.getAttribute("href")),
  );
  expect(hrefs).not.toContain(stalePathname);
  expect(hrefs[0]).not.toBe(stalePathname);
});

test("one head-only GET is sent per offered part", async ({ page }) => {
  const stubbed = await stubRecordingsPage(page, (route, url) => {
    if (url.includes("/start/1008.500/")) {
      return route.fulfill({ contentType: "application/mp4", body: Buffer.alloc(8) });
    }
    return route.fulfill({
      contentType: "application/mp4",
      headers: { "X-Corvette-Next-Start": "1008.500" },
      body: Buffer.alloc(8),
    });
  });
  await selectFrontAndLoadLastTenMinutes(page);
  await expect(offerLinks(page)).toHaveCount(2);
  await expect.poll(() => stubbed.probeRequests.length).toBe(2);
  expect(stubbed.probeMethods).toEqual(["GET", "GET"]);
  expect(new Set(stubbed.probeRequests).size).toBe(2);
});

test("a next-start chain stops at eight offered parts", async ({ page }) => {
  const stubbed = await stubRecordingsPage(page, (route, url) =>
    mp4Answer(route, {
      "X-Corvette-Next-Start": (Number(startOf(url)) + 1).toFixed(3),
    }),
  );
  await selectFrontAndLoadLastTenMinutes(page);
  await expect(offerLinks(page)).toHaveCount(8);
  await expect.poll(() => new Set(stubbed.probeRequests).size).toBe(8);
  expect(stubbed.probeRequests.length).toBe(8);
  await expect(page.getByRole("link", { name: "Download part 8" })).toBeVisible();
});
