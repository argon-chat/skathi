import { warnCompat } from "../compat";

// `appWindow` is the host's native object, installed before the app bundle evaluates; this module
// is only its namespace. `getCurrentWindow()` exists so ported Tauri code runs unmodified.
const windowHost = globalThis.__SK_WINDOW__;
// Lazy: a runtime without the windowing host (e.g. the unit test harness) must not be warned about an
// API it never touches.
const missing = () =>
  warnCompat("appWindow", "the windowing host is missing — `appWindow` is inert; run the app in the Skathi host.");
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
const hooks = new Map<string, { read: () => void; changed: (() => void) | undefined }>();
const readHook = () => {
  for (const hook of hooks.values()) hook.read();
};
const changedHook = () => {
  for (const hook of hooks.values()) hook.changed?.();
};

/**
 * Renderer glue: runs `read` on every `appWindow` state read and `changed` when that state changes; `key` names the
 * renderer, and a reload's call replaces its predecessor's.
 */
export function trackWindowReads(key: string, read: () => void, changed?: () => void): void {
  hooks.set(key, { read, changed });
}
// A reload re-evaluates this module but not the host: the previous evaluation's wrappers are undone first.
const UNTRACK = Symbol.for("skathi.appWindow.untrack");

// The host's getters read a plain snapshot. Each read also runs the renderers' read hooks, whose signal the
// host's change notice bumps, so effects track a read and nothing else. Own wrappers over the class's members,
// not a Proxy: `event.target === appWindow` must hold.
function trackReads(target: SkAppWindow, onChange: (fn: () => void) => () => void): void {
  const slot = target as unknown as Record<symbol, unknown>;
  const previous = slot[UNTRACK];
  if (typeof previous === "function") previous();
  const own = Object.getOwnPropertyDescriptors(target);
  const members: Record<string, PropertyDescriptor> = {};
  for (
    let proto: object | null = Object.getPrototypeOf(target) as object | null;
    proto && proto !== EventTarget.prototype && proto !== Object.prototype;
    proto = Object.getPrototypeOf(proto) as object | null
  ) {
    for (const [key, d] of Object.entries(Object.getOwnPropertyDescriptors(proto))) {
      if (key !== "constructor" && !(key in members)) members[key] = d;
    }
  }
  Object.assign(members, own);
  const unsubscribe = onChange(changedHook);
  Object.defineProperty(target, UNTRACK, {
    configurable: true,
    value: () => {
      unsubscribe();
      for (const key of Object.keys(members)) {
        const d = own[key];
        if (d) Object.defineProperty(target, key, d);
        else Reflect.deleteProperty(target, key);
      }
      delete slot[UNTRACK];
    },
  });
  for (const [key, d] of Object.entries(members)) {
    const get = d.get;
    if (get) {
      Object.defineProperty(target, key, {
        ...d,
        get(this: unknown) {
          const value: unknown = get.call(this);
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
