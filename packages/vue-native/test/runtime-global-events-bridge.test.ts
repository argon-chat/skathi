import { describe, expect, test } from "bun:test";
import { resolve } from "node:path";
import { createRuntime, type RealmGlobal } from "./runtime-harness";
import { readTsSource } from "./ts-source";

type Listener = (event: Event) => unknown;
interface Targetish {
  addEventListener(type: string, listener: Listener, options?: boolean | AddEventListenerOptions): void;
  removeEventListener(type: string, listener: Listener, options?: boolean | EventListenerOptions): void;
  dispatchEvent(event: Event): boolean;
}
interface WindowLike extends Targetish {
  __VN_UNLOAD__(): void;
}

const prelude = readTsSource(resolve(import.meta.dir, "../../../crates/vn-app/src/prelude.ts"));
function world() {
  const nativeWindow = new EventTarget();
  const document = Object.assign(new EventTarget(), { head: {} });
  let originalMessages = 0;
  document.addEventListener("message", () => originalMessages++);
  const identities: unknown[] = [];
  const rt = createRuntime({
    globals: {
      document,
      Event,
      EventTarget,
      addEventListener: nativeWindow.addEventListener.bind(nativeWindow),
      removeEventListener: nativeWindow.removeEventListener.bind(nativeWindow),
      dispatchEvent: nativeWindow.dispatchEvent.bind(nativeWindow),
    },
    beforeRuntime(global) {
      new Function("globalThis", prelude)(global);
      for (const target of [global, document]) {
        for (const name of ["addEventListener", "removeEventListener", "dispatchEvent"]) {
          const value: unknown = Reflect.get(target, name);
          identities.push(value);
          Object.defineProperty(target, name, { value, writable: false, configurable: false });
        }
      }
    },
  });
  rt.createApp({ render: () => null }).mount();
  const target = rt.nodeOps.createElement("button");
  rt.nodeOps.insert(target, rt.global.document.body, null);
  return {
    rt,
    realm: rt.global as RealmGlobal,
    g: rt.global as unknown as WindowLike,
    document: rt.global.document as unknown as Targetish,
    target,
    identities,
    originalMessages: () => originalMessages,
  };
}

describe("bootstrap global event forwarding with sealed builtin methods", () => {
  test("host clicks reach window/document without redefining their sealed methods", () => {
    const { rt, g, document, target, identities } = world();
    const seen: string[] = [];
    g.addEventListener("click", () => seen.push("window-c"), true);
    document.addEventListener("click", () => seen.push("document-c"), true);
    target.addEventListener("click", () => seen.push("target"));
    document.addEventListener("click", () => seen.push("document-b"));
    g.addEventListener("click", () => seen.push("window-b"));
    rt.host.dispatcher(target.__vnId, rt.EV_CODES.click, 0, 0, 0, 0, "");
    expect(seen).toEqual(["window-c", "document-c", "target", "document-b", "window-b"]);
    expect<unknown[]>([
      g.addEventListener,
      g.removeEventListener,
      g.dispatchEvent,
      document.addEventListener,
      document.removeEventListener,
      document.dispatchEvent,
    ]).toEqual(identities);
    expect(Object.getOwnPropertyDescriptor(document, "dispatchEvent")?.writable).toBe(false);
    expect(rt.advisories().some((warning: string) => warning.includes("could not be intercepted"))).toBe(false);
    g.__VN_UNLOAD__();
  });

  test("runtime dispatch preserves cancellation/once while native listeners still receive native events", () => {
    const { rt, g, document, originalMessages } = world();
    let captures = 0,
      bubbles = 0,
      pageMessages = 0;
    document.addEventListener("fullscreenchange", () => captures++, { capture: true, once: true });
    document.addEventListener("fullscreenchange", (event: Event) => {
      bubbles++;
      event.preventDefault();
    });
    expect(document.dispatchEvent(new Event("fullscreenchange", { bubbles: true, cancelable: true }))).toBe(false);
    expect(document.dispatchEvent(new Event("fullscreenchange", { bubbles: true, cancelable: true }))).toBe(false);
    expect([captures, bubbles]).toEqual([1, 2]);
    const message = () => pageMessages++;
    document.addEventListener("message", message);
    expect(document.dispatchEvent(new Event("message"))).toBe(true);
    expect([originalMessages(), pageMessages]).toEqual([1, 1]);
    document.removeEventListener("message", message);
    document.dispatchEvent(new Event("message"));
    expect([originalMessages(), pageMessages]).toEqual([2, 1]);
    expect(rt.advisories().some((warning: string) => warning.includes("could not be intercepted"))).toBe(false);
    g.__VN_UNLOAD__();
  });

  test("reload drops the old delegate and its native page listeners but keeps bootstrap/native listeners", () => {
    const { realm, g, document, originalMessages } = world();
    let oldMessages = 0,
      newMessages = 0,
      newRuntimeEvents = 0;
    document.addEventListener("message", () => oldMessages++);
    document.dispatchEvent(new Event("message"));
    g.__VN_UNLOAD__();
    document.dispatchEvent(new Event("message"));
    expect([originalMessages(), oldMessages]).toEqual([2, 1]);
    const second = createRuntime({ realm });
    second.createApp({ render: () => null }).mount();
    document.addEventListener("message", () => newMessages++);
    document.addEventListener("fullscreenchange", () => newRuntimeEvents++);
    document.dispatchEvent(new Event("message"));
    document.dispatchEvent(new Event("fullscreenchange"));
    expect([originalMessages(), oldMessages, newMessages, newRuntimeEvents]).toEqual([3, 1, 1, 1]);
    expect(second.advisories().some((warning: string) => warning.includes("could not be intercepted"))).toBe(false);
    g.__VN_UNLOAD__();
  });
});
