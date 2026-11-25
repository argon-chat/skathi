// A context menu in a pop-out: `focusable={false}`, `taskbar={false}` and placed with `mapFromMain()`.
// On macOS the clicks are AppKit events queued on the app (`__sk_window_post_mouse`), so they pass
// through NSApplication and NSWindow, where key state is decided. Key and main state need an active
// app, so on macOS the host is first started allowed to take the foreground; where the session does
// not let it (a locked screen), it is started again in the background and those specs skip.

import { afterAll, describe, expect, test } from "bun:test";
import { launch, MACOS, WINDOWS, type LaunchOptions, type SkApp } from "../src/index";

const options: LaunchOptions = { app: "packages/test/fixtures/popout-menu/src/main.ts", label: "popout-menu" };

if (!MACOS && !WINDOWS) {
  describe("pop-out menu", () => {
    test.skip("Windows and macOS only: mapFromMain() is null and no owner relation exists elsewhere", () => {});
  });
} else {
  let app: SkApp = await launch(MACOS ? { ...options, env: { SK_WINDOW_FOCUS: "normal" } } : options);
  afterAll(() => app.close());

  const T = 30_000;
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  type Popout = {
    node: number;
    title: string;
    shown: boolean;
    foreground: boolean;
    ownedByMain: boolean;
    rect: [number, number, number, number] | null;
    mainRect: [number, number, number, number] | null;
  };
  type Native = { is_key_window: boolean; is_main_window: boolean } & Record<string, unknown>;
  type Rect = { x: number; y: number; width: number; height: number };

  const popouts = (): Promise<Popout[]> => app.eval("JSON.parse(__sk_window_popouts())");
  const menu = async () => (await popouts()).find((p) => p.title === "Menu");
  const state = <T>(js: string): Promise<T> => app.eval(`(() => { const M = __SK_TEST_MENU__; return ${js}; })()`);
  const set = async (js: string) => {
    await app.eval(`(() => { const S = __SK_TEST_MENU__.S; ${js}; return "ok"; })()`);
    await sleep(400);
  };
  async function until<V>(what: string, probe: () => Promise<V>, ms = 8000): Promise<NonNullable<V>> {
    const deadline = Date.now() + ms;
    for (;;) {
      const v = await probe();
      if (v) return v;
      if (Date.now() > deadline) throw new Error(`timed out waiting for ${what} (last: ${JSON.stringify(v)})`);
      await sleep(50);
    }
  }
  async function native(kind: "main" | "Menu" | "Control"): Promise<Native> {
    const { windows } = await app.send("windows", { native: true });
    const w = (windows as { kind: string; title: string; native?: Native }[]).find((w) =>
      kind === "main" ? w.kind === "main" : w.title === kind,
    );
    if (!w?.native) throw new Error(`no native state for ${kind}`);
    return w.native;
  }
  async function press(node: number, selector: string, click: boolean): Promise<void> {
    const box = await app.locator(selector).boundingBox();
    const [x, y] = [box.x + box.w / 2, box.y + box.h / 2];
    const sent = MACOS
      ? await app.eval(`__sk_window_post_mouse(${node}, ${x}, ${y}, "${click ? "click" : "move"}")`)
      : await app.eval(`__sk_window_click(${node}, ${x}, ${y})`);
    expect(sent).toBe(true);
  }
  /** macOS: whether the session lets the app be active, which key and main state need; a locked screen does not. */
  async function becomesActive(): Promise<boolean> {
    const deadline = Date.now() + 3000;
    while (Date.now() < deadline) {
      if ((await native("main")).is_key_window) return true;
      await sleep(100);
    }
    return false;
  }
  const ACTIVE = MACOS && (await becomesActive());
  if (MACOS && !ACTIVE) {
    await app.close();
    app = await launch(options);
  }
  const placed = async () => {
    await state("M.placeBelow() && 'ok'");
    return until("the menu to be shown", async () => {
      const m = await menu();
      return m?.shown ? m : null;
    });
  };

  describe("pop-out menu", () => {
    test(
      "mapFromMain() answers for the hidden menu, and placing by it puts the menu under the anchor",
      async () => {
        const before = await until("the menu to map the anchor", () => state<Rect | null>("M.map()"));
        expect(before.width).toBe(100);
        expect(before.height).toBe(24);
        await placed();
        const after = await until("the menu to be moved", async () => {
          const r = await state<Rect | null>("M.map()");
          return r && Math.abs(r.x) <= 1 && Math.abs(r.y + 24) <= 1 ? r : null;
        });
        expect(after.width).toBe(100);
        if (MACOS) {
          const m = (await menu())!;
          const anchor = await app.locator("#anchor").boundingBox();
          expect(Math.abs(m.rect![0] - m.mainRect![0] - anchor.x)).toBeLessThanOrEqual(1);
        }
      },
      T,
    );

    test(
      "the menu belongs to the main window and is never the foreground window",
      async () => {
        const m = (await menu())!;
        expect(m.ownedByMain).toBe(true);
        expect(m.foreground).toBe(false);
        if (MACOS) {
          const n = await native("Menu");
          expect(n.nonactivating_panel).toBe(true);
          expect(n.can_become_key).toBe(false);
          expect(n.can_become_main).toBe(false);
          expect(n.level).toBe(0);
        }
      },
      T,
    );

    test.skipIf(!MACOS)(
      "a click goes through AppKit to the menu, which does not become key and causes no blur",
      async () => {
        const blurs = await state<number>("M.S.blurs");
        const node = (await menu())!.node;

        await press(node, ".item.two", true);
        await until("the click to land", async () => (await state<number>("M.S.clicks")) === 1);
        expect(await state<string>("M.S.picked")).toBe("two");

        await sleep(300);
        expect((await native("Menu")).is_key_window).toBe(false);
        expect(await state<number>("M.S.blurs")).toBe(blurs);
      },
      T,
    );

    test.skipIf(!ACTIVE)(
      "the main window stays key and main through a click in the menu",
      async () => {
        const blurs = await state<number>("M.S.blurs");
        await press((await menu())!.node, ".item.one", true);
        await until("the click to land", async () => (await state<string>("M.S.picked")) === "one");
        await sleep(300);
        const main = await native("main");
        expect(main.is_key_window).toBe(true);
        expect(main.is_main_window).toBe(true);
        expect(await state<number>("M.S.blurs")).toBe(blurs);
        expect(await app.eval("document.hasFocus()")).toBe(true);
      },
      T,
    );

    test(
      "hover follows the pointer in the menu while it is not the active window",
      async () => {
        await app.locator(".item.one").hover();
        await until("hover on the first item", async () => (await state<string>("M.S.hovered")) === "one");
        await app.locator(".item.two").hover();
        await until("hover on the second item", async () => (await state<string>("M.S.hovered")) === "two");
        if (MACOS) expect((await native("Menu")).is_key_window).toBe(false);
      },
      T,
    );

    test.skipIf(!MACOS)(
      "the menu moves with the main window",
      async () => {
        const m = (await menu())!;
        const [dx, dy] = [40, 30];
        await app.eval(
          `(() => { const b = __SK_WINDOW__.appWindow.getBounds(); __SK_WINDOW__.appWindow.setBounds({ x: b.x + ${dx}, y: b.y + ${dy}, width: b.width, height: b.height }); return "ok"; })()`,
        );
        const moved = await until("the main window to move", async () => {
          const n = await menu();
          return n && Math.abs(n.mainRect![0] - m.mainRect![0] - dx) <= 1 ? n : null;
        });
        expect(Math.abs(moved.mainRect![1] - m.mainRect![1] - dy)).toBeLessThanOrEqual(1);
        expect(Math.abs(moved.rect![0] - m.rect![0] - dx)).toBeLessThanOrEqual(1);
        expect(Math.abs(moved.rect![1] - m.rect![1] - dy)).toBeLessThanOrEqual(1);
        const r = await state<Rect>("M.map()");
        expect(Math.abs(r.x)).toBeLessThanOrEqual(1);
        expect(Math.abs(r.y + 24)).toBeLessThanOrEqual(1);
      },
      T,
    );

    test.skipIf(!MACOS)(
      "the menu hides and comes back with the main window, minimized or hidden",
      async () => {
        for (const [away, back] of [
          ["minimize", "restore"],
          ["hide", "show"],
        ]) {
          await app.eval(`__SK_WINDOW__.appWindow.${away}(), "ok"`);
          await until(`the menu to go with ${away}()`, async () => (await menu())?.shown === false);
          await app.eval(`__SK_WINDOW__.appWindow.${back}(), "ok"`);
          const shown = await until(
            `the menu to come back with ${back}()`,
            async () => {
              const m = await menu();
              return m?.shown ? m : null;
            },
            15_000,
          );
          expect(shown.ownedByMain).toBe(true);
        }
      },
      T,
    );

    test.skipIf(!MACOS)(
      "a hidden menu leaves the main window and stays hidden when the main window is shown again",
      async () => {
        await set("S.visible = false");
        const hidden = await until("the menu to hide", async () => {
          const m = await menu();
          return m && !m.shown ? m : null;
        });
        expect(hidden.ownedByMain).toBe(false);
        await app.eval('__SK_WINDOW__.appWindow.minimize(), "ok"');
        await sleep(800);
        await app.eval('__SK_WINDOW__.appWindow.restore(), "ok"');
        await until("the main window to be back", async () => (await native("main")).is_visible === true, 15_000);
        await sleep(500);
        expect((await menu())!.shown).toBe(false);
        await set("S.visible = true");
        const shown = await until("the menu to be shown again", async () => {
          const m = await menu();
          return m?.shown && m.ownedByMain ? m : null;
        });
        expect(shown.ownedByMain).toBe(true);
      },
      T,
    );

    test.skipIf(!MACOS)(
      "shown while the main window is in fullscreen, the menu joins it and leaves with it",
      async () => {
        await set("S.visible = false");
        await app.eval('__SK_WINDOW__.appWindow.setFullscreen(true), "ok"');
        await until(
          "fullscreen",
          async () => (await app.eval("__SK_WINDOW__.appWindow.isFullscreen()")) === true,
          15_000,
        );
        await sleep(1500);
        await set("S.visible = true");
        const m = await until(
          "the menu to be a shown child",
          async () => {
            const x = await menu();
            return x?.shown && x.ownedByMain ? x : null;
          },
          15_000,
        );
        expect(m.foreground).toBe(false);
        const space = [await native("main"), await native("Menu")].map((n) => n.is_on_active_space);
        expect(space).toEqual([true, true]);
        await app.eval('__SK_WINDOW__.appWindow.setFullscreen(false), "ok"');
        await until(
          "windowed",
          async () => (await app.eval("__SK_WINDOW__.appWindow.isFullscreen()")) === false,
          15_000,
        );
        await sleep(1500);
        const after = (await menu())!;
        expect(after.shown).toBe(true);
        expect(after.ownedByMain).toBe(true);
      },
      60_000,
    );

    test(
      "closing the menu window leaves the main window working",
      async () => {
        await set("S.menuOpen = false");
        await until("the menu to be gone", async () => (await menu()) === undefined);
        if (MACOS) {
          await app.eval('__SK_WINDOW__.appWindow.minimize(), "ok"');
          await sleep(800);
          await app.eval('__SK_WINDOW__.appWindow.restore(), "ok"');
          await until("the main window to be back", async () => (await native("main")).is_visible === true, 15_000);
        }
        await set("S.menuOpen = true");
        await until("the menu to exist again", async () => (await menu()) !== undefined);
      },
      T,
    );

    test.skipIf(!ACTIVE)(
      "control: the same posted click on a focusable pop-out does take key from the main window",
      async () => {
        await set("S.control = true");
        const control = await until("the control window", async () => {
          const c = (await popouts()).find((p) => p.title === "Control");
          return c?.shown ? c : null;
        });
        const blurs = await state<number>("M.S.blurs");
        await press(control.node, ".ctl", true);
        await until("the control window to be key", async () => (await native("Control")).is_key_window === true);
        expect((await native("main")).is_key_window).toBe(false);
        await until("the main window's blur", async () => (await state<number>("M.S.blurs")) > blurs);
        await set("S.control = false");
      },
      T,
    );

    test(
      "nothing in the run was a JS error",
      async () => {
        await app.expectNoErrors();
      },
      T,
    );
  });
}
