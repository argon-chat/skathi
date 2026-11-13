import { afterAll, describe, test } from "bun:test";
import { expect, launch } from "../src/index";
import type { VnApp } from "../src/index";

const ROWS = 1000;
const T = 60_000;
const LIST = "#list";

const env = { VN_SCENARIO: String(ROWS) };

const cached = await launch({ app: "listperf", window: [1600, 1000], label: "retained-on", env });
const plain = await launch({
  app: "listperf",
  window: [1600, 1000],
  label: "retained-off",
  env: { ...env, VN_NO_CACHE: "1" },
});
afterAll(() => Promise.all([cached.close(), plain.close()]));

const settle = (ms = 400) => new Promise((r) => setTimeout(r, ms));

const rowRect = (app: VnApp, i: number) =>
  app.eval(
    `(() => { const r = document.querySelector('[data-row="${i}"]').getBoundingClientRect();
      return [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)]; })()`,
  );

const scrollTop = (app: VnApp) =>
  app.eval(`document.querySelector(${JSON.stringify(LIST)}).scrollTop`);

describe("retained-render cache", () => {
  test("a long list lowers to the same boxes with the cache on and off", async () => {
    await settle();
    expect(await cached.eval(`document.querySelectorAll(".row").length`)).toBe(ROWS);
    expect(await plain.eval(`document.querySelectorAll(".row").length`)).toBe(ROWS);

    // The last one is the interesting case: it is laid out and never painted, and a cached
    // boundary has to keep reporting it.
    for (const i of [0, 3, 40, 900, ROWS - 1]) {
      expect(await rowRect(cached, i)).toEqual(await rowRect(plain, i));
    }

    const tag = (app: VnApp) =>
      app.eval(
        `(() => { const r = document.querySelector('[data-row="3"] .tag').getBoundingClientRect();
          return [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)]; })()`,
      );
    expect(await tag(cached)).toEqual(await tag(plain));
  }, T);

  test("an off-screen row answers boundingBox through the devtools wire too", async () => {
    // A replayed subtree runs no bounds probe, so the map is refilled from the recording — and if
    // that ever stops happening, this is the assertion that notices.
    const far = await cached.locator(`[data-row="900"]`).boundingBox();
    const farPlain = await plain.locator(`[data-row="900"]`).boundingBox();
    expect(far.w).toBe(farPlain.w);
    expect(far.h).toBe(farPlain.h);
    expect(Math.round(far.y)).toBe(Math.round(farPlain.y));
    expect(far.y).toBeGreaterThan(1000);
  }, T);

  test("scrolling to the bottom moves both instances the same way", async () => {
    // `scroll` is a DELTA through the container's own `ScrollHandle` — the same path
    // `el.scrollBy()` would take — so this is 22 000 px down, i.e. past the end of a 25 000 px
    // list.
    for (const app of [cached, plain]) {
      await app.send("scroll", { selector: LIST, dy: 22_000 });
    }
    await settle();

    const a = await scrollTop(cached);
    const b = await scrollTop(plain);
    expect(a).toBeGreaterThan(1000);
    expect(a).toBe(b);

    expect(await rowRect(cached, 900)).toEqual(await rowRect(plain, 900));
  }, T);

  test("a row scrolled into view still takes a click", async () => {
    const visible = await cached.eval(
      `(() => { const box = document.querySelector(${JSON.stringify(LIST)}).getBoundingClientRect();
        const rows = Array.from(document.querySelectorAll(".row"));
        const hit = rows.find((el) => { const r = el.getBoundingClientRect();
          return r.y > box.y + 20 && r.y + r.height < box.y + box.height - 20; });
        return hit ? Number(hit.getAttribute("data-row")) : -1; })()`,
    );
    expect(visible).toBeGreaterThan(0);

    await cached.locator(`[data-row="${visible}"]`).click();
    await settle(300);
    expect(await cached.eval(`document.querySelector("#clicked").textContent.trim()`)).toBe(
      `clicked ${visible}`,
    );
  }, T);

  test("the whole scroll container is cached, and still routes its wheel", async () => {
    const chain = async (app: VnApp) => (await app.send("perf")).scrollChain as number;
    expect(await chain(cached)).toBe(await chain(plain));
    expect(await chain(cached)).toBeGreaterThan(0);

    // Two: a REAL wheel, through gpui's hit test, over the cached box.
    for (const app of [cached, plain]) {
      await app.eval(`document.querySelector(${JSON.stringify(LIST)}).scrollTop = 0`);
    }
    await settle();
    const box = await cached.locator(LIST).boundingBox();
    await cached.send("wheel", { x: box.x + box.w / 2, y: box.y + box.h / 2, dx: 0, dy: 400 });
    await settle();

    expect(await scrollTop(cached)).toBeGreaterThan(0);
    await expect(cached.locator("#pos")).toContainText(
      `scrollTop ${Math.round((await scrollTop(cached)) as number)}`,
    );
  }, T);

  test("neither instance logs an error or a new advisory", async () => {
    await cached.expectNoErrors();
    await plain.expectNoErrors();
  }, T);
});
