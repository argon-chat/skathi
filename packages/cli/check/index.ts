import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { requireAppDependencies } from "../cli/app-deps";
import { loadAppConfig } from "../cli/config";
import { appShadersDir, compileAppEffects, EffectsError, hasAppEffects } from "../effects/compile";
import { resolveCompiler } from "../effects/redslangc";
import { quietProgress, type Progress } from "../cli/term";
import { Builtins } from "./builtins";
import { REGISTRY } from "./codes";
import { parseCheckConfig } from "./config";
import { cssFindings, readStyleSources } from "./css";
import { diagnosticsOf, PLATFORM, posix, rangeOf, ZERO_RANGE, type Finding } from "./finding";
import { BundleLiveness } from "./bundle";
import { buildGraph, GraphFailure, type ModuleGraph } from "./graph";
import { Packages } from "./packages";
import { buildProgram, readUnits, type TypeScript } from "./program";
import { Rules } from "./rules";
import type { CheckReport } from "./schema";
import { Typing } from "./typing";

export type { CheckReport, Diagnostic } from "./schema";
export { explain, formatPretty, listCodes } from "./report";

const PROD_TS = join(import.meta.dir, "..", "prod.ts");

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
  /** The production bundle: a rule finding in code it cannot run is not reported. */
  bundle?: ModuleGraph["bundle"];
  /** Stylesheets and pages whose CSS the build includes, besides the components in `modules`. */
  styles?: string[];
  /** The build runs Tailwind over the app's CSS. */
  tailwind?: boolean;
}

export function loadTypeScript(appRoot: string): TypeScript {
  for (const base of [join(import.meta.dir, "..", "package.json"), join(appRoot, "package.json")]) {
    try {
      return createRequire(base)("typescript") as TypeScript;
    } catch {}
  }
  throw new CheckSetupError("sk check needs the typescript package: add it to the app (bun add -d typescript)");
}

export async function runCheck(options: CheckOptions): Promise<CheckReport> {
  const progress = options.progress ?? quietProgress;
  const config = loadAppConfig(options.target, { configPath: options.configPath });
  requireAppDependencies(config.appRoot, dirname(config.entry));
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
  findings.push(...(await effectFindings(config.appRoot, progress)));

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
    bundle: graph.bundle,
    styles: graph.styles ?? [],
    tailwind: graph.tailwind ?? false,
    allow: checkConfig.allow,
    deps: options.deps ?? true,
    findings,
  });
}

/** The app's shaders/*.slang, compiled as sk build would: one SK0005 per error, at the author's line. */
export async function effectFindings(appRoot: string, progress: Progress = quietProgress): Promise<Finding[]> {
  if (!hasAppEffects(appRoot)) return [];
  const dir = appShadersDir(appRoot);
  progress.step("effect shaders");
  let compiler: string;
  try {
    compiler = (
      await resolveCompiler({
        log: (line) => progress.info(line),
        onBytes: (done, total) => progress.bytes(done, total),
      })
    ).path;
  } catch (e) {
    const message = `the effect shaders were not compiled: ${e instanceof Error ? e.message : String(e)}`;
    return [{ code: "SK0006", message, file: dir, range: ZERO_RANGE, related: [] }];
  }
  try {
    await compileAppEffects(appRoot, compiler);
    return [];
  } catch (e) {
    if (!(e instanceof EffectsError)) throw e;
    return e.problems
      .filter((p) => p.severity === "error")
      .map((p): Finding => {
        const own = isAbsolute(p.file);
        const line = own ? Math.max(0, p.line - 1) : 0;
        const character = own ? Math.max(0, p.column - 1) : 0;
        return {
          code: "SK0005",
          message: own ? p.message : `${p.file}${p.line ? `:${p.line}` : ""}: ${p.message}`,
          file: own ? p.file : dir,
          range: { start: { line, character }, end: { line, character } },
          related: [],
        };
      });
  }
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
  const styled = [...input.modules.filter((m) => /\.(vue|svelte)$/i.test(m)), ...(input.styles ?? [])];
  if (styled.length) {
    progress.step("CSS");
    findings.push(
      ...cssFindings({ sources: readStyleSources(styled), packages, deps, tailwind: input.tailwind ?? false }),
    );
  }
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
    const liveness = input.bundle ? new BundleLiveness(typescript, input.bundle) : null;
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
      if (deps || !unit.owner) {
        const found: Finding[] = [];
        rules.run(unit, file, found);
        findings.push(...found.filter((f) => !liveness || liveness.live(f.file, f.range)));
      }
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
  const { diagnostics, suppressed } = diagnosticsOf(findings, root, packages, input.allow ?? new Map());
  const owners = new Set<string>();
  for (const u of input.modules) {
    const owner = packages.ownerOf(u);
    if (owner && !PLATFORM.has(owner.name) && (input.deps ?? true)) owners.add(`${owner.name}@${owner.version}`);
  }
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
