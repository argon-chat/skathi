// The host preludes' checked-in JavaScript, checked against its TypeScript.
//
// `crates/vn-app/src/prelude.js` and `window.prelude.js` are GENERATED from the
// `.ts` beside them and `include_str!`'d into the binary. Nothing in the Rust
// build regenerates them — it must not, because `cargo build` may not require
// bun — so the only thing standing between "the `.ts` is the source" and "the
// `.ts` is decoration" is this file. Without it, an edit to `window.prelude.ts`
// that was never regenerated would type-check, pass every other test, and ship
// the OLD windowing surface, silently.
//
// It imports `TARGETS` and `render` from the generator rather than restating
// either, so what is compared is the generator's own output — a second copy of
// the transpiler options here would be a second thing to keep in step, and the
// options (`useDefineForClassFields: false` in particular) are exactly where a
// divergence would be invisible.
//
// It also pins the two properties of the artifact that are NOT the transpiler's
// doing, because both are load-bearing and neither is obvious from reading the
// `.ts`: the DO-NOT-EDIT header, and the `"use strict"` directive that bun
// drops and the generator puts back.

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { basename } from "node:path";

import { TARGETS, render } from "../scripts/gen-preludes";

describe("generated host preludes", () => {
  for (const target of TARGETS) {
    const name = basename(target.out);

    test(`${name} is byte-identical to what the generator emits`, () => {
      const want = render(target);
      const got = readFileSync(target.out, "utf8");
      // Compared as text, not as a hash: when this fails the diff IS the
      // report, and `bun packages/vue-native/scripts/gen-preludes.ts` is the
      // fix.
      expect(got).toBe(want);
    });

    test(`${name} says it is generated, and from where`, () => {
      const got = readFileSync(target.out, "utf8");
      // The header is the only warning a contributor gets before editing a file
      // whose edits are erased by the next regeneration.
      expect(got.startsWith("// GENERATED from ")).toBe(true);
      expect(got).toContain(basename(target.source));
      expect(got).toContain("gen-preludes.ts");
    });
  }

  test("the window prelude is strict, and the boot prelude is not", () => {
    // Not a style rule — a behaviour one, and the single thing the transpiler
    // silently changes. `window.prelude.ts` opens with `"use strict"`; bun
    // parses its input as a module and drops the directive prologue; `rt.eval`
    // then evaluates the artifact as a classic SCRIPT, where the difference is
    // real (an assignment to a read-only global throws instead of doing
    // nothing). `prelude.js` has never been strict and is not made strict here,
    // because that would be a behaviour change riding along inside a move.
    const strict = TARGETS.filter((t) => t.strict).map((t) => basename(t.out));
    expect(strict).toEqual(["window.prelude.js"]);

    for (const target of TARGETS) {
      const got = readFileSync(target.out, "utf8");
      // After the header comment and before the first statement: a comment does
      // not end the directive prologue, so this really is a directive.
      const firstCode = got.split("\n").find((l) => l.length > 0 && !l.startsWith("//"));
      expect(firstCode === '"use strict";').toBe(target.strict);
    }
  });

  test("the source is TypeScript that erases to the artifact, not a copy of it", () => {
    // The whole point of the exercise: if the `.ts` were the `.js` with a new
    // extension, every one of the assertions above would still pass and nothing
    // would have been gained. This pins that the sources actually carry types.
    for (const target of TARGETS) {
      const src = readFileSync(target.source, "utf8");
      expect(src).not.toBe(readFileSync(target.out, "utf8"));
    }
    const win = TARGETS.find((t) => basename(t.out) === "window.prelude.js");
    expect(win).toBeDefined();
    const src = readFileSync(win!.source, "utf8");
    // `VnAppWindow` is the load-bearing one: it comes from
    // `src/protocol.d.ts`, so the object the prelude BUILDS and the
    // `import { appWindow } from "vue-native"` the runtime EXPORTS are now one
    // declaration. Losing it would be losing the reason to have done this.
    expect(src).toContain("as VnAppWindow");
    expect(src).toContain("VnWindowState");
  });
});
