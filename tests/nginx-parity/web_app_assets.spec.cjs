const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// A missing file under the catch-all location is answered with the shell at
// status 200, so every check here reads the body, not only the status.

const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

test("the web app manifest arrives as never-cached JSON", async ({ request }) => {
  const response = await request.get("/manifest.json");
  expect(response.status()).toBe(200);
  expect(response.headers()["content-type"]).toMatch(/^application\/json/);
  expect(response.headers()["cache-control"]).toContain("no-store");
  const manifest = await response.json();
  expect(manifest).toHaveProperty("start_url");
  expect(manifest.icons).toHaveLength(3);
});

for (const icon of ["icon-192.png", "icon-512.png", "icon-512-maskable.png"]) {
  test(`the icon /icons/${icon} arrives as PNG bytes`, async ({ request }) => {
    const response = await request.get(`/icons/${icon}`);
    expect(response.status()).toBe(200);
    expect(response.headers()["content-type"]).toMatch(/^image\/png/);
    const body = await response.body();
    expect([...body.subarray(0, 8)]).toEqual(PNG_SIGNATURE);
  });
}
