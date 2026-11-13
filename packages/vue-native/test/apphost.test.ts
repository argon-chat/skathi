// `appHost` (src/apphost.ts) over stand-ins for crates/vn-app/src/embed.rs's
// `__vn_host_*`: embedded, and in the exe where they do not exist.
import { afterEach, expect, test } from "bun:test";

type Dispatcher = (token: number, ok: boolean, text: string) => void;
type AppHostModule = typeof import("../src/apphost");

const G = globalThis as Record<string, unknown>;
let fresh = 0;

/** apphost.ts evaluated anew, against whatever `__vn_host_*` are set now. */
async function load(): Promise<AppHostModule> {
  return (await import(`../src/apphost.ts?case=${++fresh}`)) as AppHostModule;
}

function embed(functions: string[]) {
  const calls: Array<[string, string]> = [];
  let dispatch: Dispatcher = () => {};
  let next = 0;
  G.__vn_host_set_dispatcher = (fn: Dispatcher) => (dispatch = fn);
  G.__vn_host_call = (name: string, args: string) => {
    calls.push([name, args]);
    return functions.includes(name) ? ++next : 0;
  };
  G.__vn_host_functions = () => JSON.stringify(functions);
  return { calls, dispatch: (t: number, ok: boolean, text: string) => dispatch(t, ok, text) };
}

afterEach(() => {
  delete G.__vn_host_set_dispatcher;
  delete G.__vn_host_call;
  delete G.__vn_host_functions;
  (G.__VN_HOST_LIFECYCLE__ as { reset?: () => void } | undefined)?.reset?.();
  delete G.__VN_HOST__;
  delete G.__VN_HOST_LIFECYCLE__;
});

test("in the exe appHost is unavailable, and a call rejects and says why once", async () => {
  const warn = console.warn;
  const said: string[] = [];
  console.warn = (m: string) => said.push(m);
  try {
    const { appHost } = await load();
    expect(appHost.available).toBe(false);
    expect(appHost.functions).toEqual([]);
    const e = (await appHost.call("ping").catch((x: unknown) => x)) as Error & { function: string };
    expect(e.name).toBe("AppHostError");
    expect(e.function).toBe("ping");
    await appHost.call("ping").catch(() => {});
    expect(said.length).toBe(1);
    expect(said[0]).toMatch(/^\[vue-native compat\] appHost\.call\("ping"\): this app is not embedded/);
    expect(typeof appHost.on("x", () => {})).toBe("function");
  } finally {
    console.warn = warn;
  }
});

test("a call sends its arguments as JSON and settles with the host's answer", async () => {
  const h = embed(["db.query", "ping"]);
  const { appHost } = await load();
  expect(appHost.available).toBe(true);
  expect(appHost.functions).toEqual(["db.query", "ping"]);

  const ok = appHost.call<{ rows: number[] }>("db.query", "select 1", { limit: 2 });
  expect(h.calls[0]).toEqual(["db.query", '["select 1",{"limit":2}]']);
  h.dispatch(1, true, '{"rows":[1,2]}');
  expect(await ok).toEqual({ rows: [1, 2] });

  const bad = appHost.call("ping");
  h.dispatch(2, false, "the host function panicked");
  const e = (await bad.catch((x: unknown) => x)) as Error & { function: string };
  expect(e.name).toBe("AppHostError");
  expect(e.message).toBe("the host function panicked");
  expect(e.function).toBe("ping");

  const unknown = (await appHost.call("nope").catch((x: unknown) => x)) as Error;
  expect(unknown).toBeInstanceOf(TypeError);
  expect(unknown.message).toBe('appHost.call: the embedder has no function "nope" (it has "db.query", "ping")');

  const cyclic: Record<string, unknown> = {};
  cyclic.self = cyclic;
  expect(await appHost.call("ping", cyclic).catch((x: unknown) => x)).toBeInstanceOf(TypeError);
});

test("events reach their listeners until they unsubscribe, past one that throws", async () => {
  const h = embed([]);
  const { appHost } = await load();
  const got: unknown[] = [];
  const error = console.error;
  console.error = () => {};
  try {
    appHost.on("status", () => {
      throw new Error("boom");
    });
    const off = appHost.on("status", (d) => got.push(d));
    appHost.on("other", () => got.push("wrong"));
    h.dispatch(0, true, '{"event":"status","data":{"online":true}}');
    off();
    h.dispatch(0, true, '{"event":"status","data":{"online":false}}');
  } finally {
    console.error = error;
  }
  expect(got).toEqual([{ online: true }]);
});

test("the public global is the same facade and reload retires old listeners and calls", async () => {
  const h = embed(["slow"]);
  const first = await load();
  expect(G.__VN_HOST__).toBe(first.appHost);
  const seen: unknown[] = [];
  first.appHost.on("change", (data) => seen.push(data));
  const old = first.appHost.call("slow").catch((error: unknown) => error as Error);
  first.resetAppHost();
  expect(((await old) as Error).message).toContain("page unloaded");
  h.dispatch(0, true, '{"event":"change","data":"stale"}');
  expect(seen).toEqual([]);
  const second = await load();
  expect(G.__VN_HOST__).toBe(second.appHost);
  second.appHost.on("change", (data) => seen.push(data));
  h.dispatch(1, true, '"late answer"');
  h.dispatch(0, true, '{"event":"change","data":"current"}');
  expect(seen).toEqual(["current"]);
});
