# hello app assets

## `sticker.tgs`

A Telegram sticker: gzipped Bodymovin JSON with the `"tgs": 1` marker,
512×512, 60 fps, 120 frames (2 s), 555 bytes — inside every one of Telegram's
submission constraints, so it is a faithful `.tgs` and not just "a gzipped
JSON". Used by `src/pages/MotionPage.vue` to demo `<lottie>`.

**Provenance.** The polystar shape and its fill colour are taken verbatim from
`crates/vn-lottie/tests/fixtures/star.json`, which comes from
[`LottieFiles/test-files`](https://github.com/LottieFiles/test-files) and is
public domain (CC0). The radii are scaled ×4 onto the 512×512 canvas, and the
group transform carries an **authored** 0→360° rotation plus a 78%→104%→78%
scale pulse over the 120 frames. Both derivation and addition are CC0.

The rotation is the point: `star.json` — and the `sticker.tgs` in the crate's
fixtures that was derived from it — have **no animated properties at all**
(verified by walking the document for `"a": 1`), so they can demonstrate the
transport but never show motion. The crate's fixture README describing
`star.json` as an "animated transform" is inaccurate.
