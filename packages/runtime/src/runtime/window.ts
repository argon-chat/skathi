import { shallowRef } from "@vue/runtime-core";
import { warnCompat } from "../compat";

// `appWindow` is built in the host's JS prelude, which must evaluate before the app bundle; this
// module is only its namespace. `getCurrentWindow()` exists so ported Tauri code runs unmodified.
const windowHost = globalThis.__SK_WINDOW__;
// Lazy: a host without the window prelude (e.g. the unit test harness) must not be warned about an
// API it never touches.
const missing = () =>
  warnCompat(
    "appWindow",
    "the windowing host is missing (the window prelude did not evaluate) — " +
      "`appWindow` is inert; run the app in the Skathi host.",
  );
const inertWindow = new Proxy(
  {},
  {
    get(_t, key) {
      missing();
      // Verbs stay callable and `is*` answers false, so inert code paths do not throw.
      return typeof key === "string" && key.startsWith("is") ? () => false : () => undefined;
    },
    set() {
      missing();
      return true;
    },
  },
);

const STATE_METHOD = /^(is|get)[A-Z]/;
const readHooks = new Map<string, () => void>();
const readHook = () => {
  for (const hook of readHooks.values()) hook();
};

/** Renderer glue: runs `hook` on every `appWindow` state read, as Vue tracks one; `key` names the renderer, and a reload's call replaces its predecessor's. */
export function trackWindowReads(key: string, hook: () => void): void {
  readHooks.set(key, hook);
}
// A reload re-evaluates this module but not the prelude: the previous evaluation's wrappers are undone first.
const UNTRACK = Symbol.for("skathi.appWindow.untrack");

// The prelude's getters read a plain snapshot. Each read also reads one signal, which the host bumps
// whenever that snapshot changes, so effects track a read and nothing else. In place, not a Proxy:
// `event.target === appWindow` must hold.
function trackReads(target: SkAppWindow, onChange: (fn: () => void) => () => void): void {
  const slot = target as unknown as Record<symbol, unknown>;
  const previous = slot[UNTRACK];
  if (typeof previous === "function") previous();
  const original = Object.getOwnPropertyDescriptors(target);
  const version = shallowRef(0);
  let bumps = 0;
  const unsubscribe = onChange(() => {
    version.value = ++bumps;
  });
  Object.defineProperty(target, UNTRACK, {
    configurable: true,
    value: () => {
      unsubscribe();
      for (const [key, d] of Object.entries(original)) Object.defineProperty(target, key, d);
      delete slot[UNTRACK];
    },
  });
  for (const [key, d] of Object.entries(original)) {
    const get = d.get;
    if (get) {
      Object.defineProperty(target, key, {
        ...d,
        get(this: unknown) {
          const value: unknown = get.call(this);
          void version.value;
          readHook();
          return value;
        },
      });
      continue;
    }
    const fn: unknown = d.value;
    if (!STATE_METHOD.test(key) || typeof fn !== "function") continue;
    Object.defineProperty(target, key, {
      ...d,
      value(this: unknown, ...args: unknown[]): unknown {
        const value: unknown = Reflect.apply(fn, this, args);
        void version.value;
        readHook();
        return value;
      },
    });
  }
}

if (windowHost?.onChange) trackReads(windowHost.appWindow, windowHost.onChange);

export const appWindow: SkAppWindow = windowHost ? windowHost.appWindow : (inertWindow as unknown as SkAppWindow);
export const defineWindow: (options: Record<string, unknown>) => void = windowHost ? windowHost.defineWindow : missing;
export const getCurrentWindow = (): SkAppWindow => appWindow;
