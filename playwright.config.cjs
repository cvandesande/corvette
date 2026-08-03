const { defineConfig } = require(process.env.PLAYWRIGHT_TEST_PATH);

module.exports = defineConfig({
  testDir: "tests/ui",
  use: {
    baseURL: "http://127.0.0.1:18080",
    browserName: "chromium",
    headless: true,
  },
  webServer: {
    command:
      "LEPTOS_SITE_ADDR=127.0.0.1:18080 LEPTOS_RELOAD_PORT=18081 " +
      "LEPTOS_SITE_ROOT=target/site target/release/corvette-ui-server",
    url: "http://127.0.0.1:18080",
    reuseExistingServer: false,
  },
});
