import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/measure-on-mount/src/main.ts",
  window: [480, 320],
  label: "measure",
});
afterAll(() => app.close());

const rem = await launch({
  app: "packages/vn-test/fixtures/measure-rem-transition/src/main.ts",
  window: [480, 360],
  label: "measure-rem",
});
afterAll(() => rem.close());

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

const REAL = 160;

const stage = (name: string) => app.locator(`#stage-${name}`);

describe("a layout read lays out", () => {
  test(
    "every stage a Vue app measures from sees the browser's number, not 0",
    async () => {
      // `onMounted` and the `nextTick` after it both run BEFORE the first frame: the app bundle is
      // still evaluating, and Vue drains its microtask queue at the end of that same script.
      await expect(stage("mounted")).toHaveText(`mounted=${REAL}`);
      await expect(stage("nextTick")).toHaveText(`nextTick=${REAL}`);
      // The first `requestAnimationFrame` and the first `setTimeout(0)` fire from the first JS
      // wave, which also runs before a draw — in a browser the document has been laid out long
      // before either.
      await expect(stage("raf")).toHaveText(`raf=${REAL}`);
      await expect(stage("timeout")).toHaveText(`timeout=${REAL}`);
      await expect(stage("late")).toHaveText(`late=${REAL}`);
    },
    T,
  );

  test(
    "the measured height is written back and the box is not clipped away",
    async () => {
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
      // `innerWidth` / `innerHeight` are PUSHED values — JS cannot pull them — and the FIRST
      // snapshot was never pushed.
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

  test(
    "no errors and no advisories on the way",
    async () => {
      await app.expectNoErrors();
    },
    T,
  );
});

describe("a rem recompile is not a style change", () => {
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
      // `late` runs 600 ms in, long after a 300 ms transition would have ended, so the early
      // stages above are being compared against the layout that actually stands rather than
      // against each other.
      await expect(rem.locator("#stage-late")).toHaveText(`late=${REM_REAL}`);
      const panel = await rem.locator("#probe-panel").resolve();
      bunExpect(panel.node.bounds?.h).toBe(REM_REAL);
    },
    T,
  );

  test(
    "no errors and no advisories on the way",
    async () => {
      await rem.expectNoErrors();
    },
    T,
  );
});

/**
 * Every later JS wave runs inside a gpui update that holds the window, so the same read answered
 * **0**.
 */
describe("a later wave", () => {
  const LIST_H = 8 + 24 + 4 + 24 + 8;

  async function check(stage: string) {
    await expect(late.locator(`#seen-${stage}`)).toHaveText(/=\d/);
    const list = await late.locator(`#list-${stage}`).resolve();
    const w = Math.round(list.node.bounds?.w ?? 0);
    bunExpect(w).toBeGreaterThan(0);
    await expect(late.locator(`#seen-${stage}`)).toHaveText(`${stage}=${w},${LIST_H},${LIST_H}`);
    const bubble = await late.locator(`#bubble-${stage}`).resolve();
    bunExpect(Math.round(bubble.node.bounds?.w ?? 0)).toBe(Math.round(w * 0.6));
  }

  test(
    "mounted from a timer",
    async () => {
      await check("timeout");
    },
    T,
  );

  test(
    "mounted after a fetch completes",
    async () => {
      await late.eval(`__VN_TEST_LATE__.fetchMount(${JSON.stringify(`http://127.0.0.1:${server.port}/chats`)})`);
      await check("fetch");
    },
    T,
  );

  test(
    "mounted from a click, synthesized and real",
    async () => {
      await late.locator("#mount-click").click();
      await check("click");
      await late.locator("#mount-realclick").click({ real: true });
      await check("realclick");
    },
    T,
  );

  test(
    "mounted from a mousedown and a keydown, under gpui's listener",
    async () => {
      // Both handlers run inside gpui's dispatch, where no frame can be drawn.
      await late.locator("#mount-down").click({ real: true });
      await check("mousedown");
      await late.locator("#mount-key").press("k");
      await check("keydown");
    },
    T,
  );

  test(
    "no errors and no advisories on the way",
    async () => {
      await late.expectNoErrors();
    },
    T,
  );
});
