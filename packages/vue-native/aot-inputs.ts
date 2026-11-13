// Build-time input archive for a future typed AOT frontend. This is deliberately
// separate from VNPK: source inputs are neither executable code nor JSC bytecode.
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import type ts from "typescript";

type TypeScript = typeof ts;
type InputKind = "source" | "vue" | "vue-module" | "vue-script" | "transformed-module" | "config";
interface Input { path: string; kind: InputKind; text: string; loader?: string }
const sha256 = (text: string | Uint8Array) => createHash("sha256").update(text).digest("hex");
const slash = (path: string) => path.split(sep).join("/");
const compare = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;

export interface AotEngineIdentity {
  jscPin: string | null;
  asset: string | null;
  aot: boolean | null;
  sha256: string | null;
}

export function aotEngineIdentity(pins: string | null): AotEngineIdentity {
  const values = new Map<string, string>();
  for (const line of (pins ?? "").split(/\r?\n/)) {
    const match = /^\s*(webkit\.[a-z0-9]+)\s*=\s*([^#]*?)\s*(?:#.*)?$/.exec(line);
    if (match) values.set(match[1]!, match[2]!.trim());
  }
  const flag = values.get("webkit.aot");
  if (flag !== undefined && flag !== "0" && flag !== "1") throw new Error(`Invalid webkit.aot pin: ${flag}`);
  return {
    jscPin: values.get("webkit.version") || null,
    asset: values.get("webkit.asset") || null,
    aot: flag === undefined ? null : flag === "1",
    sha256: values.get("webkit.sha256") || null,
  };
}

export interface AotInputsOptions {
  out: string;
  appRoot: string;
  entry: string;
  typescript: TypeScript;
  aliases: Record<string, string>;
  declarations?: string[];
  sourceRoots?: Record<string, string>;
}

export class AotInputs {
  private readonly inputs = new Map<string, Input>();
  private readonly modules = new Map<string, { path: string; text: string }>();
  private readonly virtual = new Map<string, { path: string; text: string }>();
  private readonly unresolved = new Map<string, { importer: string; specifier: string }>();
  private readonly key: (path: string) => string;

  constructor(private readonly options: AotInputsOptions) {
    this.key = options.typescript.sys.useCaseSensitiveFileNames
      ? (path) => resolve(path)
      : (path) => resolve(path).toLowerCase();
  }

  private pathId(path: string): string {
    const appPath = relative(this.options.appRoot, path);
    if (!isAbsolute(appPath) && appPath !== ".." && !appPath.startsWith(`..${sep}`)) {
      const first = slash(appPath).split("/", 1)[0]!;
      const reserved = Object.keys(this.options.sourceRoots ?? {}).some((name) => first.toLowerCase() === `@${name}`.toLowerCase());
      if (reserved) throw new Error(`AOT app input ${appPath} collides with a named source-root anchor`);
      return slash(appPath);
    }
    for (const [name, root] of Object.entries(this.options.sourceRoots ?? {})) {
      if (!/^[a-z][a-z0-9-]*$/i.test(name)) throw new Error(`Invalid AOT source-root name: ${name}`);
      const rootPath = relative(root, path);
      if (!isAbsolute(rootPath) && rootPath !== ".." && !rootPath.startsWith(`..${sep}`)) return `@${name}/${slash(rootPath)}`;
    }
    if (isAbsolute(appPath)) throw new Error(`AOT input ${path} is on another volume; add a stable sourceRoots anchor`);
    return slash(appPath);
  }

  private add(path: string, text: string, kind: InputKind, loader?: string): void {
    this.inputs.set(`${kind}:${this.key(path)}`, { path: resolve(path), kind, text, loader });
  }

  source(path: string, original: string, transformed = original): void {
    if (this.virtual.has(this.key(path))) throw new Error(`AOT source ${path} collides with a virtual Vue module`);
    this.add(path, original, "source");
    if (transformed !== original) this.add(path, transformed, "transformed-module");
    this.modules.set(this.key(path), { path: resolve(path), text: transformed });
  }

  private addVirtual(path: string, text: string): void {
    if (this.options.typescript.sys.fileExists(path) || this.modules.has(this.key(path))) {
      throw new Error(`AOT virtual Vue module ${path} collides with a real source file`);
    }
    const existing = this.virtual.get(this.key(path));
    if (existing && existing.text !== text) throw new Error(`AOT virtual Vue module ${path} has conflicting contents`);
    this.virtual.set(this.key(path), { path, text });
  }

  vue(path: string, original: string, compiled: string, loader: "js" | "ts", originalScript = ""): void {
    this.add(path, original, "vue");
    this.add(path, compiled, "vue-module", loader);
    const virtualPath = `${resolve(path)}.${loader}`;
    this.addVirtual(virtualPath, compiled);
    // compileScript can erase type imports used only by defineProps macros.
    if (originalScript) {
      this.add(path, originalScript, "vue-script", loader);
      const scriptPath = `${resolve(path)}.original.${loader}`;
      this.addVirtual(scriptPath, originalScript);
    }
  }

  private collectTypes(): { version: string; config: string | null; diagnostics: { code: number; message: string }[] } {
    const { typescript, appRoot, entry, aliases, declarations = [] } = this.options;
    const read = (path: string): string | undefined => {
      const known = this.virtual.get(this.key(path)) ?? this.modules.get(this.key(path));
      if (known) return known.text;
      const text = typescript.sys.readFile(path);
      if (text !== undefined) this.add(path, text, /\.json$/i.test(path) ? "config" : "source");
      return text;
    };
    const configPath = typescript.findConfigFile(appRoot, typescript.sys.fileExists);
    let roots: string[] = [];
    let options: ts.CompilerOptions = {
      target: typescript.ScriptTarget.ESNext,
      module: typescript.ModuleKind.ESNext,
      moduleResolution: typescript.ModuleResolutionKind.Bundler,
    };
    if (configPath) {
      const visited = new Set<string>();
      const projects: ts.ParsedCommandLine[] = [];
      const visit = (path: string): void => {
        if (visited.has(this.key(path))) return;
        visited.add(this.key(path));
        const config = typescript.readConfigFile(path, read);
        if (config.error) throw new Error(typescript.flattenDiagnosticMessageText(config.error.messageText, "\n"));
        const parsed = typescript.parseJsonConfigFileContent(config.config, { ...typescript.sys, readFile: read }, dirname(path));
        const errors = parsed.errors.filter((diagnostic) => diagnostic.code !== 18003);
        if (errors.length) throw new Error(errors.map((diagnostic) => typescript.flattenDiagnosticMessageText(diagnostic.messageText, "\n")).join("\n"));
        projects.push(parsed);
        for (const reference of parsed.projectReferences ?? []) {
          visit(typescript.resolveProjectReferencePath(reference));
        }
      };
      visit(configPath);
      // Use the entry's project (a Vue/Vite solution tsconfig has files: []) so its aliases and ambient types survive.
      const selected = projects.find((project) => project.fileNames.some((path) => this.key(path) === this.key(entry))) ?? projects[0]!;
      options = selected.options;
      roots = selected.fileNames.filter((path) => /\.[cm]?[jt]sx?$/i.test(path));
    }
    const host = typescript.createCompilerHost({ ...options, noEmit: true, allowJs: true });
    host.readFile = read;
    host.fileExists = (path) => this.virtual.has(this.key(path)) || typescript.sys.fileExists(path);
    host.resolveModuleNames = (names, importer) => names.map((specifier) => {
      const alias = aliases[specifier];
      // Bun's resolved runtime modules are already roots; keep the package's declaration entry too,
      // or .d.ts files and every type-only dependency behind them are dropped.
      const resolved = typescript.resolveModuleName(specifier, importer, options, host).resolvedModule
        ?? (specifier.endsWith(".vue") ? typescript.resolveModuleName(`${specifier}.ts`, importer, options, host).resolvedModule : undefined)
        ?? (alias ? typescript.resolveModuleName(specifier, alias, options, host).resolvedModule : undefined)
        ?? (alias ? { resolvedFileName: alias, extension: /\.[cm]?tsx?$/.test(alias) ? typescript.Extension.Ts : typescript.Extension.Js } : undefined);
      if (!resolved) {
        const value = { importer: this.pathId(importer), specifier };
        this.unresolved.set(`${value.importer}:${specifier}`, value);
      }
      return resolved;
    });
    // Override CompilerHost's getSourceFile so virtual Vue modules and post-plugin TS participate.
    host.getSourceFile = (path, languageVersion) => {
      const text = read(path);
      return text === undefined ? undefined : typescript.createSourceFile(path, text, languageVersion, true);
    };
    const program = typescript.createProgram({
      rootNames: [...new Set([entry, ...roots, ...declarations, ...[...this.modules.values()].map((value) => value.path), ...[...this.virtual.values()].map((value) => value.path)])].sort(compare),
      options: { ...options, noEmit: true, allowJs: true },
      host,
    });
    // Force resolution without running type checking or changing build acceptance.
    program.getSourceFiles();
    return {
      version: typescript.version,
      config: configPath ? this.pathId(configPath) : null,
      diagnostics: program.getOptionsDiagnostics().map((diagnostic) => ({ code: diagnostic.code, message: typescript.flattenDiagnosticMessageText(diagnostic.messageText, "\n") })),
    };
  }

  write(details: { script: string; scriptUrl: string; defines: Record<string, string>; engine: AotEngineIdentity; sdkVersion: string }): string {
    const typescript = this.collectTypes();
    const { out, entry, aliases } = this.options;
    const inputs = [...this.inputs.values()].map((input) => ({ ...input, id: this.pathId(input.path) }));
    const identities = new Set<string>();
    for (const input of inputs) {
      const identity = `${input.kind}:${input.id}`;
      if (identities.has(identity)) throw new Error(`AOT input identity collision: ${identity}`);
      identities.add(identity);
    }
    const blobs = join(out, "blobs");
    mkdirSync(blobs, { recursive: true });
    const written = new Set<string>();
    const files = inputs.map((input) => {
      const hash = sha256(input.text);
      const blob = `blobs/${hash}`;
      if (!written.has(hash)) {
        const path = join(out, blob);
        writeFileSync(`${path}.tmp`, input.text);
        renameSync(`${path}.tmp`, path);
        written.add(hash);
      }
      return { path: input.id, kind: input.kind, ...(input.loader ? { loader: input.loader } : {}), sha256: hash, bytes: Buffer.byteLength(input.text), blob };
    }).sort((a, b) => compare(a.path, b.path) || compare(a.kind, b.kind));
    const manifest = {
      schema: "vue-native.aot-inputs", version: 1, stage: "typed-source-inputs",
      entry: this.pathId(entry),
      typescript,
      sdkVersion: details.sdkVersion,
      engine: details.engine,
      script: { url: details.scriptUrl, sha256: sha256(details.script), bytes: Buffer.byteLength(details.script) },
      defines: details.defines,
      aliases: Object.fromEntries(Object.entries(aliases).sort(([a], [b]) => compare(a, b)).map(([key, value]) => [key, this.pathId(value)])),
      files,
      unresolvedImports: [...this.unresolved.values()].sort((a, b) => compare(a.importer, b.importer) || compare(a.specifier, b.specifier)),
      limitations: [
        "Source archive, not a native AOT image; no host ABI or builtin-effect descriptors are inferred from declarations.",
        "Original paths are app-root-relative or use a named source-root anchor; blob names are content hashes. An AOT frontend needs a resolver that maps these paths to blobs.",
        "Vue modules and TS/JS source rewrites are retained before Bun transpilation. Bun-generated asset/query/HTML modules, CSS installation and module linking still require a frontend adapter.",
        "TypeScript file-module resolution is recorded in addition to Bun's runtime graph; unresolvedImports also includes ambient module names that have no file mapping. A frontend adapter must handle them. This archive does not assert type-checking or AOT compatibility.",
      ],
    };
    const manifestPath = join(out, "manifest.json");
    writeFileSync(`${manifestPath}.tmp`, JSON.stringify(manifest, null, 2) + "\n");
    renameSync(`${manifestPath}.tmp`, manifestPath);
    return manifestPath;
  }
}
