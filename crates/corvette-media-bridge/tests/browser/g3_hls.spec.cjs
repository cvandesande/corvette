const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Issue #12 item G3's own browser-based Verify step: a real `hls.js` player
// (the same library U2 vendors for its own expanded-view fallback, per the
// plan's own text) plays this item's own actual output --
// `examples/g3_browser_fixture.rs`'s real playlist and CMAF segments, served
// over a real HTTP listener -- to `HAVE_CURRENT_DATA`, without ever raising a
// fatal `hls.js` error or a `<video>` error event.
//
// `G3_HLS_PLAYLIST_URL` is read from the environment rather than hardcoded:
// `scripts/run_g3_hls_browser_check.sh` starts the fixture on an ephemeral
// port (reading its own bound port back from its stdout) and passes the
// resulting `http://` playlist URL in here, so this spec never has to
// reserve or guess a fixed port -- mirroring `g2_fmp4_ws.spec.cjs`'s own
// `G2_FMP4_WS_URL` convention exactly.
const playlistUrl = process.env.G3_HLS_PLAYLIST_URL;

test("a real hls.js player plays this item's own real HLS output to HAVE_CURRENT_DATA", async ({
  page,
}) => {
  test.skip(
    !playlistUrl,
    "G3_HLS_PLAYLIST_URL is not set -- run via scripts/run_g3_hls_browser_check.sh",
  );

  // Only surfaced on failure (via `result.reason`, asserted below): a page
  // console error is otherwise silent, and this is often the only place a
  // real hls.js/decoder failure explains itself in more detail than a bare
  // event.
  const consoleErrors = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      consoleErrors.push(message.text());
    }
  });

  await page.setContent('<video id="v" muted></video>');
  // Injects hls.js's own UMD bundle as an inline script (not a `<script
  // src>` the page's own CSP/origin would have to fetch), matching how a
  // real vendored copy would ship in the built UI -- exercises this item's
  // own server against the identical library U2 will use, not a stand-in.
  await page.addScriptTag({ path: require.resolve("hls.js/dist/hls.min.js") });

  const result = await page.evaluate(async (url) => {
    return new Promise((resolve) => {
      const video = document.getElementById("v");
      let settled = false;
      let messagesReceived = 0;

      const finish = (outcome) => {
        if (settled) {
          return;
        }
        settled = true;
        resolve(outcome);
      };

      if (!window.Hls || !window.Hls.isSupported()) {
        finish({ ok: false, reason: "Hls.isSupported() returned false in this browser" });
        return;
      }

      const hls = new window.Hls();
      hls.on(window.Hls.Events.FRAG_LOADED, () => {
        messagesReceived += 1;
      });
      hls.on(window.Hls.Events.ERROR, (_event, data) => {
        if (data.fatal) {
          finish({ ok: false, reason: `hls.js fatal error: ${data.type}/${data.details}` });
        }
      });
      hls.loadSource(url);
      hls.attachMedia(video);

      video.addEventListener("error", () => {
        const mediaError = video.error;
        finish({
          ok: false,
          reason: `<video> fired an error event: code=${mediaError && mediaError.code} message=${mediaError && mediaError.message}`,
        });
      });

      const checkReady = () => {
        if (video.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA) {
          finish({ ok: true, readyState: video.readyState, messagesReceived });
        }
      };
      video.addEventListener("loadeddata", checkReady);
      video.addEventListener("canplay", checkReady);
      // A muted video may autoplay; playback is what makes a real browser
      // actually invoke its decoder rather than only inspecting metadata.
      video.play().catch(() => {});
      const pollInterval = setInterval(checkReady, 50);

      // Generous relative to G2's own 15s: this item's own first segment is
      // not available until roughly the 2-second target segment duration
      // has elapsed (see `hls`'s own module doc), plus hls.js's own
      // manifest-load and playlist-refresh cadence on top of that.
      setTimeout(() => {
        clearInterval(pollInterval);
        finish({
          ok: false,
          reason: `timed out waiting for HAVE_CURRENT_DATA (readyState=${video.readyState}, messagesReceived=${messagesReceived})`,
        });
      }, 20000);
    });
  }, playlistUrl);

  expect(result.ok, [result.reason, ...consoleErrors].join("\n")).toBe(true);
  expect(result.readyState).toBeGreaterThanOrEqual(2);
  expect(result.messagesReceived).toBeGreaterThan(0);
});
