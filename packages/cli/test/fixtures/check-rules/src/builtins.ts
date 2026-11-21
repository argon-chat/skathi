Array.prototype.push = function (...items: unknown[]) {
  return items.length;
};
delete (String.prototype as { padStart?: unknown }).padStart;
Math.max = () => 0;
(Object.prototype as Record<string, unknown>).extra = 1;
Array.prototype["indexOf"] = () => -1;
const AP = Array.prototype;
AP.slice = () => [];

Object.defineProperty(Array.prototype, "includes", { value: () => false });
Object.defineProperties(Array.prototype, { fill: { value: () => [] } });
Object.assign(Array.prototype, { map: () => [] });
Reflect.set(Math, "min", () => 0);

Object.setPrototypeOf(Array.prototype, null);
(Map as unknown as { __proto__: unknown }).__proto__ = {};

globalThis.Map = class {} as unknown as MapConstructor;
window.Array = [] as unknown as ArrayConstructor;
delete (globalThis as { Set?: unknown }).Set;
console.error = () => {};
for (const name of ["pop", "shift"] as const) Array.prototype[name] = () => undefined;
declare const which: "keys" | "values";
Array.prototype[which] = () => [][Symbol.iterator]();
declare const Runtime: { isAot(): boolean };
if (!Runtime.isAot()) console.warn = () => {};

export const done = true;
