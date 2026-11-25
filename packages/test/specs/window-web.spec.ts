// The main window's standard surface the host implements natively: matchMedia lifetimes, screen,
// window geometry, document.title and navigator.windowControlsOverlay.

import { afterAll, describe, expect, test } from "bun:test";
import { launch, LINUX } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/fullscreen/src/main.ts",
  window: [480, 320],
  label: "window-web",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const js = async <V>(src: string): Promise<V> => JSON.parse(await app.eval(`JSON.stringify(${src})`)) as V;
const gc = () => app.send("mem", { gc: true });

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, what: string): Promise<V> {
  const deadline = Date.now() + 10_000;
  let v = await read();
  while (!ok(v)) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}: ${JSON.stringify(v)}`);
    await sleep(50);
    v = await read();
  }
  return v;
}

const setTheme = (theme: string) => app.eval(`__SK_WINDOW__.appWindow.theme = ${JSON.stringify(theme)}; "ok"`);

describe("matchMedia", () => {
  test(
    "a query nobody holds is collected; one with a listener or onchange keeps firing until it has none",
    async () => {
      await setTheme("light");
      await until(() => js<boolean>(`matchMedia("(prefers-color-scheme: light)").matches`), Boolean, "light");
      await app.eval(`(() => {
        globalThis.__mq = { seen: [], abort: new AbortController() };
        __mq.unheld = new WeakRef(matchMedia("(prefers-color-scheme: dark)"));
        matchMedia("(prefers-color-scheme: dark)").addEventListener("change", (e) => __mq.seen.push("listener:" + e.matches));
        matchMedia("(prefers-color-scheme: dark)").onchange = (e) => __mq.seen.push("onchange:" + e.matches);
        matchMedia("(prefers-color-scheme: dark)").addListener((e) => __mq.seen.push("legacy:" + e.matches));
        const aborted = matchMedia("(prefers-color-scheme: dark)");
        aborted.addEventListener("change", () => __mq.seen.push("aborted"), { signal: __mq.abort.signal });
        __mq.aborted = new WeakRef(aborted);
        const once = matchMedia("(prefers-color-scheme: dark)");
        once.addEventListener("change", (e) => __mq.seen.push("once:" + e.matches), { once: true });
        __mq.once = new WeakRef(once);
        return "ok";
      })()`);
      await app.eval(`__mq.abort.abort(); "ok"`);
      // The conservative stack scan can keep a dead query for a collection or two.
      await until(
        async () => {
          await gc();
          return js<boolean>(`__mq.unheld.deref() === undefined && __mq.aborted.deref() === undefined`);
        },
        Boolean,
        "the unheld queries to be collected",
      );

      await setTheme("dark");
      const fired = await until(
        () => js<string[]>(`__mq.seen`),
        (s) => s.length >= 4,
        "change events",
      );
      expect(fired.sort()).toEqual(["legacy:true", "listener:true", "once:true", "onchange:true"]);

      await gc();
      await gc();
      expect(await js<boolean>(`__mq.once.deref() === undefined`)).toBe(true);
      await setTheme("light");
      const back = await until(
        () => js<string[]>(`__mq.seen`),
        (s) => s.length >= 7,
        "change events back",
      );
      expect(back.slice(4).sort()).toEqual(["legacy:false", "listener:false", "onchange:false"]);
      await setTheme("system");
    },
    T,
  );

  test(
    "a query not listened to still reads the current environment",
    async () => {
      await app.eval(`globalThis.__held = matchMedia("(prefers-color-scheme: dark)"); "ok"`);
      await setTheme("dark");
      expect(await until(() => js<boolean>(`__held.matches`), Boolean, "matches")).toBe(true);
      await setTheme("light");
      expect(await until(() => js<boolean>(`!__held.matches`), Boolean, "no match")).toBe(true);
      expect(await js<[boolean, string]>(`[__held instanceof MediaQueryList, __held.media]`)).toEqual([
        true,
        "(prefers-color-scheme: dark)",
      ]);
      await setTheme("system");
    },
    T,
  );
});

describe("the window", () => {
  test(
    "document.title collapses whitespace and appWindow.title reads it back",
    async () => {
      await app.eval(`document.title = "  Two \\t words "; "ok"`);
      expect(await js<[string, string]>(`[document.title, __SK_WINDOW__.appWindow.title]`)).toEqual([
        "Two words",
        "Two words",
      ]);
    },
    T,
  );

  // Linux CI is X11 under a window manager, where a move places the frame and the position read is the client's.
  test.skipIf(LINUX)(
    "moveTo and moveBy place the window, and screenX/screenY follow",
    async () => {
      const work = await js<{ x: number; y: number }>(`({ x: screen.availLeft, y: screen.availTop })`);
      await app.eval(`moveTo(${work.x + 40}, ${work.y + 30}); "ok"`);
      const at = await until(
        () => js<[number, number]>(`[screenX, screenY]`),
        ([x, y]) => Math.abs(x - work.x - 40) <= 1 && Math.abs(y - work.y - 30) <= 1,
        "moveTo",
      );
      await app.eval(`moveBy(25, 15); "ok"`);
      await until(
        () => js<[number, number]>(`[screenLeft, screenTop]`),
        ([x, y]) => Math.abs(x - at[0] - 25) <= 1 && Math.abs(y - at[1] - 15) <= 1,
        "moveBy",
      );
    },
    T,
  );

  test(
    "screen describes the display and is a Screen",
    async () => {
      const s = await js<{
        w: number;
        h: number;
        aw: number;
        ah: number;
        depth: number;
        type: string;
        native: boolean;
      }>(
        `({ w: screen.width, h: screen.height, aw: screen.availWidth, ah: screen.availHeight, depth: screen.colorDepth,
            type: screen.orientation.type, native: screen instanceof Screen && screen.orientation instanceof ScreenOrientation })`,
      );
      expect(s.w).toBeGreaterThan(0);
      expect(s.aw).toBeLessThanOrEqual(s.w);
      expect(s.ah).toBeLessThanOrEqual(s.h);
      expect(s.depth).toBe(24);
      expect(s.type).toBe(s.w >= s.h ? "landscape-primary" : "portrait-primary");
      expect(s.native).toBe(true);
      await app.eval(
        `screen.orientation.lock("portrait").then(() => (globalThis.__lock = "locked"), (e) => (globalThis.__lock = e.name)); "ok"`,
      );
      expect(await until(() => js<string | null>(`globalThis.__lock ?? null`), Boolean, "lock()")).toBe(
        "NotSupportedError",
      );
    },
    T,
  );

  test(
    "windowControlsOverlay reports the title bar area as a DOMRect",
    async () => {
      const wco = await until(
        () =>
          js<{ visible: boolean; rect: boolean; x: number; width: number; height: number; inner: number }>(
            `(() => { const o = navigator.windowControlsOverlay, r = o.getTitlebarAreaRect();
              return { visible: o.visible, rect: r instanceof DOMRect, x: r.x, width: r.width, height: r.height, inner: innerWidth }; })()`,
          ),
        (w) => w.visible,
        "a visible overlay",
      );
      expect(wco.rect).toBe(true);
      expect(wco.height).toBe(40);
      expect(wco.x + wco.width).toBeLessThanOrEqual(wco.inner);
    },
    T,
  );
});
