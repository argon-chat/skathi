import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, relative, resolve, sep } from "node:path";
import { loadAppConfig } from "../cli/config";
import { quietProgress, type Progress } from "../cli/term";
import { Builtins } from "./builtins";
import { codeInfo, REGISTRY } from "./codes";
import { parseCheckConfig, Suppressions } from "./config";
import { rangeOf, ZERO_RANGE, type Finding } from "./finding";
import { buildGraph, GraphFailure, type ModuleGraph } from "./graph";
import { Packages } from "./packages";
import { buildProgram, readUnits, type TypeScript } from "./program";
import { Rules } from "./rules";
import type { CheckReport, Diagnostic } from "./schema";
import { Typing } from "./typing";

export type { CheckReport, Diagnostic } from "./schema";
export { explain, formatPretty, listCodes } from "./report";

const PROD_TS = join(import.meta.dir, "..", "prod.ts");

/** Packages the SDK provides and answers for under AOT; an app cannot swap or retype them. */
const PLATFORM = new Set([
  "vue",
  "@vue/runtime-core",
  "@vue/runtime-dom",
  "@vue/reactivity",
  "@vue/shared",
  "@skathi-native/runtime",
]);

export class CheckSetupError extends Error {}

export interface CheckOptions {
  target: string;
  configPath?: string | undefined;
  /** Check node_modules too (default); false: the app's own code only. */
  deps?: boolean;
  toolVersion: string;
  progress?: Progress | undefined;
}

export interface AnalyzeInput {
  typescript: TypeScript;
  root: string;
  entry: string;
  modules: string[];
  aliases?: Record<string, string>;
  declarations?: string[];
  allow?: Map<string, string[]>;
  deps?: boolean;
  toolVersion: string;
  /** Findings made before the analysis (configuration problems). */
  findings?: Finding[];
  progress?: Progress | undefined;
}

export function loadTypeScript(appRoot: string): TypeScript {
  for (const base of [join(import.meta.dir, "..", "package.json"), join(appRoot, "package.json")]) {
    try {
      return createRequire(base)("typescript") as TypeScript;
    } catch {}
  }
  throw new CheckSetupError("sk check needs the typescript package: add it to the app (bun add -d typescript)");
}

const posix = (path: string) => path.split(sep).join("/");

export async function runCheck(options: CheckOptions): Promise<CheckReport> {
  const progress = options.progress ?? quietProgress;
  const config = loadAppConfig(options.target, { configPath: options.configPath });
  const configFile = config.configPath ?? join(config.appRoot, "sk.config.json");
  const checkConfig = parseCheckConfig(config.raw.check);
  const configText = checkConfig.problems.length ? readFileSync(configFile, "utf8").replace(/^﻿/, "") : "";
  const findings: Finding[] = checkConfig.problems.map(({ message, key }) => {
    const at = configText.indexOf(JSON.stringify(key));
    const before = configText.slice(0, Math.max(0, at)).split("\n");
    const line = before.length - 1;
    const character = before[line]!.length;
    return {
      code: "SK0003",
      message: `sk.config.json ${message}`,
      file: configFile,
      range:
        at < 0
          ? ZERO_RANGE
          : { start: { line, character }, end: { line, character: character + JSON.stringify(key).length } },
      related: [],
    };
  });
  const typescript = loadTypeScript(config.appRoot);
  const base = { typescript, root: config.appRoot, entry: config.entry, toolVersion: options.toolVersion, progress };
  if (findings.length) return analyze({ ...base, modules: [], findings });

  progress.step("module graph");
  let graph: ModuleGraph;
  try {
    graph = await buildGraph({
      prodTs: PROD_TS,
      entry: config.entry,
      appRoot: config.appRoot,
      configPath: config.configPath,
    });
  } catch (e) {
    if (!(e instanceof GraphFailure)) throw e;
    findings.push({
      code: "SK0004",
      message: `the production build fails: ${e.message.split(/\r?\n/).find((l) => l.trim()) ?? e.message}`,
      file: config.entry,
      range: ZERO_RANGE,
      help: e.message.includes("\n") ? e.message : undefined,
      related: [],
    });
    return analyze({ ...base, modules: [], findings });
  }
  if (graph.errors.length) {
    for (const error of graph.errors) {
      const line = Math.max(0, (error.line ?? 1) - 1);
      const character = Math.max(0, error.column ?? 0);
      findings.push({
        code: "SK0004",
        message: error.specifier
          ? `\`${error.specifier}\` does not resolve, so the bundle cannot be built: ${error.message}`
          : `the production build fails: ${error.message}`,
        file: error.file ?? config.entry,
        range: { start: { line, character }, end: { line, character } },
        related: [],
      });
    }
    return analyze({ ...base, modules: [], findings });
  }
  progress.detail(`${graph.modules.length} files`);
  return analyze({
    ...base,
    modules: graph.modules,
    aliases: graph.aliases,
    declarations: graph.declarations,
    allow: checkConfig.allow,
    deps: options.deps ?? true,
  });
}

export function analyze(input: AnalyzeInput): CheckReport {
  const { typescript } = input;
  const progress = input.progress ?? quietProgress;
  const deps = input.deps ?? true;
  const root = resolve(input.root);
  const packages = new Packages(root);
  const findings: Finding[] = [...(input.findings ?? [])];
  const { units, unreadable } = readUnits(input.modules, packages);
  for (const u of unreadable)
    findings.push({
      code: "SK0001",
      message: `cannot read this file: ${u.message}`,
      file: u.path,
      range: ZERO_RANGE,
      related: [],
    });

  const analysed = deps ? units : units.filter((u) => !u.owner);
  if (units.length) {
    progress.step("TypeScript program", units.length);
    const built = buildProgram({
      typescript,
      appRoot: root,
      entry: input.entry,
      units,
      aliases: input.aliases ?? {},
      declarations: input.declarations ?? [],
      onFile: () => progress.tick(),
    });
    if (built.configNote) progress.warn(built.configNote);
    const { program, checker } = built;
    const rules = new Rules(typescript, checker, new Builtins(typescript, program, checker));
    progress.step("AOT rules", units.length);
    for (const unit of units) {
      progress.tick();
      const file = unit.sourceFile ?? program.getSourceFile(unit.programPath);
      if (!file) {
        findings.push({
          code: "SK0001",
          message: "the TypeScript program could not load this file",
          file: unit.path,
          range: ZERO_RANGE,
          related: [],
        });
        continue;
      }
      unit.sourceFile = file;
      const syntax = program.getSyntacticDiagnostics(file);
      const first = syntax[0];
      if (first) {
        const start = first.start ?? 0;
        findings.push({
          code: "SK0002",
          message: `this file does not parse, so it was not checked: ${typescript.flattenDiagnosticMessageText(first.messageText, " ")}`,
          file: unit.path,
          range: rangeOf(file, start, start + (first.length ?? 0)),
          related: [],
        });
        delete unit.sourceFile;
        continue;
      }
      if (deps || !unit.owner) rules.run(unit, file, findings);
    }
    progress.step("typing quality");
    const typing = new Typing(
      typescript,
      program,
      checker,
      packages,
      units.filter((u) => u.sourceFile),
    );
    const typingFindings: Finding[] = [];
    if (deps) typing.untypedPackages(typingFindings);
    typing.imports(typingFindings);
    typing.appCode(typingFindings);
    findings.push(...(deps ? typingFindings : typingFindings.filter((f) => !packages.ownerOf(f.file))));
    return report(
      input,
      root,
      packages,
      findings,
      analysed.length + unreadable.length,
      deps ? typing.results() : typing.results().filter((s) => !s.owner),
    );
  }
  return report(input, root, packages, findings, unreadable.length, []);
}

function report(
  input: AnalyzeInput,
  root: string,
  packages: Packages,
  findings: Finding[],
  files: number,
  scores: ReturnType<Typing["results"]>,
): CheckReport {
  const rel = (file: string) => posix(relative(root, file));
  const suppress = new Suppressions(input.allow ?? new Map());
  const diagnostics: Diagnostic[] = [];
  const owners = new Set<string>();
  let suppressed = 0;
  for (const f of findings) {
    const info = codeInfo(f.code);
    const owner = packages.ownerOf(f.file);
    if (owner && PLATFORM.has(owner.name)) continue;
    const file = rel(f.file);
    if (!info.fatal && suppress.matches(f.code, file, owner?.name ?? null)) {
      suppressed++;
      continue;
    }
    diagnostics.push({
      code: f.code,
      severity: info.severity,
      message: f.message,
      file,
      package: owner ? { name: owner.name, version: owner.version } : null,
      range: f.range,
      help: f.help ?? info.fix,
      related: f.related.map((r) => ({ file: rel(r.file), range: r.range, message: r.message })),
    });
  }
  for (const u of input.modules) {
    const owner = packages.ownerOf(u);
    if (owner && !PLATFORM.has(owner.name) && (input.deps ?? true)) owners.add(`${owner.name}@${owner.version}`);
  }
  diagnostics.sort(
    (a, b) =>
      (a.file < b.file ? -1 : a.file > b.file ? 1 : 0) ||
      a.range.start.line - b.range.start.line ||
      a.range.start.character - b.range.start.character ||
      a.code.localeCompare(b.code),
  );
  const typing = scores
    .filter((s) => !(s.owner && PLATFORM.has(s.owner.name)))
    .map((s) => ({
      package: s.owner?.name ?? null,
      version: s.owner?.version ?? null,
      declarations: s.declarations,
      any: s.any,
    }))
    .sort(
      (a, b) => (a.package ?? "").localeCompare(b.package ?? "") || (a.version ?? "").localeCompare(b.version ?? ""),
    );
  return {
    schema: "skathi.check",
    version: 1,
    tool: { name: "sk", version: input.toolVersion },
    root: posix(root),
    diagnostics,
    summary: {
      errors: diagnostics.filter((d) => d.severity === "error").length,
      warnings: diagnostics.filter((d) => d.severity === "warning").length,
      suppressed,
      files,
      packages: owners.size,
      typing,
    },
  };
}

export function exitCode(report: CheckReport, denyWarnings: boolean): number {
  if (report.diagnostics.some((d) => REGISTRY.get(d.code)?.fatal)) return 2;
  if (report.summary.errors) return 1;
  if (denyWarnings && report.summary.warnings) return 1;
  return 0;
}
