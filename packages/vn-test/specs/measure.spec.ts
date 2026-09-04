// A DOM layout read lays out, the way a browser's does.
//
//   bun test packages/vn-test/specs/measure.spec.ts
//
// The regression these pin is `apps/meet-probe`'s join card, which stopped
// painting between m19 and m22 — see research/reports/m22/adopt-merge-and-
// joincard.md. The app measures its own tab panel from `onMounted` + `nextTick`
// and writes the answer back as the wrapper's `height`:
//
//     contentHeight.value = `${active.scrollHeight}px`;
//
// A browser lays out ON READ, so it answers 280 and the card is 280 tall. This
// engine lays out during a frame's prepaint, and none had happened yet — so it
// answered **0**, the app wrote `height: 0px`, the `overflow: hidden` beside it
// clipped the entire card body away, and nothing ever re-measured, because the
// app had its answer. Laid out, hit-testable, painting nothing.
//
// `fixtures/measure-on-mount` is that card in twelve lines. It measures from
// every stage a real app measures from and reports what each one saw.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/measure-on-mount/src/main.ts",
  window: [480, 320],
  label: "measure",
});
afterAll(() => app.close());

const T = 30_000;

/** The fixture's content box: 20 px padding, twice, around two 60 px rows. */
const REAL = 160;

/** `stage=value` for one of the fixture's stage rows. */
const stage = (name: string) => app.locator(`#stage-${name}`);

describe("a layout read lays out", () => {
  test(
    "every stage a Vue app measures from sees the browser's number, not 0",
    async () => {
      // `onMounted` and the `nextTick` after it both run BEFORE the first frame:
      // the app bundle is still evaluating, and Vue drains its microtask queue
      // at the end of that same script. This is the one the join card used.
      await expect(stage("mounted")).toHaveText(`mounted=${REAL}`);
      await expect(stage("nextTick")).toHaveText(`nextTick=${REAL}`);
      // The first `requestAnimationFrame` and the first `setTimeout(0)` fire
      // from the first JS wave, which also runs before a draw — in a browser
      // the document has been laid out long before either.
      await expect(stage("raf")).toHaveText(`raf=${REAL}`);
      await expect(stage("timeout")).toHaveText(`timeout=${REAL}`);
      // …and the control: a stage late enough that a frame has certainly been
      // painted always worked, which is what made this a silent divergence.
      await expect(stage("late")).toHaveText(`late=${REAL}`);
    },
    T,
  );

  test(
    "the measured height is written back and the box is not clipped away",
    async () => {
      // The whole point: the app stores what it measured. A `0` there is not a
      // wrong number in a log line, it is a box with `overflow: hidden` and no
      // height, and every child inside it painting nothing.
      const box = await app.locator("#probe-box").resolve();
      bunExpect(box.node.bounds?.h).toBe(REAL);
      const content = await app.locator("#probe-content").resolve();
      bunExpect(content.node.bounds?.h).toBe(REAL);
    },
    T,
  );

  test(
    "the boot viewport reaches JS",
    async () => {
      // Found by the fix above, and previously masked by it. `innerWidth` /
      // `innerHeight` are PUSHED values — JS cannot pull them — and the FIRST
      // snapshot was never pushed. Nothing noticed because frame 1's snapshot
      // (no painted root yet, so the window's client area) always differed from
      // frame 2's (the root's real box), and frame 2's "resize" corrected it.
      // Once a layout could run before frame 1 the two agreed, and JS kept
      // redjs's built-in default viewport for the life of the process.
      const root = await app.eval(
        "JSON.stringify([innerWidth, innerHeight," +
          " document.documentElement.getBoundingClientRect().width," +
          " document.documentElement.getBoundingClientRect().height])",
      );
      const [iw, ih, rw, rh] = JSON.parse(String(root.value ?? root)) as number[];
      bunExpect(Math.round(iw)).toBe(Math.round(rw));
      bunExpect(Math.round(ih)).toBe(Math.round(rh));
    },
    T,
  );

  test("no errors and no advisories on the way", async () => {
    await app.expectNoErrors();
  }, T);
});
