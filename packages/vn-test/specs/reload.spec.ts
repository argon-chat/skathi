// `location.reload()` on the real host and engine (m26 client sweep #3,
// docs/RUNTIME.md §5.2): the app runs again in the SAME JavaScript realm, and
// the runtime that comes up must not run on its predecessor's classes.
//
//   bun test packages/vn-test/specs/reload.spec.ts
//
// test/runtime-reload.test.ts pins the bookkeeping in a fake global. This spec
// checks the same things against JavaScriptCore's real global object, where
// key enumeration and `delete` are the engine's: after a reload,
// ResizeObserver is a new class and fires, and a global the old app added is
// gone. Any fixture will do; this one borrows `events`. The last two tests
// reload again and check that the devtools Network panel still records fetch
// and WebSocket: on `events`, whose devtools are on from boot, and on
// `devtools-late`, which turns them on itself after the runtime has run.

import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/events/src/main.ts",
  window: [480, 320],
  label: "reload",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const js = async (src: string) => app.eval(src);

/** Polls `cond` (a JS expression) until it is true. */
async function until(cond: string): Promise<boolean> {
  for (let i = 0; i < 100; i++) {
    if ((await js(`String(!!(${cond}))`)) === "true") return true;
    await sleep(100);
  }
  return false;
}

test("after location.reload() the runtime's classes are new, ResizeObserver fires, and the old app's globals are gone", async () => {
  await until("document.querySelector('#trigger')");
  // The old app: a global, a symbol-keyed flag (Mediabunny's shape), and a
  // note of the classes it ran on. `__VN_*` names survive a reload by design.
  await js(`(() => {
    globalThis.oldAppGlobal = 1;
    globalThis[Symbol.for("vn-test loaded")] = true;
    globalThis.__VN_TEST_OLD__ = { RO: ResizeObserver, MO: MutationObserver, Window: Window };
    location.reload();
    return "asked";
  })()`);
  expect(await until("typeof oldAppGlobal === 'undefined' && document.querySelector('#trigger')")).toBe(true);
  expect(await js("String(globalThis[Symbol.for('vn-test loaded')])")).toBe("undefined");
  expect(
    await js("[ResizeObserver !== __VN_TEST_OLD__.RO, MutationObserver !== __VN_TEST_OLD__.MO, Window !== __VN_TEST_OLD__.Window, window.constructor === Window].join()"),
  ).toBe("true,true,true,true");
  // The observer that stopped firing after a reload, before the fix.
  await js(`(() => {
    globalThis.__VN_TEST_RO_SEEN__ = 0;
    new ResizeObserver(() => { __VN_TEST_RO_SEEN__++; }).observe(document.querySelector("#trigger"));
    return "observing";
  })()`);
  expect(await until("__VN_TEST_RO_SEEN__ > 0")).toBe(true);
  const { entries } = await app.logs(0);
  expect(entries.some((e) => e.text.includes("re-runs the app in the SAME JavaScript realm"))).toBe(true);
}, T);


// The devtools Network tee (crates/vn-app/src/devtools/net.rs) is installed
// once per process and has to stay underneath what each new runtime puts on the
// global: a reload's runtime replaces `WebSocket` with a new realm proxy. The
// ring is the host's, so it goes on across the reload, and the host puts a
// marker row between the runs. Checked in both install orders: at boot (`events`, the
// devtools on from the start) and late (`devtools-late`, turned on by the app
// after the runtime ran), which is the order that used to lose WebSocket.
type Host = Awaited<ReturnType<typeof launch>>;
type Row = { seq: number; kind: string; url: string; phase?: string; status?: number; cause?: string };

async function untilOn(host: Host, cond: string): Promise<boolean> {
  for (let i = 0; i < 100; i++) {
    if ((await host.eval(`String(!!(${cond}))`)) === "true") return true;
    await sleep(100);
  }
  return false;
}

/** fetch + WebSocket, a reload, the same again; `mounted` says the new app is up. */
async function networkAcrossReload(host: Host, mounted: string): Promise<void> {
  const server = Bun.serve({
    port: 0,
    hostname: "127.0.0.1",
    fetch(req, srv) {
      if (new URL(req.url).pathname === "/ws") {
        if (srv.upgrade(req)) return undefined as unknown as Response;
        return new Response("no upgrade", { status: 400 });
      }
      return new Response("ok", { headers: { "content-type": "text/plain" } });
    },
    websocket: {
      message(ws, msg) {
        ws.send(`echo:${msg}`);
      },
    },
  });
  try {
    const http = `http://127.0.0.1:${server.port}`;
    const ws = `ws://127.0.0.1:${server.port}/ws`;
    const rows = async (since: number): Promise<{ entries: Row[]; next: number }> => host.send("net", { since });
    const since = (await rows(0)).next;

    // The old app: a fetch, and a socket it leaves open for the reload to close.
    await host.eval(`(() => {
      const st = globalThis.__VN_TEST_NET__ = { fetch: 0, ws: 0 };
      fetch(${JSON.stringify(http + "/before")}).then((r) => r.text()).then(() => { st.fetch = 1; });
      const held = new WebSocket(${JSON.stringify(ws + "?held")});
      held.onopen = () => { st.ws = 1; };
      return "started";
    })()`);
    expect(await untilOn(host, "__VN_TEST_NET__.fetch === 1 && __VN_TEST_NET__.ws === 1")).toBe(true);
    await host.eval(`(() => { globalThis.netOldApp = 1; location.reload(); return "asked"; })()`);
    expect(await untilOn(host, `typeof netOldApp === 'undefined' && ${mounted}`)).toBe(true);

    // The new app: the same two, on the new runtime's globals.
    await host.eval(`(() => {
      const st = globalThis.__VN_TEST_NET__ = { fetch: 0, ws: 0 };
      fetch(${JSON.stringify(http + "/after")}).then((r) => r.text()).then(() => { st.fetch = 1; });
      const s = new WebSocket(${JSON.stringify(ws + "?after")});
      s.onopen = () => s.send("hi");
      s.onmessage = () => { st.ws = 1; s.close(); };
      return "started";
    })()`);
    expect(await untilOn(host, "__VN_TEST_NET__.fetch === 1 && __VN_TEST_NET__.ws === 1")).toBe(true);

    let got: Row[] = [];
    for (let i = 0; i < 50; i++) {
      got = (await rows(since)).entries;
      if (got.some((e) => e.kind === "ws" && e.phase === "close" && e.url.includes("?after"))) break;
      await sleep(100);
    }
    const at = (pred: (e: Row) => boolean) => got.findIndex(pred);
    const marker = at((e) => e.kind === "reload");
    const before = at((e) => e.kind === "fetch" && e.url.endsWith("/before") && e.status === 200);
    const after = at((e) => e.kind === "fetch" && e.url.endsWith("/after") && e.status === 200);
    const opened = at((e) => e.kind === "ws" && e.phase === "open" && e.url.includes("?after"));
    const closed = at((e) => e.kind === "ws" && e.phase === "close" && e.url.includes("?after"));
    expect(before).toBeGreaterThanOrEqual(0);
    expect(marker).toBeGreaterThan(before);
    expect(got[marker]?.cause).toBe("location.reload()");
    expect(after).toBeGreaterThan(marker);
    expect(opened).toBeGreaterThan(marker);
    expect(closed).toBeGreaterThan(opened);
    // The old app's socket, closed by the reload: the tee heard it.
    expect(got.some((e) => e.kind === "ws" && e.phase === "close" && e.url.includes("?held"))).toBe(true);
    // One ring: the seqs run on through the reload.
    expect(got.every((e, i) => i === 0 || e.seq > got[i - 1].seq)).toBe(true);
  } finally {
    server.stop(true);
  }
}

test("the devtools Network panel records fetch and WebSocket across location.reload()", async () => {
  await networkAcrossReload(app, "document.querySelector('#trigger')");
}, T);

test("…and does when the app turned the devtools on itself, after the runtime ran", async () => {
  const late = await launch({
    app: "packages/vn-test/fixtures/devtools-late/src/main.ts",
    window: [420, 220],
    label: "reload-late",
  });
  try {
    // The order this test is for: off at boot, so the tee went in after the
    // runtime (the host logs the boot state).
    const { entries } = await late.logs(0);
    expect(entries.some((e) => e.text.includes("[devtools] disabled at boot"))).toBe(true);
    expect(await untilOn(late, "document.querySelector('#late')")).toBe(true);
    await networkAcrossReload(late, "document.querySelector('#late')");
  } finally {
    await late.close();
  }
}, T);
