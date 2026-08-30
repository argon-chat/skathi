// Reads one of the runtime's TypeScript sources and hands back plain JS.
//
// The three harnesses in this directory all work the same way: they read a
// src/ file as TEXT, strip its import/export syntax, and evaluate the rest with
// every ambient name bound as a PARAMETER, so each world is private. That trick
// needs the text to be something `new Function` can parse — and since m20 these
// files are TypeScript, which it cannot.
//
// So the text goes through bun's own transpiler first. Not a second toolchain:
// `Bun.Transpiler` is the same parser and the same emitter `Bun.build` uses to
// produce dist/vendor.js, so the harness runs what ships, minus the bundling.
// Types (and type-only imports, and `declare` fields) are erased; every value
// declaration, and every comment position the harness regexes rely on, survives.
//
// `useDefineForClassFields: false` mirrors tsconfig.json, for the reason stated
// there: a bare `x: T` field declaration must be ERASED, not turned into a
// constructor-prologue `Object.defineProperty(this, "x", …)` that would run
// after `super()` and clobber what a base constructor had just set.

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
