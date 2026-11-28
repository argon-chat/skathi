import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { builtinEffects } from "../src/data/shaders/builtins";
import { guides } from "../src/data/shaders/entries";
import { libraryEntries } from "../src/data/shaders/library";
import { entryMarkdown } from "../src/lib/reference";

const cli = new URL("../../packages/cli/effects/", import.meta.url);

describe("shader API from the library and compiler reflection", () => {
  test("keeps editorial URLs and all overloads from the shipped modules", () => {
    const paths = [
      "sk.slang",
      ...readdirSync(new URL("sk/", cli))
        .filter((name) => name.endsWith(".slang"))
        .map((name) => `sk/${name}`),
    ];
    const entries = libraryEntries(
      Object.fromEntries(paths.map((path) => [path, readFileSync(new URL(path, cli), "utf8")])),
      guides,
    );
    const find = (slug: string) => entries.find((e) => e.slug === slug);
    expect(find("floor-mod")?.syntax).toContain("floorMod<let N : int>");
    expect(find("floor-mod")?.summary).toContain("-0.25 mod 1");
    expect(find("effect-input")?.syntax).toContain("float2 local;");
    expect(find("i-effect")?.syntax).toContain("static bool straightAlpha();");
    expect(find("i-effect")?.syntax).toContain("Defaults to `shade`");
    expect(find("gaussian")?.syntax).toContain("7 reads per axis");
    expect(find("color-harmonies")?.syntax).toContain("hue 0, 120, 240");
    expect(find("color-harmonies")?.syntax).toContain("hue -30, 0, +30");
    expect(find("cubic-bezier")?.syntax?.match(/static CubicBezier easeIn\(\);/g)).toHaveLength(1);
    expect(find("linear-stops")?.examples?.some((example) => example.code.includes("LinearStops<3>"))).toBe(true);
    expect(find("sk-range")?.syntax).toMatch(/\[SkRange\(float \w+, float \w+\)\]/);
    expect(find("sk-range")?.examples?.length).toBeGreaterThan(0);
    expect(entries.every((e) => e.summary.length > 0)).toBe(true);
    expect(entries.some((e) => e.name === "SkEffectQuad")).toBe(false);
  });

  test("discovers public declarations in new modules without an editorial record", () => {
    const files = {
      "sk/demo.slang": `
      module "sk.demo";
      public namespace Ops {
        /// Scale a point.
        public float scale(float p, float gain = 2.0) { /* } */ return p * gain; }
        public float2 scale(float2 p, float gain = 2.0) { return p * gain; }
        float hidden() { return 0; }
      }
      public struct Value {
        public float value;
        public float read() { return value; }
        float internal() { return 0; }
      }
    `,
    };
    const entries = libraryEntries(files, []);
    expect(entries.map((entry) => entry.name)).toEqual(["Ops.scale", "Value"]);
    expect(entries[0]?.summary).toBe("Scale a point.");
    expect(entries[0]?.syntax).toContain("float2 scale(float2 p, float gain = 2.0);");
    expect(entries[1]?.syntax).toContain("float read();");
    expect(entries[1]?.syntax).not.toContain("internal");
    expect(entries[0]?.since).toBeUndefined();
    expect(() => libraryEntries(files, [guides[0]!])).toThrow(/Missing shader module/);
  });

  test("exports built-in CSS signatures, defaults and limits to readers and agents", () => {
    const frost = builtinEffects.find((e) => e.name === "frost()")!;
    const ref = {
      id: "shaders",
      title: "Shader library",
      noun: "effects",
      categories: [],
      categoryNotes: {},
      entries: builtinEffects,
      source: frost.source!,
      syntax: "slang" as const,
    };
    const doc = entryMarkdown(ref, frost, new URL("https://example.test"));
    expect(doc).toContain("frost(<number:strength>, <length:radius>?, <color:tint>?)");
    expect(doc).toContain("12px");
    expect(doc).toContain("0…25");
    expect(frost.params?.map((p) => p.optional)).toEqual([false, true, true]);
  });
});
