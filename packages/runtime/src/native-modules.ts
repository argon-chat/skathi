// `native:<name>`: a module the embedding program registered (crates/sk-app/src/embed/module.rs),
// or, when there is none, a stand-in so the import itself never fails: `available` is false, every
// function throws and `on` never fires. The dev loader and the production bundle both ask here.

type NativeModule = Record<string, unknown>;

const modules = new Map<string, NativeModule>();

// Asked of any module object by bundler interop, promise resolution and coercion.
const ORDINARY = new Set<PropertyKey>(["then", "__esModule", "default", "toJSON"]);

function standIn(name: string, why: string): NativeModule {
  const base = {};
  const functions = new Proxy(base, {
    get(target, key, receiver) {
      if (typeof key === "symbol" || ORDINARY.has(key) || key in target) return Reflect.get(target, key, receiver);
      return () => {
        throw new Error(`native:${name}.${key}(): ${why}; check \`available\` before calling it`);
      };
    },
  });
  const module = Object.create(functions) as NativeModule;
  module.available = false;
  module.on = () => () => {};
  module.off = () => {};
  return module;
}

export function nativeModule(name: string): NativeModule {
  let module = modules.get(name);
  if (module) return module;
  if (typeof __sk_native_module !== "function") {
    module = standIn(
      name,
      `native:${name} is provided by the program that embeds this app, and this app is not embedded`,
    );
  } else {
    module =
      __sk_native_module(name) ?? standIn(name, `the program that embeds this app has no native module "${name}"`);
  }
  modules.set(name, module);
  return module;
}
