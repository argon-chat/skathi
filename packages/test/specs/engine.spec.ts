import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";
import type { Locator } from "../src/locator";

const app = await launch({ app: "hello", window: [1100, 820], label: "engine" });
afterAll(() => app.close());

const T = 30_000;

const idOf = async (loc: Locator) => (await loc.resolve()).node.id;

const pickAt = async (x: number, y: number) => {
  await app.send("mouse", { x, y, kind: "move" });
  return app.send("pick");
};

describe("hover is pointer geometry", () => {
  test(
    "a keystroke does not put the hover out",
    async () => {
      const tab = app.locator(".tab").nth(1);
      await expect(tab).toHaveText("List");

      await tab.hover();
      await expect(tab).toBeHovered();
      await expect(tab).toHaveEngineStyle("background", "a: 0.13333334");

      await app.press("a");
      await app.press("Shift+Tab");

      await expect(tab).toBeHovered();
      await expect(tab).toHaveEngineStyle("background", "a: 0.13333334");

      await tab.unhover();
      await expect(tab).not.toBeHovered();
      await expect(tab).toHaveEngineStyle("background", "a: 0.050980393");
    },
    T,
  );

  test(
    "the same holds for a rung on a different shape",
    async () => {
      // `.btn` names only `background-color` in its `transition`, so the animator owns half the rung
      // and gpui's paint-time ladder the other half.
      const btn = app.locator(".store-row .btn").first();
      await btn.hover();
      await expect(btn).toHaveEngineStyle("background", "a: 0.8");
      await app.press("b");
      await expect(btn).toBeHovered();
      await expect(btn).toHaveEngineStyle("background", "a: 0.8");
      await btn.unhover();
      await expect(btn).toHaveEngineStyle("background", "a: 0.34901962");
    },
    T,
  );
});

describe("pick is gpui's hit test", () => {
  test(
    "a rotated box is hit where it IS, not where its rectangle is",
    async () => {
      await app.locator(".tab").nth(2).click();
      const card = app.locator(".tx-card");
      await expect(card).toContainText("rotate");
      const id = await idOf(card);
      const { x, y, w, h } = await card.boundingBox();

      // The painted shape is that rectangle turned 15° about its centre, so the two disagree at the
      // corners in both directions.
      const A = (15 * Math.PI) / 180;
      const cx = x + w / 2;
      const cy = y + h / 2;
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

      bunExpect((await pickAt(cx, cy)).path).toContain(id);

      const [ox, oy] = fwd(w / 2 - 4, h / 2 - 4);
      bunExpect(oy).toBeGreaterThan(y + h);
      bunExpect((await pickAt(ox, oy)).path).toContain(id);

      // (2) The mirror image: the RECTANGLE's own bottom-right corner is outside the rotated shape,
      // so the picker must not name the card there.
      const px = x + w - 1;
      const py = y + h - 1;
      const [lx] = back(px, py);
      bunExpect(Math.abs(lx)).toBeGreaterThan(w / 2);
      bunExpect((await pickAt(px, py)).path).not.toContain(id);
    },
    T,
  );

  test(
    "`mouse <selector>` aims through the transform",
    async () => {
      await app.locator(".tab").nth(2).click();
      const card = app.locator(".tx-card");
      const id = await idOf(card);

      const at = await app.send("mouse", { selector: ".tx-card", kind: "move" });
      bunExpect(typeof at.x).toBe("number");
      bunExpect((await app.send("pick")).path).toContain(id);
    },
    T,
  );

  test(
    "an element scaled to nothing is not hittable",
    async () => {
      // `scale(0)` has no inverse, and gpui answers "no hit" for it — the browser's answer too.
      await app.locator(".tab").nth(2).click();
      const spin = app.locator(".tx-spin-box");
      const b = await spin.boundingBox();
      const hit = await pickAt(b.x + b.w / 2, b.y + b.h / 2);
      // Its centre is a fixed point of that rotation, so the picker names it whatever the clock says
      // — which is the property a rectangle test cannot have for an animated transform.
      bunExpect(hit.path).toContain(await idOf(spin));
    },
    T,
  );
});
