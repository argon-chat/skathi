import { describe, expect, test } from "bun:test";
import { installFonts } from "../src/fonts";

describe("native font attachment", () => {
  test("an absent native API leaves the document alone", () => {
    const document = {};
    expect(installFonts({}, document)).toBeNull();
    expect(Object.keys(document)).toEqual([]);
  });
  test("legacy JSON font globals do not activate a script fallback", () => {
    let called = false;
    const host = {
      __vn_fonts_list() {
        called = true;
        return "[]";
      },
    };
    expect(installFonts(host, {})).toBeNull();
    expect(called).toBe(false);
  });
  test("the native attachment receives the exact mirror document", () => {
    const document = {};
    const nativeSet = {};
    const host = {
      __vn_fonts_attach(value: object) {
        expect(value).toBe(document);
        return nativeSet;
      },
    };
    expect<unknown>(installFonts(host, document)).toBe(nativeSet);
  });
  test("a missing document does not allocate a native set", () => {
    let called = false;
    const host = {
      __vn_fonts_attach() {
        called = true;
        return {};
      },
    };
    expect(installFonts(host, null)).toBeNull();
    expect(installFonts(host, undefined)).toBeNull();
    expect(called).toBe(false);
  });
});
