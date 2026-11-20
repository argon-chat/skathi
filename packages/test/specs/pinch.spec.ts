import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/pinch/src/main.ts",
  window: [400, 320],
  label: "trackpad-pinch",
});
afterAll(() => app.close());

test("trackpad zoom bubbles at the gesture centre, preserves modifiers and leaves scrolling to the app", async () => {
  await app.locator("#probe").waitFor();
  await app.eval(`globalThis.samples = [];
    const box = document.getElementById("scrollbox");
    for (const type of ["gesturestart", "gesturechange", "wheel", "gestureend"]) {
      box.addEventListener(type, e => samples.push([e.type, e.target.id, e instanceof GestureEvent, e.scale,
        e.rotation, e.ctrlKey, e.altKey, e.shiftKey, e.metaKey, e.clientX, e.clientY, e.deltaMode, e.deltaY, e.cancelable]));
    }`);
  const sent = await app.send("pinch", { selector: "#probe", scale: 1.5, alt: true, shift: true, meta: true });
  expect(sent.defaultPrevented).toBe(false);
  const values = JSON.parse(String(await app.eval("JSON.stringify(samples)"))) as (
    string | number | boolean | null
  )[][];
  expect(values.map((v) => v[0])).toEqual(["gesturestart", "gesturechange", "wheel", "gestureend"]);
  for (const entry of values) {
    expect(entry[1]).toBe("probe");
    expect(entry.slice(6, 11)).toEqual([true, true, true, sent.x, sent.y]);
    expect(entry[13]).toBe(true);
  }
  expect(values.filter((v) => v[2]).map((v) => [v[3], v[4], v[5]])).toEqual([
    [1, 0, false],
    [1.5, 0, false],
    [1.5, 0, false],
  ]);
  expect(values[2]![5]).toBe(true);
  expect(values[2]![11]).toBe(0);
  expect(Number(values[2]![12])).toBeCloseTo(-100 * Math.log(1.5), 5);
  expect(await app.eval("document.getElementById('scrollbox').scrollTop")).toBe(0);

  await app.eval(`samples.length = 0;
    document.getElementById("probe").addEventListener("gesturechange", e => e.preventDefault(), {once:true});`);
  expect((await app.send("pinch", { selector: "#probe", scale: 2 })).defaultPrevented).toBe(true);
  expect(JSON.parse(String(await app.eval("JSON.stringify(samples.map(row => row[0]))")))).toEqual([
    "gesturestart",
    "gesturechange",
    "gestureend",
  ]);
  await app.eval(`samples.length = 0;
    document.getElementById("probe").addEventListener("wheel", e => e.preventDefault(), {once:true});`);
  expect((await app.send("pinch", { selector: "#probe", scale: 0.5 })).defaultPrevented).toBe(true);
  expect(await app.eval("samples[2][12] > 0")).toBe(true);
  expect(await app.eval("document.getElementById('scrollbox').scrollTop")).toBe(0);
});

test("successive updates multiply and a cancellation resets the gesture", async () => {
  await app.locator("#probe").waitFor();
  await app.eval(`globalThis.scaleEvents = [];
    for (const name of ["gesturestart", "gesturechange", "gestureend"]) {
      document.getElementById("probe").addEventListener(name, e => scaleEvents.push([name, e.scale, e.ctrlKey]));
    }`);
  const at = { selector: "#probe", ctrl: true };
  await app.send("pinch", { ...at, kind: "start" });
  await app.send("pinch", { ...at, kind: "change", scale: 1.5 });
  await app.send("pinch", { ...at, kind: "change", scale: 0.5 });
  await app.send("pinch", { ...at, kind: "cancel" });
  await app.send("pinch", { selector: "#probe", scale: 2 });
  expect(JSON.parse(String(await app.eval("JSON.stringify(scaleEvents)")))).toEqual([
    ["gesturestart", 1, true],
    ["gesturechange", 1.5, true],
    ["gesturechange", 0.75, true],
    ["gestureend", 0.75, true],
    ["gesturestart", 1, false],
    ["gesturechange", 2, false],
    ["gestureend", 2, false],
  ]);
});

test("d3-zoom handles pinch and ordinary Ctrl+wheel", async () => {
  await app.locator("#map").waitFor();
  expect((await app.send("pinch", { selector: "#map", scale: 1.2 })).defaultPrevented).toBe(true);
  const zoomed = Number(await app.eval("document.getElementById('map').dataset.scale"));
  expect(zoomed).toBeGreaterThan(1);
  await app.send("wheel", { selector: "#map", dy: 80, ctrl: true });
  const afterWheel = Number(await app.eval("document.getElementById('map').dataset.scale"));
  expect(afterWheel).toBeGreaterThan(0);
  expect(afterWheel).toBeLessThan(zoomed);
  expect(await app.eval("document.getElementById('scrollbox').scrollTop")).toBe(0);
});
