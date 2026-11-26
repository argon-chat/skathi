import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { artifactPath, launch, readPng, SCREENSHOTS, WINDOWS, type Bounds, type SkApp } from "../src/index";

type MenuWindow = { id: number; kind: string; owner?: number; screen: Bounds };
async function windows(app: SkApp) {
  return (await app.send("windows")).windows as MenuWindow[];
}
async function popups(app: SkApp, count: number) {
  const deadline = Date.now() + 5000;
  let open: MenuWindow[];
  do {
    open = (await windows(app)).filter((w) => w.kind === "popup");
    if (open.length === count) return open;
    await Bun.sleep(40);
  } while (Date.now() < deadline);
  throw new Error(
    `Expected ${count} popup windows, found ${open.length}. ${JSON.stringify((await app.logs()).entries.slice(-6))}`,
  );
}
for (const [framework, entry] of [
  ["Vue", "main"],
  ["Svelte", "svelte"],
] as const) {
  describe(framework + " native context menu", () => {
    let app: SkApp;
    beforeAll(async () => {
      app = await launch({
        app: "packages/test/fixtures/context-menu/src/" + entry + ".ts",
        label: "context-menu-" + entry,
        window: [300, 140],
      });
    }, 120_000);
    afterAll(async () => {
      await app?.close();
    });

    test("opens at the pointer, handles disabled items, and selects from a native submenu", async () => {
      await app.eval('__SK_WINDOW__.appWindow.theme = "light"');
      const button = app.locator("button").nth(1);
      const bounds = await button.boundingBox();
      const x = Math.round(bounds.x + 4),
        y = Math.round(bounds.y + bounds.h - 3);
      await app.send("mouse", { x, y, kind: "click", button: 2 });
      const [popup] = await popups(app, 1);
      const main = (await windows(app)).find((w) => w.kind === "main")!;
      expect(popup!.screen.w).toBeGreaterThan(150);
      expect(popup!.screen.h).toBeGreaterThan(96);
      if (WINDOWS) {
        expect(Math.abs(popup!.screen.x - main.screen.x - x)).toBeLessThanOrEqual(2);
        expect(Math.abs(popup!.screen.y - main.screen.y - y)).toBeLessThanOrEqual(2);
        expect(popup!.screen.y + popup!.screen.h).toBeGreaterThan(main.screen.y + main.screen.h);
      }
      await app.press("ArrowDown");
      expect(await app.eval('document.querySelector("[data-active]").textContent')).toBe("Open");
      if (SCREENSHOTS) {
        const light = artifactPath("context-menu-light-" + entry, "png");
        const dark = artifactPath("context-menu-dark-" + entry, "png");
        await app.send("screenshot", { window: popup!.id, path: light });
        await app.eval('__SK_WINDOW__.appWindow.theme = "dark"');
        const sample = (path: string) => readPng(path).at(80, 50)[0];
        const deadline = Date.now() + 4000;
        do {
          await Bun.sleep(50);
          await app.send("screenshot", { window: popup!.id, path: dark });
        } while (sample(light) - sample(dark) < 60 && Date.now() < deadline);
        expect(sample(light) - sample(dark)).toBeGreaterThanOrEqual(60);
      }
      await app.press("ArrowDown");
      await app.press("ArrowRight");
      expect(await popups(app, 2)).toHaveLength(2);
      await app.press("Escape");
      expect(await popups(app, 1)).toHaveLength(1);
      await app.press("ArrowRight");
      const nested = (await popups(app, 2)).find((p) => p.id !== popup!.id)!;
      expect(nested.owner).toBe(popup!.id);
      await app.send("mouse", { window: nested.id, x: 45, y: 16, kind: "click", button: 0 });
      await popups(app, 0);
      expect(await app.eval('document.querySelector("output").textContent')).toBe("Copied Draft.txt");
    }, 30_000);

    test("opens a submenu on hover and ignores a disabled action", async () => {
      await app.locator("button").first().click({ real: true });
      const [menu] = await popups(app, 1);
      await app.send("mouse", { window: menu!.id, x: 32, y: 50, kind: "click" });
      await popups(app, 1);
      await app.send("mouse", { window: menu!.id, x: 32, y: menu!.screen.h - 18, kind: "move" });
      const nested = (await popups(app, 2)).find((p) => p.owner === menu!.id);
      expect(nested).toBeDefined();
      await app.send("mouse", { window: nested!.id, x: 32, y: 16, kind: "click" });
      await popups(app, 0);
      expect(await app.eval('document.querySelector("output").textContent')).toBe("Copied Notes.txt");
    }, 30_000);

    test("resizes for reactive content and belongs to the window containing the anchor", async () => {
      await app.eval("showDetached(true)");
      await app.locator("#detached-trigger").waitFor();
      const owner = (await windows(app)).find((win) => win.kind === "popout");
      expect(owner).toBeDefined();
      await app.send("mouse", { window: owner!.id, kind: "click", x: 20, y: 10 });
      const [menu] = await popups(app, 1);
      expect(menu!.owner).toBe(owner!.id);
      await app.eval("resizeMenu()");
      let resized = menu!;
      const until = Date.now() + 3000;
      while (resized.screen.w <= menu!.screen.w && Date.now() < until) {
        await Bun.sleep(40);
        resized = (await popups(app, 1))[0]!;
      }
      expect(resized.screen.w).toBeGreaterThan(menu!.screen.w);
      expect(resized.screen.h).toBeGreaterThan(menu!.screen.h);
      await app.eval("showDetached(false); resizeMenu()");
      await popups(app, 0);
      expect(await app.eval('document.querySelector("[role=menu]") === null')).toBe(true);
    }, 30_000);

    test("opens at the focused element, retains focus, and dismisses on an outside press", async () => {
      expect(
        await app.eval(
          'const trigger = document.querySelector("button"); trigger.focus(); document.activeElement === trigger',
        ),
      ).toBe(true);
      await app.press("Shift+F10");
      await popups(app, 1);
      expect(await app.eval('document.querySelector("[data-active]").textContent')).toBe("Open");
      await app.press("Enter");
      await popups(app, 0);
      expect(await app.eval('document.querySelector("output").textContent')).toBe("Opened Notes.txt");
      expect(await app.eval('document.activeElement === document.querySelector("button")')).toBe(true);
      await app.locator("button").nth(1).click({ real: true });
      await popups(app, 1);
      await app.send("mouse", { x: 285, y: 18, kind: "click" });
      await popups(app, 0);
      await app.locator("button").first().click({ real: true });
      await popups(app, 1);
      await app.eval("closeMenu()");
      await popups(app, 0);
    }, 30_000);
  });
}
