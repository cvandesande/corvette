const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Frigate's nginx rewrites its own bundle's BASE_PATH tokens from a request
// header, and one of those rules fires on any <body>: it injects a
// window.baseUrl script into every HTML response. This site emits no BASE_PATH
// tokens, so that injection is the only rewrite it can receive -- and whether
// an injected script ahead of the module's mount point leaves the bundle
// bootable is not answerable from a response body.
//
// "Booted" here means the router rendered: the wasm was fetched with a MIME
// type the browser accepts for streaming instantiation, instantiated, and
// mounted.

const bootedApp = (page) => page.getByRole("link", { name: "Recordings" });

test("the bundle boots through the deployment's nginx", async ({ page }) => {
  const uncaught = [];
  page.on("pageerror", (error) => uncaught.push(error.message));

  await page.goto("/");

  await expect(bootedApp(page)).toBeVisible();
  expect(uncaught).toEqual([]);
});

test("a client route reached directly boots the same bundle", async ({ page }) => {
  await page.goto("/events");

  await expect(bootedApp(page)).toBeVisible();
  await expect(page.getByRole("heading", { name: "Events", exact: true })).toBeVisible();
});

test.describe("behind an ingress path prefix", () => {
  test.use({ extraHTTPHeaders: { "X-Ingress-Path": "/sub" } });

  test("the injected base-url script arrives and does not stop the boot", async ({
    page,
  }) => {
    const uncaught = [];
    page.on("pageerror", (error) => uncaught.push(error.message));

    await page.goto("/");

    expect(await page.evaluate(() => window.baseUrl)).toBe("/sub/");
    await expect(bootedApp(page)).toBeVisible();
    expect(uncaught).toEqual([]);
  });
});
