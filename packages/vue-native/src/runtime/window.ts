import { warnCompat } from "../compat";

// `appWindow` is built in the host's JS prelude, which must evaluate before the app bundle; this
// module is only its namespace. `getCurrentWindow()` exists so ported Tauri code runs unmodified.
const windowHost = globalThis.__VN_WINDOW__;
// Lazy: a host without the window prelude (e.g. the unit test harness) must not be warned about an
// API it never touches.
const missing = () =>
  warnCompat(
    "appWindow",
    "the windowing host is missing (the window prelude did not evaluate) — " +
      "`appWindow` is inert; run the app in the vue-native host.",
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

export const appWindow: VnAppWindow = windowHost ? windowHost.appWindow : (inertWindow as unknown as VnAppWindow);
export const defineWindow: (options: Record<string, unknown>) => void = windowHost ? windowHost.defineWindow : missing;
export const getCurrentWindow = (): VnAppWindow => appWindow;
