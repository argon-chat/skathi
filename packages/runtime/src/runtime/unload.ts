import { resetGlobals, traceCallbackBegin, traceCallbackEnd } from "./realm";
import { nativeListenFns, passedThrough } from "./listeners";
import { interceptedGlobals } from "./state";
import { warnCompat } from "../compat";
import { intersectionObservers } from "./intersection-observer";
import { hostSteps } from "./layout-steps";
import { resizeDue, resizeObservers } from "./resize-observer";

// location.reload(): the host (`__sk_reload`) calls `__SK_UNLOAD__`, then re-evaluates the bundle in the same realm,
// so the outgoing app's timers, frames, idle callbacks, WebSockets (listeners removed first, so reconnect logic
// cannot run) and runtime-sourced window/document listeners are undone here, after `pagehide` and the unmount.

type Fn = (...args: unknown[]) => unknown;
type SocketLike = {
  close(): void;
  addEventListener?: Fn;
  removeEventListener?: Fn;
  [handler: string]: unknown;
};

/// Called with each frame's timestamp before a rAF callback runs, so `document.timeline` follows it.
export const frameStampers: ((timestamp: number) => void)[] = [];

const realm = (() => {
  const g = globalThis as unknown as Record<string, unknown>;
  // The engine's own functions, kept ONCE per process (see `nativeListenFns`).
  const native = (g.__SK_NATIVE_SCHED__ ??= {
    setTimeout: g.setTimeout,
    clearTimeout: g.clearTimeout,
    setInterval: g.setInterval,
    clearInterval: g.clearInterval,
    requestAnimationFrame: g.requestAnimationFrame,
    cancelAnimationFrame: g.cancelAnimationFrame,
    requestIdleCallback: g.requestIdleCallback,
    cancelIdleCallback: g.cancelIdleCallback,
    WebSocket: g.WebSocket,
  }) as Record<string, Fn | undefined>;
  const timers = new Set<unknown>();
  const frames = new Set<unknown>();
  const idles = new Set<unknown>();
  const sockets = new Map<SocketLike, { type: string; fn: unknown; opts: unknown }[]>();

  const track = (setName: string, live: Set<unknown>, once: boolean) => {
    const set = native[setName];
    if (typeof set !== "function") return;
    g[setName] = function (cb: unknown, ...rest: unknown[]) {
      if (typeof cb !== "function") return set.call(g, cb, ...rest); // the string form
      let id: unknown;
      const run = once
        ? function (this: unknown, ...a: unknown[]) {
            live.delete(id);
            if (live === frames && typeof a[0] === "number") for (const stamp of frameStampers) stamp(a[0]);
            const token = traceCallbackBegin?.(cb) ?? 0;
            if (token === 0) return (cb as Fn).apply(this, a);
            try {
              return (cb as Fn).apply(this, a);
            } finally {
              traceCallbackEnd!(token);
            }
          }
        : cb;
      id = set.call(g, run, ...rest);
      live.add(id);
      return id;
    };
  };
  const untrack = (clearName: string, ...lives: Set<unknown>[]) => {
    const clear = native[clearName];
    if (typeof clear !== "function") return;
    g[clearName] = function (id: unknown) {
      for (const live of lives) live.delete(id);
      return clear.call(g, id);
    };
  };
  track("setTimeout", timers, true);
  track("setInterval", timers, false);
  // One id space for both (HTML): either clear cancels either.
  untrack("clearTimeout", timers);
  untrack("clearInterval", timers);
  track("requestAnimationFrame", frames, true);
  untrack("cancelAnimationFrame", frames);
  track("requestIdleCallback", idles, true);
  untrack("cancelIdleCallback", idles);

  const NativeWS = native.WebSocket;
  if (typeof NativeWS === "function") {
    g.WebSocket = new Proxy(NativeWS, {
      construct(target, args, newTarget) {
        const ws = Reflect.construct(target, args, newTarget) as SocketLike;
        const added: { type: string; fn: unknown; opts: unknown }[] = [];
        sockets.set(ws, added);
        const add = ws.addEventListener;
        const remove = ws.removeEventListener;
        if (typeof add === "function") {
          Object.defineProperty(ws, "addEventListener", {
            value(type: string, fn: unknown, opts?: unknown) {
              added.push({ type, fn, opts });
              return add.call(ws, type, fn, opts);
            },
            configurable: true,
            writable: true,
          });
          add.call(ws, "close", () => sockets.delete(ws));
        }
        if (typeof remove === "function") {
          Object.defineProperty(ws, "removeEventListener", {
            value(type: string, fn: unknown, opts?: unknown) {
              const i = added.findIndex((a) => a.type === type && a.fn === fn);
              if (i >= 0) added.splice(i, 1);
              return remove.call(ws, type, fn, opts);
            },
            configurable: true,
            writable: true,
          });
        }
        return ws as object;
      },
    });
  }

  const close = () => {
    for (const { target, name, fn, opts } of passedThrough.splice(0)) {
      try {
        nativeListenFns(target).remove?.(name, fn, opts);
      } catch {
        /* already gone */
      }
    }
    for (const { slot, delegate } of interceptedGlobals.splice(0)) slot.clear(delegate);
    for (const [ws, added] of sockets) {
      const remove = Object.getOwnPropertyDescriptor(ws, "removeEventListener")?.value as Fn | undefined;
      for (const a of added.splice(0)) remove?.call(ws, a.type, a.fn, a.opts);
      for (const h of ["onopen", "onmessage", "onerror", "onclose"]) ws[h] = null;
      try {
        ws.close();
      } catch {
        /* closing a closed socket */
      }
    }
    sockets.clear();
    for (const id of timers) native.clearTimeout?.call(g, id);
    for (const id of frames) native.cancelAnimationFrame?.call(g, id);
    for (const id of idles) native.cancelIdleCallback?.call(g, id);
    timers.clear();
    frames.clear();
    idles.clear();
  };
  return { close };
})();

{
  const g = globalThis as unknown as {
    location?: Record<string, unknown>;
    __SK_UNLOAD__?: (() => void) | undefined;
    __SK_TEARDOWN__?: () => void;
  };
  if (g.location && typeof __sk_reload === "function") {
    try {
      Object.defineProperty(g.location, "reload", {
        value: function reload() {
          __sk_reload();
        },
        configurable: true,
        writable: true,
      });
    } catch (e) {
      warnCompat(
        "location.reload:install",
        `location.reload() could not be installed on this engine and does nothing (${e})`,
      );
    }
  }
  g.__SK_UNLOAD__ = () => {
    g.__SK_UNLOAD__ = undefined;
    delete (globalThis as { __SK_SCREEN_SOURCE__?: unknown }).__SK_SCREEN_SOURCE__;
    warnCompat(
      "location.reload",
      "location.reload() re-runs the app in the SAME JavaScript realm: its timers, animation frames, " +
        "WebSockets, window/document listeners, media tracks " +
        "and AudioContexts are closed and the globals it added are deleted, but " +
        "changes it made to built-in objects and to globals that were already there stay, fetches in " +
        "flight settle into the old code, IndexedDB connections stay until the old code " +
        "ends them, beforeunload is not asked, and DOMContentLoaded / load do not fire again",
    );
    try {
      const ev = typeof Event === "function" ? new Event("pagehide") : null;
      if (ev) {
        Object.defineProperty(ev, "persisted", { value: false });
        (globalThis as unknown as { dispatchEvent?: (e: Event) => boolean }).dispatchEvent?.(ev);
      }
    } catch (e) {
      __sk_log("reload: pagehide: " + e);
    }
    try {
      g.__SK_TEARDOWN__?.();
    } catch (e) {
      __sk_log("reload: teardown: " + e);
    }
    (globalThis as { __SK_HOST_LIFECYCLE__?: { reset: () => void } }).__SK_HOST_LIFECYCLE__?.reset();
    realm.close();
    // The host keeps calling the old observer step until the next runtime installs its own; tell it to stop.
    resizeObservers.length = 0;
    resizeDue.length = 0;
    intersectionObservers.clear();
    if (hostSteps) __sk_layout_watch(false);
    // Last: the teardown above still calls through the globals this deletes.
    const kept = resetGlobals();
    if (kept)
      warnCompat(
        "location.reload:globals",
        `location.reload(): ${kept} global(s) the old app declared cannot be deleted (a top-level var or ` +
          "function in a classic script, or a non-configurable property), so the new app finds them set",
      );
  };
}
