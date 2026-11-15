const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

const EXPECTED_ICONS = [
  { src: "/icons/icon-192.png", sizes: "192x192", type: "image/png", purpose: "any" },
  { src: "/icons/icon-512.png", sizes: "512x512", type: "image/png", purpose: "any" },
  { src: "/icons/icon-512-maskable.png", sizes: "512x512", type: "image/png", purpose: "maskable" },
];

test("the shell links the manifest", async ({ page }) => {
  await page.goto("/");
  const links = page.locator('link[rel="manifest"]');
  await expect(links).toHaveCount(1);
  const href = await links.first().evaluate((link) => link.href);
  expect(href.endsWith("/manifest.json")).toBe(true);
});

test("the browser parses the manifest with no errors", async ({ page }) => {
  await page.goto("/");
  const shellColour = await page.locator('meta[name="theme-color"]').getAttribute("content");
  expect(shellColour).toBeTruthy();
  const session = await page.context().newCDPSession(page);
  const { url, errors, data } = await session.send("Page.getAppManifest");
  expect(errors).toEqual([]);
  expect(url.endsWith("/manifest.json")).toBe(true);
  expect(data).toBeTruthy();
  const manifest = JSON.parse(data);
  expect(manifest.name).toBe("Corvette");
  expect(manifest.short_name).toBe("Corvette");
  expect(manifest.id).toBe("/");
  expect(manifest.start_url).toBe("/");
  expect(manifest.scope).toBe("/");
  expect(manifest.display).toBe("standalone");
  expect(manifest.background_color).toBe(shellColour);
  expect(manifest.theme_color).toBe(shellColour);
  expect(manifest.icons).toEqual(EXPECTED_ICONS);
});

test("each manifest icon is a PNG of its stated size", async ({ page }) => {
  await page.goto("/");
  const manifestUrl = await page
    .locator('link[rel="manifest"]')
    .first()
    .evaluate((link) => link.href);
  const manifestResponse = await page.request.get(manifestUrl);
  const manifest = await manifestResponse.json();
  expect(manifest.icons).toEqual(EXPECTED_ICONS);
  // This server answers an unknown path with the shell at status 200, so each
  // icon is checked by the bytes it carries, never by status or content type.
  for (const icon of manifest.icons) {
    const iconResponse = await page.request.get(icon.src);
    const bytes = await iconResponse.body();
    expect(bytes.subarray(0, 8), icon.src).toEqual(PNG_SIGNATURE);
    const stated = icon.sizes.split("x").map(Number);
    expect([bytes.readUInt32BE(16), bytes.readUInt32BE(20)], icon.src).toEqual(stated);
  }
});
