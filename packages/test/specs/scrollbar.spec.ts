// Before this feature it scrolled blind: no thumb, no track, nothing that said there was more
// below.

import { afterAll, describe, test } from "bun:test";
import { artifactPath, changedRegion, expect, launch, readPng } from "../src/index";
import type { Rect } from "../src/index";

const app = await launch({ app: "hello", window: [1000, 700], label: "scrollbar" });
afterAll(() => app.close());

const T = 30_000;
const BOX = ".scrollbox";

const settle = (ms = 200) => new Promise((r) => setTimeout(r, ms));

const scrollTop = () => app.eval(`document.querySelector(${JSON.stringify(BOX)}).scrollTop`);

const shot = async (name: string) => {
  const path = await app.screenshot(artifactPath(`scrollbar-${name}`, "png"));
  return readPng(path);
};

/**
 * The strip the vertical bar occupies: the right-hand 16 px of the box, inset top and bottom so
 * the box's own border cannot be mistaken for a thumb.
 */
const trackStrip = (box: { x: number; y: number; w: number; h: number }): Rect => ({
  x: Math.round(box.x + box.w - 16),
  y: Math.round(box.y + 4),
  w: 16,
  h: Math.round(box.h - 8),
});

describe("scrollbars", () => {
  test(
    "an overflow box paints a thumb once it has scrolled",
    async () => {
      await app.locator("a").nth(1).click();
      await settle(700);
      const box = await app.locator(BOX).boundingBox();
      expect(box.w).toBeGreaterThan(100);

      const before = await shot("rest");

      await app.send("wheel", { x: box.x + box.w / 2, y: box.y + box.h / 2, dx: 0, dy: 60 });
      await settle(150);
      const after = await shot("scrolled");

      const region = changedRegion(before, after, 12, trackStrip(box));
      expect(region).not.toBeNull();
      expect(region!.h).toBeGreaterThan(20);
      expect(region!.w).toBeLessThanOrEqual(16);

      expect(await scrollTop()).toBeGreaterThan(0);
    },
    T,
  );

  test(
    "a click on the bare track jumps the box, and fires `scroll`",
    async () => {
      const box = await app.locator(BOX).boundingBox();
      const before = (await scrollTop()) as number;
      await app.send("mouse", { x: box.x + box.w - 7, y: box.y + box.h - 20, kind: "click" });
      await settle(250);
      const after = (await scrollTop()) as number;
      expect(after).toBeGreaterThan(before + 100);
      // The page reports the position out of its own `@scroll` handler, which is the DOM event — a
      // track click never had one to fire before, because there was no wheel to hang it on.
      await expect(app.locator(".card")).toContainText(`scrollTop: ${Math.round(after)}px`);
    },
    T,
  );

  test(
    "dragging the thumb scrolls the box",
    async () => {
      const box = await app.locator(BOX).boundingBox();
      // Back to the top, so the thumb is somewhere known.
      await app.send("mouse", { x: box.x + box.w - 7, y: box.y + 6, kind: "click" });
      await settle(250);
      const before = (await scrollTop()) as number;

      const x = box.x + box.w - 7;
      await app.send("mouse", { x, y: box.y + 20, kind: "down" });
      for (const dy of [20, 60, 100, 140]) {
        await app.send("mouse", { x, y: box.y + 20 + dy, kind: "move" });
        await settle(40);
      }
      await app.send("mouse", { x, y: box.y + 160, kind: "up" });
      await settle(250);

      const after = (await scrollTop()) as number;
      expect(after).toBeGreaterThan(before + 100);
    },
    T,
  );

  test(
    "the wheel still scrolls, and still reaches the app's listeners",
    async () => {
      const box = await app.locator(BOX).boundingBox();
      await app.send("mouse", { x: box.x + box.w - 7, y: box.y + 6, kind: "click" });
      await settle(250);
      const before = (await scrollTop()) as number;
      await app.send("wheel", { x: box.x + box.w / 2, y: box.y + box.h / 2, dx: 0, dy: 90 });
      await settle(200);
      expect((await scrollTop()) as number).toBeGreaterThan(before);
      // If the scrollbar had swallowed the event, this would still read 0.0.
      await expect(app.locator(".card")).toContainText("last wheel ΔY over the card: 90.0");
    },
    T,
  );

  test(
    "`scrollbar-width: none` hides the bar and keeps the scrolling",
    async () => {
      const box = await app.locator(BOX).boundingBox();
      await app.eval(`document.querySelector(${JSON.stringify(BOX)}).style.setProperty("scrollbar-width", "none")`);
      await settle(200);
      const before = await shot("hidden-rest");
      const top = (await scrollTop()) as number;

      await app.send("wheel", { x: box.x + box.w / 2, y: box.y + box.h / 2, dx: 0, dy: 60 });
      await settle(200);
      const after = await shot("hidden-scrolled");

      expect((await scrollTop()) as number).toBeGreaterThan(top);
      expect(changedRegion(before, after, 12, trackStrip(box))).toBeNull();

      await app.eval(`document.querySelector(${JSON.stringify(BOX)}).style.setProperty("scrollbar-width", "auto")`);
      await settle(150);
    },
    T,
  );
});
