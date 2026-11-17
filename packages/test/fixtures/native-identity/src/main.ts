import { createApp, h } from "@skathi-native/runtime";

type Ctor = new (...args: never[]) => object;
type Fn = (...args: never[]) => unknown;

const isConstructor = (f: unknown): boolean => {
  try {
    Reflect.construct(Object, [], f as Ctor);
    return true;
  } catch {
    return false;
  }
};

const throwsTypeError = (run: () => unknown): boolean => {
  try {
    run();
    return false;
  } catch (e) {
    return e instanceof TypeError;
  }
};

const nativeSource = (f: Fn | Ctor, name: string): boolean =>
  new RegExp(`^function ${name}\\(\\) \\{\\s+\\[native code\\]\\s+\\}$`).test(Function.prototype.toString.call(f));

const constructs: [string, Ctor, unknown[]][] = [
  ["Object", Object, []],
  ["Array", Array, [3]],
  ["Function", Function, ["return 1"]],
  ["String", String, ["s"]],
  ["Number", Number, [1]],
  ["Boolean", Boolean, [true]],
  ["Map", Map, [[[1, 2]]]],
  ["Set", Set, [[1, 2]]],
  ["WeakMap", WeakMap, []],
  ["WeakSet", WeakSet, []],
  ["WeakRef", WeakRef, [{}]],
  ["FinalizationRegistry", FinalizationRegistry, [() => {}]],
  ["Promise", Promise, [(resolve: (v: number) => void) => resolve(1)]],
  ["Date", Date, [0]],
  ["RegExp", RegExp, ["a+", "g"]],
  ["ArrayBuffer", ArrayBuffer, [8]],
  ["DataView", DataView, [new ArrayBuffer(8)]],
  ["Int8Array", Int8Array, [4]],
  ["Uint8Array", Uint8Array, [4]],
  ["Uint8ClampedArray", Uint8ClampedArray, [4]],
  ["Int16Array", Int16Array, [4]],
  ["Uint16Array", Uint16Array, [4]],
  ["Int32Array", Int32Array, [4]],
  ["Uint32Array", Uint32Array, [4]],
  ["Float32Array", Float32Array, [4]],
  ["Float64Array", Float64Array, [4]],
  ["BigInt64Array", BigInt64Array, [4]],
  ["BigUint64Array", BigUint64Array, [4]],
  ["Error", Error, ["e"]],
  ["TypeError", TypeError, ["e"]],
  ["RangeError", RangeError, ["e"]],
  ["AggregateError", AggregateError, [[], "e"]],
  ["Proxy", Proxy, [{}, {}]],
  ["URL", URL, ["https://example.com/a"]],
  ["AbortController", AbortController, []],
  ["EventTarget", EventTarget, []],
  ["Event", Event, ["x"]],
];

const results: Record<string, boolean> = {};
const check = (name: string, probe: () => boolean) => {
  try {
    results[name] = probe();
  } catch {
    results[name] = false;
  }
};

for (const [name, C, args] of constructs) {
  check(`new ${name}`, () => {
    const made = Reflect.construct(C, args) as object;
    const shaped = made !== null && (typeof made === "object" || typeof made === "function");
    // Proxy has no `prototype`, so `instanceof Proxy` would throw.
    return shaped && (typeof C.prototype !== "object" || made instanceof C);
  });
  check(`${name} is a constructor`, () => isConstructor(C));
  check(`${name} prints as native code`, () => nativeSource(C, name));
}

for (const [name, C] of [
  ["BigInt", BigInt],
  ["Symbol", Symbol],
] as const) {
  check(`${name} is a constructor`, () => isConstructor(C));
  check(`new ${name} throws a TypeError`, () => throwsTypeError(() => Reflect.construct(C as unknown as Ctor, [1])));
  check(`class extends ${name} defines`, () => {
    const Sub = class extends (C as unknown as Ctor) {};
    return typeof Sub === "function";
  });
  check(`${name} prints as native code`, () => nativeSource(C, name));
}
check("BigInt(1) is 1n", () => BigInt(1) === 1n);
check("Symbol() is a symbol", () => typeof Symbol("s") === "symbol");

const plain: [string, Fn][] = [
  ["max", Math.max],
  ["min", Math.min],
  ["parseInt", parseInt],
  ["parse", JSON.parse],
  ["map", Array.prototype.map as Fn],
  ["get", Reflect.get as Fn],
  ["freeze", Object.freeze as Fn],
  ["fromCharCode", String.fromCharCode as Fn],
];
for (const [name, f] of plain) {
  check(`${name} is not a constructor`, () => !isConstructor(f));
  check(`new ${name} throws a TypeError`, () => throwsTypeError(() => Reflect.construct(f as unknown as Ctor, [])));
  check(`${name} prints as native code`, () => nativeSource(f, name));
}

check("distinct natives stay distinct", () => {
  const fns: unknown[] = [
    Math.max,
    Math.min,
    Math.sin,
    Math.cos,
    Array.prototype.keys,
    Array.prototype.values,
    Array.prototype.entries,
    Map.prototype.get,
    Map.prototype.has,
    Set.prototype.has,
    Set.prototype.add,
    Object.keys,
    Object.values,
    BigInt,
    Symbol,
    Map,
    Set,
  ];
  return new Set(fns).size === fns.length;
});
check("shared natives stay shared", () => {
  const ap = Array.prototype as unknown as Record<symbol, unknown>;
  const mp = Map.prototype as unknown as Record<symbol, unknown>;
  return (
    ap[Symbol.iterator] === Array.prototype.values &&
    Set.prototype.keys === Set.prototype.values &&
    mp[Symbol.iterator] === Map.prototype.entries &&
    String.prototype.trimLeft === String.prototype.trimStart
  );
});
check("Math.max and Math.min answer differently", () => Math.max(1, 2) === 2 && Math.min(1, 2) === 1);
check("spread and Array.from over natives", () => {
  const spread = [...new Map([[1, 2]])];
  return spread.length === 1 && Array.from(new Set([1, 2, 2])).length === 2 && [..."abc"].join() === "a,b,c";
});
check("subclasses of natives", () => {
  class A extends Array<number> {}
  class M extends Map<number, number> {}
  class P extends Promise<number> {}
  class E extends Error {}
  class U extends Uint8Array {}
  return (
    new A(3).length === 3 &&
    new M([[1, 2]]).get(1) === 2 &&
    new P((resolve) => resolve(1)) instanceof Promise &&
    new E("e") instanceof Error &&
    new U(2).length === 2
  );
});
check("Date, RegExp and typed arrays behave", () => {
  const re = new RegExp("a+", "g");
  const f64 = new Float64Array([1.5, 2.5]);
  return new Date(0).getTime() === 0 && re.flags === "g" && "caab".replace(re, "-") === "c-b" && f64[1] === 2.5;
});

const failures = Object.keys(results).filter((name) => results[name] !== true);
(globalThis as unknown as { __SK_TEST_NATIVE_IDENTITY__: unknown }).__SK_TEST_NATIVE_IDENTITY__ = {
  checked: Object.keys(results).length,
  failures,
};

createApp({ render: () => h("div", { id: "identity" }, failures.length ? failures.join(", ") : "ok") }).mount("#app");
