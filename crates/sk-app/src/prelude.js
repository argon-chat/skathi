// GENERATED from crates/sk-app/src/prelude.ts — DO NOT EDIT.
//
//   source:   crates/sk-app/src/prelude.ts
//   emitter:  Bun.Transpiler (the same one build.ts uses for dist/vendor.js)
//   regen:    bun scripts/gen-preludes.ts
//   check:    bun scripts/gen-preludes.ts --check
//
// evaluated first, as `sk://prelude.js`, before the window prelude and the app bundle.
//
// Every comment in the source is gone: bun re-prints from the AST. Read the
// .ts — it is the file with the reasoning in it, and the only one an edit
// survives.
globalThis.__VUE_OPTIONS_API__ = true;
globalThis.__VUE_PROD_DEVTOOLS__ = false;
globalThis.__VUE_PROD_HYDRATION_MISMATCH_DETAILS__ = false;
globalThis.__SK_CSS__ = [];
Object.defineProperty(globalThis, "isSk", { value: true, writable: false, enumerable: true, configurable: false });
(() => {
  const host = globalThis;
  if (host.__SK_GLOBAL_EVENTS__)
    return;
  const slots = new WeakMap;
  const install = (target) => {
    const source = target;
    const bind = (name) => {
      const method = source[name];
      return typeof method === "function" ? method.bind(target) : undefined;
    };
    const native = Object.freeze({
      add: bind("addEventListener"),
      remove: bind("removeEventListener"),
      dispatch: bind("dispatchEvent")
    });
    let delegate;
    const slot = Object.freeze({
      native,
      set(next) {
        delegate = next;
      },
      clear(previous) {
        if (delegate === previous)
          delegate = undefined;
      }
    });
    for (const [name, key] of [
      ["addEventListener", "add"],
      ["removeEventListener", "remove"],
      ["dispatchEvent", "dispatch"]
    ]) {
      Object.defineProperty(target, name, {
        value: (...args) => {
          const method = delegate?.[key] ?? native[key];
          return method ? method(...args) : key === "dispatch" ? true : undefined;
        },
        configurable: true,
        writable: true
      });
    }
    slots.set(target, slot);
  };
  install(globalThis);
  if (host.document)
    install(host.document);
  Object.defineProperty(globalThis, "__SK_GLOBAL_EVENTS__", {
    value: Object.freeze({ get: (target) => slots.get(target) }),
    configurable: false,
    writable: false
  });
})();
