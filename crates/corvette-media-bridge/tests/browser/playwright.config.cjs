const { defineConfig } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Drives a browser against issue #12 item G2's own real fMP4-over-WebSocket
// output, hosted by `examples/g2_browser_fixture.rs` -- an external process
// `scripts/run_g2_fmp4_ws_browser_check.sh` starts and stops around this run,
// the same shape `tests/nginx-parity/playwright.config.cjs` uses for the
// nginx it drives. There is no `webServer` entry here for the same reason:
// this item's own fixture is a plain Rust binary, not something Playwright's
// `webServer` (built for an HTTP server with a health-check URL) can usefully
// own the lifecycle of, and the driving script already needs to read the
// fixture's own dynamically bound port from its stdout before Playwright can
// even be given a URL to use.
module.exports = defineConfig({
  testDir: __dirname,
  use: {
    browserName: "chromium",
    headless: true,
  },
});
