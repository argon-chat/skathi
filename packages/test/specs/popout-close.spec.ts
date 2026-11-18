// A pop-out closing while the app moves the main window in the same turn, as an app that gives the
// returning panel its room does. The move runs inside the frame, with gpui's App borrowed, and
// Win32 delivers its WM_MOVE/WM_SIZE synchronously: gpui must report them later, not drop them.

import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "popout",
  window: [620, 460],
  label: "popout-close",
  // gpui's own errors reach the log only with SK_LOG.
  env: { SK_LOG: "error" },
});
afterAll(() => app.close());

const popouts = async (): Promise<{ title: string }[]> => app.eval("JSON.parse(__sk_window_popouts())");

async function until(what: string, probe: () => Promise<boolean>, ms = 8000): Promise<void> {
  const deadline = Date.now() + ms;
  while (!(await probe())) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await Bun.sleep(50);
  }
}

test("closing a pop-out while the main window moves drops no move or resize", async () => {
  for (const width of [660, 700, 640]) {
    await app.locator(".pop").click();
    await until("the palette to open", async () => (await popouts()).some((w) => w.title.startsWith("Palette")));
    await app.eval(`(() => {
      document.querySelector(".back").click();
      const b = __SK_WINDOW__.appWindow.getContentBounds();
      __SK_WINDOW__.appWindow.setContentBounds({ x: b.x + 10, width: ${width} });
      return "ok";
    })()`);
    await until("the palette to close", async () => (await popouts()).length === 0);
    await until("the main window to take its new width", async () => {
      const b = JSON.parse(
        await app.eval(`JSON.stringify(__SK_WINDOW__.refresh() && __SK_WINDOW__.appWindow.getContentBounds())`),
      );
      return Math.round(b.width) === width;
    });
  }
  const { entries } = await app.logs(0);
  expect(entries.filter((e) => e.text.includes("RefCell already borrowed")).map((e) => e.text)).toEqual([]);
}, 60_000);
