import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const source = readFileSync(join(import.meta.dir, "../../../crates/vn-app/src/window.prelude.ts"), "utf8");
const fragment = source.slice(source.indexOf("  // ---- Fullscreen API"), source.indexOf("  // ---- windowControlsOverlay"));
const body = new Bun.Transpiler({ loader: "ts" }).transformSync(fragment);

function world(initialFullscreen = false) {
  const document: any = new EventTarget();
  const frames: (() => void)[] = [];
  const windowCalls: boolean[] = [];
  const elementCalls: number[] = [];
  const ownerCalls: { owner: number; on: boolean }[] = [];
  const ownerStates = new Map<number, boolean>();
  let windowFullscreen = initialFullscreen;
  const client = { width: 0, height: 0 };
  let observer: any;
  class Element extends EventTarget {
    __vnId: number;
    isConnected = true;
    constructor(id: number) { super(); this.__vnId = id; }
    dispatchEvent(event: Event) {
      const result = super.dispatchEvent(event);
      if (this.isConnected && event.bubbles) document.dispatchEvent(new Event(event.type));
      return result;
    }
  }
  const first = new Element(1) as any;
  const second = new Element(2) as any;
  document.documentElement = first;
  document.body = first;
  const global = { innerWidth: 0, innerHeight: 0, MutationObserver: class {
    callback: () => void;
    constructor(callback: () => void) { this.callback = callback; observer = this; }
    observe() {}
    disconnect() {}
  } };
  const api = new Function("document", "Element", "Event", "ro", "refresh", "warn", "__vn_window_set_fullscreen",
    "__vn_fullscreen_element", "__vn_fullscreen_window", "requestAnimationFrame", "setTimeout", "globalThis", body +
    "\nreturn { onFullscreenWindowChanged, exitAllFullscreen };")(
    document, Element, Event,
    (object: object, key: string, get: () => unknown) => Object.defineProperty(object, key, { get }),
    () => ({ fullscreen: windowFullscreen, clientWidth: client.width, clientHeight: client.height }), () => {}, (on: boolean) => windowCalls.push(on),
    (id: number) => { elementCalls.push(id); return id >= 0 && id <= 2; },
    (owner: number, on?: boolean) => {
      if (on !== undefined) {
        if (owner === 0) windowCalls.push(on);
        else ownerCalls.push({ owner, on });
      }
      return owner === 0 ? windowFullscreen : ownerStates.get(owner) ?? false;
    },
    (callback: () => void) => { frames.push(callback); return frames.length; }, setTimeout, global,
  );
  return { document, first, second, client, global, windowCalls, elementCalls, ownerCalls, ownerStates, api,
    windowChanged(on: boolean) { windowFullscreen = on; api.onFullscreenWindowChanged(on); },
    frame() { const pending = frames.splice(0); for (const frame of pending) frame(); },
    removed() { observer?.callback(); },
  };
}

function paint(w: { frame(): void }, count: number) {
  for (let i = 0; i < count; i++) w.frame();
}

describe("element fullscreen lifecycle", () => {
  test("events and promises follow native resize and bubble to document once", async () => {
    const w = world();
    const seen: any[] = [];
    w.document.addEventListener("fullscreenchange", () => seen.push(w.document.fullscreenElement));
    const entering = w.first.requestFullscreen();
    expect(w.document.fullscreenElement).toBe(w.first);
    expect(w.elementCalls).toEqual([1]);
    expect(seen).toEqual([]);
    w.windowChanged(true);
    paint(w, 5); await entering;
    const exiting = w.document.exitFullscreen();
    expect(w.document.fullscreenElement).toBeNull();
    w.windowChanged(false);
    paint(w, 5); await exiting;
    w.frame();
    expect(seen).toEqual([w.first, null]);
    expect(w.windowCalls).toEqual([true, false]);
    expect(w.elementCalls).toEqual([1, 0]);
  });

  test("nested targets and preexisting window fullscreen restore correctly", async () => {
    const w = world(true);
    const first = w.first.requestFullscreen(); paint(w, 6); await first;
    const second = w.second.requestFullscreen(); paint(w, 6); await second;
    expect(w.document.fullscreenElement).toBe(w.second);
    const pop = w.document.exitFullscreen(); paint(w, 6); await pop;
    expect(w.document.fullscreenElement).toBe(w.first);
    const exit = w.document.exitFullscreen(); paint(w, 6); await exit;
    expect(w.document.fullscreenElement).toBeNull();
    expect(w.windowCalls).toEqual([true, true]);
    expect(w.elementCalls).toEqual([1, 2, 1, 0]);
    expect(w.api.exitAllFullscreen()).toBe(false);
  });

  test("Escape exits element fullscreen and ignores a window-only fullscreen state", async () => {
    const w = world();
    const entering = w.first.requestFullscreen(); w.windowChanged(true); paint(w, 5); await entering;
    expect(w.api.exitAllFullscreen()).toBe(true);
    expect(w.document.fullscreenElement).toBeNull();
    expect(w.windowCalls).toEqual([true, false]);
    expect(w.api.exitAllFullscreen()).toBe(false);
    w.windowChanged(false); w.frame();
    const windowOnly = world(true);
    expect(windowOnly.api.exitAllFullscreen()).toBe(false);
    expect(windowOnly.windowCalls).toEqual([]);
  });

  test("removing a fullscreen element restores document and window state", async () => {
    const w = world();
    const entering = w.first.requestFullscreen(); w.windowChanged(true); paint(w, 5); await entering;
    w.first.isConnected = false;
    w.first.__vnId = 0;
    w.removed();
    expect(w.document.fullscreenElement).toBeNull();
    expect(w.elementCalls.at(-1)).toBe(0);
    expect(w.windowCalls.at(-1)).toBe(false);
    w.windowChanged(false); w.frame();
    await expect(w.first.requestFullscreen()).rejects.toThrow("connected element");
  });

  test("secondary fullscreen targets its owner and preserves an already covering window", async () => {
    const w = world();
    const owner = { __vnId: 99 };
    w.first.ownerDocument = { documentElement: owner };
    w.ownerStates.set(99, true);
    const entering = w.first.requestFullscreen(); paint(w, 6); await entering;
    expect(w.windowCalls).toEqual([]);
    expect(w.ownerCalls).toEqual([{ owner: 99, on: true }]);
    const exiting = w.document.exitFullscreen(); paint(w, 6); await exiting;
    expect(w.ownerCalls).toEqual([{ owner: 99, on: true }]);
    expect(w.windowCalls).toEqual([]);
  });

  test("closing a secondary owner cannot redirect its fullscreen exit to the main window", async () => {
    const w = world();
    const owner = { __vnId: 99 };
    w.first.ownerDocument = { documentElement: owner };
    const entering = w.first.requestFullscreen(); w.ownerStates.set(99, true); paint(w, 6); await entering;
    owner.__vnId = 0;
    w.first.isConnected = false;
    w.first.__vnId = 0;
    w.removed(); w.frame();
    expect(w.document.fullscreenElement).toBeNull();
    expect(w.windowCalls).toEqual([]);
    expect(w.ownerCalls).toEqual([{ owner: 99, on: true }]);
  });

  test("primary event waits for painted bounds and published resize metrics", async () => {
    const w = world();
    let bounds = { width: 900, height: 650 };
    w.first.getBoundingClientRect = () => bounds;
    w.global.innerWidth = 900; w.global.innerHeight = 650;
    const seen: object[] = [];
    w.document.addEventListener("fullscreenchange", () => seen.push({ ...bounds }));
    const entering = w.first.requestFullscreen();
    w.client.width = 2560; w.client.height = 1440;
    w.windowChanged(true);
    expect(seen).toEqual([]);
    w.frame();
    expect(seen).toEqual([]);
    bounds = { width: 2560, height: 1440 };
    w.frame();
    expect(seen).toEqual([]);
    w.global.innerWidth = 2560; w.global.innerHeight = 1440;
    paint(w, 3); 
    await entering;
    expect(seen).toEqual([{ width: 2560, height: 1440 }]);
  });

  test("secondary entry waits for native state and the owning window layout", async () => {
    const w = world();
    const owner = { __vnId: 99, getBoundingClientRect: () => ({ width: 200, height: 100 }) };
    w.first.ownerDocument = { documentElement: owner };
    let bounds = { width: 50, height: 30 };
    w.first.getBoundingClientRect = () => bounds;
    let settled = false;
    const entering = w.first.requestFullscreen().then(() => { settled = true; });
    w.frame(); await Promise.resolve();
    expect(settled).toBe(false);
    w.ownerStates.set(99, true);
    w.frame(); await Promise.resolve();
    expect(settled).toBe(false);
    bounds = { width: 200, height: 100 };
    paint(w, 3); 
    paint(w, 3); await entering;
    expect(settled).toBe(true);
  });

  test("an enter and exit in one JS wave settle without an OS state delta", async () => {
    const w = world();
    let changes = 0;
    w.document.addEventListener("fullscreenchange", () => changes++);
    const entering = w.first.requestFullscreen();
    const exiting = w.document.exitFullscreen();
    w.frame();
    paint(w, 2); 
    await Promise.all([entering, exiting]);
    expect(changes).toBe(2);
    expect(w.document.fullscreenElement).toBeNull();
  });
});


describe("native window fullscreen notifications", () => {
  test("window listeners see the new state without a DOM fullscreen owner", () => {
    const observerSource = source.slice(source.indexOf("  __vn_window_observe("),
      source.indexOf("  // The three return annotations"));
    let observe: (kinds: string, json: string) => boolean;
    const nativeChanges: boolean[] = [];
    const appWindow = new Function("EventTarget", "Event", "emit", "onFullscreenWindowChanged", "__vn_window_observe", "liveQueries",
      "let S; const appWindow = new EventTarget(); appWindow.isFullscreen = () => S.fullscreen;\n" +
      new Bun.Transpiler({ loader: "ts" }).transformSync(observerSource) + "\nreturn appWindow;")(
      EventTarget, Event, (target: EventTarget, kind: string) => target.dispatchEvent(new Event(kind)),
      (on: boolean) => nativeChanges.push(on), (callback: typeof observe) => { observe = callback; }, [],
    );
    const seen: boolean[] = [];
    appWindow.addEventListener("fullscreenchange", () => seen.push(appWindow.isFullscreen()));
    observe!("sync", JSON.stringify({ fullscreen: false }));
    expect(seen).toEqual([]);
    observe!("fullscreenchange", JSON.stringify({ fullscreen: true }));
    observe!("fullscreenchange", JSON.stringify({ fullscreen: false }));
    expect(seen).toEqual([true, false]);
    expect(nativeChanges).toEqual([true, false]);
  });
});
