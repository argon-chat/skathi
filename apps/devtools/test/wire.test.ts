import { afterEach, expect, test } from "bun:test";
import { listTargets, send } from "../src/wire";

const savedFetch = globalThis.fetch;
const saved = Object.getOwnPropertyDescriptor(globalThis, "__vn_devtools_targets");
afterEach(() => {
  globalThis.fetch = savedFetch;
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

const target = (port: number) => ({ slot: `fixture-${port}`, dir: "fixture", port, pid: port, token: "fixture-only", alive: true, self: false });
function mockWire() {
  const started: { cmd: string; port: number; finish: (value?: unknown) => void }[] = [];
  globalThis.fetch = ((url: string, init: RequestInit) => new Promise(resolve => {
    const cmd = JSON.parse(String(init.body)).cmd;
    started.push({ cmd, port: new URL(url).port as unknown as number, finish: (value = cmd) => resolve({ text: async () => JSON.stringify({ ok: true, data: value }) }) });
  })) as typeof fetch;
  return started;
}
const tick = () => new Promise<void>(resolve => queueMicrotask(resolve));

test("a stalled target cannot block commands to a new attachment", async () => {
  const calls = mockWire();
  const stalled = send(target(49100), "mem");
  const fresh = send(target(49101), "ping");
  expect(calls.map(call => call.cmd)).toEqual(["mem", "ping"]);
  calls[1]!.finish("attached");
  expect(await fresh).toBe("attached");
  calls[0]!.finish();
  await stalled;
});

test("interactive commands overtake queued background work on the same target", async () => {
  const calls = mockWire(), host = target(49102);
  const first = send(host, "ping");
  const background = send(host, "pick", {}, undefined, { priority: "background" });
  const click = send(host, "eval");
  expect(calls.map(call => call.cmd)).toEqual(["ping"]);
  calls[0]!.finish(); await first; await tick();
  expect(calls.map(call => call.cmd)).toEqual(["ping", "eval"]);
  calls[1]!.finish(); await click; await tick();
  calls[2]!.finish(); await background;
  expect(calls.map(call => call.cmd)).toEqual(["ping", "eval", "pick"]);
});

test("stale queued commands are cancelled before reaching the old target", async () => {
  const calls = mockWire(), host = target(49103);
  const first = send(host, "ping");
  let current = true;
  const stale = send(host, "highlight", {}, undefined, { isCurrent: () => current });
  const rejected = stale.catch(error => error);
  current = false;
  calls[0]!.finish(); await first;
  expect(String(await rejected)).toContain("attachment changed");
  expect(calls.length).toBe(1);
  const next = send(host, "ping"); await tick();
  calls[1]!.finish(); expect(await next).toBe("ping");
});
