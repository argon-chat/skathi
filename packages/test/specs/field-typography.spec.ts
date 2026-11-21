import { afterAll, expect, setDefaultTimeout, test } from "bun:test";
import { type Bounds, launch } from "../src/index";

setDefaultTimeout(30_000);
const app = await launch({
  app: "packages/test/fixtures/field-typography/src/main.ts",
  window: [640, 640],
  label: "field-typography",
});
afterAll(() => app.close());

async function box(id: string): Promise<Bounds> {
  const bounds = (await app.query("#" + id)).nodes[0]?.bounds;
  if (!bounds) throw new Error(id + " has no bounds");
  return bounds;
}

async function waitForBox(id: string, predicate: (bounds: Bounds) => boolean): Promise<Bounds> {
  const deadline = Date.now() + 5_000;
  let bounds = await box(id);
  while (!predicate(bounds) && Date.now() < deadline) {
    await Bun.sleep(30);
    bounds = await box(id);
  }
  expect(predicate(bounds)).toBe(true);
  return bounds;
}

const computed = (id: string, property: string) =>
  app.eval(
    "getComputedStyle(document.getElementById(" +
      JSON.stringify(id) +
      ")).getPropertyValue(" +
      JSON.stringify(property) +
      ")",
  );
const value = (id: string) => app.eval("document.getElementById(" + JSON.stringify(id) + ").value");

test("text input measures the placeholder, shrinks to its minimum, then grows without losing the caret", async () => {
  const empty = await box("empty");
  expect(empty.w).toBeLessThan(20);
  const placeholder = await box("input");
  expect(placeholder.w).toBeGreaterThan(55);
  expect(await computed("input", "field-sizing")).toBe("content");

  const input = app.locator("#input");
  await input.type("ab");
  await waitForBox("input", (bounds) => Math.round(bounds.w) === 44);
  await input.press("ArrowLeft");
  await input.type("MMMMMMMMMMMMMMMM");
  const capped = await waitForBox("input", (bounds) => Math.round(bounds.w) === 112);
  expect(capped.h).toBe(placeholder.h);
  await input.type("!");
  expect(await value("input")).toBe("aMMMMMMMMMMMMMMMM!b");
  expect(await app.eval("document.activeElement.id")).toBe("input");
});

test("textarea ignores rows, adds soft and hard lines, caps its height, and keeps editing at the caret", async () => {
  const area = app.locator("#area");
  const initial = await box("area");
  expect(initial.h).toBeLessThan(35);
  expect(Math.round((await box("bounded-area")).w)).toBe(96);
  expect(Math.round((await box("bounded-area")).h)).toBe(44);
  await area.type("abcdefghijklmnop");
  const wrapped = await waitForBox("area", (bounds) => bounds.h > initial.h && bounds.w >= 111);
  expect(Math.round(wrapped.h - initial.h)).toBe(20);
  await area.press("Enter");
  await area.type("second");
  await area.press("Enter");
  await area.type("third");
  await waitForBox("area", (bounds) => Math.round(bounds.h) === 68);
  expect(Number((await app.send("scroll", { selector: "#area", dy: 1_000 })).scrollTop)).toBeGreaterThan(0);
  await area.press("ArrowLeft");
  await area.type("!");
  expect(await value("area")).toBe("abcdefghijklmnop\nsecond\nthir!d");
  expect(await app.eval("document.activeElement.id")).toBe("area");
  await app.eval('document.getElementById("area").value = ""');
  await waitForBox("area", (bounds) => Math.abs(bounds.w - initial.w) < 1 && Math.abs(bounds.h - initial.h) < 1);
});

test("closed select sizes to the selected label and observes changes", async () => {
  const initial = await box("select");
  const fixed = await box("fixed-select");
  expect(initial.w).toBeLessThan(fixed.w);
  await app.eval('document.getElementById("select").value = "l"');
  await waitForBox("select", (bounds) => bounds.w > initial.w + 20);
  await app.eval('document.getElementById("select").value = "s"');
  await waitForBox("select", (bounds) => Math.abs(bounds.w - initial.w) < 1);
});

test("trimming combines both font edges without changing the distance between lines", async () => {
  const plain = await box("untrimmed");
  const start = await box("cap-start");
  const end = await box("alphabetic-end");
  const both = await box("cap-both");
  const ex = await box("ex-both");
  const twoLines = await box("cap-two-lines");

  expect(await computed("cap-start", "text-box-edge")).toBe("cap alphabetic");
  expect(await computed("cap-both", "text-box-trim")).toBe("trim-both");
  expect(start.h).toBeLessThan(plain.h);
  expect(end.h).toBeLessThan(plain.h);
  expect(both.h).toBeLessThan(Math.min(start.h, end.h));
  expect(Math.abs(plain.h - start.h + (plain.h - end.h) - (plain.h - both.h))).toBeLessThanOrEqual(2);
  expect(ex.h).toBeLessThan(both.h);
  expect(Math.round(twoLines.h - both.h)).toBe(42);
});
