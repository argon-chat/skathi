// The tier-3 windowing namespace: `import { appWindow } from "@skathi-native/runtime"`. The implementation lives in the
// host; what these tests pin is the NAMESPACE: apps reach it through an import, and a runtime without the host
// degrades loudly instead of silently.

import { afterEach, describe, expect, test } from "bun:test";
import { computed, nextTick, watch } from "@vue/runtime-core";
import { createRuntime, type MirrorElement } from "./runtime-harness";

const windowSlot = globalThis as { __SK_WINDOW__?: unknown };

afterEach(() => {
  delete windowSlot.__SK_WINDOW__;
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
    windowSlot.__SK_WINDOW__ = host;
    const rt = createRuntime();

    expect(rt.appWindow as unknown).toBe(host.appWindow);
    rt.appWindow.maximize();
    expect(calls).toEqual(["maximize"]);
    expect(rt.advisories()).toEqual([]);
  });
});

// The host's shape: a class whose prototype holds getters and `is*()` over a plain snapshot, and `onChange`
// subscribers called after it moves.
function snapshotHost() {
  const state = { title: "One", maximized: false, outer: { x: 0, y: 0, width: 800, height: 600 } };
  const listeners = new Set<() => void>();
  class AppWindow extends EventTarget {
    get title() {
      return state.title;
    }
    set title(v: string) {
      state.title = v;
      for (const fn of listeners) fn();
    }
    isMaximized() {
      return state.maximized;
    }
    getBounds() {
      return { ...state.outer };
    }
    maximize() {}
  }
  const appWindow = new AppWindow();
  const publish = (next: Partial<typeof state>) => {
    Object.assign(state, next);
    for (const fn of listeners) fn();
  };
  const host = {
    appWindow,
    defineWindow: () => {},
    onChange: (fn: () => void) => {
      listeners.add(fn);
      return () => void listeners.delete(fn);
    },
  };
  return { host, publish, listeners };
}

describe("appWindow reactivity", () => {
  test("a computed over title and isMaximized() recomputes when the host publishes", () => {
    const { host, publish } = snapshotHost();
    windowSlot.__SK_WINDOW__ = host;
    const rt = createRuntime();
    let runs = 0;
    const label = computed(() => {
      runs += 1;
      return `${rt.appWindow.title}:${rt.appWindow.isMaximized()}`;
    });
    expect(label.value).toBe("One:false");
    publish({ maximized: true });
    expect(label.value).toBe("One:true");
    rt.appWindow.title = "Two";
    expect(label.value).toBe("Two:true");
    expect(runs).toBe(3);
    expect(label.value).toBe("Two:true");
    expect(runs).toBe(3);
  });

  test("watch over appWindow.title and getBounds() re-runs, and keeps object identity", async () => {
    const { host, publish } = snapshotHost();
    windowSlot.__SK_WINDOW__ = host;
    const rt = createRuntime();
    expect(rt.appWindow as unknown).toBe(host.appWindow);
    const titles: string[] = [];
    const widths: number[] = [];
    const stopTitle = watch(
      () => rt.appWindow.title,
      (t) => titles.push(t),
    );
    const stopBounds = watch(
      () => rt.appWindow.getBounds().width,
      (w) => widths.push(w),
    );
    publish({ title: "Renamed" });
    publish({ outer: { x: 0, y: 0, width: 1024, height: 600 } });
    await nextTick();
    expect(titles).toEqual(["Renamed"]);
    expect(widths).toEqual([1024]);
    stopTitle();
    stopBounds();
  });

  test("an effect that reads nothing from appWindow is not re-run by its changes", () => {
    const { host, publish } = snapshotHost();
    windowSlot.__SK_WINDOW__ = host;
    const rt = createRuntime();
    let runs = 0;
    const other = computed(() => {
      runs += 1;
      return 1;
    });
    expect(other.value).toBe(1);
    publish({ maximized: true });
    rt.appWindow.maximize();
    expect(other.value).toBe(1);
    expect(runs).toBe(1);
  });

  test("a second evaluation of the runtime (a reload) wraps the originals once", () => {
    const { host, publish, listeners } = snapshotHost();
    windowSlot.__SK_WINDOW__ = host;
    createRuntime();
    const rt = createRuntime();
    expect(listeners.size).toBe(1);
    let runs = 0;
    const max = computed(() => {
      runs += 1;
      return rt.appWindow.isMaximized();
    });
    expect(max.value).toBe(false);
    publish({ maximized: true });
    expect(max.value).toBe(true);
    expect(runs).toBe(2);
  });
});

describe("no windowing host", () => {
  test("touching the inert window warns once and never throws", () => {
    const rt = createRuntime();
    expect(() => rt.appWindow.maximize()).not.toThrow();
    expect(rt.appWindow.isMaximized()).toBe(false);
    expect(() => {
      rt.appWindow.backdrop = "mica";
    }).not.toThrow();

    expect(rt.advisories().length).toBe(1);
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
    windowSlot.__SK_WINDOW__ = host;
    const rt = createRuntime();
    rt.createApp({ render: () => null }).mount();
    const node = rt.nodeOps.createElement("button");
    rt.nodeOps.insert(node, rt.nodesById.get(1) as MirrorElement, null);
    const delivered: string[] = [];
    node.addEventListener("keydown", (event) => void delivered.push((event as Event & { key: string }).key));
    expect(rt.host.dispatcher(node.__skId, rt.EV_CODES.keydown, 0, 0, 0, 0, "Escape")).toBe(false);
    expect(active).toBe(false);
    expect(delivered).toEqual([]);
    rt.host.dispatcher(node.__skId, rt.EV_CODES.keydown, 0, 0, 0, 0, "Escape");
    rt.host.dispatcher(node.__skId, rt.EV_CODES.keydown, 0, 0, 0, 0, "a");
    expect(delivered).toEqual(["Escape", "a"]);
  });
});
