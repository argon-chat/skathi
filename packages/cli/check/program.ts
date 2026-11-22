import { sfcCompiler } from "../runtime-path";
import { readFileSync } from "node:fs";
import { dirname, isAbsolute, resolve } from "node:path";
import type ts from "typescript";
import type { PackageInfo, Packages } from "./packages";

export type TypeScript = typeof ts;

/** One file of the bundle as the analysis sees it. */
export interface Unit {
  /** The file on disk (.vue or .svelte for a component). */
  path: string;
  /** Its name in the TypeScript program: `<path>.ts`/`.js` for a component. */
  programPath: string;
  /** The text positions refer to; for a component everything but its scripts is blanked. */
  text: string;
  /** TypeScript rather than JavaScript. */
  typed: boolean;
  owner: PackageInfo | null;
  sourceFile?: ts.SourceFile;
}

export interface Unreadable {
  path: string;
  owner: PackageInfo | null;
  message: string;
}

const TS_FILE = /\.[cm]?tsx?$/i;
const COMPONENT_FILE = /\.(vue|svelte)$/i;

/** The scripts of an SFC at their own offsets, everything else replaced by spaces. */
export function vueScripts(source: string, path: string): { text: string; typed: boolean } | null {
  const { descriptor } = sfcCompiler(dirname(path)).parse(source, { filename: path });
  const blocks = [descriptor.script, descriptor.scriptSetup].filter((b) => b !== null && !b.src);
  if (!blocks.length) return null;
  const chars = source.replace(/[^\r\n]/g, " ").split("");
  for (const block of blocks) {
    const start = block!.loc.start.offset;
    for (let i = 0; i < block!.content.length; i++) chars[start + i] = block!.content[i]!;
  }
  return { text: chars.join(""), typed: blocks.some((b) => /^tsx?$/.test(b!.lang ?? "")) };
}

const blank = (text: string) => text.replace(/[^\r\n]/g, " ");

/** The `<script>` blocks of a Svelte component at their own offsets, everything else replaced by spaces. */
export function svelteScripts(source: string): { text: string; typed: boolean } | null {
  // A `<script>` in a comment or in `<svelte:head>` is markup, not the component's script.
  const scan = source.replace(/<!--[\s\S]*?-->|<svelte:head[\s>][\s\S]*?<\/svelte:head\s*>/gi, blank);
  const chars = blank(source).split("");
  let found = false;
  let typed = false;
  for (const m of scan.matchAll(/<script(\s[^>]*)?>([\s\S]*?)<\/script\s*>/gi)) {
    const attrs = m[1] ?? "";
    const start = m.index + "<script".length + attrs.length + 1;
    for (let i = 0; i < m[2]!.length; i++) chars[start + i] = source[start + i]!;
    found = true;
    if (/\blang\s*=\s*["']?(ts|typescript)\b/i.test(attrs)) typed = true;
  }
  return found ? { text: chars.join(""), typed } : null;
}

export function readUnits(
  modules: string[],
  packages: Packages,
): { units: Unit[]; unreadable: Unreadable[]; skipped: number } {
  const units: Unit[] = [];
  const unreadable: Unreadable[] = [];
  let skipped = 0;
  for (const path of modules) {
    const owner = packages.ownerOf(path);
    let source: string;
    try {
      source = readFileSync(path, "utf8").replace(/^﻿/, "");
    } catch (e) {
      unreadable.push({ path, owner, message: (e as Error).message });
      continue;
    }
    if (COMPONENT_FILE.test(path)) {
      let scripts: ReturnType<typeof vueScripts>;
      try {
        scripts = /\.svelte$/i.test(path) ? svelteScripts(source) : vueScripts(source, path);
      } catch (e) {
        unreadable.push({ path, owner, message: (e as Error).message });
        continue;
      }
      if (!scripts) {
        skipped++;
        continue;
      }
      units.push({
        path,
        programPath: `${path}.${scripts.typed ? "ts" : "js"}`,
        text: scripts.text,
        typed: scripts.typed,
        owner,
      });
    } else {
      units.push({ path, programPath: path, text: source, typed: TS_FILE.test(path), owner });
    }
  }
  return { units, unreadable, skipped };
}

export interface ProgramInput {
  typescript: TypeScript;
  appRoot: string;
  entry: string;
  units: Unit[];
  aliases: Record<string, string>;
  declarations: string[];
  onFile?: (() => void) | undefined;
  /** The tsconfig as read by `programConfig`; read again when left out. */
  config?: ProgramConfig;
  oldProgram?: ts.Program | undefined;
  /** Parsed files other than the units, kept across programs built one after another. */
  sourceFiles?: Map<string, ts.SourceFile>;
}

export interface BuiltProgram {
  program: ts.Program;
  checker: ts.TypeChecker;
  configNote: string | null;
}

export interface ProgramConfig {
  options: ts.CompilerOptions;
  /** The files the tsconfig includes, roots of the program besides the units. */
  roots: string[];
  note: string | null;
}

export function programConfig(typescript: TypeScript, appRoot: string, entry: string): ProgramConfig {
  const fallback: ts.CompilerOptions = {
    target: typescript.ScriptTarget.ESNext,
    module: typescript.ModuleKind.ESNext,
    moduleResolution: typescript.ModuleResolutionKind.Bundler,
  };
  const configPath = typescript.findConfigFile(appRoot, typescript.sys.fileExists);
  if (!configPath) return { options: fallback, roots: [], note: null };
  try {
    const projects: ts.ParsedCommandLine[] = [];
    const visited = new Set<string>();
    const visit = (path: string) => {
      if (visited.has(resolve(path))) return;
      visited.add(resolve(path));
      const config = typescript.readConfigFile(path, typescript.sys.readFile);
      if (config.error) throw new Error(typescript.flattenDiagnosticMessageText(config.error.messageText, "\n"));
      const parsed = typescript.parseJsonConfigFileContent(config.config, typescript.sys, dirname(path));
      projects.push(parsed);
      for (const reference of parsed.projectReferences ?? []) visit(typescript.resolveProjectReferencePath(reference));
    };
    visit(configPath);
    const key = (p: string) => resolve(p).toLowerCase();
    const selected = projects.find((p) => p.fileNames.some((f) => key(f) === key(entry))) ?? projects[0]!;
    return {
      options: selected.options,
      roots: selected.fileNames.filter((f) => TS_FILE.test(f)),
      note: null,
    };
  } catch (e) {
    return { options: fallback, roots: [], note: `${configPath}: ${(e as Error).message}; checked with defaults` };
  }
}

export function buildProgram(input: ProgramInput): BuiltProgram {
  const { typescript, units, aliases } = input;
  const { options: base, roots, note } = input.config ?? programConfig(typescript, input.appRoot, input.entry);
  const options: ts.CompilerOptions = {
    ...base,
    noEmit: true,
    allowJs: true,
    checkJs: false,
    skipLibCheck: true,
    declaration: false,
    composite: false,
    incremental: false,
    maxNodeModuleJsDepth: 0,
  };
  const caseless = !typescript.sys.useCaseSensitiveFileNames;
  const key = (p: string) => (caseless ? resolve(p).toLowerCase() : resolve(p));
  const own = new Map(units.map((u) => [key(u.programPath), u]));
  const host = typescript.createCompilerHost(options, true);
  const readFile = host.readFile.bind(host);
  const fileExists = host.fileExists.bind(host);
  const getSourceFile = host.getSourceFile.bind(host);
  host.readFile = (path) => own.get(key(path))?.text ?? readFile(path);
  host.fileExists = (path) => own.has(key(path)) || fileExists(path);
  host.getSourceFile = (path, language, onError, create) => {
    const unit = own.get(key(path));
    if (!unit) {
      const cached = input.sourceFiles?.get(key(path));
      if (cached) return cached;
      const file = getSourceFile(path, language, onError, create);
      if (file) input.sourceFiles?.set(key(path), file);
      return file;
    }
    input.onFile?.();
    unit.sourceFile = typescript.createSourceFile(
      path,
      unit.text,
      language,
      true,
      unit.typed ? typescript.ScriptKind.TS : /\.jsx$/i.test(path) ? typescript.ScriptKind.JSX : undefined,
    );
    return unit.sourceFile;
  };
  const directoryAliases = Object.entries(aliases).filter(([, path]) => typescript.sys.directoryExists(path));
  host.resolveModuleNames = (names, importer) =>
    names.map((specifier) => {
      // The bundle's aliases win over node_modules, as they do in the build.
      const directory = directoryAliases.find(([name]) => specifier === name || specifier.startsWith(name + "/"));
      const alias = aliases[specifier];
      if (alias && !typescript.sys.directoryExists(alias) && !COMPONENT_FILE.test(alias)) {
        if (TS_FILE.test(alias)) return { resolvedFileName: alias, extension: typescript.Extension.Ts };
        // A built .mjs stands in for a package: take that package's own declarations.
        return (
          typescript.resolveModuleName(specifier, alias, options, host).resolvedModule ?? {
            resolvedFileName: alias,
            extension: typescript.Extension.Js,
          }
        );
      }
      const target =
        alias ?? (directory && resolve(directory[1], specifier.slice(directory[0].length + 1))) ?? specifier;
      const found = typescript.resolveModuleName(target, importer, options, host).resolvedModule;
      if (found) return found;
      if (COMPONENT_FILE.test(target) && (target.startsWith(".") || isAbsolute(target))) {
        for (const ext of ["ts", "js"] as const) {
          const path = resolve(dirname(importer), `${target}.${ext}`);
          if (own.has(key(path)))
            return {
              resolvedFileName: path,
              extension: ext === "ts" ? typescript.Extension.Ts : typescript.Extension.Js,
            };
        }
      }
      return undefined;
    });
  const program = typescript.createProgram({
    rootNames: [...new Set([...units.map((u) => u.programPath), ...input.declarations, ...roots])],
    options,
    host,
    ...(input.oldProgram && { oldProgram: input.oldProgram }),
  });
  return { program, checker: program.getTypeChecker(), configNote: note };
}
