// The host preludes' checked-in JavaScript, checked against its TypeScript: the Rust build does not regenerate
// `prelude.js` / `window.prelude.js`, so this stops a stale artifact shipping. Also pins the `"use strict"` directive
// that bun drops and the generator puts back.

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
  }

  test("a strict prelude starts with the directive, and only a strict one", () => {
    for (const target of TARGETS) {
      const got = readFileSync(target.out, "utf8");
      // After the header comment and before the first statement: a comment does
      // not end the directive prologue, so this really is a directive.
      const firstCode = got.split("\n").find((l) => l.length > 0 && !l.startsWith("//"));
      expect(firstCode === '"use strict";').toBe(target.strict);
    }
  });
});
