import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import ts from "typescript";
import { fileFrameworkFindings } from "../check/frameworks";
import type { CheckReport } from "../check/schema";
import { analyzeFrameworks, FRAMEWORKS_APP } from "./check-fixtures";

let cached: CheckReport | undefined;
const report = () => (cached ??= analyzeFrameworks());

/** SK5 diagnostics of one file as `line:column:code`, one-based. */
const found = (file: string) =>
  report()
    .diagnostics.filter((d) => d.file === file && d.code.startsWith("SK5"))
    .map((d) => `${d.range.start.line + 1}:${d.range.start.character + 1}:${d.code}`);

/** Codes found in one small source. */
const codes = (name: string, source: string) =>
  fileFrameworkFindings(join(FRAMEWORKS_APP, "src", name), source, ts).map((f) => f.code);

describe("Vue and Svelte features (SK5xxx)", () => {
  test("a Vue component: vapor, HTML strings, .lazy, CSS modules, preprocessors; toggles and transitions pass", () => {
    expect(found("src/App.vue")).toEqual([
      "1:1:SK5001",
      "2:15:SK5004",
      "11:8:SK5003",
      "12:6:SK5003",
      "15:10:SK5010",
      "25:1:SK5004",
      "31:1:SK5011",
    ]);
  });

  test("a Svelte component: {@html}, hydrate, preprocessors; toggles, select bindings and animate: pass", () => {
    expect(found("src/Widget.svelte")).toEqual(["2:12:SK5009", "11:6:SK5003", "20:1:SK5011"]);
    expect(found("src/Element.svelte")).toEqual(["1:1:SK5002", "4:16:SK5002"]);
  });

  test("imports from vue and vue-router, and the app's .tsx files", () => {
    expect(found("src/main.ts")).toEqual(["1:10:SK5009", "1:24:SK5002", "2:45:SK5008"]);
    expect(found("src/vapor.ts")).toEqual(["1:10:SK5001"]);
    expect(found("src/Panel.tsx")).toEqual(["1:1:SK5012"]);
  });

  test("supported features are not reported", () => {
    expect(found("src/Fine.vue")).toEqual([]);
    expect(found("src/Fine.svelte")).toEqual([]);
  });

  test("a dependency's findings are one SK5013 with its places; check.allow suppresses by glob", () => {
    const dep = report().diagnostics.filter((d) => d.package?.name === "toggle-kit");
    expect(dep.map((d) => d.code)).toEqual(["SK5013"]);
    expect(dep[0]!.related.map((r) => r.range.start.line + 1)).toEqual([7]);
    expect(found("src/legacy/ssr.ts")).toEqual([]);
    expect(report().summary.suppressed).toBeGreaterThan(0);
    expect(analyzeFrameworks({ deps: false }).diagnostics.some((d) => d.code === "SK5013")).toBe(false);
  });

  test("scripts: type-only imports, other modules and look-alikes are not reported", () => {
    expect(codes("a.ts", 'import type { VueElement } from "vue";\nimport { type useHost } from "vue";')).toEqual([]);
    expect(codes("b.ts", 'import { hydrate } from "some-lib";\nconst createSSRApp = 1;')).toEqual([]);
    expect(codes("c.ts", 'import { createWebHashHistory as hash } from "vue-router";')).toEqual(["SK5008"]);
    expect(codes("d.ts", 'import { defineCustomElement } from "@skathi-native/runtime";')).toEqual(["SK5002"]);
  });

  test("templates: what only looks like a feature is not reported", () => {
    const vue = (template: string) => codes("X.vue", `<template>${template}</template>`);
    expect(vue('<input :type="kind" v-model="x" /><p title="v-html">innerHTML</p>')).toEqual([]);
    const svelte = (markup: string) => codes("X.svelte", markup);
    expect(svelte("<!-- {@html x} --><p>{'{@html x}'}</p>")).toEqual([]);
    expect(svelte("<p>{@html x}</p>")).toEqual(["SK5003"]);
  });
});
