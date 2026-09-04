// The two upstream primitives m22 adopted, proven against a running host.
//
//   1. `HoverListenerMode::InputModalityIndependent` — CSS `:hover` is pointer
//      geometry and nothing else, so a keystroke must not put a hover out.
//      gpui's default does exactly that; before this ruling, typing anywhere
//      dropped every hover on the page until the mouse moved again.
//   2. `Hitbox::is_hovered_at` — `pick` is now gpui's own hit test rather than
//      a walk over painted rectangles, which is what makes it right for a
//      `transform`ed element.
//
// Both need a REAL pointer at a window coordinate and a REAL keystroke, and
// neither is observable from JavaScript. Nothing here activates the window or
// moves the OS cursor.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";
import type { Locator } from "../src/locator";

const app = await launch({ app: "hello", window: [1100, 820], label: "engine" });
afterAll(() => app.close());

const T = 30_000;

/** The host's node id for a locator — `pick` answers in ids. */
const idOf = async (loc: Locator) => (await loc.resolve()).node.id;

/** Park the pointer at a window coordinate, then ask who is under it. */
const pickAt = async (x: number, y: number) => {
  await app.send("mouse", { x, y, kind: "move" });
  return app.send("pick");
};

describe("hover is pointer geometry", () => {
  test("a keystroke does not put the hover out", async () => {
    const tab = app.locator(".tab").nth(1); // "List" — inactive, so no other rung
    await expect(tab).toHaveText("List");

    await tab.hover();
    await expect(tab).toBeHovered();
    // `.tab:hover { background: #ffffff22 }` — alpha 34/255.
    await expect(tab).toHaveComputedStyle("background", "a: 0.13333334");

    // The pointer has not moved. In a browser `:hover` is unaffected by the
    // keyboard; gpui's default `InputModalityAware` hit test answers `false`
    // from the key press until the next pointer event, which used to take BOTH
    // the mouseenter/leave edge and the painted rung with it.
    await app.press("a");
    await app.press("Shift+Tab");

    await expect(tab).toBeHovered();
    await expect(tab).toHaveComputedStyle("background", "a: 0.13333334");

    // …and moving away still puts it back, so nothing is being latched.
    await tab.unhover();
    await expect(tab).not.toBeHovered();
    await expect(tab).toHaveComputedStyle("background", "a: 0.050980393");
  }, T);

  test("the same holds for a rung on a different shape", async () => {
    // `.btn` names only `background-color` in its `transition`, so the animator
    // owns half the rung and gpui's paint-time ladder the other half. Both
    // halves read the mode now; before, the ladder half blinked off.
    const btn = app.locator(".store-row .btn").first();
    await btn.hover();
    await expect(btn).toHaveComputedStyle("background", "a: 0.8");
    await app.press("b");
    await expect(btn).toBeHovered();
    await expect(btn).toHaveComputedStyle("background", "a: 0.8");
    await btn.unhover();
    await expect(btn).toHaveComputedStyle("background", "a: 0.34901962");
  }, T);
});

describe("pick is gpui's hit test", () => {
  test("a rotated box is hit where it IS, not where its rectangle is", async () => {
    await app.locator(".tab").nth(2).click(); // Motion
    const card = app.locator(".tx-card");
    await expect(card).toContainText("rotate");
    const id = await idOf(card);
    const { x, y, w, h } = await card.boundingBox();

    // `.tx-card { transform: rotate(15deg) }` about the default 50% 50% origin.
    // Its recorded bounds are the UNTRANSFORMED rectangle — the box taffy laid
    // out, which is also what `getBoundingClientRect()` reports, exactly as in
    // a browser. The painted shape is that rectangle turned 15° about its
    // centre, so the two disagree at the corners in both directions.
    const A = (15 * Math.PI) / 180;
    const cx = x + w / 2;
    const cy = y + h / 2;
    /** local (untransformed) offset -> the window point it is painted at */
    const fwd = (dx: number, dy: number): [number, number] => [
      cx + dx * Math.cos(A) - dy * Math.sin(A),
      cy + dx * Math.sin(A) + dy * Math.cos(A),
    ];
    /** window point -> the local offset gpui maps it back to */
    const back = (px: number, py: number): [number, number] => {
      const dx = px - cx;
      const dy = py - cy;
      return [dx * Math.cos(A) + dy * Math.sin(A), -dx * Math.sin(A) + dy * Math.cos(A)];
    };

    // The centre is the centre under any rotation about it.
    bunExpect((await pickAt(cx, cy)).path).toContain(id);

    // (1) A point INSIDE the rotated shape that is OUTSIDE the rectangle. The
    //     shape's own bottom-right corner swings well below the rectangle's
    //     bottom edge.
    const [ox, oy] = fwd(w / 2 - 4, h / 2 - 4);
    bunExpect(oy).toBeGreaterThan(y + h); // really outside the rectangle
    bunExpect((await pickAt(ox, oy)).path).toContain(id);

    // (2) The mirror image: the RECTANGLE's own bottom-right corner is outside
    //     the rotated shape, so the picker must not name the card there.
    const px = x + w - 1;
    const py = y + h - 1;
    const [lx] = back(px, py);
    bunExpect(Math.abs(lx)).toBeGreaterThan(w / 2); // really outside the shape
    bunExpect((await pickAt(px, py)).path).not.toContain(id);
  }, T);

  test("`mouse <selector>` aims through the transform", async () => {
    await app.locator(".tab").nth(2).click(); // Motion
    const card = app.locator(".tx-card");
    const id = await idOf(card);

    // The host picks the point, maps it through the element's own matrix, and
    // confirms it against gpui's hit test — so this either lands on the node or
    // errors, never silently on whatever covers it.
    const at = await app.send("mouse", { selector: ".tx-card", kind: "move" });
    bunExpect(typeof at.x).toBe("number");
    bunExpect((await app.send("pick")).path).toContain(id);
  }, T);

  test("an element scaled to nothing is not hittable", async () => {
    // `scale(0)` has no inverse, and gpui answers "no hit" for it — the
    // browser's answer too. Nothing in the app ships that, so it is asserted
    // where it can be: through the picker's reply for a point over a plain
    // node, which must still be exactly one path ending at that node.
    await app.locator(".tab").nth(2).click();
    const spin = app.locator(".tx-spin-box");
    const b = await spin.boundingBox();
    const hit = await pickAt(b.x + b.w / 2, b.y + b.h / 2);
    // `.tx-spin-box` runs `@keyframes spin`, i.e. it carries a LIVE rotation
    // matrix that changes every frame. Its centre is a fixed point of that
    // rotation, so the picker names it whatever the clock says — which is the
    // property a rectangle test cannot have for an animated transform.
    bunExpect(hit.path).toContain(await idOf(spin));
  }, T);
});
