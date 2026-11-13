// In a browser the trap's `focusout` handler takes the focus back before the trigger hears
// anything.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/dialog-escape/src/main.ts",
  window: [420, 320],
  label: "dialog-escape",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const state = async () =>
  JSON.parse(
    String(
      await app.eval(
        "JSON.stringify({ log: __VN_TEST_ESCAPE__.log, layers: __VN_TEST_ESCAPE__.layers, active: __VN_TEST_ESCAPE__.active() })",
      ),
    ),
  ) as { log: string[]; layers: string[]; active: string };

const sinceDialog = (log: string[]) => log.slice(log.findIndex((l) => l.startsWith("scope:dialog remembers")));

async function openDialog(): Promise<void> {
  await app.eval("__VN_TEST_ESCAPE__.log.length = 0");
  await app.locator("#tag").focus();
  for (let i = 0; i < 50 && (await state()).active !== "tag"; i++) await sleep(50);
  bunExpect((await state()).active).toBe("tag");
  await app.locator("#tag").press("Menu");
  await expect(app.locator("#menu")).toHaveCount(1);
  await app.locator("#report").click({ real: true });
  await expect(app.locator("#dialog")).toHaveCount(1);
  await expect(app.locator("#menu")).toHaveCount(0);
  await sleep(300);
}

describe("a dialog opened from a context menu", () => {
  test(
    "the focus handed back to the menu's trigger never reaches it, and Escape closes the dialog",
    async () => {
      await openDialog();
      const open = await state();
      bunExpect(open.log).toContain("scope:menu remembers tag");
      const after = sinceDialog(open.log);
      bunExpect(after).toContain("scope:menu returns to tag");
      bunExpect(after.filter((l) => l.endsWith(":tag"))).toEqual([]);
      bunExpect(open.layers).toEqual(["dialog"]);
      bunExpect(open.active).toBe("dlg-btn");
      await expect(app.locator("#tip")).toHaveCount(0);

      await app.press("Escape");
      await expect(app.locator("#dialog")).toHaveCount(0);
      bunExpect((await state()).log).toContain("escape:dialog");
    },
    T,
  );

  test(
    "a press on a focusable button outside the trap: the button hears nothing, and the focus stays inside",
    async () => {
      await openDialog();
      await app.eval("__VN_TEST_ESCAPE__.log.length = 0");
      await app.locator("#outside").click({ real: true });
      await sleep(300);
      const s = await state();
      bunExpect(s.log.filter((l) => l.endsWith(":outside"))).toEqual([]);
      bunExpect(s.active).toBe("dlg-btn");
      await app.press("Escape");
      await expect(app.locator("#dialog")).toHaveCount(0);
    },
    T,
  );
});
