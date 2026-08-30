// The multi-window spike, driven end to end — docs/MULTIWINDOW.md.
//
//   bun test packages/vn-test/specs/popout.spec.ts
//
// One Vue app, one retained tree, TWO OS windows. The claim this file exists to
// hold down is the one in §0 of the design doc:
//
//   a REAL platform click, dispatched into window B's own `Window`, runs the
//   same JS handler on the same `ref`, and text painted in window A moves.
//
// Nothing here focuses a window or moves the real cursor. The click is
// `__vn_window_click`, which feeds `gpui::PlatformInput` to the pop-out's own
// `Window::dispatch_event` — the same entry point the Win32 message loop uses.
// The devtools `click` command cannot do this: it drives the *published*
// window, which is always the primary one (crates/vn-app/src/popout.rs).

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";

const app = await launch({ app: "popout", window: [620, 460], label: "popout" });
afterAll(() => app.close());

/** Generous: a cold `bun test` pays for the first frame of a real GPU app. */
const T = 30_000;

const popouts = () => app.eval("JSON.parse(__vn_window_popouts())");
const panelId = () => app.eval("document.querySelector('.panel').id");

/** Poll until `probe` is truthy — the pop-out open is deferred by design (§3.2). */
async function until<T>(what: string, probe: () => Promise<T>, ms = 8000): Promise<T> {
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

/** The panel's rectangle in window A, captured before it is popped out. */
let inWindowA: { x: number; y: number; w: number; h: number };

describe("pop-out", () => {
  test("the app mounts with the panel painted in the main window", async () => {
    await expect(app.locator(".panel")).toHaveCount(1);
    await expect(app.locator(".count")).toHaveText("0");
    await expect(app.locator(".mirror")).toContainText("count = 0");
    // Nothing is popped out yet.
    bunExpect(await popouts()).toEqual([]);

    inWindowA = await app.locator(".panel").boundingBox();
    // It is laid out in the 620-wide main window, below the header, the lede,
    // the mirror row and the buttons.
    bunExpect(inWindowA.y).toBeGreaterThan(100);
  }, T);

  test("state is shared before the split, so the two readouts agree", async () => {
    // `.inc-a` is the "+1 (from window A)" button.
    await app.locator(".inc-a").click();
    await expect(app.locator(".count")).toHaveText("1");
    await expect(app.locator(".mirror")).toContainText("count = 1");
  }, T);

  test("popping out opens a SECOND OS window painting that subtree", async () => {
    const id = await panelId();
    bunExpect(typeof id).toBe("number");

    await app.locator(".pop").click();

    const list = await until("the pop-out window to exist", async () => {
      const l = await popouts();
      return l.length === 1 ? l : null;
    });
    bunExpect(list[0].node).toBe(id);
    bunExpect(list[0].windowId).toBeGreaterThan(0);

    // The host said so out loud, once.
    const { entries } = await app.logs(0);
    bunExpect(entries.some((e) => e.text.includes("popped out into window"))).toBe(true);

    // The main window stopped painting it: the hint that only shows while the
    // panel is away is now painted, which it could not be if `out` were false.
    await expect(app.locator(".hint")).toBeVisible();
  }, T);

  test("the subtree left the main window's layout but stayed in the tree", async () => {
    // Still queryable, still styled, still one node — a pop-out is a PAINT-level
    // detachment, not a tree-level one (D-M05).
    await expect(app.locator(".panel")).toHaveCount(1);
    await expect(app.locator(".count")).toHaveText("1");

    // …and its rectangle is now WINDOW B's, not window A's. Window B's client
    // area is 380 wide and `PopoutShell` hands the subtree the whole of it, so
    // the panel is laid out against 380 px rather than against the 620-wide
    // main window's padded content column, and it sits at the very top of its
    // own window instead of below the header and the buttons.
    const box = await app.locator(".panel").boundingBox();
    bunExpect(box.w).toBe(380);
    bunExpect(box.w).not.toBe(inWindowA.w);
    bunExpect(box.y).toBeLessThan(inWindowA.y);
    bunExpect(box.y).toBeLessThan(20);
  }, T);

  test("A REAL CLICK IN WINDOW B UPDATES TEXT IN WINDOW A", async () => {
    const id = await panelId();
    const box = await app.locator(".inc-b").boundingBox();
    const x = box.x + box.w / 2;
    const y = box.y + box.h / 2;

    // Straight into window B's own `Window::dispatch_event` — its hit test, its
    // element tree, its listener. No cursor moves, nothing is focused.
    const hit = await app.eval(`__vn_window_click(${id}, ${x}, ${y})`);
    bunExpect(hit).toBe(true);

    // The counter painted in window B moved…
    await expect(app.locator(".count")).toHaveText("2");
    // …and so did the one painted in window A, off the same reactive wave.
    await expect(app.locator(".mirror")).toContainText("count = 2");

    // Once more, to rule out a one-off.
    await app.eval(`__vn_window_click(${id}, ${x}, ${y})`);
    await expect(app.locator(".mirror")).toContainText("count = 3");
  }, T);

  test("window A still drives the same state while the panel is away", async () => {
    await app.locator(".inc-a").click();
    await expect(app.locator(".mirror")).toContainText("count = 4");
    await expect(app.locator(".count")).toHaveText("4");
  }, T);

  test("closing the window returns the subtree and fires the JS callback", async () => {
    const id = await panelId();
    await app.eval(`__vn_window_close(${id})`);

    await until("the pop-out to be gone", async () => (await popouts()).length === 0);

    // The JS `onClose` callback ran — `out` is false again, so the "bring it
    // back" button is hidden and the hint is gone.
    await expect(app.locator(".hint")).toBeHidden();

    // The panel is painted by the main window again: its box is back inside the
    // 620-wide window, below the header, and it kept its state.
    await expect(app.locator(".count")).toHaveText("4");
    const box = await app.locator(".panel").boundingBox();
    bunExpect(box.y).toBeGreaterThan(60);

    const { entries } = await app.logs(0);
    bunExpect(entries.some((e) => e.text.includes("closed — the subtree is back"))).toBe(true);
  }, T);

  test("a second pop-out of the same node is refused, not queued", async () => {
    const id = await panelId();
    await app.eval(`__vn_window_open(${id}, "{}")`);
    await until("the pop-out to exist", async () => (await popouts()).length === 1);
    // A-M02: the same node id may be painted by exactly one window (§5).
    await app.eval(`__vn_window_open(${id}, "{}")`);
    bunExpect((await popouts()).length).toBe(1);
    await app.eval(`__vn_window_close(${id})`);
    await until("the pop-out to be gone", async () => (await popouts()).length === 0);
  }, T);

  test("nothing in the run was a JS error", async () => {
    await app.expectNoErrors();
  }, T);
});
