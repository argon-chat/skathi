// Reads one of the runtime's TypeScript sources and hands back plain JS.
//
// The harnesses in this directory read a src/ file as text, strip its import/export syntax and evaluate the rest with
// every ambient name bound as a parameter, so each world is private. `new Function` cannot parse TypeScript, so the
// text goes through `Bun.Transpiler`, the same parser and emitter `Bun.build` uses for dist/vendor.js: types are
// erased, values and comments survive.
//
// `useDefineForClassFields: false` mirrors tsconfig.json: a bare `x: T` field must be erased, not turned into a
// constructor-prologue define that runs after `super()`.

import { readFileSync } from "node:fs";

const transpiler = new Bun.Transpiler({
  loader: "ts",
  target: "browser",
  tsconfig: JSON.stringify({ compilerOptions: { target: "esnext", useDefineForClassFields: false } }),
});

/** One src/*.ts file, as the JavaScript the bundler would emit for it. */
export function readTsSource(path) {
  return transpiler.transformSync(readFileSync(path, "utf8"));
}
