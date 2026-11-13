// So the assertions here are deliberately the ones an app would make: write it, read it back on
// the next line, and check the pixels moved.

import { afterAll, describe, test } from "bun:test";
import { expect, launch } from "../src/index";

const T = 60_000;
const LIST = "#list";

const app = await launch({
  app: "listperf",
  window: [1600, 1000],
  label: "scroll",
  env: { VN_SCENARIO: "1000" },
});
afterAll(() => app.close());

const settle = (ms = 400) => new Promise((r) => setTimeout(r, ms));

const onList = (js: string) =>
  app.eval(
    `(() => { const l = document.querySelector(${JSON.stringify(LIST)}); return ${js}; })()`,
  );

const topOf = (row: number) =>
  app.eval(
    `Math.round(document.querySelector('[data-row="${row}"]').getBoundingClientRect().top)`,
  );

const reset = async () => {
  await onList("(l.scrollTop = 0, l.scrollTop)");
  await settle(200);
};

describe("el.scrollTop / scrollLeft", () => {
  test("a write moves the box and the getter answers it on the next line", async () => {
    await reset();
    expect(await onList("(l.scrollTop = 1234, l.scrollTop)")).toBe(1234);
    expect(await onList("l.scrollLeft")).toBe(0);

    await settle();
    const zero = await topOf(0);
    expect(zero).toBeLessThan(-1000);
  }, T);

  test("a write past either end clamps, exactly as a wheel would", async () => {
    await reset();
    const max = ((await onList("l.scrollHeight")) as number) - ((await onList("l.clientHeight")) as number);
    expect(await onList("(l.scrollTop = 1e9, l.scrollTop)")).toBe(max);
    expect(await onList("(l.scrollTop = -500, l.scrollTop)")).toBe(0);
  }, T);

  test("scrollHeight is the CONTENT box, so scroll-to-bottom reaches the bottom", async () => {
    await reset();
    const content = (await onList("l.scrollHeight")) as number;
    const viewport = (await onList("l.clientHeight")) as number;
    expect(content).toBeGreaterThan(20_000);
    expect(viewport).toBeLessThan(1000);

    const landed = (await onList("(l.scrollTop = l.scrollHeight, l.scrollTop)")) as number;
    expect(landed).toBe(content - viewport);
    await settle();
    const last = await topOf(999);
    expect(last).toBeGreaterThan(0);
    expect(last).toBeLessThan(1000);
  }, T);

  test("a content size read in the same wave as a DOM change lays out; under a gpui listener it SAYS it is a frame old", async () => {
    const row0 = `document.querySelector('[data-row="0"]').style`;
    await reset();
    const before = (await onList("l.scrollHeight")) as number;
    const grown = (await onList(
      `(${row0}.setProperty("height", "4000px"), ${row0}.setProperty("flex-shrink", "0"), l.scrollHeight)`,
    )) as number;
    expect(grown).toBeGreaterThan(before + 3000);

    await onList(
      `(globalThis.__vnSameWave = null, l.addEventListener("keydown", (e) => {
        if (e.key !== "x" || globalThis.__vnSameWave !== null) return;
        ${row0}.removeProperty("height");
        globalThis.__vnSameWave = l.scrollHeight;
      }), 0)`,
    );
    await app.locator(LIST).focus();
    await app.locator(LIST).press("x");
    let sameWave: unknown = null;
    for (let i = 0; i < 40 && sameWave === null; i++) {
      sameWave = await app.eval("globalThis.__vnSameWave");
      if (sameWave === null) await settle(50);
    }
    expect(sameWave).toBe(grown);

    const { entries } = await app.logs();
    const advised = entries.filter(
      (e) => typeof e.text === "string" && e.text.includes("LAST PAINTED frame"),
    );
    expect(advised.length).toBe(1);
    expect(advised[0].text).toContain("requestAnimationFrame");

    // …and after the frame the number is exact, which is what the advisory tells the app to wait
    // for.
    await settle();
    expect(await onList("l.scrollHeight")).toBe(before);
    await onList(`(${row0}.removeProperty("flex-shrink"), 0)`);
  }, T);

  test("the DOM `scroll` event fires — once, on the next frame", async () => {
    // Asynchronously, because CSSOM-View fires it from the update-the-rendering loop and not from
    // the assignment.
    await reset();
    await onList("(l.scrollTop = 4321, 0)");
    await expect(app.locator("#pos")).toContainText("scrollTop 4321");
  }, T);

  test("scrollTo and scrollBy take both spellings, and leave a named axis alone", async () => {
    await reset();
    await onList("(l.scrollTo({ top: 600 }), 0)");
    expect(await onList("l.scrollTop")).toBe(600);
    await onList("(l.scrollBy({ top: 150 }), 0)");
    expect(await onList("l.scrollTop")).toBe(750);
    await onList("(l.scrollTo(0, 90), 0)");
    expect(await onList("l.scrollTop")).toBe(90);
    await onList("(l.scrollTo({ top: 300 }), 0)");
    expect(await onList("[l.scrollLeft, l.scrollTop]")).toEqual([0, 300]);
  }, T);

  test("an element that does not scroll reports zeros and swallows a write", async () => {
    // A browser's behaviour exactly, which is why it earns no advisory.
    const head = `document.querySelector(".head")`;
    expect(await app.eval(`(${head}.scrollTop = 500, ${head}.scrollTop)`)).toBe(0);
  }, T);
});

describe("overflow-y: auto alone (css-overflow-3 §3.1)", () => {
  test("the x axis computes to auto and scrolls a too-wide child, by script and by wheel", async () => {
    await app.eval(`(() => {
      const box = document.createElement("div");
      box.id = "pair";
      box.setAttribute("style", "position: absolute; left: 20px; top: 20px; width: 200px; height: 120px; overflow-y: auto");
      const wide = document.createElement("div");
      wide.setAttribute("style", "width: 600px; height: 60px; background-color: #c33");
      box.appendChild(wide);
      document.body.appendChild(box);
      return 0;
    })()`);
    await settle();
    const pair = `document.querySelector("#pair")`;
    expect(await app.eval(`[getComputedStyle(${pair}).overflowX, getComputedStyle(${pair}).overflowY]`)).toEqual([
      "auto",
      "auto",
    ]);
    expect(await app.eval(`${pair}.scrollWidth`)).toBe(600);
    expect(await app.eval(`(${pair}.scrollLeft = 150, ${pair}.scrollLeft)`)).toBe(150);
    await app.eval(`(${pair}.scrollLeft = 0, 0)`);
    await settle();
    await app.send("wheel", { selector: "#pair", dx: 120 });
    await settle();
    expect(await app.eval(`${pair}.scrollLeft`)).toBeGreaterThan(0);
    await app.eval(`(${pair}.remove(), 0)`);
    await settle();
  }, T);
});

describe("el.scrollIntoView()", () => {
  test("a row far below the fold comes to the top", async () => {
    // The off-screen half matters: row 900 is ~22 500 px down and has never been painted, but the
    // whole document is laid out, so it has an exact rectangle to aim at.
    await reset();
    const listTop = (await onList("Math.round(l.getBoundingClientRect().top)")) as number;

    await app.eval(`document.querySelector('[data-row="900"]').scrollIntoView()`);
    expect(await topOf(900)).toBe(listTop);
    expect(await onList("l.scrollTop")).toBeGreaterThan(20_000);

    await settle();
    expect(await topOf(900)).toBe(listTop);
  }, T);

  test("block: center and block: end put it where they say", async () => {
    await reset();
    const box = await app.eval(
      `(() => { const r = document.querySelector(${JSON.stringify(LIST)}).getBoundingClientRect();
        return [Math.round(r.top), Math.round(r.height)]; })()`,
    );
    const [top, height] = box as [number, number];

    await app.eval(`document.querySelector('[data-row="500"]').scrollIntoView({ block: "center" })`);
    const centred = await app.eval(
      `(() => { const r = document.querySelector('[data-row="500"]').getBoundingClientRect();
        return Math.round(r.top + r.height / 2); })()`,
    );
    expect(Math.abs((centred as number) - (top + height / 2))).toBeLessThanOrEqual(1);

    await app.eval(`document.querySelector('[data-row="100"]').scrollIntoView({ block: "end" })`);
    const bottom = await app.eval(
      `Math.round(document.querySelector('[data-row="100"]').getBoundingClientRect().bottom)`,
    );
    expect(Math.abs((bottom as number) - (top + height))).toBeLessThanOrEqual(1);
  }, T);

  test("block: nearest leaves a row that is already visible alone", async () => {
    await app.eval(`document.querySelector('[data-row="100"]').scrollIntoView({ block: "end" })`);
    const before = await onList("l.scrollTop");
    await app.eval(`document.querySelector('[data-row="100"]').scrollIntoView({ block: "nearest" })`);
    expect(await onList("l.scrollTop")).toBe(before);
  }, T);
});

describe("what is advisory'd rather than done", () => {
  test('behavior: "smooth" jumps to the exact position and says so, once', async () => {
    await reset();
    await onList('(l.scrollTo({ top: 2000, behavior: "smooth" }), 0)');
    expect(await onList("l.scrollTop")).toBe(2000);
    await onList('(l.scrollBy({ top: 100, behavior: "smooth" }), 0)');
    await app.eval(`document.querySelector('[data-row="10"]').scrollIntoView({ behavior: "smooth" })`);
    await settle(200);

    const { entries } = await app.logs();
    const smooth = entries.filter(
      (e) => typeof e.text === "string" && e.text.includes("scroll behavior"),
    );
    expect(smooth.length).toBe(1);
    expect(smooth[0].text).toContain("INSTANTLY");
  }, T);

  test("nothing else in this app logs an error", async () => {
    // Advisories are opt-in here on purpose: the `smooth` one above is expected.
    await app.expectNoErrors();
  }, T);
});
