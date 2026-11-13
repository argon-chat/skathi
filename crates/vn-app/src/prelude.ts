// Evaluated first, before window.prelude.js and the app bundle: Vue's esm-bundler build reads the
// `__VUE_*` flags off `globalThis` at module-eval time, bun will not inline them, and ESM import
// hoisting would otherwise run Vue first. Generated into prelude.js (`bun run preludes`).

globalThis.__VUE_OPTIONS_API__ = true;
globalThis.__VUE_PROD_DEVTOOLS__ = false;
globalThis.__VUE_PROD_HYDRATION_MISMATCH_DETAILS__ = false;
globalThis.__VN_CSS__ = [];
// `window.isVn` is true here and undefined in browsers. Read-only and non-configurable, so a
// polyfill cannot half-succeed at rewriting it.
Object.defineProperty(globalThis, "isVn", { value: true, writable: false, enumerable: true, configurable: false });
// redjs ships no `history`; vue-router's dev build touches it at startup. Inert stub.
// TODO: move into redjs as a native class.
globalThis.history = {
  state: null,
  length: 1,
  scrollRestoration: "auto",
  pushState() {},
  replaceState() {},
  go() {},
  back() {},
  forward() {},
};

// AOT locks engine builtins before entering the compiled application. Keep
// public event methods stable and change only their private delegate on reload.
(() => {
  type Method = (...args: unknown[]) => unknown;
  type Methods = { add?: Method; remove?: Method; dispatch?: Method };
  type Slot = { native: Readonly<Methods>; set(delegate: Methods): void; clear(delegate: Methods): void };
  type Bridge = { get(target: object): Slot | undefined };
  const host = globalThis as unknown as { document?: object; __VN_GLOBAL_EVENTS__?: Bridge };
  if (host.__VN_GLOBAL_EVENTS__) return;
  const slots = new WeakMap<object, Slot>();
  const install = (target: object) => {
    const source = target as Record<string, unknown>;
    const bind = (name: string): Method | undefined => {
      const method = source[name];
      return typeof method === "function" ? method.bind(target) : undefined;
    };
    const native = Object.freeze({
      add: bind("addEventListener"),
      remove: bind("removeEventListener"),
      dispatch: bind("dispatchEvent"),
    });
    let delegate: Methods | undefined;
    const slot = Object.freeze({
      native,
      set(next: Methods) {
        delegate = next;
      },
      clear(previous: Methods) {
        if (delegate === previous) delegate = undefined;
      },
    });
    for (const [name, key] of [
      ["addEventListener", "add"],
      ["removeEventListener", "remove"],
      ["dispatchEvent", "dispatch"],
    ] as const) {
      Object.defineProperty(target, name, {
        value: (...args: unknown[]) => {
          const method = delegate?.[key] ?? native[key];
          return method ? method(...args) : key === "dispatch" ? true : undefined;
        },
        // The window prelude still composes its bootstrap wrapper; AOT applies
        // its existing builtin locks after both preludes have completed.
        configurable: true,
        writable: true,
      });
    }
    slots.set(target, slot);
  };
  install(globalThis);
  if (host.document) install(host.document);
  Object.defineProperty(globalThis, "__VN_GLOBAL_EVENTS__", {
    value: Object.freeze({ get: (target: object) => slots.get(target) }),
    configurable: false,
    writable: false,
  });
})();
