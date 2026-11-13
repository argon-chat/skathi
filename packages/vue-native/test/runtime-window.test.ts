// The tier-3 windowing namespace: `import { appWindow } from "vue-native"`. The implementation lives in the host's JS
// prelude; what these tests pin is the NAMESPACE: apps reach it through an import, and a host without the prelude
// degrades loudly instead of silently.

import { afterEach, describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement } from "./runtime-harness";

const windowSlot = globalThis as { __VN_WINDOW__?: unknown };

afterEach(() => {
  delete windowSlot.__VN_WINDOW__;
});

function fakeHost() {
  const calls: unknown[] = [];
  const appWindow = {
    maximize: () => calls.push("maximize"),
    isMaximized: () => true,
  };
  return {
    calls,
    host: {
      appWindow,
      defineWindow: (o: unknown) => calls.push(["defineWindow", o]),
      getCurrentWindow: () => appWindow,
    },
  };
}

describe("appWindow namespace", () => {
  test("re-exports the host's appWindow rather than a copy of it", () => {
    const { calls, host } = fakeHost();
    windowSlot.__VN_WINDOW__ = host;
    const rt = createRuntime();

    expect(rt.appWindow as unknown).toBe(host.appWindow);
    rt.appWindow.maximize();
    expect(calls).toEqual(["maximize"]);
    expect(rt.advisories()).toEqual([]);
  });

  test("getCurrentWindow() is appWindow — the name ported Tauri code uses", () => {
    const { host } = fakeHost();
    windowSlot.__VN_WINDOW__ = host;
    const rt = createRuntime();
    expect(rt.getCurrentWindow()).toBe(rt.appWindow);
  });

  test("defineWindow forwards to the host", () => {
    const { calls, host } = fakeHost();
    windowSlot.__VN_WINDOW__ = host;
    const rt = createRuntime();
    rt.defineWindow({ title: "Notes" });
    expect(calls).toEqual([["defineWindow", { title: "Notes" }]]);
  });
});

describe("no windowing host", () => {
  test("the advisory is LAZY — importing the module warns about nothing", () => {
    const rt = createRuntime();
    expect(rt.advisories()).toEqual([]);
  });

  test("touching the inert window warns once and never throws", () => {
    const rt = createRuntime();
    expect(() => rt.appWindow.maximize()).not.toThrow();
    expect(rt.appWindow.isMaximized()).toBe(false);
    expect(() => {
      rt.appWindow.backdrop = "mica";
    }).not.toThrow();

    const advisories = rt.advisories();
    expect(advisories.length).toBe(1);
    expect(advisories[0]).toContain("windowing host is missing");
    expect(advisories[0]).toContain("appWindow` is inert");
  });

  test("defineWindow without a host warns instead of throwing", () => {
    const rt = createRuntime();
    expect(() => rt.defineWindow({ title: "x" })).not.toThrow();
    expect(rt.advisories().length).toBe(1);
  });
});

describe("native fullscreen Escape", () => {
  test("one Escape leaves fullscreen before app listeners, then normal key delivery resumes", () => {
    let active = true;
    const { host } = fakeHost();
    Object.assign(host, {
      escapeFullscreen: () => {
        if (!active) return false;
        active = false;
        return true;
      },
    });
    windowSlot.__VN_WINDOW__ = host;
    const rt = createRuntime();
    rt.createApp({ render: () => null }).mount();
    const node = rt.nodeOps.createElement("button");
    rt.nodeOps.insert(node, rt.nodesById.get(1) as MirrorElement, null);
    const delivered: string[] = [];
    node.addEventListener("keydown", (event) => void delivered.push((event as VnEvent & { key: string }).key));
    expect(rt.host.dispatcher(node.__vnId, rt.EV_CODES.keydown, 0, 0, 0, 0, "Escape")).toBe(false);
    expect(active).toBe(false);
    expect(delivered).toEqual([]);
    rt.host.dispatcher(node.__vnId, rt.EV_CODES.keydown, 0, 0, 0, 0, "Escape");
    rt.host.dispatcher(node.__vnId, rt.EV_CODES.keydown, 0, 0, 0, 0, "a");
    expect(delivered).toEqual(["Escape", "a"]);
  });
});
