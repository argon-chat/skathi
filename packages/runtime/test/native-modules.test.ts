// `native:<name>` (src/native-modules.ts): the embedder's module object when it registered one,
// otherwise a stand-in an import never fails on.
import { afterEach, expect, test } from "bun:test";

type NativeModules = typeof import("../src/native-modules");

const G = globalThis as Record<string, unknown>;
let fresh = 0;

async function load(): Promise<NativeModules> {
  return (await import(`../src/native-modules.ts?case=${++fresh}`)) as NativeModules;
}

afterEach(() => {
  delete G.__sk_native_module;
});

/** What Bun's `__toESM` makes of a CommonJS module: own properties copied, the prototype kept. */
function bundlerView(module: object): Record<string, unknown> {
  const view = Object.create(Object.getPrototypeOf(module)) as Record<string, unknown>;
  for (const key of Object.getOwnPropertyNames(module)) {
    Object.defineProperty(view, key, { get: () => (module as Record<string, unknown>)[key], enumerable: true });
  }
  return view;
}

test("outside an embedder every native module is unavailable: its functions throw and on never fires", async () => {
  const { nativeModule } = await load();
  const notes = nativeModule("notes");
  expect(nativeModule("notes")).toBe(notes);
  expect(notes.available).toBe(false);
  const search = notes.search as () => unknown;
  expect(() => search()).toThrow(Error);
  expect(() => (bundlerView(notes).get as () => unknown)()).toThrow(Error);
  const off = (notes.on as (event: string, cb: () => void) => () => void)("progress", () => {
    throw new Error("an unavailable module fired an event");
  });
  expect(typeof off).toBe("function");
  off();
  // Awaiting or coercing the module must not mistake it for a promise or call a stand-in.
  expect(notes.then).toBeUndefined();
  expect(notes.__esModule).toBeUndefined();
  expect(await Promise.resolve(notes)).toBe(notes);
  expect(String(notes)).toBe("[object Object]");
});

test("embedded, a registered module is the embedder's object and an unknown one is unavailable", async () => {
  const notes = { available: true, get: (id: number) => ({ id }) };
  G.__sk_native_module = (name: string) => (name === "notes" ? notes : undefined);
  const { nativeModule } = await load();
  expect(nativeModule("notes")).toBe(notes);
  const other = nativeModule("other");
  expect(other.available).toBe(false);
  expect(() => (other.anything as () => unknown)()).toThrow(Error);
});
