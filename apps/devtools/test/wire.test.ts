import { afterEach, expect, test } from "bun:test";
import { listTargets } from "../src/wire";

const saved = Object.getOwnPropertyDescriptor(globalThis, "__vn_devtools_targets");
afterEach(() => {
  if (saved) Object.defineProperty(globalThis, "__vn_devtools_targets", saved);
  else Reflect.deleteProperty(globalThis, "__vn_devtools_targets");
});

test("discovery shares a pending scan while the UI timer continues", async () => {
  let calls = 0;
  let inputTimerRan = false;
  const snapshot = { preferred: "target-slot", self: null, targets: [] };
  Object.defineProperty(globalThis, "__vn_devtools_targets", {
    configurable: true,
    value: () => JSON.stringify(++calls < 3 ? { pending: true } : snapshot),
  });
  const timer = setTimeout(() => { inputTimerRan = true; }, 10);
  try {
    const first = listTargets();
    const second = listTargets();
    expect(first).toBe(second);
    expect(await first).toEqual(snapshot);
    expect(inputTimerRan).toBe(true);
    expect(calls).toBe(3);
  } finally { clearTimeout(timer); }
});

test("discovery also accepts a synchronous older host and can rescan", async () => {
  let calls = 0;
  Object.defineProperty(globalThis, "__vn_devtools_targets", {
    configurable: true,
    value: () => JSON.stringify({ preferred: `scan-${++calls}`, self: null, targets: [] }),
  });
  expect((await listTargets()).preferred).toBe("scan-1");
  expect((await listTargets()).preferred).toBe("scan-2");
});

test("a malformed discovery reply is reported without wedging the next scan", async () => {
  Object.defineProperty(globalThis, "__vn_devtools_targets", { configurable: true, value: () => "invalid" });
  expect((await listTargets()).error).toContain("did not parse");
  Object.defineProperty(globalThis, "__vn_devtools_targets", {
    configurable: true, value: () => JSON.stringify({ preferred: "recovered", self: null, targets: [] }),
  });
  expect((await listTargets()).preferred).toBe("recovered");
});
