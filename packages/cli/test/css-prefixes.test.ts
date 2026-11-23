// css-prefixes.ts — the vendor-prefixed declarations the builders drop — and
// its agreement with sk-css, which drops the same ones at runtime
// (crates/sk-css/src/prefix.rs).
//
//   bun test test/css-prefixes.test.ts      (from packages/cli)

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { WEBKIT_APPLIED, isDroppedPrefix, pruneVendorPrefixes } from "../css-prefixes";

const HERE = dirname(fileURLToPath(import.meta.url));
const PREFIX_RS = join(HERE, "../../../crates/sk-css/src/prefix.rs");

describe("css-prefixes.ts", () => {
  test("its list is prefix.rs's WEBKIT_APPLIED, name for name and in order", () => {
    const rs = readFileSync(PREFIX_RS, "utf-8");
    const body = /WEBKIT_APPLIED: &\[&str\] = &\[([\s\S]*?)\];/.exec(rs);
    expect(body).not.toBeNull();
    const names = [...body![1]!.matchAll(/"([^"]+)"/g)].map((m) => m[1]!);
    expect(names.length).toBeGreaterThan(40);
    expect([...WEBKIT_APPLIED]).toEqual(names);
  });

  test("-moz-, -ms-, -o- and the unlisted -webkit- spellings are dropped", () => {
    for (const p of [
      "-moz-osx-font-smoothing",
      "-moz-tab-size",
      "-ms-overflow-style",
      "-o-transition",
      "-webkit-touch-callout",
      "-WEBKIT-TAP-HIGHLIGHT-COLOR",
    ]) {
      expect(isDroppedPrefix(p)).toBe(true);
    }
    for (const p of [
      "-webkit-app-region",
      "-webkit-line-clamp",
      "-webkit-box-orient",
      "-webkit-text-fill-color",
      "-webkit-background-clip",
      "-webkit-mask-image",
      "-webkit-backdrop-filter",
      "-webkit-user-select",
      "-webkit-font-smoothing",
      "transform",
      "--webkit-like",
    ]) {
      expect(isDroppedPrefix(p)).toBe(false);
    }
  });

  test("a sheet loses only those declarations; selectors, at-rules and values stay", () => {
    const css = [
      "html { -webkit-text-size-adjust: 100%; -moz-tab-size: 4; tab-size: 4 }",
      "body { -webkit-font-smoothing: antialiased; -moz-osx-font-smoothing: grayscale }",
      ".clamp { display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical }",
      ".bar { -webkit-app-region: drag; -webkit-touch-callout: none }",
      "::-webkit-scrollbar { width: 6px }",
      "@-webkit-keyframes spin { to { -webkit-transform: rotate(1turn); -moz-transform: rotate(1turn) } }",
    ].join("\n");
    const out = pruneVendorPrefixes(css);
    for (const gone of ["-moz-tab-size", "-moz-osx-font-smoothing", "-webkit-touch-callout", "-moz-transform"]) {
      expect(out).not.toContain(gone);
    }
    for (const kept of [
      "-webkit-text-size-adjust: 100%",
      "-webkit-font-smoothing: antialiased",
      "tab-size: 4",
      "display: -webkit-box",
      "-webkit-line-clamp: 2",
      "-webkit-box-orient: vertical",
      "-webkit-app-region: drag",
      "::-webkit-scrollbar",
      "@-webkit-keyframes spin",
      "-webkit-transform: rotate(1turn)",
    ]) {
      expect(out).toContain(kept);
    }
  });

  test("a sheet with nothing to drop, or one postcss cannot parse, comes back as it was", () => {
    const plain = ".a { color: red; -webkit-user-select: none }";
    expect(pruneVendorPrefixes(plain)).toBe(plain);
    const broken = ".a { -moz-tab-size: 4; color: red";
    expect(pruneVendorPrefixes(broken)).toBe(broken);
  });
});
