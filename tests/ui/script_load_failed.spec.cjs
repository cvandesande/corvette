const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// A player script that fails, stalls, or cannot be mounted must still land the
// live screen on a picture path or the explicit no-path state.

const JPEG = Buffer.from(
  "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q==",
  "base64",
);

const FRONT_CAMERA = {
  name: "front",
  display_name: "Front",
  order: 0,
  detect_width: 1920,
  detect_height: 1080,
};

// A relay address that refuses at once, so any MoQ attempt fails fast.
const DEAD_RELAY_URL = "https://127.0.0.1:9/anon";

const setup = async (page) => {
  await page.addInitScript((url) => {
    window.__corvetteMoqRelayUrl = url;
  }, DEAD_RELAY_URL);
  await Promise.all([
    page.route("**/corvette/api/v1/cameras", (route) =>
      route.fulfill({ json: { cameras: [FRONT_CAMERA] } }),
    ),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/*/latest.jpg*", (route) =>
      route.fulfill({ contentType: "image/jpeg", body: JPEG }),
    ),
    // Never answered, so the live-path label stays on its value.
    page.route("**/live/hls/**", () => {}),
  ]);
};

test("a moq player script that fails to load ends on the HLS path", async ({ page }) => {
  await setup(page);
  await page.route("**/vendor/moq-watch.bundle.js", (route) => route.abort());
  await page.goto("/");

  expect(await page.evaluate(() => typeof VideoDecoder)).toBe("function");

  await page.getByRole("button", { name: /expand front to full view/i }).click();

  await expect(page.locator(".live-path-indicator")).toHaveAttribute("data-live-path", "hls", {
    timeout: 3_000,
  });
  await expect(page.locator(".expanded-player moq-watch")).toHaveCount(0);
});

test("a moq player script that stalls ends on the HLS path after the script bound", async ({
  page,
}) => {
  test.setTimeout(30_000);
  await setup(page);
  await page.route("**/vendor/moq-watch.bundle.js", () => {});
  await page.goto("/");
  await page.getByRole("button", { name: /expand front to full view/i }).click();

  await expect(page.locator(".live-path-indicator")).toHaveAttribute(
    "data-live-path",
    "connecting",
    { timeout: 2_000 },
  );
  await expect(page.locator(".live-path-indicator")).toHaveAttribute("data-live-path", "hls", {
    timeout: 15_000,
  });
});

test("a moq player element that cannot be created ends on the HLS path", async ({ page }) => {
  await setup(page);
  await page.addInitScript(() => {
    const original = Document.prototype.createElement;
    Document.prototype.createElement = function (tag, ...options) {
      if (String(tag).toLowerCase() === "moq-watch") {
        throw new Error("injected");
      }
      return original.call(this, tag, ...options);
    };
  });
  await page.goto("/");
  await page.getByRole("button", { name: /expand front to full view/i }).click();

  await expect(page.locator(".live-path-indicator")).toHaveAttribute("data-live-path", "hls", {
    timeout: 3_000,
  });
});

test("a moq player element that cannot be appended ends on the HLS path", async ({ page }) => {
  await setup(page);
  await page.addInitScript(() => {
    const original = Node.prototype.appendChild;
    Node.prototype.appendChild = function (child) {
      if (child && child.localName === "moq-watch") {
        throw new Error("injected");
      }
      return original.call(this, child);
    };
  });
  await page.goto("/");
  await page.getByRole("button", { name: /expand front to full view/i }).click();

  await expect(page.locator(".live-path-indicator")).toHaveAttribute("data-live-path", "hls", {
    timeout: 3_000,
  });
});

test("an hls script that fails to load ends on the no-path state", async ({ page }) => {
  await setup(page);
  await page.addInitScript(() => {
    delete window.VideoDecoder;
    HTMLMediaElement.prototype.canPlayType = () => "";
  });
  await page.route("**/vendor/hls.min.js", (route) => route.abort());
  await page.goto("/");
  await page.getByRole("button", { name: /expand front to full view/i }).click();

  await expect(page.locator(".live-path-indicator")).toHaveAttribute("data-live-path", "none", {
    timeout: 3_000,
  });
  await expect(page.locator(".live-path-indicator")).toHaveText("Live view unavailable");
});

test("a moq player script that completes after the script bound does not revive the failed script", async ({
  page,
}) => {
  test.setTimeout(40_000);
  await setup(page);
  await page.addInitScript(() => {
    window.__moqElementsCreated = 0;
    const original = Document.prototype.createElement;
    Document.prototype.createElement = function (tag, ...options) {
      if (String(tag).toLowerCase() === "moq-watch") {
        window.__moqElementsCreated++;
      }
      return original.call(this, tag, ...options);
    };
  });
  let held;
  await page.route("**/vendor/moq-watch.bundle.js", (route) => {
    held = route;
  });
  await page.goto("/");
  await page.getByRole("button", { name: /expand front to full view/i }).click();

  await expect(page.locator(".live-path-indicator")).toHaveAttribute("data-live-path", "hls", {
    timeout: 15_000,
  });

  // The bound has already failed the stalled request; answering it now fires a load that must stay ignored.
  await page.evaluate(() => {
    const script = document.querySelector('script[src$="moq-watch.bundle.js"]');
    window.__moqScriptLoaded = new Promise((resolve) => {
      script.addEventListener("load", resolve, { once: true });
    });
  });
  await held.fulfill({ contentType: "application/javascript", body: "" });
  await page.evaluate(() => window.__moqScriptLoaded);

  await page.getByRole("button", { name: "Close" }).click();
  await page.getByRole("button", { name: /expand front to full view/i }).click();

  await expect(page.locator(".live-path-indicator")).toHaveAttribute("data-live-path", "hls", {
    timeout: 3_000,
  });
  expect(await page.evaluate(() => window.__moqElementsCreated)).toBe(0);
});
