// Nothing here focuses it, moves the cursor or sends global input — every gesture is a devtools
// command, which is why the suite can run while somebody is using the machine.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, launch } from "../src/index";

const app = await launch({ app: "hello", window: [1100, 820], label: "hello" });
afterAll(() => app.close());

const T = 30_000;

const tab = (i: number) => app.locator(".tab").nth(i);
const FORMS = 0;
const MOTION = 2;

describe("hello", () => {
  test(
    "the shell mounted with all eleven tabs",
    async () => {
      await expect(app.locator(".tab")).toHaveCount(11);
      await expect(tab(FORMS)).toHaveText("Forms");
      await expect(tab(MOTION)).toHaveText("Motion");
    },
    T,
  );

  test(
    "clicking a tab moves the active class",
    async () => {
      await expect(tab(FORMS)).toHaveClass("router-link-active");
      await expect(tab(MOTION)).not.toHaveClass("router-link-active");

      await tab(MOTION).click();

      await expect(tab(MOTION)).toHaveClass("router-link-active");
      await expect(tab(FORMS)).not.toHaveClass("router-link-active");
      await expect(app.locator(".card .title")).toContainText("Motion");

      await tab(FORMS).click();
      await expect(tab(FORMS)).toHaveClass("router-link-active");
    },
    T,
  );

  test(
    "the pinia counter increments on click, and every tab sees it",
    async () => {
      const bump = app.locator(".store-row .btn").first();
      await expect(bump).toHaveText("visits++ (0)");

      await bump.click();
      await expect(bump).toHaveText("visits++ (1)");
      await bump.click();
      await bump.click();
      await expect(bump).toHaveText("visits++ (3)");

      await expect(app.locator(".bar .store")).toHaveText("pinia · session · guest · 3");
    },
    T,
  );

  test(
    "typing into the native editor echoes through v-model",
    async () => {
      const field = app.locator(".field").first();
      await field.type("yuuki");

      await expect(field).toHaveText("yuuki");
      await expect(app.locator(".hint").first()).toHaveText('v-model echo: "yuuki" (5 chars)');
    },
    T,
  );

  test(
    "preventDefault() on keydown still cancels a character",
    async () => {
      // A real keystroke goes through the widget's interceptor, so the app's veto has to hold.
      const field = app.locator(".field").first();
      await field.type("7");
      await expect(field).toHaveText("yuuki");
      await expect(app.locator(".hint").nth(4)).toContainText("digits blocked by preventDefault: 1");
    },
    T,
  );

  test(
    "KeepAlive hands the page back with its state",
    async () => {
      const field = app.locator(".field").first();
      const keep = app.locator(".hint.keep");
      const before = Number(/fired (\d+)×/.exec(await keep.textContent())![1]);

      await tab(MOTION).click();
      // The cached subtree is DETACHED — it is not painted, so it has no bounds, which is exactly
      // how `toBeHidden` reads it.
      await expect(field).toBeHidden();

      await tab(FORMS).click();
      await expect(field).toBeVisible();
      await expect(field).toHaveText("yuuki");
      await expect(keep).toContainText(`onActivated fired ${before + 1}×`);
      await expect(app.locator(".store-row .btn").first()).toHaveText("visits++ (3)");
    },
    T,
  );

  test(
    "minimise → restore fires visibilitychange both ways",
    async () => {
      // It now comes from gpui's visibility observer, out of band.
      const user32 = dlopen("user32.dll", {
        ShowWindow: { args: [FFIType.u64, FFIType.i32], returns: FFIType.i32 },
        IsIconic: { args: [FFIType.u64], returns: FFIType.i32 },
      });
      const shot = await app.send("screenshot", {
        path: join(tmpdir(), `sk-hello-hwnd-${process.pid}.png`),
        backend: "render_to_image",
      });
      rmSync(shot.path, { force: true });
      const hwnd = BigInt(shot.hwnd);
      await app.eval(
        "globalThis.__vis = []; " +
          "document.addEventListener('visibilitychange', () => " +
          "__vis.push(document.visibilityState + '/' + document.hidden)); " +
          "for (const k of ['minimize', 'restore']) " +
          "__SK_WINDOW__.appWindow.addEventListener(k, () => __vis.push(k)); 'ok'",
      );
      await app.eval("__sk_window_verb('minimize'); 'ok'");
      const until = Date.now() + 10_000;
      while (!user32.symbols.IsIconic(hwnd) && Date.now() < until) await Bun.sleep(50);
      bunExpect(user32.symbols.IsIconic(hwnd)).not.toBe(0);
      // Give the observer's foreground task its turn before restoring.
      await Bun.sleep(300);
      user32.symbols.ShowWindow(hwnd, 4 /* SW_SHOWNOACTIVATE */);
      let seen: string[] = [];
      while (Date.now() < until + 10_000) {
        seen = JSON.parse(await app.eval("JSON.stringify(__vis)"));
        if (seen.length >= 4) break;
        await Bun.sleep(50);
      }
      bunExpect(seen).toEqual(["hidden/true", "minimize", "visible/false", "restore"]);
      bunExpect(await app.eval("document.visibilityState")).toBe("visible");
      user32.close();
    },
    T,
  );

  test(
    "the app logged nothing broken",
    async () => {
      await app.expectNoErrors();
    },
    T,
  );
});
