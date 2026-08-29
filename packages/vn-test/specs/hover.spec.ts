// `:hover`, proven rather than assumed.
//
// This is the spec that could not be written before the harness existed, and
// the reason `styles` grew a `live` bucket (research/reports/m19/vn-test.md §3).
//
// The hover ladder is decided by gpui's HIT TEST at paint time, not by anything
// JavaScript can be asked. So the gesture has to be a real `PlatformInput` at a
// window coordinate — `locator.hover()`, the `mouse` command — and the readback
// has to be the style in EFFECT, not the rung the cascade produced. `base` and
// `hover` are both there whether or not a pointer is anywhere near; `live` is
// the one that changes when one is.
//
// Nothing here activates the window or moves the OS cursor. The pointer gpui
// hit-tests against is its own `mouse_position`, which is why this runs while
// somebody else is using the mouse.

import { afterAll, describe, test } from "bun:test";
import { expect, launch, parseHsla, styleValue } from "../src/index";

const app = await launch({ app: "hello", window: [1100, 820], label: "hover" });
afterAll(() => app.close());

const T = 30_000;

/** The live alpha of an element's background — the channel both rules move. */
const liveAlpha = async (selector: string, index = 0) => {
  const { computed } = await app.locator(selector).nth(index).styles();
  return parseHsla(styleValue(computed.live ?? [], "background") ?? "")?.a;
};

describe("hover", () => {
  test("the pointer really lands on the element", async () => {
    const tab = app.locator(".tab").nth(1); // "List" — inactive, so no other rung
    await expect(tab).toHaveText("List");
    await expect(tab).not.toBeHovered();

    await tab.hover();

    // gpui saw the pointer cross the hitbox — z-order, content masks and
    // `pointer-events` all had their vote before this could be true.
    await expect(tab).toBeHovered();
  }, T);

  test("the :hover rule lands on the computed style", async () => {
    const tab = app.locator(".tab").nth(1);

    // `.tab { background: #ffffff0d }` -> alpha 13/255.
    await tab.unhover();
    await expect(tab).not.toBeHovered();
    await expect(tab).toHaveComputedStyle("background", "a: 0.050980393");

    await tab.hover();

    // `.tab:hover { background: #ffffff22 }` -> alpha 34/255, arrived at over
    // the 150 ms the base rule's `transition` asks for. The assertion retries,
    // which is exactly why it does not need to know that.
    await expect(tab).toHaveComputedStyle("background", "a: 0.13333334");
    // The same rule also lifts the text to full white.
    await expect(tab).toHaveComputedStyle("color", "a: 1.0");

    // The RUNG was there all along — this is the control that shows `live` is
    // reporting something the static buckets cannot.
    const { computed } = await tab.styles();
    expect(styleValue(computed.hover ?? [], "background")).toContain("a: 0.13333334");
    expect(styleValue(computed.base ?? [], "background")).toContain("a: 0.050980393");
  }, T);

  test("moving away puts it back", async () => {
    const tab = app.locator(".tab").nth(1);
    await tab.hover();
    await expect(tab).toHaveComputedStyle("background", "a: 0.13333334");

    await tab.unhover();

    await expect(tab).not.toBeHovered();
    await expect(tab).toHaveComputedStyle("background", "a: 0.050980393");
  }, T);

  test("only the element under the pointer is hovered", async () => {
    const list = app.locator(".tab").nth(1);
    const motion = app.locator(".tab").nth(2);

    await list.hover();
    await expect(list).toBeHovered();
    await expect(motion).not.toBeHovered();
    expect(await liveAlpha(".tab", 2)).toBeCloseTo(0.050980393, 5);

    // One move, two edges: gpui reports the leave and the enter together.
    await motion.hover();
    await expect(motion).toBeHovered();
    await expect(list).not.toBeHovered();
    await expect(motion).toHaveComputedStyle("background", "a: 0.13333334");
    await expect(list).toHaveComputedStyle("background", "a: 0.050980393");
  }, T);

  test("a hover on a different rule shape works too", async () => {
    // `.btn` in FormsPage: a heavier base, a heavier hover, and a `transition`
    // that names only `background-color` — so the border is snapped by gpui's
    // own ladder while the background is interpolated by the animator. `live`
    // reports the one the animator owns; docs/TESTING.md says why the other is
    // invisible from here.
    const btn = app.locator(".store-row .btn").first();
    await btn.unhover();
    await expect(btn).toHaveComputedStyle("background", "a: 0.34901962");

    await btn.hover();
    await expect(btn).toBeHovered();
    await expect(btn).toHaveComputedStyle("background", "a: 0.8");

    await btn.unhover();
    await expect(btn).toHaveComputedStyle("background", "a: 0.34901962");
  }, T);

  test("hovering is not clicking", async () => {
    // A `mouse move` must not fire the handler. If it did, every `hover()` in
    // this file would have been silently bumping the store.
    const bump = app.locator(".store-row .btn").first();
    const before = await bump.textContent();
    await bump.hover();
    await bump.unhover();
    await expect(bump).toHaveText(before);
  }, T);
});
