// A same-realm reload (m26 client sweep #3): `location.reload()` and the dev
// host's cold restart evaluate the runtime AGAIN in the realm the first one
// ran in. These tests run two runtimes in one fake global, the way
// crates/vn-app/src/reload.rs does: `__VN_UNLOAD__`, then a second evaluation.
//
// Before the fix the second runtime kept the first one's `ResizeObserver`,
// whose poll was parked on an animation frame the unload had cancelled, so no
// observer fired again. The Argon Dashboard's grid collapsed and popovers
// stayed parked at their anchors.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

/** Animation frames that run only when the test says so, and can be cancelled. */
function frameQueue() {
  const frames = new Map<number, () => void>();
  let next = 1;
  return {
    globals: {
      requestAnimationFrame: (cb: () => void) => (frames.set(next, cb), next++),
      cancelAnimationFrame: (id: number) => frames.delete(id),
    },
    run() {
      const due = [...frames.values()];
      frames.clear();
      for (const cb of due) cb();
    },
    get size() {
      return frames.size;
    },
  };
}

type G = Record<PropertyKey, unknown> & {
  __VN_UNLOAD__?: () => void;
  ResizeObserver: new (cb: (entries: { contentRect: { width: number } }[]) => void) => { observe(n: unknown): void };
};

describe("a same-realm reload", () => {
  test("the next runtime's ResizeObserver replaces the old one, and it fires", () => {
    const first = createRuntime();
    const g = first.global as unknown as G;
    const OldRO = g.ResizeObserver;
    // The old app observes something, so the host runs the old runtime's step...
    const a = first.nodeOps.createElement("div");
    first.host.boundsById.set(a.__vnId, "0,0,50,10");
    const old: number[] = [];
    new OldRO((entries) => old.push(...entries.map((e) => e.contentRect.width))).observe(a);
    expect(first.host.layout.observing).toBe(true);
    // ...until the unload lets every observation go and says so.
    g.__VN_UNLOAD__!();
    expect(first.host.layout.observing).toBe(false);
    expect(first.host.layout.step()).toBe(-1);
    expect(old).toEqual([]);

    const second = createRuntime({ realm: first.global });
    expect(g.ResizeObserver).not.toBe(OldRO);
    const b = second.nodeOps.createElement("div");
    second.host.boundsById.set(b.__vnId, "0,0,120,30");
    const seen: number[] = [];
    new g.ResizeObserver((entries) => seen.push(...entries.map((e) => e.contentRect.width))).observe(b);
    second.host.layout.step();
    expect(seen).toEqual([120]);
    expect(old).toEqual([]);
  });

  test("an old runtime's step, still called before the next one installs its own, delivers nothing", () => {
    const raf = frameQueue();
    const first = createRuntime({ globals: { ...raf.globals } });
    const g = first.global as unknown as G;
    const a = first.nodeOps.createElement("div");
    first.host.boundsById.set(a.__vnId, "0,0,50,10");
    const old: number[] = [];
    new g.ResizeObserver((entries) => old.push(...entries.map((e) => e.contentRect.width))).observe(a);
    g.__VN_UNLOAD__!();
    // A host that has not heard the `false` yet calls the old step anyway.
    first.host.layout.observing = true;
    first.host.layout.step();
    expect(old).toEqual([]);
    expect(first.host.layout.observing).toBe(false);
  });

  test("every class and interface the runtime installs is the new runtime's", () => {
    const first = createRuntime();
    const g = first.global as unknown as Record<string, unknown>;
    const names = [
      "MutationObserver",
      "ResizeObserver",
      "DocumentFragment",
      "getComputedStyle",
      "CSS",
      "HTMLInputElement",
      "Window",
      "Text",
      "CSSStyleDeclaration",
      "DOMRect",
      "SubmitEvent",
      "ClipboardEvent",
      "launchQueue",
    ];
    const before = Object.fromEntries(names.map((n) => [n, g[n]]));
    for (const n of names) expect([n, typeof before[n]]).not.toEqual([n, "undefined"]);
    (g.__VN_UNLOAD__ as () => void)();
    createRuntime({ realm: first.global });
    for (const n of names) {
      expect([n, g[n] === before[n]]).toEqual([n, false]);
      expect([n, typeof g[n]]).toEqual([n, typeof before[n]]);
    }
    expect(g.constructor).toBe(g.Window);
  });

  test("native FileReader keeps its engine identity across reloads", () => {
    class NativeFileReader {}
    const first = createRuntime({ globals: { FileReader: NativeFileReader } });
    expect(first.global.FileReader).toBe(NativeFileReader);
    (first.global.__VN_UNLOAD__ as () => void)();
    createRuntime({ realm: first.global });
    expect(first.global.FileReader).toBe(NativeFileReader);
  });
  test("without an unload (the dev host's cold restart) the next runtime still installs its own", () => {
    const first = createRuntime();
    const g = first.global as unknown as Record<string, unknown>;
    const before = { RO: g.ResizeObserver, MO: g.MutationObserver, Text: g.Text, Window: g.Window };
    const second = createRuntime({ realm: first.global });
    expect(g.ResizeObserver).not.toBe(before.RO);
    expect(g.MutationObserver).not.toBe(before.MO);
    expect(g.Window).not.toBe(before.Window);
    expect(g.constructor).toBe(g.Window);
    // `new Text()` builds a node in the NEW mirror tree.
    const t = new (g.Text as new (s: string) => { id: number })("hi");
    expect(second.nodeById(t.__vnId)).toBe(t);
    expect(first.nodeById(t.__vnId)).toBeUndefined();
  });

  test("document.createTextNode and head.childNodes are the new runtime's", () => {
    const doc = { head: {} as Record<string, unknown> } as Record<string, unknown>;
    const first = createRuntime({ globals: { document: doc } });
    const create = doc.createTextNode as (s: string) => { id: number };
    expect(first.nodeById(create("a").__vnId)).toBeDefined();
    (first.global.__VN_UNLOAD__ as () => void)();
    const second = createRuntime({ realm: first.global });
    const created = (doc.createTextNode as (s: string) => { id: number })("b");
    expect(doc.createTextNode).not.toBe(create);
    expect(second.nodeById(created.__vnId)).toBe(created);
    // A <style> the new app puts in <head> is what head.childNodes lists.
    const style = second.nodeOps.createElement("style");
    (doc.head.appendChild as (n: unknown) => unknown)(style);
    expect([...doc.head.childNodes]).toEqual([style]);
  });

  test("document.createElement wraps the ENGINE's after a reload, not the old wrapper", () => {
    let engineCalls = 0;
    const doc = {
      head: {},
      createElement(tag: string) {
        engineCalls++;
        return { tagName: String(tag).toUpperCase() };
      },
    } as Record<string, unknown>;
    const first = createRuntime({ globals: { document: doc } });
    const wrapper = doc.createElement;
    for (let i = 0; i < 3; i++) {
      (first.global.__VN_UNLOAD__ as (() => void) | undefined)?.();
      createRuntime({ realm: first.global });
    }
    expect(doc.createElement).not.toBe(wrapper);
    const a = (doc.createElement as (t: string) => Record<string, unknown>)("a");
    // One engine call per element: no wrapper of a wrapper.
    expect(engineCalls).toBe(1);
    expect(typeof a.setAttribute).toBe("function");
  });

  test("navigator.userActivation reads the new runtime's activation", () => {
    const nav = {} as Record<string, unknown>;
    const first = createRuntime({ navigator: nav });
    const old = nav.userActivation;
    expect(old).toBeDefined();
    (first.global.__VN_UNLOAD__ as () => void)();
    createRuntime({ realm: first.global, navigator: nav });
    expect(nav.userActivation).not.toBe(old);
  });

  test("the unload deletes the globals the old app added, string and symbol keys alike, and keeps the rest", () => {
    const first = createRuntime({ globals: { location: { href: "app://redpew/" }, preludeFlag: 1 } });
    const g = first.global as unknown as Record<PropertyKey, unknown>;
    const loaded = Symbol.for("mediabunny loaded");
    g.appState = { user: 1 };
    g[loaded] = true;
    g.__VN_HOST_STATE__ = "host";
    g.__vn_host_fn = () => 1;
    Object.defineProperty(g, "pinned", { value: 1, configurable: false });
    (g.__VN_UNLOAD__ as () => void)();
    expect("appState" in g).toBe(false);
    expect(loaded in g).toBe(false);
    // What the realm had before the first runtime ran, and the host's names.
    expect(g.preludeFlag).toBe(1);
    expect(g.location).toEqual(expect.objectContaining({ href: "app://redpew/" }));
    expect(g.__VN_HOST_STATE__).toBe("host");
    expect(typeof g.__vn_host_fn).toBe("function");
    // A non-configurable one cannot go, and the advisory counts it.
    expect(g.pinned).toBe(1);
    expect(first.advisories().some((l: string) => l.includes("1 global(s) the old app declared cannot be deleted"))).toBe(true);
  });

  test("the process's own launch reaches launchQueue once, not again after a reload", async () => {
    const launch = { url: "argon://invite/abc" };
    const first = createRuntime({ launch });
    const got: unknown[] = [];
    const lq = () => first.global.launchQueue as { setConsumer(fn: (p: { targetURL: unknown }) => void): void };
    lq().setConsumer((p) => got.push(p.targetURL));
    await Promise.resolve();
    expect(got).toEqual(["argon://invite/abc"]);
    (first.global.__VN_UNLOAD__ as () => void)();
    createRuntime({ realm: first.global, launch });
    lq().setConsumer((p) => got.push(p.targetURL));
    await new Promise((r) => setTimeout(r, 5));
    expect(got).toEqual(["argon://invite/abc"]);
  });
});
