const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Issue #22 item R2's own Verify step: "All cameras" is now `/recordings`'
// default camera selection (D-5), and choosing a preset in that mode packs
// one placeholder tile per camera into `.all-cameras-grid` via
// `monitor_layout::pack_tiles`, re-packing on resize (D-2(b)).
//
// Item R3 replaces those placeholders with real per-tile playback: one
// shared scrub position drives N independent `TimelinePlayer` instances
// (D-1(c)), each with its own "not yet settled" indicator and no cross-tile
// barrier (D-1's rejected option (b)).
//
// Item R4 adds D-3(d)'s no-footage representation: a camera with zero
// retained recordings anywhere in the loaded range gets an explicit
// placeholder, and a camera with a real gap at the shared scrub position
// (but clips elsewhere in range) gets a dimmed still frame at its own
// nearest playable time, labeled with its signed offset from the scrub
// position. Resolved 2026-09-06 (a targeted fix after independent review):
// a camera with previews but no clips at all gets the SAME gap treatment,
// sourced from its nearest preview instead -- Frigate prunes recording
// clips and preview intervals independently per `retain.mode`, so a
// surviving preview with no surviving clip is routine, not an anomaly.

const mockCameraConfig = (page, cameras) => {
  return Promise.all([
    page.route("**/api/config", (route) => route.fulfill({ json: { cameras } })),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/recordings/summary?*", (route) => route.fulfill({ json: {} })),
    // "All" mode's own combined preview fetch (`camera == "all"`, F-13):
    // every test below that loads a range triggers this regardless of
    // whether it cares about previews, since R3's `AllCamerasPlayback` now
    // gates its whole grid on this resolving alongside `cameras`.
    page.route("**/api/preview/all/start/*/end/*", (route) => route.fulfill({ json: [] })),
  ]);
};

// Builds N enabled cameras named "camera-0".."camera-{n-1}", matching
// `monitor.spec.cjs`'s own helper shape.
const nCameras = (count) => {
  const cameras = {};
  for (let index = 0; index < count; index += 1) {
    cameras[`camera-${index}`] = {
      enabled: true,
      friendly_name: `Camera ${index}`,
      ui: { order: index },
      detect: { width: 1920, height: 1080 },
    };
  }
  return cameras;
};

// A short, fixed cycle of aspect ratios (16:9, 4:3, portrait 9:16), matching
// `monitor.spec.cjs`'s own helper shape, so a generated fixture actually
// exercises `pack_tiles`'s aspect-ratio-aware placement rather than every
// tile sharing one shape.
const MIXED_ASPECT_RATIOS = [
  { width: 1920, height: 1080 },
  { width: 1280, height: 960 },
  { width: 1080, height: 1920 },
];

const nMixedAspectCameras = (count) => {
  const cameras = {};
  for (let index = 0; index < count; index += 1) {
    const { width, height } = MIXED_ASPECT_RATIOS[index % MIXED_ASPECT_RATIOS.length];
    cameras[`camera-${index}`] = {
      enabled: true,
      friendly_name: `Camera ${index}`,
      ui: { order: index },
      detect: { width, height },
    };
  }
  return cameras;
};

const rectsOverlap = (a, b) => {
  const epsilon = 0.5;
  return (
    a.x + epsilon < b.x + b.width &&
    b.x + epsilon < a.x + a.width &&
    a.y + epsilon < b.y + b.height &&
    b.y + epsilon < a.y + a.height
  );
};

// Mirrors `monitor_layout.rs`'s own bounds/no-overlap test oracle, asserted
// here against real rendered DOM boxes (`.all-cameras-tile`) rather than
// `pack_tiles` called directly.
const assertTilesPackWithinGrid = (tileBoxes, gridBox) => {
  const epsilon = 0.5;
  for (const [index, box] of tileBoxes.entries()) {
    expect(box.width, `tile ${index} has a non-zero width`).toBeGreaterThan(0);
    expect(box.height, `tile ${index} has a non-zero height`).toBeGreaterThan(0);
    expect(box.x, `tile ${index}'s left edge is within the grid`).toBeGreaterThanOrEqual(
      gridBox.x - epsilon,
    );
    expect(box.y, `tile ${index}'s top edge is within the grid`).toBeGreaterThanOrEqual(
      gridBox.y - epsilon,
    );
    expect(
      box.x + box.width,
      `tile ${index}'s right edge is within the grid`,
    ).toBeLessThanOrEqual(gridBox.x + gridBox.width + epsilon);
    expect(
      box.y + box.height,
      `tile ${index}'s bottom edge is within the grid`,
    ).toBeLessThanOrEqual(gridBox.y + gridBox.height + epsilon);
  }
  for (let i = 0; i < tileBoxes.length; i += 1) {
    for (let j = i + 1; j < tileBoxes.length; j += 1) {
      expect(
        rectsOverlap(tileBoxes[i], tileBoxes[j]),
        `tiles ${i} and ${j} should not overlap`,
      ).toBe(false);
    }
  }
};

// A `<input type="range" step="1">`'s own value-sanitization algorithm
// snaps any programmatically-assigned value to the nearest whole step
// *from its own `min`* -- so reading back a sub-second fractional value can
// differ slightly from whatever fractional instant a test asked for, purely
// as a browser-level artifact unrelated to `snap_or_raw`/`playable_time`'s
// own arithmetic. These assertions compare at whole-second resolution, the
// same granularity every offset label in this file already renders at
// (`format_gap_offset`'s own `{:+.0}s`).
const expectPlayheadValueNear = (playhead, expectedSeconds) =>
  expect
    .poll(async () => Math.round(Number(await playhead.inputValue())))
    .toBe(Math.round(expectedSeconds));

const tileBoxes = (page) =>
  page.locator(".all-cameras-tile").evaluateAll((elements) =>
    elements.map((element) => {
      const rect = element.getBoundingClientRect();
      return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
    }),
  );

test("recordings defaults to All cameras with no explicit selection", async ({ page }) => {
  await mockCameraConfig(page, nCameras(2));

  await page.goto("/recordings");

  await expect(page.getByLabel("Camera")).toHaveValue("all");
  await expect(page.locator('option[value="all"]')).toHaveText("All cameras");
});

for (const cameraCount of [2, 4, 5]) {
  test(`choosing a preset in All mode packs ${cameraCount} placeholder tiles with no overlap`, async ({
    page,
  }) => {
    await mockCameraConfig(page, nMixedAspectCameras(cameraCount));

    await page.goto("/recordings");
    await page.getByRole("button", { name: "Last hour" }).click();

    const tiles = page.locator(".all-cameras-tile");
    await expect(tiles).toHaveCount(cameraCount);
    for (let index = 0; index < cameraCount; index += 1) {
      // `toContainText`, not an exact match: since R3, each tile also
      // renders its own `TimelinePlayer` (a play/pause button and a
      // `<video>` fallback-text node) alongside the camera name heading this
      // assertion cares about.
      await expect(tiles.nth(index)).toContainText(`Camera ${index}`);
    }

    await page.waitForFunction(
      (expectedCount) =>
        [...document.querySelectorAll(".all-cameras-tile")].every(
          (tile) => tile.getBoundingClientRect().width > 0,
        ) && document.querySelectorAll(".all-cameras-tile").length === expectedCount,
      cameraCount,
    );

    const gridBox = await page.locator(".all-cameras-grid").boundingBox();
    assertTilesPackWithinGrid(await tileBoxes(page), gridBox);
  });
}

test("resizing the viewport after tiles render re-packs them to the new grid box", async ({
  page,
}) => {
  const cameraCount = 3;
  await page.setViewportSize({ width: 1280, height: 900 });
  await mockCameraConfig(page, nMixedAspectCameras(cameraCount));

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();
  await expect(page.locator(".all-cameras-tile")).toHaveCount(cameraCount);

  const boxesBeforeResize = await tileBoxes(page);
  const gridBoxBeforeResize = await page.locator(".all-cameras-grid").boundingBox();
  assertTilesPackWithinGrid(boxesBeforeResize, gridBoxBeforeResize);

  // `.all-cameras-grid` is sized in `vh` (`styles.css`), so a taller viewport
  // gives it a real, different pixel box -- this is `pack_tiles`'s resize
  // path (D-2(b)), never exercised anywhere else in this codebase.
  await page.setViewportSize({ width: 1280, height: 1500 });

  await expect
    .poll(async () => (await page.locator(".all-cameras-grid").boundingBox()).height)
    .toBeGreaterThan(gridBoxBeforeResize.height);
  await expect
    .poll(async () => (await tileBoxes(page))[0].height)
    .not.toBeCloseTo(boxesBeforeResize[0].height, 0);

  const gridBoxAfterResize = await page.locator(".all-cameras-grid").boundingBox();
  assertTilesPackWithinGrid(await tileBoxes(page), gridBoxAfterResize);
});

test("All mode fetches each real camera's own recordings and never a synthetic \"all\" camera", async ({
  page,
}) => {
  const recordingsRequests = [];
  page.on("request", (request) => {
    const { pathname } = new URL(request.url());
    if (/^\/api\/[^/]+\/recordings$/.test(pathname)) {
      recordingsRequests.push(pathname);
    }
  });

  await mockCameraConfig(page, nCameras(3));
  await page.route("**/api/camera-*/recordings?*", (route) => route.fulfill({ json: [] }));
  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();
  await expect(page.locator(".all-cameras-tile")).toHaveCount(3);

  // Since R3, each tile owns its own `fetch_recording_segments` call against
  // its own real camera name (invariant 5) -- this is the item that first
  // makes this request happen at all; R2 only ever asserted its absence.
  await expect.poll(() => recordingsRequests.length).toBe(3);
  expect(recordingsRequests).not.toContain("/api/all/recordings");
  // A sorted-array comparison, not `Set` equality: invariant 5 requires each
  // real camera fetched *exactly* once, and a `Set` would silently collapse
  // a duplicate fetch for the same camera into the same membership result.
  expect(recordingsRequests.slice().sort()).toEqual([
    "/api/camera-0/recordings",
    "/api/camera-1/recordings",
    "/api/camera-2/recordings",
  ]);
});

// Issue #25 item S2 (D-6(a), invariant 6): `RecordingContext.review_activity`
// (`recordings.rs`) is one already-combined "All" mode resource, forwarded
// straight through `AllCamerasPlayback`/`AllCamerasGrid` with no
// transformation and never refetched or merged per camera -- this is the
// first test that exercises "All" mode's own `/api/review` request count at
// all, since `review_activity` was never forwarded to "All" mode before this
// item.
for (const cameraCount of [1, 3]) {
  test(`All mode requests /api/review exactly once regardless of camera count (${cameraCount} cameras)`, async ({
    page,
  }) => {
    const reviewRequests = [];
    page.on("request", (request) => {
      const { pathname } = new URL(request.url());
      if (pathname === "/api/review") {
        reviewRequests.push(request.url());
      }
    });

    await mockCameraConfig(page, nCameras(cameraCount));
    await page.route("**/api/camera-*/recordings?*", (route) => route.fulfill({ json: [] }));
    await page.goto("/recordings");
    await page.getByRole("button", { name: "Last hour" }).click();
    await expect(page.locator(".all-cameras-tile")).toHaveCount(cameraCount);

    await expect.poll(() => reviewRequests.length).toBe(1);
  });
}

test("moving the shared scrub updates each tile independently of the others' clip boundaries", async ({
  page,
}) => {
  const now = Date.now() / 1000;
  await mockCameraConfig(page, nCameras(2));
  // camera-0 has a real gap between two retained clips at `now - 1800`..
  // `now - 1700`; camera-1's own single clip spans the whole loaded range
  // with no boundary to cross. Both cameras' own first clip starts 100
  // seconds into the loaded range (`now - 3600`, "Last hour"), so both
  // tiles start in R4's own gap state (D-3(d)) before the first scrub below
  // -- neither camera has retained footage in that first sliver of the
  // range.
  await page.route("**/api/camera-0/recordings?*", (route) =>
    route.fulfill({
      json: [
        { start_time: now - 3500, end_time: now - 1800, motion: null },
        { start_time: now - 1700, end_time: now - 100, motion: null },
      ],
    }),
  );
  await page.route("**/api/camera-1/recordings?*", (route) =>
    route.fulfill({ json: [{ start_time: now - 3500, end_time: now - 100, motion: null }] }),
  );

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();

  const tileVideo = (camera) =>
    page.locator(`.all-cameras-tile[data-camera="${camera}"] .timeline-player`);
  const scrubTo = (time) =>
    page.getByRole("slider", { name: "All-cameras playhead" }).evaluate((input, value) => {
      input.value = value;
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }, String(time));

  // Scrubs into both cameras' first clip, past R4's own initial gap state --
  // the first point in this test both tiles are guaranteed to render a
  // `TimelinePlayer` at all.
  await scrubTo(now - 3000);
  await expect(page.locator(".all-cameras-tile video")).toHaveCount(2);
  await expect(tileVideo("camera-0")).toHaveAttribute(
    "src",
    `/api/camera-0/start/${now - 3500}/end/${now - 1800}/clip.mp4`,
  );
  await expect(tileVideo("camera-1")).toHaveAttribute(
    "src",
    `/api/camera-1/start/${now - 3500}/end/${now - 100}/clip.mp4`,
  );

  await scrubTo(now - 1600);
  await expect(tileVideo("camera-0")).toHaveAttribute(
    "src",
    `/api/camera-0/start/${now - 1700}/end/${now - 100}/clip.mp4`,
  );
  // camera-1's own single continuous clip never had a boundary to cross, so
  // its source stays exactly what it was -- proving the shared scrub drives
  // each tile independently rather than through one shared player.
  await expect(tileVideo("camera-1")).toHaveAttribute(
    "src",
    `/api/camera-1/start/${now - 3500}/end/${now - 100}/clip.mp4`,
  );
});

test("the \"not yet settled\" indicator is independent per tile", async ({ page }) => {
  await mockCameraConfig(page, nCameras(2));
  // A segment clamped to the whole loaded range on both cameras, so each
  // tile's initial source starts exactly at the shared scrub position (the
  // loaded range's own start) -- its `requested_offset` is exactly `0`,
  // which is also a real, un-stubbed `<video>`'s own default `currentTime`
  // before any media loads.
  await page.route("**/api/camera-0/recordings?*", (route) =>
    route.fulfill({ json: [{ start_time: 0, end_time: 9_999_999_999, motion: null }] }),
  );
  await page.route("**/api/camera-1/recordings?*", (route) =>
    route.fulfill({ json: [{ start_time: 0, end_time: 9_999_999_999, motion: null }] }),
  );

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();
  await expect(page.locator(".all-cameras-tile video")).toHaveCount(2);

  const unsettledIndicator = (camera) =>
    page.locator(`.all-cameras-tile[data-camera="${camera}"] .all-cameras-tile-unsettled`);

  // Neither tile's own <video> ever actually loads real media under this
  // mock, so both tiles start -- and, absent any confirmation, stay -- "not
  // yet settled" (D-1(c)'s own honest-signal requirement).
  await expect(unsettledIndicator("camera-0")).toBeVisible();
  await expect(unsettledIndicator("camera-1")).toBeVisible();

  // Confirms only camera-0's own landing, exactly as a real `seeked`/
  // `loadedmetadata` event would once this tile's source finishes loading.
  await page
    .locator('.all-cameras-tile[data-camera="camera-0"] video')
    .dispatchEvent("loadedmetadata");

  await expect(unsettledIndicator("camera-0")).toHaveCount(0);
  // camera-1's own tile never received a confirming event and is still
  // reporting unsettled -- no cross-tile barrier (D-1's rejected option (b)).
  await expect(unsettledIndicator("camera-1")).toBeVisible();
});

test("a camera with no recordings anywhere in range shows the no-recordings placeholder", async ({
  page,
}) => {
  await mockCameraConfig(page, nCameras(2));
  // camera-0 has zero retained segments anywhere in the loaded range, and
  // (via `mockCameraConfig`'s own empty-json "All" preview route) zero
  // previews either -- the plain no-recordings placeholder is reserved for
  // this genuinely-nothing-here combination (R4's targeted fix, resolved
  // 2026-09-06: clips-empty-but-previews-non-empty gets the gap treatment
  // instead, see the dedicated test below). camera-1 has full coverage, as
  // a per-tile control.
  await page.route("**/api/camera-0/recordings?*", (route) => route.fulfill({ json: [] }));
  await page.route("**/api/camera-1/recordings?*", (route) =>
    route.fulfill({ json: [{ start_time: 0, end_time: 9_999_999_999, motion: null }] }),
  );

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();

  const tile = (camera) => page.locator(`.all-cameras-tile[data-camera="${camera}"]`);

  await expect(tile("camera-0").locator(".all-cameras-tile-no-recordings")).toContainText(
    "No recordings for this camera in this range.",
  );
  // Never falls through to a dimmed nearest-frame preview or a live player --
  // `playable_time` is never even called on an empty `clips` list, which
  // would otherwise panic (this item's own named premise).
  await expect(tile("camera-0").locator(".all-cameras-tile-gap")).toHaveCount(0);
  await expect(tile("camera-0").locator("video")).toHaveCount(0);

  // camera-1's own full coverage is unaffected by camera-0's empty clips
  // list -- the placeholder is per-tile, not page-wide.
  await expect(tile("camera-1").locator("video")).toHaveCount(1);
});

// Issue #25 item S3 (D-2(b)): the shared playhead now snaps to the *union's*
// own nearest playable time, not any one camera's own. A single-camera
// fixture can no longer exercise camera-0's own per-tile gap treatment at a
// scrubbed instant its own clips don't cover, because with nothing else in
// the union, the shared playhead would itself snap away from that instant
// before camera-0's own tile ever saw it. camera-1 here fully covers the
// whole loaded range (mirroring "moving the shared scrub updates each tile
// independently" above), so the union always covers every instant this test
// scrubs to -- the shared playhead never snaps, and camera-0 alone still
// shows its own per-tile gap treatment (R4, untouched by this item) at each
// of those instants, preserving this test's original intent under D-2(b)'s
// own accepted tradeoff.
test("a real gap at the scrubbed time shows a dimmed nearest-frame preview with a signed offset label", async ({
  page,
}) => {
  const now = Date.now() / 1000;
  await mockCameraConfig(page, nCameras(2));
  // camera-0 has a real gap between two retained clips at `now - 1800`..
  // `now - 1700`. The loaded range's own start (`now - 3600`, "Last hour")
  // falls before either clip, so the tile starts in the gap state too.
  await page.route("**/api/camera-0/recordings?*", (route) =>
    route.fulfill({
      json: [
        { start_time: now - 3500, end_time: now - 1800, motion: null },
        { start_time: now - 1700, end_time: now - 100, motion: null },
      ],
    }),
  );
  // camera-1 fully covers the whole loaded range -- there is no instant this
  // test scrubs to that the union doesn't also cover.
  await page.route("**/api/camera-1/recordings?*", (route) =>
    route.fulfill({ json: [{ start_time: now - 3500, end_time: now - 100, motion: null }] }),
  );

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();

  const tile = page.locator('.all-cameras-tile[data-camera="camera-0"]');
  await expect(tile.locator(".all-cameras-tile-gap")).toBeVisible();

  const playhead = page.getByRole("slider", { name: "All-cameras playhead" });
  const scrubTo = (time) =>
    playhead.evaluate((input, value) => {
      input.value = value;
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }, String(time));

  // Nearer to camera-0's second clip (starts `now - 1700`): the nearest
  // footage is *later* than the scrub position, a positive offset.
  await scrubTo(now - 1750);
  // D-2(b)'s positive case: camera-1's own coverage here keeps the shared
  // signal from moving away from the requested instant at all, even though
  // camera-0 alone lacks footage for it.
  await expectPlayheadValueNear(playhead, now - 1750);
  await expect(tile.locator(".all-cameras-tile-gap-preview")).toHaveAttribute(
    "src",
    `/api/camera-0/recordings/${now - 1700}/snapshot.jpg?height=720`,
  );
  await expect(tile.locator(".all-cameras-tile-gap-offset")).toContainText("+50s");

  // Nearer to camera-0's first clip's own end margin (`now - 1801`, one
  // second inside the clip's own `now - 1800` end): the offset flips sign --
  // the nearest footage is now *earlier* than the scrub position.
  await scrubTo(now - 1795);
  await expectPlayheadValueNear(playhead, now - 1795);
  await expect(tile.locator(".all-cameras-tile-gap-preview")).toHaveAttribute(
    "src",
    `/api/camera-0/recordings/${now - 1801}/snapshot.jpg?height=720`,
  );
  await expect(tile.locator(".all-cameras-tile-gap-offset")).toContainText("-6s");

  // Scrubbing back into real coverage removes the gap placeholder and
  // renders the tile's own `TimelinePlayer` again -- the shared scrub
  // position itself never snapped away from any of these instants (D-2(b)'s
  // union-snap semantics, positive case); only this tile's own rendering
  // reacted to the gap.
  await scrubTo(now - 3000);
  await expect(tile.locator(".all-cameras-tile-gap")).toHaveCount(0);
  await expect(tile.locator("video")).toHaveCount(1);
});

// D-2(b)'s union-snap semantics, negative case: with no second camera to
// cover camera-0's own gap, the *shared* playhead itself now snaps to the
// union's nearest playable time -- the exact tradeoff D-2(b) accepted, and
// the first test in this file to exercise the shared playhead's own
// `snap_or_raw` guard (S1) live, now that S3 wires a real merged clips list
// into it.
test("dragging to an instant no camera covers snaps the shared playhead to the nearest covered instant", async ({
  page,
}) => {
  const now = Date.now() / 1000;
  await mockCameraConfig(page, nCameras(1));
  await page.route("**/api/camera-0/recordings?*", (route) =>
    route.fulfill({
      json: [
        { start_time: now - 3500, end_time: now - 1800, motion: null },
        { start_time: now - 1700, end_time: now - 100, motion: null },
      ],
    }),
  );

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();

  const playhead = page.getByRole("slider", { name: "All-cameras playhead" });
  const scrubTo = (time) =>
    playhead.evaluate((input, value) => {
      input.value = value;
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }, String(time));

  // Nearer to the second clip's own start (`now - 1700`): the shared
  // playhead snaps forward to it.
  await scrubTo(now - 1750);
  await expectPlayheadValueNear(playhead, now - 1700);

  // Nearer to the first clip's own end margin (`now - 1801`): the snap
  // flips direction.
  await scrubTo(now - 1795);
  await expectPlayheadValueNear(playhead, now - 1801);
});

test("a camera with previews but no clips gets the gap treatment, not the no-recordings placeholder", async ({
  page,
}) => {
  const now = Date.now() / 1000;
  await mockCameraConfig(page, nCameras(1));
  // camera-0 has zero retained recording clips anywhere in the loaded
  // range, but Frigate still retained a low-res preview covering part of
  // it -- Frigate prunes recording segments and preview intervals
  // independently per `retain.mode` (`frigate/record/cleanup.py:150-260`
  // at v0.17.2), so a preview outliving its camera's last recording clip
  // is a routine, expected outcome of a non-`all` retain mode, not an
  // anomaly (R4's targeted fix, resolved 2026-09-06 STOP-AND-ASK gate).
  await page.route("**/api/camera-0/recordings?*", (route) => route.fulfill({ json: [] }));
  await page.route("**/api/preview/all/start/*/end/*", (route) =>
    route.fulfill({
      json: [
        {
          camera: "camera-0",
          src: "/clips/previews/camera-0/hour.mp4",
          type: "video/mp4",
          start: now - 1800,
          end: now - 1700,
        },
      ],
    }),
  );

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();

  const tile = page.locator('.all-cameras-tile[data-camera="camera-0"]');
  // Never the plain no-recordings placeholder -- there genuinely is a
  // usable preview frame, per the resolved decision.
  await expect(tile.locator(".all-cameras-tile-no-recordings")).toHaveCount(0);
  // Never a live `TimelinePlayer` either -- there is no full-resolution
  // clip for this camera to play at all.
  await expect(tile.locator(".timeline-player")).toHaveCount(0);
  await expect(tile.locator(".all-cameras-tile-gap")).toBeVisible();

  const scrubTo = (time) =>
    page.getByRole("slider", { name: "All-cameras playhead" }).evaluate((input, value) => {
      input.value = value;
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }, String(time));

  // Before the preview's own span: the nearest playable instant is the
  // preview's own start, later than the scrub position -- a positive
  // offset, and the preview video's own `#t=` fragment points at its
  // first frame (offset `0` within the preview).
  await scrubTo(now - 2000);
  await expect(tile.locator(".all-cameras-tile-gap-preview")).toHaveAttribute(
    "src",
    "/clips/previews/camera-0/hour.mp4#t=0.000",
  );
  await expect(tile.locator(".all-cameras-tile-gap-offset")).toContainText("+200s");

  // After the preview's own span: the nearest playable instant stops one
  // second short of its exclusive end (mirroring `playable_time`'s own
  // `CLIP_END_MARGIN_SECONDS` margin), earlier than the scrub position --
  // the offset flips sign, and the `#t=` fragment moves to 99s into the
  // 100-second preview.
  await scrubTo(now - 1600);
  await expect(tile.locator(".all-cameras-tile-gap-preview")).toHaveAttribute(
    "src",
    "/clips/previews/camera-0/hour.mp4#t=99.000",
  );
  await expect(tile.locator(".all-cameras-tile-gap-offset")).toContainText("-101s");
});

// Issue #25 item S3 wires the shared, per-camera-hoisted `merged_media`
// (S2, D-4(a)/D-6(a)) into `crate::timeline::RecordingScrubber` for the
// first time -- this is the first test asserting the *merged track itself*
// (as opposed to any one tile) reflects more than one camera's own data.
test("the merged track shows availability, motion, and review spans from more than one camera", async ({
  page,
}) => {
  const now = Date.now() / 1000;
  await mockCameraConfig(page, nCameras(2));
  // Two well-separated (>1s gap) clips, each with its own motion --
  // `push_contiguous_range`'s own merge-gap tolerance (`MAX_SEGMENT_GAP_SECONDS`)
  // keeps these as two distinct spans rather than folding them into one.
  await page.route("**/api/camera-0/recordings?*", (route) =>
    route.fulfill({ json: [{ start_time: now - 3500, end_time: now - 3000, motion: 5 }] }),
  );
  await page.route("**/api/camera-1/recordings?*", (route) =>
    route.fulfill({ json: [{ start_time: now - 1500, end_time: now - 1000, motion: 5 }] }),
  );
  await page.route("**/api/review?*", (route) =>
    route.fulfill({
      json: [
        { start_time: now - 3200, end_time: now - 3190, severity: "alert", camera: "camera-0" },
        { start_time: now - 1200, end_time: now - 1190, severity: "detection", camera: "camera-1" },
      ],
    }),
  );

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();

  const track = page.locator(".recording-timeline .timeline-track");
  // One availability span and one motion span per camera -- neither clip
  // overlaps or nears the other, so D-4(a)'s union never folds them
  // together, and D-6(a)'s plain concatenation keeps both cameras' own
  // motion ranges intact.
  await expect(track.locator(".timeline-availability")).toHaveCount(2);
  await expect(track.locator(".timeline-motion-recording")).toHaveCount(2);
  // Reviews were already forwarded as one already-combined resource before
  // this item (S2); this is the first test confirming both severities
  // render as markers once threaded through the shared scrubber.
  await expect(track.locator(".activity-alert")).toHaveCount(1);
  await expect(track.locator(".activity-detection")).toHaveCount(1);
});

// D-5(b), invariant 3: a merged span must not expose which camera(s)
// contributed to it. Reuses the two-camera, two-separate-span fixture above
// so there are real merged spans to inspect.
test("no merged availability span exposes which camera contributed to it", async ({ page }) => {
  const now = Date.now() / 1000;
  await mockCameraConfig(page, nCameras(2));
  await page.route("**/api/camera-0/recordings?*", (route) =>
    route.fulfill({ json: [{ start_time: now - 3500, end_time: now - 3000, motion: null }] }),
  );
  await page.route("**/api/camera-1/recordings?*", (route) =>
    route.fulfill({ json: [{ start_time: now - 1500, end_time: now - 1000, motion: null }] }),
  );

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();

  const spans = page.locator(".recording-timeline .timeline-availability");
  await expect(spans).toHaveCount(2);

  const attributesByElement = await spans.evaluateAll((elements) =>
    elements.map((element) =>
      Object.fromEntries(
        [...element.attributes].map((attribute) => [attribute.name, attribute.value]),
      ),
    ),
  );

  for (const attributes of attributesByElement) {
    expect(
      attributes["data-camera"],
      "a merged availability span should not carry a data-camera attribute",
    ).toBeUndefined();
    expect(
      attributes.title,
      "a merged availability span should not carry a title attribute",
    ).toBeUndefined();
    for (const [name, value] of Object.entries(attributes)) {
      expect(value, `attribute ${name} on a merged span should not name camera-0`).not.toContain(
        "camera-0",
      );
      expect(value, `attribute ${name} on a merged span should not name camera-1`).not.toContain(
        "camera-1",
      );
    }
  }
});

// Invariant 4's own mandatory guard, live: `ReviewMarkers`' own `on:click`
// handler must call `snap_or_raw`, not `playable_time` directly, since
// `playable_time` panics via `.expect(...)` on an empty clips slice --
// exactly what a merged "All" mode clips list can legitimately be (G-16).
// camera-0's own `/recordings` route is deliberately delayed so this test
// deterministically exercises the ordering `review_activity` already
// resolves in during real use: it is fetched independently at page mount
// (`recordings.rs`'s `RecordingContext`, keyed only on the selected camera,
// not on the loaded range or the camera list), so it is already resolved
// well before any per-camera `recording_clips` resource -- hoisted into
// `AllCamerasGrid` by S2 -- ever starts fetching. `AllCamerasGrid` gates
// `RecordingScrubber`'s own rendering on both resolving, so the review
// marker only becomes visible once camera-0's own (empty) clips resolve --
// at which point the merged clips list is empty and this guard is what
// keeps the click from panicking.
test("a review marker rendered once a camera's own recording_clips resolves empty can be clicked without crashing", async ({
  page,
}) => {
  const now = Date.now() / 1000;
  const consoleErrors = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      consoleErrors.push(message.text());
    }
  });
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(String(error)));

  await mockCameraConfig(page, nCameras(1));
  await page.route("**/api/camera-0/recordings?*", async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 300));
    await route.fulfill({ json: [] });
  });
  await page.route("**/api/review?*", (route) =>
    route.fulfill({
      json: [
        { start_time: now - 1800, end_time: now - 1790, severity: "alert", camera: "camera-0" },
      ],
    }),
  );

  await page.goto("/recordings");
  await page.getByRole("button", { name: "Last hour" }).click();

  // The scrubber (and the review marker inside it) stays gated behind the
  // loading state while camera-0's own resource is still pending, even
  // though the review data backing that marker already arrived.
  await expect(page.getByText("Loading recording availability")).toBeVisible();

  const marker = page.getByRole("button", { name: /alert activity at/i });
  await expect(marker).toBeVisible();

  await marker.click();

  expect(
    pageErrors,
    "clicking a review marker over an empty merged clips list threw an uncaught exception",
  ).toEqual([]);
  // Three known-benign lines this harness itself produces, unrelated to
  // this item's own correctness (`expanded_view.spec.cjs`'s own identical
  // precedent): no favicon is served here, this release bundle still wires
  // cargo-leptos's dev-mode hot-reload WebSocket client (nothing to connect
  // to outside `cargo leptos watch`), and an unrelated upstream (Frigate)
  // connection refusal this test's own mocked routes don't cover.
  const unexpectedConsoleErrors = consoleErrors.filter(
    (line) =>
      !line.includes("favicon") &&
      !line.includes("live_reload") &&
      !/50[0-9] \(.*Gateway\)/.test(line),
  );
  expect(
    unexpectedConsoleErrors,
    "clicking a review marker over an empty merged clips list logged an unexpected console error",
  ).toEqual([]);

  // Positively confirms the click landed rather than merely "nothing
  // visibly broke": the shared playhead moves to the review's own
  // unclamped start time, proving `snap_or_raw` returned it unchanged
  // rather than ever calling into `playable_time`.
  await expectPlayheadValueNear(
    page.getByRole("slider", { name: "All-cameras playhead" }),
    now - 1800,
  );
});
