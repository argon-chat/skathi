// Nothing here may be reported.
class Mine {
  push(): number {
    return 0;
  }
}
Mine.prototype.push = function () {
  return 1;
};

export function shadowed() {
  const Array = { prototype: { push: 0 } };
  Array.prototype.push = 1;
  const Math = { max: 0 };
  Math.max = 1;
  return [Array, Math];
}

export function shadowedFunction(Function: (code: string) => unknown) {
  return new (Function as unknown as new (code: string) => unknown)("x");
}

(Array.prototype as unknown as Record<string, unknown>).last = function () {};
Object.defineProperty(Array.prototype, "second", { value: () => 1 });

if (!Array.prototype.at) {
  Array.prototype.at = function () {
    return undefined;
  };
}
Array.prototype.flat ??= function () {
  return [];
};

const savedPush = Array.prototype.push;
Array.prototype.push = savedPush;

Date.now = () => 0;
JSON.parse = (text: string) => text;
globalThis.Promise = Promise;
(globalThis as Record<string, unknown>).myGlobal = 1;

export const kinds = [typeof eval, () => import("./lazy"), setTimeout(() => {}, 1)];
