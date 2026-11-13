import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

function world() {
  const nativeWindow = new EventTarget();
  const document = Object.assign(new EventTarget(), { head: {} });
  const calls: string[] = [];
  const dispatchWindow = nativeWindow.dispatchEvent.bind(nativeWindow);
  const rt = createRuntime({
    globals: {
      document,
      Event,
      EventTarget,
      addEventListener: nativeWindow.addEventListener.bind(nativeWindow),
      removeEventListener: nativeWindow.removeEventListener.bind(nativeWindow),
      dispatchEvent: (event: Event) => {
        calls.push(`window:${event.type}`);
        return dispatchWindow(event);
      },
    },
  });
  rt.createApp({ render: () => null }).mount();
  const parent = rt.nodeOps.createElement("section");
  const target = rt.nodeOps.createElement("div");
  rt.nodeOps.insert(parent, rt.global.document.body, null);
  rt.nodeOps.insert(target, parent, null);
  return { rt, g: rt.global as any, document: rt.global.document as any, parent, target, calls };
}

describe("fullscreen events across the mirror and global targets", () => {
  test("a connected element dispatch reaches element, document and window once in DOM order", () => {
    const { rt, g, document, parent, target, calls } = world();
    const seen: string[] = [];
    g.addEventListener("fullscreenchange", (event: Event) => seen.push(`window-c:${event.eventPhase}`), true);
    document.addEventListener("fullscreenchange", (event: Event) => seen.push(`document-c:${event.eventPhase}`), true);
    parent.addEventListener("fullscreenchange", (event: Event) => seen.push(`parent-c:${event.eventPhase}`), true);
    target.addEventListener("fullscreenchange", (event: Event) => {
      expect(event.target).toBe(target);
      seen.push(`target:${event.eventPhase}`);
    });
    document.addEventListener("fullscreenchange", (event: Event) => seen.push(`document-b:${event.eventPhase}`));
    g.addEventListener("fullscreenchange", (event: Event) => seen.push(`window-b:${event.eventPhase}`));
    expect(target.isConnected).toBe(true);
    target.dispatchEvent(new Event("fullscreenchange", { bubbles: true }));
    expect(seen).toEqual(["window-c:1", "document-c:1", "parent-c:1", "target:2", "document-b:3", "window-b:3"]);
    expect(calls).toEqual([]);
    expect(rt.advisories()).toEqual([]);
  });

  test("the disconnected-target fallback dispatches at document with once and cancellation", () => {
    const { g, document, target } = world();
    target.remove();
    const seen: string[] = [];
    g.addEventListener("fullscreenchange", (event: Event) => seen.push(`window-c:${event.eventPhase}`), true);
    document.addEventListener(
      "fullscreenchange",
      (event: Event) => {
        expect(event.target).toBe(document);
        seen.push(`document-c:${event.eventPhase}`);
      },
      { capture: true, once: true },
    );
    document.addEventListener("fullscreenchange", (event: Event) => {
      event.preventDefault();
      seen.push(`document-b:${event.eventPhase}`);
    });
    g.addEventListener("fullscreenchange", (event: Event) => seen.push(`window-b:${event.eventPhase}`));
    const event = new Event("fullscreenchange", { bubbles: true, cancelable: true });
    expect(document.dispatchEvent(event)).toBe(false);
    expect(seen).toEqual(["window-c:1", "document-c:2", "document-b:2", "window-b:3"]);
    expect(event.currentTarget).toBe(null);
    expect(event.eventPhase).toBe(0);
    expect(event.composedPath()).toEqual([]);
    seen.length = 0;
    document.dispatchEvent(new Event("fullscreenchange", { bubbles: true }));
    expect(seen).toEqual(["window-c:1", "document-b:2", "window-b:3"]);
  });

  test("window dispatch stays at window and invokes both target phases even without bubbling", () => {
    const { g, document } = world();
    const seen: string[] = [];
    document.addEventListener("fullscreenchange", () => seen.push("document"));
    g.addEventListener(
      "fullscreenchange",
      (event: Event) => {
        expect(event.target).toBe(g);
        event.stopPropagation();
        seen.push(`capture:${event.eventPhase}`);
      },
      { capture: true, once: true },
    );
    g.addEventListener("fullscreenchange", (event: Event) => seen.push(`bubble:${event.eventPhase}`));
    g.dispatchEvent(new Event("fullscreenchange"));
    expect(seen).toEqual(["capture:2", "bubble:2"]);
    seen.length = 0;
    g.dispatchEvent(new Event("fullscreenchange"));
    expect(seen).toEqual(["bubble:2"]);
  });

  test("native dispatch is preserved across same-realm reloads without wrapping old dispatchers", () => {
    let { rt, g, calls } = world();
    let nativeResize = 0;
    let fullscreen = 0;
    for (let generation = 0; generation < 4; generation++) {
      g.addEventListener("resize", () => nativeResize++);
      g.addEventListener("fullscreenchange", () => fullscreen++, { once: true });
      g.dispatchEvent(new Event("resize"));
      g.dispatchEvent(new Event("fullscreenchange"));
      g.dispatchEvent(new Event("fullscreenchange"));
      expect(nativeResize).toBe(generation + 1);
      expect(fullscreen).toBe(generation + 1);
      expect(calls.filter((call) => call === "window:resize")).toHaveLength(generation + 1);
      (g.__VN_UNLOAD__ as () => void)();
      rt = createRuntime({ realm: g });
      g = rt.global;
    }
  });
});

describe("host event names dispatched at window or document", () => {
  test("focus and blur dispatched at window reach window listeners", () => {
    // The windowing prelude's `focus` / `blur` are exactly this dispatch.
    const { g, calls } = world();
    const seen: string[] = [];
    g.addEventListener("focus", (event: Event) => {
      expect(event.target).toBe(g);
      seen.push(`focus:${event.eventPhase}`);
    });
    g.addEventListener("blur", () => seen.push("blur"));
    g.dispatchEvent(new Event("blur"));
    g.dispatchEvent(new Event("focus"));
    expect(seen).toEqual(["blur", "focus:2"]);
    expect(calls).toEqual([]);
  });

  test("a keydown dispatched at document reaches document and then window listeners", () => {
    const { g, document } = world();
    const seen: string[] = [];
    g.addEventListener("keydown", () => seen.push("window"));
    document.addEventListener("keydown", () => seen.push("document"));
    document.dispatchEvent(new Event("keydown", { bubbles: true }));
    expect(seen).toEqual(["document", "window"]);
  });

  test("names the engine fires itself still go to its EventTarget", () => {
    const { g, calls } = world();
    let resized = 0;
    let loaded = 0;
    g.addEventListener("resize", () => resized++);
    g.addEventListener("load", () => loaded++);
    g.dispatchEvent(new Event("resize"));
    g.dispatchEvent(new Event("load"));
    expect([resized, loaded]).toEqual([1, 1]);
    expect(calls).toEqual(["window:resize", "window:load"]);
  });
});

describe("fullscreen listener AbortSignal lifecycle", () => {
  test("aborting an actual player-style global listener removes it, including already-aborted signals", () => {
    const { document, target } = world();
    const controller = new AbortController();
    let seen = 0;
    document.addEventListener("fullscreenchange", () => seen++, { signal: controller.signal });
    target.dispatchEvent(new Event("fullscreenchange", { bubbles: true }));
    expect(seen).toBe(1);
    controller.abort();
    target.dispatchEvent(new Event("fullscreenchange", { bubbles: true }));
    document.addEventListener("fullscreenchange", () => seen++, { signal: controller.signal });
    document.dispatchEvent(new Event("fullscreenchange"));
    expect(seen).toBe(1);
  });

  test("abort during dispatch prevents a later listener in the same snapshot", () => {
    const { document } = world();
    const controller = new AbortController();
    const seen: string[] = [];
    document.addEventListener("fullscreenchange", () => {
      seen.push("first");
      controller.abort();
    });
    document.addEventListener("fullscreenchange", () => seen.push("aborted"), { signal: controller.signal });
    document.dispatchEvent(new Event("fullscreenchange"));
    expect(seen).toEqual(["first"]);
  });

  test("once, manual removal and realm teardown all release signal subscriptions", () => {
    const { g, document } = world();
    const controller = new AbortController();
    const signal = controller.signal;
    const add = signal.addEventListener.bind(signal);
    const remove = signal.removeEventListener.bind(signal);
    const active = new Set<unknown>();
    signal.addEventListener = ((type: string, listener: any, options: any) => {
      if (type === "abort") active.add(listener);
      add(type, listener, options);
    }) as typeof signal.addEventListener;
    signal.removeEventListener = ((type: string, listener: any, options: any) => {
      if (type === "abort") active.delete(listener);
      remove(type, listener, options);
    }) as typeof signal.removeEventListener;
    let seen = 0;
    const listener = () => seen++;
    document.addEventListener("fullscreenchange", listener, { once: true, signal });
    expect(active.size).toBe(1);
    document.dispatchEvent(new Event("fullscreenchange"));
    expect(seen).toBe(1);
    expect(active.size).toBe(0);
    document.addEventListener("fullscreenchange", listener, { signal });
    expect(active.size).toBe(1);
    document.removeEventListener("fullscreenchange", listener);
    expect(active.size).toBe(0);
    document.addEventListener("fullscreenchange", listener, { signal });
    g.addEventListener("fullscreenchange", listener, { signal });
    expect(active.size).toBe(2);
    (g.__VN_UNLOAD__ as () => void)();
    expect(active.size).toBe(0);
    controller.abort();
    expect(seen).toBe(1);
  });
});
