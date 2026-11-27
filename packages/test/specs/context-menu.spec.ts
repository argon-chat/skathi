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

    test("a mounted keymap relabels its physical key and handles the same position", async () => {
      expect(await app.eval("__sk_keyScans.KeyQ")).toBe(0x10);
      await app.eval("globalThis.savedKeyLegends = __sk_keyLegends");
      const send = (code: string, selector = "button") =>
        app.eval(`{ const target = document.querySelector(${JSON.stringify(selector)});
        for (const type of ["keydown", "keyup"]) target.dispatchEvent(new KeyboardEvent(type, {
          bubbles: true, cancelable: true, key: "й", code: ${JSON.stringify(code)}, ctrlKey: true
        })); }`);
      try {
        await app.eval('__sk_keyLegends = {KeyQ: "й"}; __sk_keyLayoutChanged()');
        await app.locator('#layout-shortcut[aria-label="Control plus Й"][data-scope="app"]').waitFor();
        await send("KeyF");
        expect(await app.eval('document.querySelector("output").textContent')).not.toBe("Shortcut fired");
        await app.eval("showDetached(true)");
        await app.locator("#detached-trigger").waitFor();
        await send("KeyQ", "#detached-trigger");
        expect(await app.eval('document.querySelector("output").textContent')).toBe("Shortcut fired");
        await app.eval('__sk_keyLegends = {KeyQ: "a"}; __sk_keyLayoutChanged()');
        await app.locator('#layout-shortcut[aria-label="Control plus A"]').waitFor();
      } finally {
        await app.eval("showDetached(false); __sk_keyLegends = savedKeyLegends; __sk_keyLayoutChanged()");
      }
    });

    test("closing a detached window balances its held shortcut", async () => {
      await app.eval("showDetached(true)");
      try {
        await app.locator("#detached-trigger").waitFor();
        await app.eval(`document.querySelector("#detached-trigger").dispatchEvent(new KeyboardEvent("keydown", {
          bubbles: true, key: "k", code: "KeyQ", ctrlKey: true
        }))`);
        expect(await app.eval('document.querySelector("output").textContent')).toBe("Shortcut fired");
      } finally {
        await app.eval("showDetached(false)");
      }
      expect(await app.eval('document.querySelector("output").textContent')).toBe("Shortcut cancelled");
    });

    test("opens at the pointer, handles disabled items, and selects from a native submenu", async () => {
      await app.eval('__SK_WINDOW__.appWindow.theme = "light"');
      const button = app.locator("button").nth(1);
      const bounds = await button.boundingBox();
      const x = Math.round(bounds.x + 4),
        y = Math.round(bounds.y + bounds.h - 3);
      await app.send("mouse", { x, y, kind: "click", button: 2 });
      const [popup] = await popups(app, 1);
      const shortcut = app.locator('[role="menuitem"] kbd[aria-label="F2"]');
      expect(await shortcut.count()).toBe(1);
      expect((await shortcut.boundingBox()).w).toBeGreaterThan(8);
      const main = (await windows(app)).find((w) => w.kind === "main")!;
      expect(popup!.screen.w).toBeGreaterThan(150);
      expect(popup!.screen.h).toBeGreaterThan(96);
      if (WINDOWS) {
        expect(Math.abs(popup!.screen.x - main.screen.x - x)).toBeLessThanOrEqual(2);
        expect(Math.abs(popup!.screen.y - main.screen.y - y)).toBeLessThanOrEqual(2);
        expect(popup!.screen.y + popup!.screen.h).toBeGreaterThan(main.screen.y + main.screen.h);
      }
      // A newly mapped native popup can already have a row under the desktop pointer.
      await app.press("Home");
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
      const copy = await app.locator('[aria-haspopup="menu"]').first().boundingBox();
      await app.send("mouse", { window: menu!.id, x: 32, y: copy.y + copy.h / 2, kind: "move" });
      const nested = (await popups(app, 2)).find((p) => p.owner === menu!.id);
      expect(nested).toBeDefined();
      await app.send("mouse", { window: nested!.id, x: 32, y: 16, kind: "click" });
      await popups(app, 0);
      expect(await app.eval('document.querySelector("output").textContent')).toBe("Copied Notes.txt");
    }, 30_000);

    test("keeps a submenu open across a diagonal and switches when the pointer turns away", async () => {
      for (const flip of WINDOWS ? [false, true] : [false]) {
        try {
          if (WINDOWS) {
            await app.eval(`document.querySelector("button").dispatchEvent(new MouseEvent("contextmenu", {
              bubbles: true, button: 2, clientX: screen.availLeft - screenX + ${flip ? "screen.availWidth - 12" : "24"},
              clientY: screen.availTop - screenY + ${flip ? "screen.availHeight - 3" : "40"}
            }))`);
          } else await app.locator("button").first().click({ real: true });
          const [parent] = await popups(app, 1);
          // Each leg must reach the menu. The host coalesces mouse moves within a frame.
          await app.eval(`{
            const menu = document.querySelector('[aria-label="File actions"]');
            menu.addEventListener("mousemove", e => {
              menu.dataset.pointerAt = Math.round(e.clientX) + "," + Math.round(e.clientY);
            });
          }`);
          const move = async (x: number, y: number) => {
            x = Math.round(x);
            y = Math.round(y);
            await app.send("mouse", { window: parent!.id, kind: "move", x, y });
            await app.locator(`[data-pointer-at="${x},${y}"]`).waitFor();
          };
          const triggers = app.locator('[aria-haspopup="menu"]');
          const copy = await triggers.first().boundingBox();
          const sibling = await triggers.nth(1).boundingBox();
          const startX = copy.x + copy.w / 2;
          const startY = copy.y + copy.h * 0.7;
          const siblingY = sibling.y + sibling.h / 2;
          await move(startX, startY);
          await app.locator('[role="menu"][aria-label="Copy"] [role="menuitem"]').first().waitFor();
          const child = (await popups(app, 2)).find((p) => p.owner === parent!.id)!;
          const left = child.screen.x < parent!.screen.x;
          if (WINDOWS) expect(left).toBe(flip);
          if (flip) expect(child.screen.y).toBeLessThan(parent!.screen.y + copy.y);
          const direction = left ? -1 : 1;
          const nearX = startX + direction * copy.w * 0.18;
          const acrossX = startX + (direction * copy.w) / 3;
          await move(nearX, copy.y + copy.h - 1);
          await move(nearX, sibling.y + 1);
          await move(acrossX, siblingY);
          await move(acrossX - direction, siblingY + 1);
          await move(acrossX - 2 * direction, siblingY);
          await Bun.sleep(250);
          expect(await app.locator('[aria-haspopup="menu"][aria-expanded="true"][data-active]').textContent()).toBe(
            "Copy",
          );
          await app.send("mouse", { window: child.id, kind: "move", x: child.screen.w / 2, y: 18 });
          await app.locator('[aria-label="Copy"] [data-active]').waitFor();

          await move(startX, startY);
          await move(acrossX, siblingY);
          if (flip) {
            await app.send("mouse", { window: parent!.id, x: acrossX, y: siblingY, kind: "click" });
          } else {
            for (const retreat of [2, 4, 8]) await move(acrossX - direction * retreat, siblingY);
          }
          await app.locator('[role="menu"][aria-label="Move"]').waitFor();
          expect(await app.locator('[aria-haspopup="menu"][aria-expanded="true"][data-active]').textContent()).toBe(
            "Move",
          );
        } finally {
          await app.eval("closeMenu()");
          await popups(app, 0);
        }
      }
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
      const prompt = app.locator('kbd[aria-label="Middle mouse button, or Cross button"]');
      expect(await prompt.count()).toBe(1);
      expect(await app.locator('.sk-kbd-key[data-device="mouse"] svg').count()).toBe(1);
      expect(await app.locator('.sk-kbd-key[data-shape="face"] svg').count()).toBe(1);
      await app.eval("showDetached(false); resizeMenu()");
      await popups(app, 0);
      expect(await app.eval('document.querySelector("[role=menu]") === null')).toBe(true);
    }, 30_000);

    test.skipIf(!SCREENSHOTS)(
      "draws the tested safe zone and leaves the missed sample on the parent",
      async () => {
        for (const debug of [false, true]) {
          await app.eval(`showMenuAim(${debug})`);
          try {
            await app.locator("button").first().click({ real: true });
            const [parent] = await popups(app, 1);
            const copy = await app.locator('[aria-haspopup="menu"]').first().boundingBox();
            const next = await app.locator('[aria-haspopup="menu"]').nth(1).boundingBox();
            const first = await app.locator('[role="menuitem"]').first().boundingBox();
            const move = (x: number, y: number) =>
              app.send("mouse", { window: parent!.id, kind: "move", x: Math.round(x), y: Math.round(y) });
            const sample = async (label: string, rgb: number[], at?: { x: number; y: number }) => {
              const path = artifactPath(`context-menu-aim-${entry}-${debug}-${label}`, "png");
              const deadline = Date.now() + 3000;
              do {
                await app.send("screenshot", { window: parent!.id, path });
                const img = readPng(path);
                const sx = img.width / parent!.screen.w,
                  sy = img.height / parent!.screen.h;
                const rect = at
                  ? { x: (at.x - 6) * sx, y: (at.y - 6) * sy, w: 12 * sx, h: 12 * sy }
                  : { x: 0, y: 0, w: img.width, h: img.height };
                const count = img.count(rect, (_, pixel) => pixel.every((v, i) => Math.abs(v - rgb[i]!) < 20));
                if (!debug || label === "tracking" || count >= 5) return count;
                await Bun.sleep(40);
              } while (Date.now() < deadline);
              return 0;
            };
            const startX = copy.x + copy.w / 2;
            await move(startX, copy.y + copy.h * 0.7);
            const child = (await popups(app, 2)).find((w) => w.owner === parent!.id)!;
            // An idle parent must paint the zone before another input or screenshot wakes its window.
            await Bun.sleep(250);
            expect((await sample("tracking", [0, 122, 174])) >= 5).toBe(debug);
            const dir = child.screen.x < parent!.screen.x ? -1 : 1;
            await move(startX + (dir * copy.w) / 3, next.y + 3);
            expect((await sample("held", [0, 131, 106])) >= 5).toBe(debug);
            await app.locator('[aria-label="Copy"][role="menu"]').waitFor();
            const miss = { x: Math.round(startX), y: Math.round(first.y + first.h / 2) };
            await move(miss.x, miss.y);
            await popups(app, 1);
            await move(first.x + 12, miss.y);
            expect((await sample("released", [212, 65, 51], miss)) >= 5).toBe(debug);
            await app.send("mouse", { window: parent!.id, kind: "click", ...miss });
            await popups(app, 0);
            expect(await app.eval('document.querySelector("output").textContent')).toBe("Opened Notes.txt");
          } finally {
            await app.eval("closeMenu(); showMenuAim(false)");
            await popups(app, 0);
          }
        }
      },
      30_000,
    );

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
