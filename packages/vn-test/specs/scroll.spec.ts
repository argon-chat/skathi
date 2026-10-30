// Programmatic scrolling, end to end — `crates/vn-tree/src/scroll.rs`.
//
// Before m23 this whole surface was inert: `scrollTop` and `scrollLeft` were
// plain fields on the JS mirror node, written only when a `scroll` event fired,
// and `scrollIntoView()` was an advisory no-op. Every app that restores a
// position, jumps to an offset or scrolls a log to the bottom did nothing at
// all, silently — which is exactly the failure the compat-first policy exists
// to prevent, and it survived three milestones because nothing asserted it.
//
// So the assertions here are deliberately the ones an app would make: write it,
// read it back on the next line, and check the pixels moved. `apps/listperf` is
// the subject because it is the shape that makes a scroll interesting — 1 000
// rows, 25 000 px of content in a 777 px box, and rows far enough below the
// fold that `scrollIntoView` has real work to do.

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

/** Runs `js` with `l` bound to the list, and JSON round-trips the answer. */
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
    // Synchronously: a `ScrollHandle` is an `Rc`, so there is no frame between
    // the assignment and the read, which is what a browser guarantees and what
    // every "restore the scroll position" helper assumes.
    await reset();
    expect(await onList("(l.scrollTop = 1234, l.scrollTop)")).toBe(1234);
    expect(await onList("l.scrollLeft")).toBe(0);

    // …and the pixels really moved: row 0 is now 1234 px above where it was.
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
    // `el.scrollTop = el.scrollHeight` is THE idiom, and it only works if
    // `scrollHeight` is the content rather than the painted viewport. Reporting
    // the viewport (which is what the rectangle says) stopped a 25 000 px list
    // one screen down and looked like an app bug.
    await reset();
    const content = (await onList("l.scrollHeight")) as number;
    const viewport = (await onList("l.clientHeight")) as number;
    expect(content).toBeGreaterThan(20_000);
    expect(viewport).toBeLessThan(1000);

    const landed = (await onList("(l.scrollTop = l.scrollHeight, l.scrollTop)")) as number;
    expect(landed).toBe(content - viewport);
    await settle();
    // The last row is now inside the box.
    const last = await topOf(999);
    expect(last).toBeGreaterThan(0);
    expect(last).toBeLessThan(1000);
  }, T);

  test("a content size read in the same wave as a DOM change lays out; under a gpui listener it SAYS it is a frame old", async () => {
    // A browser lays out on read. The host forces a layout wherever no gpui
    // dispatch is under the wave (docs/RUNTIME.md §5.8), and a devtools
    // command runs in the frame pump, so a read there is exact. Under a gpui
    // listener (`keydown`, `mousedown`, `wheel`, …) it cannot draw, so a
    // `scrollHeight` read in the same wave as a DOM change answers the last
    // painted frame, and `list.scrollTop = list.scrollHeight` would stop one
    // item short. That is the one honest gap, pinned so it cannot become
    // silent: it leaves an advisory rather than a wrong number nobody is told
    // about.
    //
    // Row 0 also gets `flex-shrink: 0`. It is an item of the column-flex
    // `#list`, so with the default `flex-shrink: 1` and `min-height: auto` a
    // `height: 4000px` shrinks back to its content, ~25 px. Chrome does the
    // same (CSS Flexbox §4.5, §9.7), so without it the row never grows.
    const row0 = `document.querySelector('[data-row="0"]').style`;
    await reset();
    const before = (await onList("l.scrollHeight")) as number;
    const grown = (await onList(
      `(${row0}.setProperty("height", "4000px"), ${row0}.setProperty("flex-shrink", "0"), l.scrollHeight)`,
    )) as number;
    expect(grown).toBeGreaterThan(before + 3000); // laid out on read

    // Under a keydown listener: shrink the row back and read in the handler.
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
    expect(sameWave).toBe(grown); // a frame old, as advertised

    const { entries } = await app.logs();
    const advised = entries.filter(
      (e) => typeof e.text === "string" && e.text.includes("LAST PAINTED frame"),
    );
    expect(advised.length).toBe(1);
    expect(advised[0].text).toContain("requestAnimationFrame");

    // …and after the frame the number is exact, which is what the advisory
    // tells the app to wait for.
    await settle();
    expect(await onList("l.scrollHeight")).toBe(before);
    await onList(`(${row0}.removeProperty("flex-shrink"), 0)`);
  }, T);

  test("the DOM `scroll` event fires — once, on the next frame", async () => {
    // Asynchronously, because CSSOM-View fires it from the update-the-rendering
    // loop and not from the assignment. The app prints the position out of its
    // own `@scroll` handler, so this asserts the event reached Vue.
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
    // An axis the caller did not name keeps its value rather than going to 0.
    await onList("(l.scrollTo({ top: 300 }), 0)");
    expect(await onList("[l.scrollLeft, l.scrollTop]")).toEqual([0, 300]);
  }, T);

  test("an element that does not scroll reports zeros and swallows a write", async () => {
    // A browser's behaviour exactly, which is why it earns no advisory.
    const head = `document.querySelector(".head")`;
    expect(await app.eval(`(${head}.scrollTop = 500, ${head}.scrollTop)`)).toBe(0);
  }, T);
});

describe("el.scrollIntoView()", () => {
  test("a row far below the fold comes to the top", async () => {
    // The off-screen half matters: row 900 is ~22 500 px down and has never
    // been painted, but the whole document is laid out, so it has an exact
    // rectangle to aim at. (That is the property `gpui::list` virtualization
    // was rejected for taking away — see the m23 perf report §7.)
    await reset();
    const listTop = (await onList("Math.round(l.getBoundingClientRect().top)")) as number;

    await app.eval(`document.querySelector('[data-row="900"]').scrollIntoView()`);
    // The rectangle is right in the SAME JS wave, before any frame has been
    // drawn — a browser lays out on read and the host corrects for the
    // un-painted scroll arithmetically (`scroll::unpainted_shift`).
    expect(await topOf(900)).toBe(listTop);
    expect(await onList("l.scrollTop")).toBeGreaterThan(20_000);

    // …and it is still right after the frame lands, i.e. the correction is not
    // applied twice.
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
    // The spec's rule, and the one that makes `scrollIntoView` idempotent —
    // a roving-focus loop calls it on every arrow key.
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
    // The POSITION is exact — which is the half app logic reads back.
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
