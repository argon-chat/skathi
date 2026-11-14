// A same-realm reload: `location.reload()` and the dev host's cold restart evaluate the runtime again in the realm
// the first one ran in. These tests run two runtimes in one fake global, as crates/sk-app/src/reload.rs does
// (`__SK_UNLOAD__`, then a second evaluation); the second must not keep the first one's parked `ResizeObserver` poll.

import { describe, expect, test } from "bun:test";
import { createRuntime, type MirrorNode } from "./runtime-harness";

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
  __SK_UNLOAD__?: () => void;
  ResizeObserver: new (cb: (entries: { contentRect: { width: number } }[]) => void) => { observe(n: unknown): void };
};

describe("a same-realm reload", () => {
  test("the next runtime's ResizeObserver replaces the old one, and it fires", () => {
    const first = createRuntime();
    const g = first.global as unknown as G;
    const OldRO = g.ResizeObserver;
    const a = first.nodeOps.createElement("div");
    first.host.boundsById.set(a.__skId, "0,0,50,10");
    const old: number[] = [];
    new OldRO((entries) => old.push(...entries.map((e) => e.contentRect.width))).observe(a);
    expect(first.host.layout.observing).toBe(true);
    g.__SK_UNLOAD__!();
    expect(first.host.layout.observing).toBe(false);
    expect(first.host.layout.step()).toBe(-1);
    expect(old).toEqual([]);

    const second = createRuntime({ realm: first.global });
    expect(g.ResizeObserver).not.toBe(OldRO);
    const b = second.nodeOps.createElement("div");
    second.host.boundsById.set(b.__skId, "0,0,120,30");
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
    first.host.boundsById.set(a.__skId, "0,0,50,10");
    const old: number[] = [];
    new g.ResizeObserver((entries) => old.push(...entries.map((e) => e.contentRect.width))).observe(a);
    g.__SK_UNLOAD__!();
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
    ];
    const before = Object.fromEntries(names.map((n) => [n, g[n]]));
    for (const n of names) expect([n, typeof before[n]]).not.toEqual([n, "undefined"]);
    (g.__SK_UNLOAD__ as () => void)();
    createRuntime({ realm: first.global });
    for (const n of names) {
      expect([n, g[n] === before[n]]).toEqual([n, false]);
      expect([n, typeof g[n]]).toEqual([n, typeof before[n]]);
    }
    expect(g.constructor as unknown).toBe(g.Window);
  });

  test("native FileReader keeps its engine identity across reloads", () => {
    class NativeFileReader {}
    const first = createRuntime({ globals: { FileReader: NativeFileReader } });
    expect(first.global.FileReader).toBe(NativeFileReader);
    (first.global.__SK_UNLOAD__ as () => void)();
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
    expect(g.constructor as unknown).toBe(g.Window);
    const t = new (g.Text as new (s: string) => MirrorNode)("hi");
    expect(second.nodeById(t.__skId)).toBe(t);
    expect(first.nodeById(t.__skId)).toBeUndefined();
  });

  test("document.createTextNode and head.childNodes are the new runtime's", () => {
    const doc: {
      head: { appendChild?: (n: unknown) => unknown; childNodes?: unknown[] };
      createTextNode?: (s: string) => MirrorNode;
    } = { head: {} };
    const first = createRuntime({ globals: { document: doc } });
    const create = doc.createTextNode!;
    expect(first.nodeById(create("a").__skId)).toBeDefined();
    (first.global.__SK_UNLOAD__ as () => void)();
    const second = createRuntime({ realm: first.global });
    const created = doc.createTextNode!("b");
    expect(doc.createTextNode).not.toBe(create);
    expect(second.nodeById(created.__skId)).toBe(created);
    const style = second.nodeOps.createElement("style");
    doc.head.appendChild!(style);
    expect([...doc.head.childNodes!]).toEqual([style]);
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
      (first.global.__SK_UNLOAD__ as (() => void) | undefined)?.();
      createRuntime({ realm: first.global });
    }
    expect(doc.createElement).not.toBe(wrapper);
    const a = (doc.createElement as (t: string) => Record<string, unknown>)("a");
    expect(engineCalls).toBe(1);
    expect(typeof a.setAttribute).toBe("function");
  });

  test("navigator.userActivation reads the new runtime's activation", () => {
    const nav = {} as Record<string, unknown>;
    const first = createRuntime({ navigator: nav });
    const old = nav.userActivation;
    expect(old).toBeDefined();
    (first.global.__SK_UNLOAD__ as () => void)();
    createRuntime({ realm: first.global, navigator: nav });
    expect(nav.userActivation).not.toBe(old);
  });

  test("the unload deletes the globals the old app added, string and symbol keys alike, and keeps the rest", () => {
    const first = createRuntime({ globals: { location: { href: "app://redjs/" }, preludeFlag: 1 } });
    const g = first.global as unknown as Record<PropertyKey, unknown>;
    const loaded = Symbol.for("mediabunny loaded");
    g.appState = { user: 1 };
    g[loaded] = true;
    g.__SK_HOST_STATE__ = "host";
    g.__sk_host_fn = () => 1;
    Object.defineProperty(g, "pinned", { value: 1, configurable: false });
    (g.__SK_UNLOAD__ as () => void)();
    expect("appState" in g).toBe(false);
    expect(loaded in g).toBe(false);
    expect(g.preludeFlag).toBe(1);
    expect(g.location).toEqual(expect.objectContaining({ href: "app://redjs/" }));
    expect(g.__SK_HOST_STATE__).toBe("host");
    expect(typeof g.__sk_host_fn).toBe("function");
    expect(g.pinned).toBe(1);
    expect(
      first.advisories().some((l: string) => l.includes("1 global(s) the old app declared cannot be deleted")),
    ).toBe(true);
  });
});
