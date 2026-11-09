// The FIRST thing this process evaluates — `vn://prelude.js`, before
// `window.prelude.js` and before the app bundle (crates/vn-app/src/lib.rs).
//
// Why it must be first is documented on the Rust const that `include_str!`s the
// generated JavaScript, because that reason is about the ORDER of the two
// `rt.eval` calls rather than about the code: Vue's esm-bundler build reads the
// three `__VUE_*` flags off `globalThis` at module-eval time, bun refuses to
// inline them, and ESM import hoisting would otherwise put Vue's own code first.
//
// This file is authored in TypeScript and the `.js` beside it is GENERATED —
// `bun packages/vue-native/scripts/gen-preludes.ts`. See that script, and
// `tsconfig.json` in this directory for how a file with no imports gets its
// ambient declarations.

globalThis.__VUE_OPTIONS_API__ = true;
globalThis.__VUE_PROD_DEVTOOLS__ = false;
globalThis.__VUE_PROD_HYDRATION_MISMATCH_DETAILS__ = false;
globalThis.__VN_CSS__ = [];
// vue-native's identity flag (apps/env.d.ts): `window.isVn === true` here and
// `undefined` in every browser, so an app can branch on the host in plain
// code, `if (window.isVn)`, without importing "vue-native". Defined first,
// with the `__VUE_*` flags, so the bundle's module scope already sees it.
// Read-only and non-configurable: an identity a script can rewrite is no
// identity, and a polyfill that sets `isVn = false` to look like a browser
// must fail, not succeed half-way.
Object.defineProperty(globalThis, "isVn", { value: true, writable: false, enumerable: true, configurable: false });
// Browser-compat: redjs ships no `history`; vue-router's dev build touches
// it at startup even with memory history (its isBrowser heuristic sees our
// window/document). Inert stub, like `location`. TODO: move into redjs
// webapi as a proper native class.
globalThis.history = {
  state: null, length: 1, scrollRestoration: "auto",
  pushState() {}, replaceState() {}, go() {}, back() {}, forward() {},
};
// (The milestone-1 500ms "tick" status heartbeat used to live here. Every
// update was a shell notify, i.e. a FULL element-tree rebuild — 40-100ms
// twice a second on a component-heavy app. Status now only moves on real
// events.)

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
    const native = Object.freeze({ add: bind("addEventListener"), remove: bind("removeEventListener"), dispatch: bind("dispatchEvent") });
    let delegate: Methods | undefined;
    const slot = Object.freeze({ native,
      set(next: Methods) { delegate = next; },
      clear(previous: Methods) { if (delegate === previous) delegate = undefined; },
    });
    for (const [name, key] of [["addEventListener", "add"], ["removeEventListener", "remove"], ["dispatchEvent", "dispatch"]] as const) {
      Object.defineProperty(target, name, {
        value: (...args: unknown[]) => {
          const method = delegate?.[key] ?? native[key];
          return method ? method(...args) : key === "dispatch" ? true : undefined;
        },
        // The window prelude still composes its bootstrap wrapper; AOT applies
        // its existing builtin locks after both preludes have completed.
        configurable: true, writable: true,
      });
    }
    slots.set(target, slot);
  };
  install(globalThis);
  if (host.document) install(host.document);
  Object.defineProperty(globalThis, "__VN_GLOBAL_EVENTS__", {
    value: Object.freeze({ get: (target: object) => slots.get(target) }),
    configurable: false, writable: false,
  });
})();
