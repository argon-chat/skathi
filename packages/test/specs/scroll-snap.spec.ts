// CSS scroll snap: each way a box scrolls (wheel, keyboard, scrollbar, script) comes to rest on a
// snap position, and a layout change puts it back on the one it was on.

import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/scroll-snap/src/main.ts",
  window: [900, 700],
  label: "scroll-snap",
  env: { SK_WINDOW_FOCUS: "never" },
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const js = (selector: string, expr: string) =>
  app.eval(`(() => { const el = document.querySelector(${JSON.stringify(selector)}); return ${expr}; })()`);
const left = async (selector: string) => Math.round((await js(selector, "el.scrollLeft")) as number);
const top = async (selector: string) => Math.round((await js(selector, "el.scrollTop")) as number);

/** Waits for a scroll position to stop moving, and answers it. */
async function rest(read: () => Promise<number>, ms = 3_000): Promise<number> {
  const deadline = Date.now() + ms;
  let last = await read();
  let still = 0;
  while (Date.now() < deadline) {
    await sleep(60);
    const now = await read();
    still = now === last ? still + 1 : 0;
    last = now;
    if (still >= 5) break;
  }
  return last;
}

const jump = async (selector: string, axis: "scrollLeft" | "scrollTop", to: number) => {
  await js(selector, `(el.scrollTo({ ${axis === "scrollLeft" ? "left" : "top"}: ${to}, behavior: "instant" }), 0)`);
  await sleep(150);
};

describe("a mandatory carousel (x, center)", () => {
  test(
    "scrollTo lands on the nearest snap position, scrollBy on the next one",
    async () => {
      expect(await js("#carousel", "(el.scrollTo({ left: 600 }), el.scrollLeft)")).toBe(550);
      expect(await js("#carousel", "(el.scrollBy({ left: 10 }), el.scrollLeft)")).toBe(850);
      expect(await js("#carousel", "(el.scrollLeft = 1390, el.scrollLeft)")).toBe(1400);
      await js("#carousel", `(el.scrollTo({ left: 260, behavior: "smooth" }), 0)`);
      expect(await rest(() => left("#carousel"))).toBe(250);
    },
    T,
  );

  test(
    "a touchpad swipe settles on the nearest slide, a wheel notch on the next one",
    async () => {
      await jump("#carousel", "scrollLeft", 0);
      await app.send("wheel", { selector: "#carousel", dx: 200 });
      expect(await rest(() => left("#carousel"))).toBe(250);

      await app.send("wheel", { selector: "#carousel", dx: 1, lines: true });
      expect(await rest(() => left("#carousel"))).toBe(550);
      await app.send("wheel", { selector: "#carousel", dx: -1, lines: true });
      expect(await rest(() => left("#carousel"))).toBe(250);
    },
    T,
  );

  test(
    "arrow keys step slide by slide",
    async () => {
      await jump("#carousel", "scrollLeft", 0);
      await app.locator("#carousel").focus();
      await app.press("ArrowRight");
      expect(await rest(() => left("#carousel"))).toBe(250);
      await app.press("ArrowRight");
      expect(await rest(() => left("#carousel"))).toBe(550);
      await app.press("ArrowLeft");
      expect(await rest(() => left("#carousel"))).toBe(250);
    },
    T,
  );

  test(
    "a layout change keeps the slide it was on",
    async () => {
      await jump("#carousel", "scrollLeft", 850);
      expect(await left("#carousel")).toBe(850);
      // Slide 3's centre moves from 1050 to 700 (200 px slides): 700 - 200.
      await js("#carousel", `(el.classList.add("narrow"), 0)`);
      expect(await rest(() => left("#carousel"))).toBe(500);
      // A wider box: 700 - 250.
      await js("#carousel", `(el.style.setProperty("width", "500px"), 0)`);
      expect(await rest(() => left("#carousel"))).toBe(450);
      await js("#carousel", `(el.classList.remove("narrow"), el.style.removeProperty("width"), 0)`);
      expect(await rest(() => left("#carousel"))).toBe(850);
    },
    T,
  );
});

describe("a proximity list (y)", () => {
  test(
    "near a snap position it snaps, far from one it stays",
    async () => {
      await jump("#near", "scrollTop", 0);
      await app.send("wheel", { selector: "#near", dy: 260 });
      expect(await rest(() => top("#near"))).toBe(300);
      await app.send("wheel", { selector: "#near", dy: 150 });
      expect(await rest(() => top("#near"))).toBe(450);
      expect(await js("#near", "(el.scrollTo({ top: 580 }), el.scrollTop)")).toBe(600);
      expect(await js("#near", "(el.scrollTo({ top: 750 }), el.scrollTop)")).toBe(750);
    },
    T,
  );
});

describe("a mandatory list (y) and the keyboard", () => {
  test(
    "Page Down, the arrows, Page Up, End and Home land on snap positions",
    async () => {
      await jump("#pages", "scrollTop", 0);
      await app.locator("#pages").focus();
      await app.press("PageDown");
      expect(await rest(() => top("#pages"))).toBe(240);
      await app.press("ArrowDown");
      expect(await rest(() => top("#pages"))).toBe(360);
      await app.press("PageUp");
      expect(await rest(() => top("#pages"))).toBe(120);
      await app.press("End");
      expect(await rest(() => top("#pages"))).toBe(900);
      await app.press("Home");
      expect(await rest(() => top("#pages"))).toBe(0);
    },
    T,
  );

  test(
    "a swipe cannot pass `scroll-snap-stop: always`",
    async () => {
      await jump("#pages", "scrollTop", 0);
      await app.send("wheel", { selector: "#pages", dy: 700 });
      expect(await rest(() => top("#pages"))).toBe(480);
    },
    T,
  );

  test(
    "a scrollbar drag snaps on release",
    async () => {
      await jump("#pages", "scrollTop", 0);
      const box = await app.locator("#pages").boundingBox();
      const x = box.x + box.w - 7;
      await app.send("mouse", { x, y: box.y + 20, kind: "move" });
      await sleep(100);
      await app.send("mouse", { x, y: box.y + 20, kind: "down" });
      for (const dy of [20, 45, 70]) {
        await app.send("mouse", { x, y: box.y + 20 + dy, kind: "move" });
        await sleep(40);
      }
      await sleep(300);
      const held = await top("#pages");
      expect(held % 120).not.toBe(0);
      await app.send("mouse", { x, y: box.y + 90, kind: "up" });
      const landed = await rest(() => top("#pages"));
      expect(landed % 120).toBe(0);
      expect(Math.abs(landed - held)).toBeLessThanOrEqual(60);
    },
    T,
  );

  test(
    "scrollIntoView() of a snap area lands on its snap position",
    async () => {
      await jump("#pages", "scrollTop", 0);
      await js("#p6", `(el.scrollIntoView({ block: "center" }), 0)`);
      expect(await rest(() => top("#pages"))).toBe(720);
    },
    T,
  );
});

describe("a bordered scroll container", () => {
  test(
    "snap positions are measured from the padding box, not the border box",
    async () => {
      expect(await js("#framed", "(el.scrollTo({ left: 0 }), el.scrollLeft)")).toBe(0);
      expect(await js("#framed", "(el.scrollTo({ left: 300 }), el.scrollLeft)")).toBe(300);
      expect(await js("#framed", "(el.scrollTo({ left: 600 }), el.scrollLeft)")).toBe(600);
      // Entries start at 80k; 5 px of scroll-margin and 20 of scroll-padding put them at 80k - 25.
      expect(await js("#ledger", "(el.scrollTo({ top: 80 }), el.scrollTop)")).toBe(55);
      expect(await js("#ledger", "(el.scrollTo({ top: 140 }), el.scrollTop)")).toBe(135);
    },
    T,
  );
});

describe("scroll-padding and scroll-margin", () => {
  test(
    "scrollIntoView() aligns the margin box to the padded edge: 40 px of padding, 10 of margin",
    async () => {
      await jump("#padded", "scrollTop", 0);
      await js("#l12", "(el.scrollIntoView(), 0)");
      await sleep(150);
      const gap = await app.eval(
        `Math.round(document.querySelector("#l12").getBoundingClientRect().top - document.querySelector("#padded").getBoundingClientRect().top)`,
      );
      expect(gap).toBe(50);
    },
    T,
  );
});
