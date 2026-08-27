const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);
const net = require("node:net");

// Issue #12 item U2's own real-endpoint Verify step: the expanded/click-to-
// enlarge view's real production client (`crates/corvette-ui/src/
// expanded_view.rs`) races a real `<moq-watch>` connection against a bounded
// timeout, falling back to a real `hls.js` player against
// `corvette-media-bridge`'s own real HLS output
// (`crates/corvette-media-bridge/tests/browser/g3_hls.spec.cjs`'s own
// fixture, `examples/g3_browser_fixture.rs`) when the relay does not answer.
//
// `U2_MOQ_RELAY_URL`, `U2_MOQ_RELAY_CERT_SHA256_HEX`, and `U2_HLS_ORIGIN` are
// read from the environment: `scripts/run_u2_expanded_view_browser_check.sh`
// starts a real pinned `moq-relay` and the real `g3_browser_fixture` on
// ephemeral ports and passes these in here, mirroring
// `U1_LIVE_VIEW_WS_ORIGIN`'s own shape exactly. When unset -- e.g. a plain
// `make check-ui` run, which starts neither fixture -- both tests below are
// skipped rather than failed; the dedicated script above is what actually
// exercises them.
const moqRelayUrl = process.env.U2_MOQ_RELAY_URL;
const moqRelayCertSha256Hex = process.env.U2_MOQ_RELAY_CERT_SHA256_HEX;
const hlsOrigin = process.env.U2_HLS_ORIGIN;
const CAMERA_NAME = "browser-check";

// A real browser's WebTransport implementation has no launch flag or
// DevTools-Protocol override that bypasses certificate validation --
// verified directly: neither `--ignore-certificate-errors` nor
// `--ignore-certificate-errors-spki-list` nor
// `Security.setIgnoreCertificateErrors` changed the outcome; every attempt
// still failed with `net::ERR_QUIC_PROTOCOL_ERROR.QUIC_TLS_CERTIFICATE_UNKNOWN`.
// The one W3C-specified mechanism for an ephemeral, non-CA certificate (which
// is also why `moq-native`'s own `tls.rs` caps a generated cert's lifetime at
// 14 days: "WebTransport certificates MUST be valid for two weeks at most")
// is `serverCertificateHashes`, supplied by the page at
// `new WebTransport(url, options)` call time -- not by the browser's launch
// flags. `<moq-watch>` itself never sets this (a real deployment dials a
// real cert-manager-issued cert per issue #12's own INV-6, so it has no
// reason to), so this patches `window.WebTransport` itself, for this test
// only, to attach it whenever a fingerprint is configured.
const patchWebTransportCertificateHashes = (page, certSha256Hex) => {
  if (!certSha256Hex) {
    return Promise.resolve();
  }
  return page.addInitScript((hex) => {
    const bytes = new Uint8Array(hex.match(/../g).map((byte) => Number.parseInt(byte, 16)));
    const OriginalWebTransport = window.WebTransport;
    window.WebTransport = class extends OriginalWebTransport {
      constructor(url, options = {}) {
        super(url, { ...options, serverCertificateHashes: [{ algorithm: "sha-256", value: bytes.buffer }] });
      }
    };
  }, certSha256Hex);
};

const mockCameraConfig = (page) => {
  return Promise.all([
    page.route("**/api/config", (route) =>
      route.fulfill({
        json: {
          cameras: { [CAMERA_NAME]: { enabled: true, friendly_name: "Browser check", ui: { order: 0 } } },
        },
      }),
    ),
    page.route("**/api/review?*", (route) => route.fulfill({ json: [] })),
    page.route("**/api/review/activity/motion?*", (route) => route.fulfill({ json: [] })),
  ]);
};

// Binds a real, ephemeral TCP listener and immediately releases it, so the
// port handed back is very likely free for the few milliseconds until the
// MoQ connection attempt below tries (and fails) to reach it -- a `connect`
// on a closed local port fails fast (loopback ICMP unreachable), which is
// what makes this test's own bounded window meaningful rather than relying
// on a QUIC-level timeout.
function findAFreeLocalPort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close((error) => (error ? reject(error) : resolve(port)));
    });
    server.on("error", reject);
  });
}

test.describe("relay reachable", () => {
  test("the MoQ path is attempted first and no fallback element is ever mounted", async ({ page }) => {
    test.skip(!moqRelayUrl, "U2_MOQ_RELAY_URL is not set -- run via scripts/run_u2_expanded_view_browser_check.sh");

    const consoleErrors = [];
    page.on("console", (message) => {
      if (message.type() === "error") {
        consoleErrors.push(message.text());
      }
    });

    await patchWebTransportCertificateHashes(page, moqRelayCertSha256Hex);
    await page.addInitScript((url) => {
      window.__corvetteMoqRelayUrl = url;
    }, moqRelayUrl);

    await mockCameraConfig(page);
    await page.goto("/");

    await page.getByRole("button", { name: /expand browser check to full view/i }).click();

    const indicator = page.locator(".live-path-indicator");
    await expect(indicator).toHaveAttribute("data-live-path", "moq", { timeout: 6_000 });

    // The strongest browser-level form of "no fallback element is ever
    // mounted": no `<video>` anywhere inside the expanded player, checked
    // right after settling on the MoQ path.
    await expect(page.locator(".expanded-player video")).toHaveCount(0);
    await expect(page.locator(".expanded-player moq-watch")).toHaveCount(1);

    // Three known-benign lines this harness itself produces, unrelated to
    // this item's own correctness: no favicon is served here, this
    // release bundle still wires cargo-leptos's dev-mode hot-reload
    // WebSocket client (nothing to connect to outside `cargo leptos
    // watch`), and the grid tile behind this modal (U1's own
    // `LiveCameraTile`, unrelated to this item's own expanded view) keeps
    // retrying G2's WebSocket endpoint, which no fixture in this test
    // provides.
    const unexpectedErrors = consoleErrors.filter(
      (line) => !line.includes("favicon") && !line.includes("live_reload") && !line.includes("live/mse/ws"),
    );
    expect(unexpectedErrors, "settled on the MoQ path, but the page logged unexpected console errors").toEqual([]);
  });
});

test("the fallback fires within the bounded timeout and mounts a real HLS video element", async ({ page }) => {
  test.skip(!hlsOrigin, "U2_HLS_ORIGIN is not set -- run via scripts/run_u2_expanded_view_browser_check.sh");

  const deadPort = await findAFreeLocalPort();

  await page.addInitScript(
    ({ moqUrl, origin }) => {
      window.__corvetteMoqRelayUrl = moqUrl;
      window.__corvetteHlsOrigin = origin;
    },
    { moqUrl: `https://127.0.0.1:${deadPort}/anon`, origin: hlsOrigin },
  );

  await mockCameraConfig(page);
  await page.goto("/");

  await page.getByRole("button", { name: /expand browser check to full view/i }).click();

  const indicator = page.locator(".live-path-indicator");
  // Comfortably above `MOQ_ATTEMPT_TIMEOUT_MS` (4s): a closed local port
  // fails immediately, so this bound exists only to catch a genuine
  // regression, not to model real network latency.
  await expect(indicator).toHaveAttribute("data-live-path", "hls", { timeout: 8_000 });

  const video = page.locator(".expanded-player video");
  await expect(video).toHaveCount(1);

  await expect
    .poll(() => video.evaluate((element) => element.readyState), { timeout: 15_000 })
    .toBeGreaterThanOrEqual(2);
});
