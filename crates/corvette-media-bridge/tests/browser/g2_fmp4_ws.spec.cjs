const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Issue #12 item G2's own browser-based Verify step: a real MSE
// `SourceBuffer` accepts this item's own actual output -- the initialization
// segment and moof/mdat fragments `examples/g2_browser_fixture.rs` streams
// over a real WebSocket -- without ever raising a `SourceBuffer`/
// `MediaSource` error, and the resulting `<video>` element reaches
// `HAVE_CURRENT_DATA`.
//
// `G2_FMP4_WS_URL` is read from the environment rather than hardcoded:
// `scripts/run_g2_fmp4_ws_browser_check.sh` starts the fixture on an
// ephemeral port (reading its own bound port back from its stdout) and
// passes the resulting `ws://` URL in here, so this spec never has to
// reserve or guess a fixed port.
const wsUrl = process.env.G2_FMP4_WS_URL;

// `avc1.42C01F`: the exact AVCProfileIndication/profile_compatibility/
// AVCLevelIndication bytes (0x42, 0xC0, 0x1F) of the real SPS
// `examples/g2_browser_fixture.rs` feeds, reused here rather than guessed so
// this codec string is byte-true to what the server actually sends.
const MSE_CODEC = 'video/mp4; codecs="avc1.42C01F"';

test("a real MSE SourceBuffer accepts this item's own fMP4-over-WebSocket output", async ({
  page,
}) => {
  test.skip(!wsUrl, "G2_FMP4_WS_URL is not set -- run via scripts/run_g2_fmp4_ws_browser_check.sh");

  // Only surfaced on failure (via `result.log`, asserted below): a page
  // console error is otherwise silent, and this is often the only place a
  // real MediaSource/decoder failure explains itself in more detail than a
  // bare event.
  const consoleErrors = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      consoleErrors.push(message.text());
    }
  });

  await page.setContent('<video id="v" muted></video>');

  const result = await page.evaluate(
    async ({ wsUrl, mseCodec }) => {
      return new Promise((resolve) => {
        const video = document.getElementById("v");
        const mediaSource = new MediaSource();
        video.src = URL.createObjectURL(mediaSource);

        const pending = [];
        let sourceBuffer = null;
        let settled = false;
        let messagesReceived = 0;

        const finish = (outcome) => {
          if (settled) {
            return;
          }
          settled = true;
          resolve(outcome);
        };

        const appendNext = () => {
          if (!sourceBuffer || sourceBuffer.updating || pending.length === 0) {
            return;
          }
          try {
            sourceBuffer.appendBuffer(pending.shift());
          } catch (error) {
            finish({ ok: false, reason: `appendBuffer threw: ${error}` });
          }
        };

        mediaSource.addEventListener("sourceopen", () => {
          try {
            sourceBuffer = mediaSource.addSourceBuffer(mseCodec);
          } catch (error) {
            finish({ ok: false, reason: `addSourceBuffer threw: ${error}` });
            return;
          }
          // "sequence" mode: play appended segments back-to-back on the
          // browser's own timeline, starting at 0, ignoring each segment's
          // own absolute baseMediaDecodeTime. The server's own tfdt values
          // are relative to when this camera's own repackaging task
          // started (one shared clock feeding every viewer's identical
          // broadcast fragments -- see `fmp4::Fragmenter`'s own doc), not
          // to when any one viewer happened to connect, so a viewer
          // connecting well after camera startup would otherwise see a
          // buffered range that starts at a large, arbitrary offset
          // instead of covering `currentTime` 0 -- exactly the real
          // integration detail a production live-view player (a later,
          // not-yet-built UI item) needs to get right too.
          sourceBuffer.mode = "sequence";
          sourceBuffer.addEventListener("error", () => {
            finish({ ok: false, reason: "SourceBuffer fired an error event" });
          });
          sourceBuffer.addEventListener("updateend", appendNext);
          appendNext();
        });
        mediaSource.addEventListener("error", () => {
          finish({ ok: false, reason: "MediaSource fired an error event" });
        });

        const socket = new WebSocket(wsUrl);
        socket.binaryType = "arraybuffer";
        socket.addEventListener("message", (event) => {
          messagesReceived += 1;
          pending.push(event.data);
          appendNext();
        });
        socket.addEventListener("error", () => {
          finish({ ok: false, reason: "WebSocket fired an error event" });
        });

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

        setTimeout(() => {
          clearInterval(pollInterval);
          finish({
            ok: false,
            reason: `timed out waiting for HAVE_CURRENT_DATA (readyState=${video.readyState}, messagesReceived=${messagesReceived})`,
          });
        }, 15000);
      });
    },
    { wsUrl, mseCodec: MSE_CODEC },
  );

  expect(result.ok, [result.reason, ...consoleErrors].join("\n")).toBe(true);
  expect(result.readyState).toBeGreaterThanOrEqual(2);
  expect(result.messagesReceived).toBeGreaterThan(1);
});
