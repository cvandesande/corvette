const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Pins the dashboard's one camera-status poll and its three readers: the
// health line above the camera grid, the state line on camera tiles, and the
// System section's row per camera. The poll runs every 30 seconds, the line
// turns amber for a camera that has had no picture for two minutes, clears
// only after two healthy polls, and admits an unreachable server.

const JPEG = Buffer.from(
  "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q==",
  "base64",
);

const FRONT = { name: "front", display_name: "Front", order: 0, detect_width: 1920, detect_height: 1080 };
const GARAGE = { name: "garage", display_name: "Garage", order: 1, detect_width: 1280, detect_height: 720 };
const LIVING = { name: "living", display_name: "Living", order: 2, detect_width: 1280, detect_height: 720, enabled: false };

const START = Date.UTC(2026, 10, 5, 10, 13, 0);
const START_SECONDS = START / 1000;

const stubDashboardApis = (page, cameras) =>
  Promise.all([
    page.route("**/corvette/api/v1/cameras", (route) => route.fulfill({ json: { cameras } })),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] })),
    page.route("**/corvette/api/v1/cameras/*/still.jpg*", (route) =>
      route.fulfill({ contentType: "image/jpeg", body: JPEG }),
    ),
  ]);

const stubStatus = (page, answer) =>
  page.route("**/corvette/api/v1/cameras/status", (route) => {
    const body = answer();
    if (body === null) {
      return route.abort();
    }
    return route.fulfill({ json: body });
  });

// The expected clock text comes from the page's own local getters at the fake
// clock's instant, never a hard-coded string.
const localClock = (page, unixSeconds) =>
  page.evaluate((seconds) => {
    const date = new Date(seconds * 1000);
    const pad = (value) => String(value).padStart(2, "0");
    return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
  }, unixSeconds);

const healthGroup = (page) => page.locator("#live .health-line-group");

const statusAnswer = (entries) => ({ cameras: entries });
const live = (name, since = START_SECONDS) => ({ name, state: "live", since });

// A fastForward only proves the timer fired. Counting finished status polls
// proves the answer reached the page, and one drained microtask proves the app
// acted on it, so the next assertion never judges a DOM the next poll is
// about to change.
const trackStatusPolls = (page) => {
  const polls = { finished: 0 };
  const isStatus = (request) =>
    new URL(request.url()).pathname === "/corvette/api/v1/cameras/status";
  page.on("requestfinished", (request) => {
    if (isStatus(request)) {
      polls.finished += 1;
    }
  });
  return polls;
};

const settledPolls = async (page, polls, expected) => {
  await expect.poll(() => polls.finished).toBe(expected);
  await page.evaluate(() => Promise.resolve());
};

test("the healthy line reads that all cameras that are on are recording", async ({ page }) => {
  await page.clock.install({ time: new Date(START) });
  await stubDashboardApis(page, [FRONT, GARAGE]);
  await stubStatus(page, () => statusAnswer([live("front"), live("garage")]));

  await page.goto("/");

  const group = healthGroup(page);
  const updated = await localClock(page, START_SECONDS);
  await expect(group.locator(".health-line-ok")).toHaveText(
    `All cameras that are on are recording. Updated ${updated}.`,
  );
  await expect(group.locator(".health-line-amber")).toHaveCount(0);
  await expect(group.locator(".health-line-offline")).toHaveCount(0);
});

test("a fault of two minutes shows an amber line naming the camera and the time it began", async ({
  page,
}) => {
  await page.clock.install({ time: new Date(START) });
  const since = START_SECONDS - 130;
  await stubDashboardApis(page, [FRONT, GARAGE]);
  await stubStatus(page, () =>
    statusAnswer([{ name: "front", state: "stale", since }, live("garage")]),
  );

  await page.goto("/");

  const group = healthGroup(page);
  const began = await localClock(page, since);
  await expect(group.locator(".health-line-amber")).toHaveText(
    `Front has had no picture since ${began}. Its live view and new recordings are paused.`,
  );
  await expect(group.locator(".health-line-ok")).toHaveCount(0);
});

test("a fault younger than two minutes shows no amber line, and one that recovers never does", async ({
  page,
}) => {
  await page.clock.install({ time: new Date(START) });
  let current = statusAnswer([
    { name: "front", state: "stale", since: START_SECONDS - 30 },
    live("garage"),
  ]);
  const polls = trackStatusPolls(page);
  await stubDashboardApis(page, [FRONT, GARAGE]);
  await stubStatus(page, () => current);

  await page.goto("/");

  const group = healthGroup(page);
  await settledPolls(page, polls, 1);
  await expect(group.locator(".health-line-ok")).toBeVisible();
  await expect(group.locator(".health-line-amber")).toHaveCount(0);

  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 2);
  await expect(group.locator(".health-line-amber")).toHaveCount(0);
  await expect(group.locator(".health-line-ok")).toBeVisible();

  current = statusAnswer([live("front"), live("garage")]);
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 3);
  await expect(group.locator(".health-line-amber")).toHaveCount(0);
  await expect(group.locator(".health-line-ok")).toBeVisible();

  // The same `since` kept `stale` until the fault is 120 seconds old: the
  // first two polls under the limit show no amber, the third does.
  const since = START_SECONDS + 30;
  current = statusAnswer([{ name: "front", state: "stale", since }, live("garage")]);
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 4);
  await expect(group.locator(".health-line-amber")).toHaveCount(0);
  await expect(group.locator(".health-line-ok")).toBeVisible();
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 5);
  await expect(group.locator(".health-line-amber")).toHaveCount(0);
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 6);
  const began = await localClock(page, since);
  await expect(group.locator(".health-line-amber")).toHaveText(
    `Front has had no picture since ${began}. Its live view and new recordings are paused.`,
  );
});

test("an amber line clears only after two healthy polls in a row", async ({ page }) => {
  await page.clock.install({ time: new Date(START) });
  const since = START_SECONDS - 130;
  let current = statusAnswer([{ name: "front", state: "stale", since }, live("garage")]);
  const polls = trackStatusPolls(page);
  await stubDashboardApis(page, [FRONT, GARAGE]);
  await stubStatus(page, () => current);

  await page.goto("/");

  const group = healthGroup(page);
  await settledPolls(page, polls, 1);
  await expect(group.locator(".health-line-amber")).toHaveCount(1);

  current = statusAnswer([live("front"), live("garage")]);
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 2);
  await expect(group.locator(".health-line-amber")).toHaveCount(1);
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 3);
  await expect(group.locator(".health-line-amber")).toHaveCount(0);
  await expect(group.locator(".health-line-ok")).toBeVisible();

  // A fault returning between the two healthy polls needs two fresh ones.
  current = statusAnswer([{ name: "front", state: "stale", since }, live("garage")]);
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 4);
  await expect(group.locator(".health-line-amber")).toHaveCount(1);
  current = statusAnswer([live("front"), live("garage")]);
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 5);
  await expect(group.locator(".health-line-amber")).toHaveCount(1);
  current = statusAnswer([{ name: "front", state: "stale", since }, live("garage")]);
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 6);
  await expect(group.locator(".health-line-amber")).toHaveCount(1);
  current = statusAnswer([live("front"), live("garage")]);
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 7);
  await expect(group.locator(".health-line-amber")).toHaveCount(1);
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 8);
  await expect(group.locator(".health-line-amber")).toHaveCount(0);
  await expect(group.locator(".health-line-ok")).toBeVisible();
});

test("an off camera never produces an amber line", async ({ page }) => {
  await page.clock.install({ time: new Date(START) });
  const longOff = START_SECONDS - 86_400;
  await stubDashboardApis(page, [FRONT, GARAGE, LIVING]);
  await stubStatus(page, () =>
    statusAnswer([
      live("front"),
      live("garage"),
      { name: "living", state: "off_by_choice", since: longOff },
    ]),
  );

  await page.goto("/");

  const group = healthGroup(page);
  await expect(group.locator(".health-line-ok")).toBeVisible();
  await expect(group.locator(".health-line-amber")).toHaveCount(0);
});

test("a failed poll shows the offline line and retries at 5, 10, 20, 40, then every 60 seconds", async ({
  page,
}) => {
  await page.clock.install({ time: new Date(START) });
  // Poll arrival times cannot tell the retry ladder from a constant short
  // delay: each answer lands a moment after its fastForward window, so every
  // retry — however long — fires once inside the next window either way. The
  // page records the delay of every timer the app schedules; the ladder's
  // 10 s, 20 s, 40 s and 60 s steps are values nothing else in the app uses.
  await page.addInitScript(() => {
    window.__setMs = [];
    const scheduled = window.setTimeout.bind(window);
    window.setTimeout = (callback, ms, ...args) => {
      window.__setMs.push(ms);
      return scheduled(callback, ms, ...args);
    };
  });
  const requests = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname === "/corvette/api/v1/cameras/status") {
      requests.push(url.pathname);
    }
  });

  let failed = true;
  await stubDashboardApis(page, [FRONT, GARAGE]);
  await stubStatus(page, () =>
    failed ? null : statusAnswer([live("front"), live("garage")]),
  );

  await page.goto("/");

  const group = healthGroup(page);
  await expect(group.locator(".health-line-offline")).toHaveText(
    "Page offline — not updated yet. Retrying.",
  );
  await expect.poll(() => requests.length).toBe(1);

  await page.clock.fastForward(5_000);
  await expect.poll(() => requests.length).toBe(2);
  await page.clock.fastForward(10_000);
  await expect.poll(() => requests.length).toBe(3);
  await page.clock.fastForward(20_000);
  await expect.poll(() => requests.length).toBe(4);
  await page.clock.fastForward(40_000);
  await expect.poll(() => requests.length).toBe(5);
  await page.clock.fastForward(60_000);
  await expect.poll(() => requests.length).toBe(6);
  await page.clock.fastForward(60_000);
  await expect.poll(() => requests.length).toBe(7);

  // The next retry (60 seconds after the last failure) finally succeeds, and
  // the following poll comes 30 seconds later, not on the retry ladder.
  failed = false;
  await page.clock.fastForward(60_000);
  await expect.poll(() => requests.length).toBe(8);
  const updated = await localClock(page, START_SECONDS + 255);
  await expect(group.locator(".health-line-ok")).toHaveText(
    `All cameras that are on are recording. Updated ${updated}.`,
  );
  await page.clock.fastForward(30_000);
  await expect.poll(() => requests.length).toBe(9);

  // The retried polls armed timers of 10, 20, 40 and 60 seconds — the ladder
  // itself. A constant retry never arms those values.
  const scheduled = await page.evaluate(() => window.__setMs);
  for (const delay of [10_000, 20_000, 40_000, 60_000]) {
    expect(scheduled).toContain(delay);
  }
});

test("the offline line names the last good update and never shows the healthy line", async ({
  page,
}) => {
  await page.clock.install({ time: new Date(START) });
  let failed = false;
  await stubDashboardApis(page, [FRONT, GARAGE]);
  await stubStatus(page, () =>
    failed ? null : statusAnswer([live("front"), live("garage")]),
  );

  await page.goto("/");

  const group = healthGroup(page);
  await expect(group.locator(".health-line-ok")).toBeVisible();
  const updated = await localClock(page, START_SECONDS);

  failed = true;
  await page.clock.fastForward(30_000);
  await expect(group.locator(".health-line-offline")).toHaveText(
    `Page offline — last updated ${updated}. Retrying.`,
  );
  await expect(group.locator(".health-line-ok")).toHaveCount(0);
});

const tileFor = (page, displayName) =>
  page
    .locator(".camera-card")
    .filter({ has: page.getByRole("link", { name: `Open ${displayName} live view` }) });

const offTileFor = (page, displayName) =>
  page.locator(".camera-card").filter({ hasText: `${displayName}: off` });

test("a camera with a persisted fault says on its tile that no new picture has arrived since the fault began", async ({
  page,
}) => {
  await page.clock.install({ time: new Date(START) });
  const since = START_SECONDS - 130;
  await stubDashboardApis(page, [FRONT, GARAGE]);
  await stubStatus(page, () =>
    statusAnswer([{ name: "front", state: "stale", since }, live("garage")]),
  );

  await page.goto("/");

  const began = await localClock(page, since);
  const front = tileFor(page, "Front");
  await expect(front.locator(".camera-card-state-amber")).toHaveText(
    `No new picture since ${began}.`,
  );
  await expect(tileFor(page, "Garage")).toHaveCount(1);
  await expect(tileFor(page, "Garage").locator(".camera-card-state")).toHaveCount(0);

  await expect(front.locator(".still-tile-image")).toBeVisible();
  await expect(front.getByRole("link", { name: "Open Front live view" })).toHaveAttribute(
    "href",
    "/camera/front",
  );
});

test("a fault younger than two minutes puts no line on the tile", async ({ page }) => {
  await page.clock.install({ time: new Date(START) });
  const polls = trackStatusPolls(page);
  await stubDashboardApis(page, [FRONT, GARAGE]);
  await stubStatus(page, () =>
    statusAnswer([{ name: "front", state: "stale", since: START_SECONDS - 30 }, live("garage")]),
  );

  await page.goto("/");
  await settledPolls(page, polls, 1);

  await expect(page.locator(".camera-card-state")).toHaveCount(0);
  await expect(healthGroup(page).locator(".health-line-ok")).toBeVisible();
});

test("a tile's line goes when its camera has been healthy for two polls", async ({ page }) => {
  await page.clock.install({ time: new Date(START) });
  const since = START_SECONDS - 130;
  let current = statusAnswer([{ name: "front", state: "stale", since }, live("garage")]);
  const polls = trackStatusPolls(page);
  await stubDashboardApis(page, [FRONT, GARAGE]);
  await stubStatus(page, () => current);

  await page.goto("/");

  const front = tileFor(page, "Front");
  await settledPolls(page, polls, 1);
  await expect(front.locator(".camera-card-state-amber")).toBeVisible();

  current = statusAnswer([live("front"), live("garage")]);
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 2);
  await expect(front.locator(".camera-card-state-amber")).toBeVisible();

  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 3);
  await expect(front.locator(".camera-card-state")).toHaveCount(0);
});

test("an off camera's tile keeps its plain off line and gets no state line", async ({ page }) => {
  await page.clock.install({ time: new Date(START) });
  const longOff = START_SECONDS - 86_400;
  const polls = trackStatusPolls(page);
  await stubDashboardApis(page, [FRONT, GARAGE, LIVING]);
  await stubStatus(page, () =>
    statusAnswer([
      live("front"),
      live("garage"),
      { name: "living", state: "off_by_choice", since: longOff },
    ]),
  );

  await page.goto("/");
  await settledPolls(page, polls, 1);

  const living = offTileFor(page, "Living");
  await expect(living.locator(".camera-card-off")).toHaveText("Living: off");
  await expect(living.locator(".camera-card-state")).toHaveCount(0);
  await expect(page.locator(".camera-card-state")).toHaveCount(0);
});

test("the page makes one status request per poll however many tiles read it", async ({ page }) => {
  await page.clock.install({ time: new Date(START) });
  const since = START_SECONDS - 130;
  const polls = trackStatusPolls(page);
  await stubDashboardApis(page, [FRONT, GARAGE]);
  await stubStatus(page, () =>
    statusAnswer([
      { name: "front", state: "stale", since },
      { name: "garage", state: "stale", since },
    ]),
  );

  await page.goto("/");

  const group = healthGroup(page);
  await settledPolls(page, polls, 1);
  await expect(group.locator(".health-line-amber")).toHaveCount(2);
  await expect(tileFor(page, "Front").locator(".camera-card-state-amber")).toHaveCount(1);
  await expect(tileFor(page, "Garage").locator(".camera-card-state-amber")).toHaveCount(1);

  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 2);
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 3);
});

test("the System section gives the System menu entry a place to land and lists every camera with its state in words", async ({
  page,
}) => {
  await page.clock.install({ time: new Date(START) });
  await stubDashboardApis(page, [FRONT, GARAGE, LIVING]);
  await stubStatus(page, () =>
    statusAnswer([
      { name: "front", state: "no_picture", since: START_SECONDS - 200 },
      live("garage", START_SECONDS - 3600),
      { name: "living", state: "off_by_choice", since: START_SECONDS - 7200 },
    ]),
  );

  await page.goto("/");

  await expect(page.getByRole("link", { name: "System", exact: true })).toHaveAttribute(
    "href",
    "/#system",
  );
  await expect(page.locator("#system")).toHaveCount(1);
  await expect(page.getByRole("heading", { name: "System", exact: true })).toBeVisible();

  const rows = page.locator("#system .system-row");
  await expect(rows).toHaveCount(3);
  await expect(page.locator("#system ul.system-list > li.system-row")).toHaveCount(3);
  const began = await localClock(page, START_SECONDS - 200);
  await expect(rows.nth(0).locator(".system-name")).toHaveText("Front");
  await expect(rows.nth(0).locator(".system-state")).toHaveText(`No picture since ${began}`);
  await expect(rows.nth(1).locator(".system-name")).toHaveText("Garage");
  await expect(rows.nth(1).locator(".system-state")).toHaveText("Live");
  await expect(rows.nth(2).locator(".system-name")).toHaveText("Living");
  await expect(rows.nth(2).locator(".system-state")).toHaveText("Off by choice");
  await expect(rows.nth(0)).toHaveClass(/system-row-fault/);
  await expect(rows.nth(1)).not.toHaveClass(/system-row-fault/);
  await expect(rows.nth(2)).not.toHaveClass(/system-row-fault/);
});

test("the System section shows a fresh fault that the health line is still quiet about", async ({
  page,
}) => {
  await page.clock.install({ time: new Date(START) });
  await stubDashboardApis(page, [FRONT, GARAGE]);
  await stubStatus(page, () =>
    statusAnswer([
      { name: "front", state: "stale", since: START_SECONDS - 30 },
      live("garage"),
    ]),
  );

  await page.goto("/");

  await expect(healthGroup(page).locator(".health-line-ok")).toBeVisible();

  const frontRow = page.locator("#system .system-row").nth(0);
  const began = await localClock(page, START_SECONDS - 30);
  await expect(frontRow.locator(".system-state")).toHaveText(
    `Picture not updating since ${began}`,
  );
  await expect(frontRow).toHaveClass(/system-row-fault/);
});

test("a camera that recovers reads Live in the System section at the next poll, with no time", async ({
  page,
}) => {
  await page.clock.install({ time: new Date(START) });
  let current = statusAnswer([
    { name: "front", state: "stale", since: START_SECONDS - 30 },
    live("garage"),
  ]);
  const polls = trackStatusPolls(page);
  await stubDashboardApis(page, [FRONT, GARAGE]);
  await stubStatus(page, () => current);

  await page.goto("/");
  await settledPolls(page, polls, 1);

  const frontRow = page.locator("#system .system-row").nth(0);
  const began = await localClock(page, START_SECONDS - 30);
  await expect(frontRow.locator(".system-state")).toHaveText(
    `Picture not updating since ${began}`,
  );

  current = statusAnswer([live("front"), live("garage")]);
  await page.clock.fastForward(30_000);
  await settledPolls(page, polls, 2);

  await expect(frontRow.locator(".system-state")).toHaveText("Live");
  await expect(frontRow).not.toHaveClass(/system-row-fault/);
});

test("while the page is offline the System section shows the offline wording instead of the list", async ({
  page,
}) => {
  await page.clock.install({ time: new Date(START) });
  let failed = false;
  const polls = trackStatusPolls(page);
  const failedPolls = { count: 0 };
  page.on("requestfailed", (request) => {
    if (new URL(request.url()).pathname === "/corvette/api/v1/cameras/status") {
      failedPolls.count += 1;
    }
  });
  await stubDashboardApis(page, [FRONT, GARAGE, LIVING]);
  await stubStatus(page, () =>
    failed
      ? null
      : statusAnswer([
          live("front"),
          live("garage"),
          { name: "living", state: "off_by_choice", since: START_SECONDS - 7200 },
        ]),
  );

  await page.goto("/");
  await settledPolls(page, polls, 1);
  await expect(page.locator("#system .system-row")).toHaveCount(3);

  // route.abort() fails the request, so the failed poll lands as a
  // requestfailed, not a finished one: count it to prove it arrived.
  failed = true;
  await page.clock.fastForward(30_000);
  await expect.poll(() => failedPolls.count).toBe(1);
  await page.evaluate(() => Promise.resolve());

  const updated = await localClock(page, START_SECONDS);
  await expect(page.locator("#system .system-note-offline")).toHaveText(
    `Page offline — last updated ${updated}. Retrying.`,
  );
  await expect(page.locator("#system .system-row")).toHaveCount(0);

  failed = false;
  await page.clock.fastForward(5_000);
  await settledPolls(page, polls, 2);
  await expect(page.locator("#system .system-row")).toHaveCount(3);
  await expect(page.locator("#system .system-note-offline")).toHaveCount(0);
});
