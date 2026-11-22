import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import ts from "typescript";
import { analyze, type AnalyzeInput } from "../check";
import { parseCheckConfig } from "../check/config";
import { allowedOf } from "../check/permissions";
import type { CheckReport, Diagnostic } from "../check/schema";
import { ensureCssEngine } from "../check/css-engine";
import { stdModuleFile } from "../runtime-path";

// Outside any test, so a first build is not cut short by a test timeout.
ensureCssEngine();

export const FIXTURES = join(import.meta.dir, "fixtures");
export const RULES = join(FIXTURES, "check-rules");
export const UNIT = join(FIXTURES, "check-unit");
export const SK = resolve(import.meta.dir, "../cli/sk.ts");

export const RULES_MODULES = [
  "src/main.ts",
  "src/builtins.ts",
  "src/clean.ts",
  "src/lazy.ts",
  "src/dynamic.ts",
  "src/loops.ts",
  "src/legacy/timers.ts",
  "src/typing.ts",
  "src/plain.js",
  "src/App.vue",
  "src/native.ts",
  "src/process.ts",
  "src/numeric.ts",
  "node_modules/native-kit/index.js",
  "node_modules/patchy/index.js",
  "node_modules/loose-types/index.js",
  "node_modules/evil-eval/index.js",
  "node_modules/shorthand-pkg/index.js",
  "node_modules/hidden-types/index.js",
].map((m) => join(RULES, m));

export function analyzeRules(options: Partial<AnalyzeInput> = {}): CheckReport {
  const config = JSON.parse(readFileSync(join(RULES, "sk.config.json"), "utf8")) as {
    check: unknown;
    permissions: unknown;
  };
  return analyze({
    typescript: ts,
    root: RULES,
    entry: join(RULES, "src/main.ts"),
    modules: RULES_MODULES,
    allow: parseCheckConfig(config.check).allow,
    permissions: allowedOf(config.permissions),
    aliases: { "skathi:numeric": stdModuleFile("skathi:numeric") },
    toolVersion: "test",
    ...options,
  });
}

export function analyzeUnit(files: string[]): CheckReport {
  return analyze({
    typescript: ts,
    root: UNIT,
    entry: join(UNIT, "fine.ts"),
    modules: files.map((f) => join(UNIT, f)),
    toolVersion: "test",
  });
}

export const CSS_APP = join(FIXTURES, "check-css");

export function analyzeCss(options: Partial<AnalyzeInput> = {}): CheckReport {
  const config = JSON.parse(readFileSync(join(CSS_APP, "sk.config.json"), "utf8")) as { check: unknown };
  return analyze({
    typescript: ts,
    root: CSS_APP,
    entry: join(CSS_APP, "src/main.ts"),
    modules: ["src/main.ts", "src/App.vue", "src/Widget.svelte"].map((m) => join(CSS_APP, m)),
    styles: [
      "index.html",
      "src/main.css",
      "src/compat.css",
      "src/more.css",
      "src/fine.css",
      "src/legacy/old.css",
      "node_modules/fancy-ui/style.css",
      "node_modules/quiet-ui/style.css",
    ].map((m) => join(CSS_APP, m)),
    allow: parseCheckConfig(config.check).allow,
    toolVersion: "test",
    ...options,
  });
}

export const FRAMEWORKS_APP = join(FIXTURES, "check-frameworks");

export function analyzeFrameworks(options: Partial<AnalyzeInput> = {}): CheckReport {
  const config = JSON.parse(readFileSync(join(FRAMEWORKS_APP, "sk.config.json"), "utf8")) as { check: unknown };
  return analyze({
    typescript: ts,
    root: FRAMEWORKS_APP,
    entry: join(FRAMEWORKS_APP, "src/main.ts"),
    modules: [
      "src/main.ts",
      "src/vapor.ts",
      "src/legacy/ssr.ts",
      "src/Panel.tsx",
      "src/App.vue",
      "src/Fine.vue",
      "src/Widget.svelte",
      "src/Element.svelte",
      "src/Fine.svelte",
      "node_modules/toggle-kit/Toggle.vue",
    ].map((m) => join(FRAMEWORKS_APP, m)),
    allow: parseCheckConfig(config.check).allow,
    toolVersion: "test",
    ...options,
  });
}

/** Diagnostics of one file as `line:code` (one-based lines). */
export const lines = (report: CheckReport, file: string) =>
  report.diagnostics.filter((d) => d.file === file).map((d) => `${d.range.start.line + 1}:${d.code}`);

export const only = (report: CheckReport, code: string): Diagnostic[] =>
  report.diagnostics.filter((d) => d.code === code);

export function sk(args: string[]): { status: number; stdout: string; stderr: string } {
  const r = Bun.spawnSync([process.execPath, SK, ...args], {
    cwd: join(import.meta.dir, ".."),
    env: { ...process.env, NO_COLOR: "1" },
  });
  return { status: r.exitCode ?? -1, stdout: r.stdout.toString(), stderr: r.stderr.toString() };
}
