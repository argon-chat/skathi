// The host preludes: `crates/sk-app/src/*.ts` -> the `.js` the Rust binary `include_str!`s.
//
// Usage
//   bun scripts/gen-preludes.ts           regenerate the checked-in artifacts
//   bun scripts/gen-preludes.ts --check   CI gate: regenerate in memory and fail if a checked-in file is stale
//
// The `.js` is checked in so `cargo build` needs no bun or node and stack traces point at a real file.
// `packages/runtime/test/prelude-generated.test.ts` keeps the bytes honest.
//
// Bun drops the directive prologue, but `rt.eval` evaluates the artifact as a classic script, where
// dropping "use strict" changes behaviour; a target may ask for the directive back, per target
// because prelude.js is not strict.

import { readFileSync, writeFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";

const REPO = resolve(import.meta.dir, "..");
const HOST = join(REPO, "crates", "sk-app", "src");

const COMMAND = "bun scripts/gen-preludes.ts";

export interface Target {
  source: string;
  out: string;
  strict: boolean;
  what: string;
}

export const TARGETS: Target[] = [
  {
    source: join(HOST, "prelude.ts"),
    out: join(HOST, "prelude.js"),
    strict: false,
    what: "evaluated first, as `sk://prelude.js`, before the window prelude and the app bundle",
  },
  {
    source: join(HOST, "window.prelude.ts"),
    out: join(HOST, "window.prelude.js"),
    strict: true,
    what: "evaluated as `sk://window.js`, after the prelude and before the app bundle",
  },
];

// The options `packages/runtime/test/ts-source.ts` passes and `build.ts` gets from tsconfig.json. `useDefineForClassFields:
// false` is load-bearing: a bare `x: T` field must be erased, not turned into a define that runs after `super()`.
const transpiler = new Bun.Transpiler({
  loader: "ts",
  target: "browser",
  tsconfig: JSON.stringify({ compilerOptions: { target: "esnext", useDefineForClassFields: false } }),
});

function header(t: Target): string {
  const src = relative(REPO, t.source).replace(/\\/g, "/");
  return (
    `// GENERATED from ${src} — DO NOT EDIT.\n` +
    `//\n` +
    `//   source:   ${src}\n` +
    `//   emitter:  Bun.Transpiler (the same one build.ts uses for dist/vendor.js)\n` +
    `//   regen:    ${COMMAND}\n` +
    `//   check:    ${COMMAND} --check\n` +
    `//\n` +
    `// ${t.what}.\n` +
    `//\n` +
    `// Every comment in the source is gone: bun re-prints from the AST. Read the\n` +
    `// .ts — it is the file with the reasoning in it, and the only one an edit\n` +
    `// survives.\n`
  );
}

export function render(t: Target): string {
  const js = transpiler.transformSync(readFileSync(t.source, "utf8"));
  // The directive goes after the header comment: a comment does not end the directive prologue.
  return header(t) + (t.strict ? `"use strict";\n` : "") + js;
}

function summarize(want: string, got: string): string {
  const w = want.split("\n");
  const g = got.split("\n");
  let first = -1;
  for (let i = 0; i < Math.max(w.length, g.length); i += 1) {
    if (w[i] !== g[i]) {
      first = i;
      break;
    }
  }
  let differing = 0;
  for (let i = 0; i < Math.max(w.length, g.length); i += 1) if (w[i] !== g[i]) differing += 1;
  return (
    `    ${differing} of ${Math.max(w.length, g.length)} line(s) differ; first at line ${first + 1}\n` +
    `      on disk:   ${JSON.stringify(g[first] ?? "<end of file>")}\n` +
    `      generated: ${JSON.stringify(w[first] ?? "<end of file>")}`
  );
}

function main(): void {
  const check = process.argv.includes("--check");
  const stale: string[] = [];

  for (const t of TARGETS) {
    const want = render(t);
    const path = relative(REPO, t.out).replace(/\\/g, "/");
    let got: string | undefined;
    try {
      got = readFileSync(t.out, "utf8");
    } catch {
      got = undefined;
    }

    if (check) {
      if (got === want) continue;
      stale.push(got === undefined ? `  ${path}\n    missing` : `  ${path}\n${summarize(want, got)}`);
      continue;
    }

    if (got === want) {
      console.log(`preludes: ${path} up to date`);
      continue;
    }
    writeFileSync(t.out, want);
    console.log(`preludes: wrote ${path} (${want.length} bytes)`);
  }

  if (check && stale.length) {
    console.error(`generated preludes are stale — run \`${COMMAND}\`:`);
    for (const s of stale) console.error(s);
    process.exit(1);
  }
  if (check) console.log(`preludes: ${TARGETS.length} artifact(s) up to date`);
}

// Only when run, never when imported: the prelude test imports `TARGETS` and `render` and must not write.
if (import.meta.main) main();
