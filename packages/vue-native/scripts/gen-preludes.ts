// The host preludes: `crates/vn-app/src/*.ts` -> the `.js` the Rust binary
// `include_str!`s.
//
// Usage
//   bun scripts/gen-preludes.ts           regenerate the checked-in artifacts
//   bun scripts/gen-preludes.ts --check   CI gate: regenerate in memory and
//                                         fail if a checked-in file is stale
//
// ---------------------------------------------------------------------------
// WHY THIS EXISTS
// ---------------------------------------------------------------------------
//
// Everything the runtime ships is TypeScript. The two strings the HOST
// evaluates were the last hand-written JavaScript in the repo, and they were
// hand-written for a reason that is not going away: JavaScriptCore executes
// JavaScript, `include_str!` is not a bundler, and both strings run BEFORE the
// app bundle exists — `vn://prelude.js` seeds the three `__VUE_*` flags Vue's
// esm-bundler build reads at module-eval time, and `vn://window.js` installs
// `document.title`, `screen`, `matchMedia` and `navigator.windowControlsOverlay`
// so that they are already their real selves when the first component's
// `<script setup>` runs.
//
// So the `.ts` is the source and the `.js` is a build artifact that is CHECKED
// IN — for the same three reasons crates/vn-effects/generated exists
// (docs/SHADERS.md §2.3), reading almost identically here:
//
//   * `cargo build` must work with no bun, no node and no `bun install`. The
//     prelude is `include_str!`'d, so a missing artifact is a COMPILE error in
//     a Rust crate, which is the worst possible place to discover a JS
//     toolchain requirement;
//   * an emitter upgrade that silently changes codegen shows up as a PR diff on
//     the thing that actually runs, rather than as a behaviour change nobody
//     reviewed;
//   * the artifact is what a debugger, a stack trace and `vn://window.js` in an
//     error header all point at, so it needs to be a real file in the tree.
//
// And it is `Bun.Transpiler`, not tsc, because that is the SAME parser and the
// same emitter `build.ts` feeds `src/*.ts` through to produce dist/vendor.js —
// one emitter for the whole JS half of the project, so "what ships" never
// depends on which tool touched a file. `test/prelude-generated.test.ts` is the
// gate that keeps the checked-in bytes honest; without it the `.ts` is
// decoration.
//
// ---------------------------------------------------------------------------
// THE ONE THING THE EMITTER WILL NOT DO FOR US
// ---------------------------------------------------------------------------
//
// Bun parses its input as a module — modules are strict by definition — and
// therefore DROPS the directive prologue. `rt.eval` evaluates the artifact as a
// classic SCRIPT, where dropping `"use strict"` is not a no-op: it is the
// difference between an assignment to a read-only global throwing and silently
// doing nothing — and `window.prelude.ts` seeds three of redjs's own globals
// behind exactly such a guard. So a target may ask for the directive back, and
// it is re-emitted at FILE scope, which makes the whole evaluated script
// strict: exactly what the IIFE's own directive did, since the IIFE is the
// whole file.
//
// It is per-target rather than automatic because `prelude.js` is NOT strict
// today, and "make the other one strict too while we are here" is precisely the
// kind of change that must not ride along inside a mechanical move.

import { readFileSync, writeFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";

// `scripts/` is one deeper than shaders.ts, which resolves the same root.
const REPO = resolve(import.meta.dir, "../../..");
const HOST = join(REPO, "crates", "vn-app", "src");

/** How this script is spelled in the header it stamps, and in an error. */
const COMMAND = "bun packages/vue-native/scripts/gen-preludes.ts";

export interface Target {
  /** The TypeScript source, absolute. */
  source: string;
  /** The artifact `include_str!` reads, absolute. */
  out: string;
  /**
   * Re-emit `"use strict";` at file scope, because the source asks for it and
   * bun drops directive prologues. See the header.
   */
  strict: boolean;
  /** One line for the artifact's header: what this file is for. */
  what: string;
}

export const TARGETS: Target[] = [
  {
    source: join(HOST, "prelude.ts"),
    out: join(HOST, "prelude.js"),
    // Sloppy, as it has always been. See the header on why this is not a
    // default.
    strict: false,
    what: "evaluated first, as `vn://prelude.js`, before the window prelude and the app bundle",
  },
  {
    source: join(HOST, "window.prelude.ts"),
    out: join(HOST, "window.prelude.js"),
    strict: true,
    what: "evaluated as `vn://window.js`, after the prelude and before the app bundle",
  },
];

// The same options `test/ts-source.ts` passes and `build.ts` gets from
// tsconfig.json. `useDefineForClassFields: false` is load-bearing for the
// reason packages/vue-native/tsconfig.json states: a bare `x: T` field must be
// ERASED, not turned into a constructor-prologue define that runs after
// `super()`.
const transpiler = new Bun.Transpiler({
  loader: "ts",
  target: "browser",
  tsconfig: { compilerOptions: { target: "esnext", useDefineForClassFields: false } },
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

/** One target, as the exact bytes that belong on disk. */
export function render(t: Target): string {
  const js = transpiler.transformSync(readFileSync(t.source, "utf8"));
  // The directive goes AFTER the header comment and BEFORE everything else: a
  // comment does not end the directive prologue, so this is still a directive
  // and not a stray expression statement.
  return header(t) + (t.strict ? `"use strict";\n` : "") + js;
}

/** `a` vs `b`, as the first line that differs and how many do. */
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
      stale.push(
        got === undefined
          ? `  ${path}\n    missing`
          : `  ${path}\n${summarize(want, got)}`,
      );
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

// Only when RUN, never when imported: `test/prelude-generated.test.ts` pulls
// `TARGETS` and `render` out of this file so that the gate tests the GENERATOR
// rather than a second copy of it — and a test run must not write to the tree.
if (import.meta.main) main();
