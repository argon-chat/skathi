// The host preludes' checked-in JavaScript, checked against its TypeScript: the Rust build does not regenerate
// `prelude.js` / `window.prelude.js`, so this stops a stale artifact shipping. Also pins the DO-NOT-EDIT header and
// the `"use strict"` directive that bun drops and the generator puts back.

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { basename } from "node:path";

import { TARGETS, render } from "../../../scripts/gen-preludes";

describe("generated host preludes", () => {
  for (const target of TARGETS) {
    const name = basename(target.out);

    test(`${name} is byte-identical to what the generator emits`, () => {
      const want = render(target);
      const got = readFileSync(target.out, "utf8");
      expect(got).toBe(want);
    });

    test(`${name} says it is generated, and from where`, () => {
      const got = readFileSync(target.out, "utf8");
      expect(got.startsWith("// GENERATED from ")).toBe(true);
      expect(got).toContain(basename(target.source));
      expect(got).toContain("gen-preludes.ts");
    });
  }

  test("the window prelude is strict, and the boot prelude is not", () => {
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
    for (const target of TARGETS) {
      const src = readFileSync(target.source, "utf8");
      expect(src).not.toBe(readFileSync(target.out, "utf8"));
    }
    const win = TARGETS.find((t) => basename(t.out) === "window.prelude.js");
    expect(win).toBeDefined();
    const src = readFileSync(win!.source, "utf8");
    expect(src).toContain("as SkAppWindow");
    expect(src).toContain("SkWindowState");
  });
});
