const { defineConfig } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Drives a browser against the nginx the parity harness has already started,
// so there is no webServer to launch here. scripts/run_nginx_parity.sh sets
// the address.
module.exports = defineConfig({
  testDir: __dirname,
  use: {
    baseURL: process.env.PARITY_BASE_URL,
    browserName: "chromium",
    headless: true,
  },
});
