// Escape closes a dialog opened from a context menu (docs/RUNTIME.md §5.21,
// packages/vue-native/src/runtime.ts `focusMoved`, crates/vn-tree/src/render.rs
// `focus_transition`).
//
// Argon's report dialog, opened from a member's context menu, did not close on
// Escape. Its menu hands the focus back to what it remembered, a tooltip
// trigger, once its exit animation ends, and by then the dialog is open and
// traps the focus. In a browser the trap's `focusout` handler takes the focus
// back before the trigger hears anything. Here the trigger still heard
// `focus` and `focusin`, so its tooltip opened above the dialog. The tooltip
// was then the highest dismissable layer, and the first Escape went to it.
//
//   bun test packages/vn-test/specs/dialog-escape.spec.ts
//
// Real presses and keys through gpui (devtools `mouse` / `key`).

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

/** What the log holds from the dialog's mount on. */
const sinceDialog = (log: string[]) => log.slice(log.findIndex((l) => l.startsWith("scope:dialog remembers")));

/**
 * The context menu from the keyboard on the focused tooltip trigger, then
 * Report: the dialog is open and the menu gone.
 *
 * In Argon a right press opens the menu. A press's focus is reported here on
 * the next frame, after the `contextmenu` (a browser focuses during
 * `mousedown`), so the menu would remember `body`, not the trigger, and the
 * path under test would not run. The Menu key on a node that already has the
 * focus opens the same menu with the same remembered node, here and in a
 * browser.
 */
async function openDialog(): Promise<void> {
  await app.eval("__VN_TEST_ESCAPE__.log.length = 0");
  await app.locator("#tag").focus();
  for (let i = 0; i < 50 && (await state()).active !== "tag"; i++) await sleep(50);
  bunExpect((await state()).active).toBe("tag");
  await app.locator("#tag").press("Menu");
  await expect(app.locator("#menu")).toHaveCount(1);
  await app.locator("#report").click({ real: true });
  await expect(app.locator("#dialog")).toHaveCount(1);
  // The menu's exit (250 ms), then its FocusScope's `setTimeout(0)`.
  await expect(app.locator("#menu")).toHaveCount(0);
  await sleep(300);
}

describe("a dialog opened from a context menu", () => {
  test("the focus handed back to the menu's trigger never reaches it, and Escape closes the dialog", async () => {
    await openDialog();
    const open = await state();
    // The path under test: the press focused `#tag`, the menu remembered it
    // and, after the dialog opened, handed the focus back to it.
    bunExpect(open.log).toContain("scope:menu remembers tag");
    const after = sinceDialog(open.log);
    bunExpect(after).toContain("scope:menu returns to tag");
    // Chromium: the trap's `focusout` handler wins, `#tag` hears nothing,
    // and the tooltip stays closed.
    bunExpect(after.filter((l) => l.endsWith(":tag"))).toEqual([]);
    bunExpect(open.layers).toEqual(["dialog"]);
    bunExpect(open.active).toBe("dlg-btn");
    await expect(app.locator("#tip")).toHaveCount(0);

    await app.press("Escape");
    await expect(app.locator("#dialog")).toHaveCount(0);
    bunExpect((await state()).log).toContain("escape:dialog");
  }, T);

  test("a press on a focusable button outside the trap: the button hears nothing, and the focus stays inside", async () => {
    await openDialog();
    await app.eval("__VN_TEST_ESCAPE__.log.length = 0");
    await app.locator("#outside").click({ real: true });
    await sleep(300);
    const s = await state();
    // The host's move: `blur` on the dialog's button names `#outside` as its
    // `relatedTarget`, the trap takes the focus back from `focusout`, and
    // the move to `#outside` is abandoned, as in Chromium.
    bunExpect(s.log.filter((l) => l.endsWith(":outside"))).toEqual([]);
    bunExpect(s.active).toBe("dlg-btn");
    await app.press("Escape");
    await expect(app.locator("#dialog")).toHaveCount(0);
  }, T);
});
