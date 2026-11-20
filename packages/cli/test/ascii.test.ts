// prod.ts's ASCII pass (ascii.ts): every escape reads as what it replaced.
//
//   bun test packages/cli/test/ascii.test.ts

import { describe, expect, test } from "bun:test";
import { asciiOnly, checkAscii, nonAsciiCount } from "../ascii";
import { preserveLiteralText } from "../ascii-literals";

/** Runs `body` as a function both ways: the escaped one must be ASCII, give
 * the same answer, and pass the parser check. */
function same(body: string) {
  const r = asciiOnly(body);
  expect(r.failed).toBeUndefined();
  expect(r.kept).toBe(0);
  expect(nonAsciiCount(r.code)).toBe(0);
  expect(new Function(r.code)()).toEqual(new Function(body)());
  expect(checkAscii(body, r.code)).toBeNull();
  return r;
}

describe("asciiOnly", () => {
  test("strings, with escapes, quotes and astral characters", () => {
    same(`return ["é ünï 你好", 'л\\'и', "😀x😀", "\\é", "a\\ b", " "];`);
  });

  test("untagged templates, nested ones and their substitutions", () => {
    same("const x = 'é'; return [`ä${x}ö`, `a${`b${'ш' + `ц`}щ`}ж`, `${{ a: 'ё' }.a}`, `\\é`];");
  });

  test("a tagged template keeps its text: the tag can read `raw`", () => {
    const body = "const raw = (s) => s.raw.join('|'); return [raw`é${1}\\ü`, String.raw`ж`, `ok é`];";
    const r = asciiOnly(body);
    expect(r.kept).toBe(3);
    expect(new Function(r.code)()).toEqual(["é|\\ü", "ж", "ok é"]);
    expect(nonAsciiCount(r.code)).toBe(3);
  });

  test("regular expressions, with classes, escapes and the u flag", () => {
    same(`
      const s = "café 😀 Ё";
      return [
        /é/.test(s), /[à-ÿ]/.exec(s)[0], /\\é/.test(s), /[Ё-ё]+/.exec(s)[0],
        /😀/u.test(s), /^[😀]$/u.test("😀"), /[😀]/.test("\\uD83D"), /[/é]/.test("/"),
        "a/é/b".split(/\\//),
      ];
    `);
  });

  test("identifiers, a BMP one and an astral one", () => {
    same("var été = 1, 𐊧 = 2, μ = { ü: 3 }; return [été, 𐊧, μ.ü, μ['ü']];");
  });

  test("comments and Unicode whitespace", () => {
    same("// é — ж\nvar a = /* ü */ 1; var　b = 2; return a + b;");
    // A line terminator in a block comment is still one, for ASI.
    same("return/* */1;");
    same("var a = 1 var b = 2 return a + b");
  });

  test("a division is not taken for a regular expression, and back", () => {
    same(`
      var a = 8, g = 2, i = 2;
      const o = { if: (x) => x, for: (x) => x };
      return [a/g/i, "é", o.if(4)/2, "ё", Symbol.for("ж") === Symbol.for("ж"), (a)/g/i, "ü"];
    `);
    same(`var r = []; if (true) /é/.test("é") && r.push(1); return [r, "ж"];`);
    same(`var of = 6, x = 3; return [of/x/2, "é", typeof /ö/, "ü"];`);
    same(`function f() { return 1 } /é/.test("é"); return "ж";`);
    same(`var a = 10; a /= 2; return [a, "é", 1.5e-3/2, "ö", .5/2, "ü", 0x1e-5];`);
  });

  test("nothing to escape: the text comes back as it was", () => {
    const r = asciiOnly("var a = 'abc'; /x/.test(a);");
    expect(r).toEqual({ code: "var a = 'abc'; /x/.test(a);", escaped: 0, kept: 0 });
  });

  test("a bundle it cannot lex comes back unchanged", () => {
    for (const bad of ["var a = 'é", "var a = `é", "var a = (1", "var a = 1)", "/* é"]) {
      const r = asciiOnly(bad);
      expect(r.failed).toBeDefined();
      expect(r.code).toBe(bad);
    }
  });
});

describe("checkAscii", () => {
  test("the production prepass preserves raw strings, frozen site identities and regexp source", () => {
    const code = [
      '"use strict";',
      "const samples = [];",
      "const owner = { tag(s, n) { samples.push(s); return [this === owner, s[0], s.raw[0], n, Object.isFrozen(s), Object.isFrozen(s.raw)]; } };",
      "function run(n) { return owner.tag`ж\\ü${n}ё`; }",
      "const first = run(2), second = run(3);",
      "function C() { this.v = 42; }",
      "const factory = () => C;",
      "return [first, second, samples[0] === samples[1], new factory`ж`().v, owner.tag`ж\\unicode`, /[Ёё]/u.source, /[Ёё]/u.test('ё')];",
    ].join("\n");
    const before = preserveLiteralText(code);
    const escaped = asciiOnly(before);
    expect(nonAsciiCount(escaped.code)).toBe(0);
    expect(checkAscii(before, escaped.code)).toBeNull();
    expect(new Function(escaped.code)()).toEqual(new Function(code)());
  });
  test("literal lowering supports a shadowed Object and RegExp constructors used after new", () => {
    const code = [
      "const Object = null;",
      "const \\u005f_skLiteralRegExp = 7;",
      "const raw = strings => strings.raw[0];",
      "let error; try { new /ж/; } catch (e) { error = e.name; }",
      "function match(){return/ё/.source}",
      "return [raw`ю`, error, new /ж/.constructor('я').source, match()];",
    ].join("\n");
    const output = asciiOnly(preserveLiteralText(code)).code;
    expect(nonAsciiCount(output)).toBe(0);
    expect(new Function(output)()).toEqual(new Function(code)());
  });
  test("says when an escape changed what the program reads", () => {
    const before = "var t = (s) => s.raw[0]; t`é`;";
    expect(checkAscii(before, "var t = (s) => s.raw[0]; t`\\u00E9`;")).not.toBeNull();
    expect(checkAscii("var a = 'é';", "var a = 'é")).not.toBeNull();
  });
});
