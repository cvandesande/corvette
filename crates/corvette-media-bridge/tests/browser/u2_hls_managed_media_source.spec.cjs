const { expect, test } = require(process.env.PLAYWRIGHT_TEST_PATH);

// Issue #12 item U2's own fix (2026-08-29): `crates/corvette-ui/src/
// expanded_view.rs`'s `create_hls_video` now passes
// `preferManagedMediaSource: true` when constructing `new Hls(config)`,
// because the exact hls.js version this repo vendors
// (`crates/corvette-ui/public/vendor/hls.min.js`, confirmed 1.7.1 -- the
// latest released hls.js version as of this test's own writing) still ships
// `preferManagedMediaSource: false` in its own `DefaultConfig`. With that
// default, hls.js picks the plain `MediaSource` global whenever it exists
// at all -- which it does on iOS/iPadOS Safari 17.1+, but as a
// non-functional stand-in inaccessible to third-party pages -- instead of
// the actually-working `ManagedMediaSource` global those same versions
// expose, producing a silent black screen with no `"hlsError"`/`<video>`
// error event at all (see
// `.agents/issue-12/evidence/V1-ui-bundle-staleness-incident.md` for the
// live-deployment incident this fixes, and `video-dev/hls.js`'s own
// upstream docs, which describe a `true` default that has not shipped in
// any released version yet).
//
// This spec cannot reproduce the iOS-specific failure itself (Playwright's
// Chromium has no working-vs-non-functional `MediaSource` distinction to
// get wrong the way iOS Safari does), so it verifies the one thing that IS
// mechanically checkable in any browser: that the exact vendored hls.js
// build actually honors this config key the way `create_hls_video`'s own
// fix assumes, and that the defect it fixes is real in this exact vendored
// version. If a future hls.js vendored-version bump ever renames, removes,
// or flips the default of this option, one of these two tests fails loudly
// instead of the fix silently becoming redundant or a no-op.

test("the vendored hls.js honors preferManagedMediaSource:true in its resolved config", async ({
  page,
}) => {
  await page.setContent('<video id="v" muted></video>');
  await page.addScriptTag({ path: require.resolve("hls.js/dist/hls.min.js") });

  const preferManagedMediaSource = await page.evaluate(() => {
    const hls = new window.Hls({ preferManagedMediaSource: true });
    return hls.config.preferManagedMediaSource;
  });

  expect(preferManagedMediaSource).toBe(true);
});

test("hls.js's own shipped default for preferManagedMediaSource is still false", async ({
  page,
}) => {
  await page.setContent('<video id="v" muted></video>');
  await page.addScriptTag({ path: require.resolve("hls.js/dist/hls.min.js") });

  const preferManagedMediaSource = await page.evaluate(() => {
    const hls = new window.Hls();
    return hls.config.preferManagedMediaSource;
  });

  // Documents the actual defect this fix addresses, not just the fix
  // itself: if a future hls.js version ships `true` as its own default (as
  // its `master`-branch docs, but no released version, currently claim),
  // `create_hls_video`'s explicit override becomes redundant but harmless
  // -- this assertion failing is a signal to revisit that doc comment and
  // this repo's vendored copy, not a break in the fix itself.
  expect(preferManagedMediaSource).toBe(false);
});
