import { afterAll, expect, setDefaultTimeout, test } from "bun:test";
import { launch, WINDOWS } from "../src/index";

setDefaultTimeout(30_000);

const app = await launch({
  app: "packages/test/fixtures/power/src/main.ts",
  label: "power",
});
afterAll(() => app.close());

const states = ["nominal", "fair", "serious", "critical"];
const schemes = ["balanced", "saving", "performance"];

async function awaitValue<T>(read: () => Promise<T>, done: (value: T) => boolean): Promise<T> {
  const deadline = Date.now() + 8000;
  let value = await read();
  while (!done(value) && Date.now() < deadline) {
    await Bun.sleep(50);
    value = await read();
  }
  return value;
}

const js = async <T>(expression: string): Promise<T> => JSON.parse(await app.eval(`JSON.stringify(${expression})`));

test("observes CPU pressure with native records and the W3C API shape", async () => {
  const shape = await js<{
    sources: string[];
    frozen: boolean;
    immediate: number;
    queued: number;
  }>(`(() => {
    globalThis.pressureRecords = [];
    globalThis.badObserve = [];
    globalThis.pressureObserver = new PressureObserver(function (records, observer) {
      if (observer !== pressureObserver) throw new Error("wrong observer");
      for (const record of records) {
        pressureRecords.push({ ...record.toJSON(), branded: record instanceof PressureRecord && record.source === "cpu" });
      }
    });
    pressureObserver.observe("gpu").catch(e => badObserve.push(e.name));
    pressureObserver.observe("cpu", { sampleInterval: -1 }).catch(e => badObserve.push(e.name));
    pressureObserver.observe("cpu", { sampleInterval: 1000 });
    return {
      sources: PressureObserver.knownSources,
      frozen: Object.isFrozen(PressureObserver.knownSources),
      immediate: pressureRecords.length,
      queued: pressureObserver.takeRecords().length,
    };
  })()`);
  expect(shape).toEqual({
    sources: ["cpu"],
    frozen: true,
    immediate: 0,
    queued: 0,
  });
  const records = await awaitValue(
    () => js<{ source: string; state: string; time: number; branded: boolean }[]>("pressureRecords"),
    (v) => v.length >= 2,
  );
  expect(records.length).toBeGreaterThanOrEqual(2);
  expect(records[0]?.source).toBe("cpu");
  expect(states).toContain(records[0]?.state ?? "");
  expect(records[0]?.time).toBeGreaterThan(0);
  expect((records[1]?.time ?? 0) - (records[0]?.time ?? 0)).toBeGreaterThanOrEqual(1000);
  expect(records[0]?.branded).toBe(true);
  expect(await js<string[]>("badObserve")).toEqual(["TypeError", "TypeError"]);
  await app.eval(`pressureObserver.unobserve("cpu"); "ok"`);
  expect(await js<unknown[]>("pressureObserver.takeRecords()")).toEqual([]);
});

test("updates stylesheets and matchMedia change events via power emulation", async () => {
  const native = (await app.send("power")) as {
    scheme: string;
    pressure: string;
    onBattery: boolean;
  };
  expect(schemes).toContain(native.scheme);
  expect(states).toContain(native.pressure);
  expect(typeof native.onBattery).toBe("boolean");
  await app.send("power", { scheme: "balanced", pressure: "nominal" });
  await awaitValue(() => js<boolean>(`matchMedia("(currently-power-scheme: balanced)").matches`), Boolean);
  expect(
    await js<boolean>(`(() => {
    globalThis.powerChanges = [];
    for (const query of ["(currently-power-scheme: saving)", "(currently-compute-pressure: critical)"]) {
      matchMedia(query).addEventListener("change", (e) => powerChanges.push([e.media, e.matches]));
    }
    return matchMedia("(currently-power-scheme)").matches;
  })()`),
  ).toBe(true);
  await app.send("power", { scheme: "saving", pressure: "critical" });
  const active = await awaitValue(
    () => js<[string, boolean][]>("powerChanges"),
    (v) => v.length >= 2,
  );
  expect(active).toEqual([
    ["(currently-power-scheme: saving)", true],
    ["(currently-compute-pressure: critical)", true],
  ]);
  const color = () => js<string>(`getComputedStyle(document.getElementById("power")).color`);
  expect(await awaitValue(color, (v) => v === "rgb(90, 100, 110)")).toBe("rgb(90, 100, 110)");
  await app.send("power", { scheme: "performance", pressure: "fair" });
  expect(await awaitValue(color, (v) => v === "rgb(10, 20, 30)")).toBe("rgb(10, 20, 30)");
  expect(await js<[string, boolean][]>("powerChanges.slice(2)")).toEqual([
    ["(currently-power-scheme: saving)", false],
    ["(currently-compute-pressure: critical)", false],
  ]);
  const reset = (await app.send("power", {
    scheme: "system",
    pressure: "system",
  })) as {
    schemeOverride: unknown;
    pressureOverride: unknown;
  };
  expect(reset.schemeOverride).toBeNull();
  expect(reset.pressureOverride).toBeNull();
});

test.if(WINDOWS)("defers pressure notification while the window is minimized", async () => {
  await app.send("power", { pressure: "nominal" });
  await app.eval(`globalThis.whileVisible = [];
    globalThis.visibleObserver = new PressureObserver(records => whileVisible.push(...records.map(r => r.state)));
    visibleObserver.observe("cpu"); "ok"`);
  await awaitValue(
    () => js<string[]>("whileVisible"),
    (v) => v.at(-1) === "nominal",
  );
  await app.eval(`__SK_WINDOW__.appWindow.minimize(); "ok"`);
  expect(await awaitValue(() => js<boolean>("document.hidden"), Boolean)).toBe(true);
  await app.send("power", { pressure: "critical" });
  await Bun.sleep(500);
  expect(await js<string[]>("whileVisible")).toEqual(["nominal"]);
  await app.eval(`__SK_WINDOW__.appWindow.restore(); "ok"`);
  expect(
    await awaitValue(
      () => js<string[]>("whileVisible"),
      (v) => v.at(-1) === "critical",
    ),
  ).toEqual(["nominal", "critical"]);
  await app.eval(`visibleObserver.disconnect(); "ok"`);
  await app.send("power", { pressure: "system" });
});
