// Svelte 5's `<SkWindow>` (`@skathi-native/runtime/svelte`): the pop-out lifecycle, props and callback props of the
// Vue component, and `appWindow` state reads that re-run runes. The same app runs from `sk dev`'s build and from a
// packaged production bundle, which compiles the runtime's own .svelte file into the app.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { join } from "node:path";
import { expect, featureExe, launch, packagedExe, REPO, type SkApp } from "../src/index";

const ENTRY = "packages/test/fixtures/svelte-window/src/main.ts";
const T = 30_000;
const BUILD_T = 20 * 60_000;

type Popout = { node: number; windowId: number; title: string; width: number; height: number };

async function until<T>(what: string, probe: () => Promise<T>, ms = 8000): Promise<NonNullable<T>> {
  const deadline = Date.now() + ms;
  let last: unknown;
  for (;;) {
    const v = await probe();
    if (v) return v;
    last = v;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what} (last: ${JSON.stringify(last)})`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

const popouts = (app: SkApp): Promise<Popout[]> => app.eval("JSON.parse(__sk_window_popouts())");
const setTitle = (app: SkApp, title: string) =>
  app.eval(`__SK_WINDOW__.appWindow.title = ${JSON.stringify(title)}; "ok"`);
const outerWidth = async (app: SkApp): Promise<number> =>
  Number(await app.eval("String(__SK_WINDOW__.appWindow.getBounds().width)"));
const setWidth = (app: SkApp, width: number) =>
  app.eval(`__SK_WINDOW__.appWindow.setBounds({ width: ${width} }); "ok"`);

async function popOut(app: SkApp): Promise<Popout> {
  await app.locator(".pop").click();
  const [palette] = await until("the palette window to exist", async () => {
    const l = await popouts(app);
    return l.length === 1 ? l : null;
  });
  return palette!;
}

describe("svelte <SkWindow> under sk dev", () => {
  let app: SkApp;
  let palette: Popout;
  afterAll(() => app?.close());

  test(
    "appWindow reads in a template follow the window",
    async () => {
      app = await launch({ app: ENTRY, window: [620, 460], label: "svelte-window" });
      const before = await outerWidth(app);
      await expect(app.locator(".wwidth")).toHaveText(String(before));
      await setTitle(app, "Svelte main");
      await expect(app.locator(".wtitle")).toHaveText("Svelte main");
      await setWidth(app, before + 40);
      const after = await until("the window to resize", async () => {
        const w = await outerWidth(app);
        return w !== before ? w : null;
      });
      await expect(app.locator(".wwidth")).toHaveText(String(after));
      bunExpect(await popouts(app)).toEqual([]);
    },
    T,
  );

  test(
    "mounting opens a second OS window with the props, callbacks and bindings of the Vue component",
    async () => {
      palette = await popOut(app);
      bunExpect(palette.windowId).toBeGreaterThan(0);
      bunExpect(palette.title).toBe("Palette - Svelte main");
      await expect(app.locator(".open-id")).toHaveText(String(palette.windowId));
      await expect(app.locator(".pal-id")).toHaveText(String(palette.windowId));
      await expect(app.locator(".pal-shown")).toHaveText("1");
      await expect(app.locator(".pal-size")).toHaveText("380x320");
      bunExpect((await app.locator(".pal-root").classes()).sort()).toEqual(["pal-root", "sk-window-root"]);
      bunExpect((await app.send("windows")).windows[1].root).toBe(palette.node);
      bunExpect(await app.eval("__SK_TEST_MAPPED__()")).toMatch(/^\d+(\.\d+)?x\d+(\.\d+)?$/);
    },
    T,
  );

  test(
    "a click in the pop-out runs the Svelte handler, and both windows show the state",
    async () => {
      const box = await app.locator(".inc-b").boundingBox();
      bunExpect(await app.eval(`__sk_window_click(${palette.node}, ${box.x + box.w / 2}, ${box.y + box.h / 2})`)).toBe(
        true,
      );
      await expect(app.locator(".count")).toHaveText("1");
      await expect(app.locator(".count-b")).toHaveText("1");
    },
    T,
  );

  test(
    "an appWindow read drives a live prop and the pop-out's own content",
    async () => {
      await setTitle(app, "Renamed");
      await until(
        "the caption to follow appWindow.title",
        async () => (await popouts(app))[0]?.title === "Palette - Renamed",
      );
      const before = await outerWidth(app);
      await setWidth(app, before - 40);
      const after = await until("the window to resize", async () => {
        const w = await outerWidth(app);
        return w !== before ? w : null;
      });
      await expect(app.locator(".pwidth")).toHaveText(String(after));
    },
    T,
  );

  test(
    "unmounting closes the window and calls onclose once, before the component is gone",
    async () => {
      await app.locator(".back").click();
      await until("the pop-out to be gone", async () => (await popouts(app)).length === 0);
      await expect(app.locator(".pal-closed")).toHaveText("1");
      await expect(app.locator(".panel")).toHaveCount(0);
      const { entries } = await app.logs(0);
      bunExpect(entries.some((e) => e.text.includes("closed — the subtree is back"))).toBe(true);
    },
    T,
  );

  test(
    "a close from the host calls onclose once and unmounts through the app's own state",
    async () => {
      const again = await popOut(app);
      bunExpect(again.title).toBe("Palette - Renamed");
      await app.eval(`__sk_window_close(${again.node}); "ok"`);
      await until("the pop-out to be gone", async () => (await popouts(app)).length === 0);
      await expect(app.locator(".pal-closed")).toHaveText("2");
      await expect(app.locator(".pal-id")).toHaveText("0");
      await expect(app.locator(".panel")).toHaveCount(0);
      await app.expectNoErrors();
    },
    T,
  );

  test(
    "a later mount() does not take the app's teardown: a reload unmounts both",
    async () => {
      await app.eval("__SK_TEST_MOUNT_EXTRA__()");
      await expect(app.locator(".extra")).toHaveCount(1);
      await app.eval('__SK_TEARDOWN__(); "ok"');
      bunExpect(await app.eval("JSON.stringify(__SK_TEST_DESTROYED__ ?? [])")).toBe('["extra","app"]');
    },
    T,
  );
});

describe("svelte <SkWindow> in a packaged build", () => {
  test(
    "the production bundle pops out, reacts to appWindow and closes",
    async () => {
      const host = await featureExe({ features: ["prod"] });
      const packed = await packagedExe({
        app: join(REPO, "packages/test/fixtures/svelte-window"),
        host,
        label: "svelte-window-prod",
      });
      const app = await launch({
        app: ENTRY,
        exe: packed.exe,
        packaged: packed.name,
        window: [620, 460],
        label: "svelte-window-prod",
      });
      try {
        const palette = await popOut(app);
        await expect(app.locator(".open-id")).toHaveText(String(palette.windowId));
        await setTitle(app, "Packaged");
        await expect(app.locator(".wtitle")).toHaveText("Packaged");
        await until("the caption to follow", async () => (await popouts(app))[0]?.title === "Palette - Packaged");
        await app.locator(".back").click();
        await until("the pop-out to be gone", async () => (await popouts(app)).length === 0);
        await expect(app.locator(".pal-closed")).toHaveText("1");
        await app.expectNoErrors();
      } finally {
        await app.close();
      }
    },
    BUILD_T,
  );
});
