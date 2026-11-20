import { expect, test } from "bun:test";
import { join } from "node:path";
import { compileStyle } from "@vue/compiler-sfc";
import { protectHapticPatterns } from "../haptic-patterns";
import { compileSvelte } from "../svelte";
import { fileFindings } from "../check/css";
import { ensureCssEngine } from "../check/css-engine";

ensureCssEngine();

const sheet = `@media (min-width: 360px) {
  @haptic-pattern --two { fallback: tick; 0ms { intensity: .3 } 70ms { intensity: .65 } }
  button { haptic: activate pattern(--two) }
}`;

test("Vue and Svelte scope the control and keep haptic times inside their media rule", async () => {
  const prepared = protectHapticPatterns(sheet);
  const vue = compileStyle({ source: prepared.css, filename: "H.vue", id: "data-v-h", scoped: true });
  expect(vue.errors).toEqual([]);
  const vueCss = prepared.restore(vue.code);
  const root = join(import.meta.dir, "../../..");
  const svelte = await compileSvelte(root, "H.svelte", `<button>Play</button><style>${sheet}</style>`, {
    dev: false,
    filename: "H.svelte",
  });
  expect(vueCss).toContain("button[data-v-h]");
  expect(svelte.css).toMatch(/button\.svelte-/);
  for (const css of [vueCss, svelte.css]) {
    expect(css).toMatch(/@media[^{}]+\{\s*@haptic-pattern --two/);
    expect(css).toMatch(/70ms\s*\{/);
    expect(css).not.toContain("--skathi-pattern-index");
  }
});

test("sk check reports invalid pulse descriptors and accepts a complete named sequence", () => {
  expect(fileFindings("H.vue", `<style scoped>${sheet}</style>`, false)).toEqual([]);
  const invalid = fileFindings("H.css", "@haptic-pattern --bad { 30ms { intensity: 2 } }", false);
  expect(invalid.map((d) => d.code)).toContain("SK4004");
});
