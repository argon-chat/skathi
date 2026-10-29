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

// The second half of the same defect, and the one the join card was still
// getting wrong after the first was fixed: the read happened, and it read a
// TRANSITION at t=0. See the describe block at the bottom of this file.
const rem = await launch({
  app: "packages/vn-test/fixtures/measure-rem-transition/src/main.ts",
  window: [480, 360],
  label: "measure-rem",
});
afterAll(() => rem.close());

// The same read in a LATER wave (m26 client sweep #8): the Argon chat list,
// mounted after a fetch. See the describe block "a later wave" below.
const late = await launch({
  app: "packages/vn-test/fixtures/measure-late-mount/src/main.ts",
  window: [480, 700],
  label: "measure-late",
});
const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 0,
  fetch: () => new Response("[]", { headers: { "content-type": "application/json" } }),
});
afterAll(async () => {
  await late.close();
  server.stop(true);
});

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

/**
 * **A `rem` recompile is not an author's style change.**
 *
 * `rem` is flattened to px when the sheet is COMPILED (gpui's style fields are
 * `Pixels`), so an app that sets its own root font-size — which is every
 * Tailwind app with a 14 px base, `apps/meet-probe` included — makes the engine
 * compile the sheet twice. `restyle_all` used to learn the basis by CASCADING
 * the document, and a cascade COMMITS `node.computed`: the pass after the
 * recompile therefore looked to the animator like a style change, and every
 * element carrying a `transition` animated from the 16 px-basis geometry to the
 * real one.
 *
 * No frame ran between the two passes, so nothing on screen showed it. A LAYOUT
 * READ did: `onMounted` measures through `sync_layout`'s forced frame, which is
 * t=0 of that transition. The join card measured its panel at **293 px** where
 * the settled layout says 278 — the padding (1.5rem: 24 not 21), one
 * `space-y-5` margin (20 not 17) and the button's height (3rem: 48 not 42),
 * which is the whole of the residual —9 px `adopt-merge-and-joincard.md` §2.7
 * left standing — and pinned it as its own height for the life of the process.
 *
 * The fixture is those three properties and nothing else. With the fix stashed
 * it reports `mounted=180`, `nextTick=180`, `raf=179` (the transition is
 * already running) and `late=165`; with it, 165 at every stage
 * (research/reports/m23/tails.md §3).
 */
describe("a rem recompile is not a style change", () => {
  /** 42 padding + 60 row + 21 margin + 42 button, all `rem`, at the app's 14 px. */
  const REM_REAL = 165;

  test(
    "a measurement before the first frame is not sampling a transition",
    async () => {
      await expect(rem.locator("#stage-mounted")).toHaveText(`mounted=${REM_REAL}`);
      await expect(rem.locator("#stage-nextTick")).toHaveText(`nextTick=${REM_REAL}`);
      await expect(rem.locator("#stage-raf")).toHaveText(`raf=${REM_REAL}`);
    },
    T,
  );

  test(
    "and it is the settled layout, not merely a consistent wrong number",
    async () => {
      // `late` runs 600 ms in, long after a 300 ms transition would have ended,
      // so the early stages above are being compared against the layout that
      // actually stands rather than against each other.
      await expect(rem.locator("#stage-late")).toHaveText(`late=${REM_REAL}`);
      const panel = await rem.locator("#probe-panel").resolve();
      bunExpect(panel.node.bounds?.h).toBe(REM_REAL);
    },
    T,
  );

  test("no errors and no advisories on the way", async () => {
    await rem.expectNoErrors();
  }, T);
});

/**
 * **A layout read in a later wave lays out too** (m26 client sweep #8).
 *
 * Everything above runs at boot, when the host's forced layout can reach the
 * window through its handle. Every later JS wave runs inside a gpui update that
 * holds the window, so the same read answered **0**. The Argon chat list mounts
 * after a fetch, measured its width in `onMounted`, stored `0px` and collapsed
 * every bubble. The forced layout now draws through the window the wave has
 * published, except under a gpui listener, where it says "stale" instead
 * (docs/RUNTIME.md §5.8).
 *
 * Each list here measures itself in `onMounted` and sizes a bubble at 60% of
 * the width it read.
 */
describe("a later wave", () => {
  /** 8 px padding, twice, around two 24 px rows with a 4 px gap. */
  const LIST_H = 8 + 24 + 4 + 24 + 8;

  /** The list's own painted width, and the one `onMounted` read. */
  async function check(stage: string) {
    await expect(late.locator(`#seen-${stage}`)).toHaveText(/=\d/);
    const list = await late.locator(`#list-${stage}`).resolve();
    const w = Math.round(list.node.bounds?.w ?? 0);
    bunExpect(w).toBeGreaterThan(0);
    await expect(late.locator(`#seen-${stage}`)).toHaveText(`${stage}=${w},${LIST_H},${LIST_H}`);
    // What the app did with the answer: a bubble at 60% of it, not at 0.
    const bubble = await late.locator(`#bubble-${stage}`).resolve();
    bunExpect(Math.round(bubble.node.bounds?.w ?? 0)).toBe(Math.round(w * 0.6));
  }

  test("mounted from a timer", async () => {
    await check("timeout");
  }, T);

  test("mounted after a fetch completes", async () => {
    await late.eval(`__VN_TEST_LATE__.fetchMount(${JSON.stringify(`http://127.0.0.1:${server.port}/chats`)})`);
    await check("fetch");
  }, T);

  test("mounted from a click, synthesized and real", async () => {
    // The devtools' click runs in the frame pump. A real one is delivered
    // after gpui's dispatch of the release (docs/RUNTIME.md §5.6), outside its
    // listeners, so the forced layout may draw in both.
    await late.locator("#mount-click").click();
    await check("click");
    await late.locator("#mount-realclick").click({ real: true });
    await check("realclick");
  }, T);

  test("mounted from a mousedown and a keydown, under gpui's listener", async () => {
    // Both handlers run inside gpui's dispatch, where no frame can be drawn.
    // Their microtask checkpoint, and with it Vue's patch and the list's
    // onMounted, is settled after the dispatch (docs/RUNTIME.md §5.8).
    await late.locator("#mount-down").click({ real: true });
    await check("mousedown");
    await late.locator("#mount-key").press("k");
    await check("keydown");
  }, T);

  test("no errors and no advisories on the way", async () => {
    await late.expectNoErrors();
  }, T);
});
