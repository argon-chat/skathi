// Vite parity for the builders (prod.ts and build.ts), so an import means the same in both:
// ?raw (text), ?url / assets (URL; `vn build` packs the file), .svg (a component with
// vite-svg-loader, else a URL), import.meta.glob (expanded at load time), ?worker (a constructor
// throwing a named NotSupportedError), .wasm (a stub with an advisory). Unsupported shapes fail by name.

import { existsSync, readFileSync, realpathSync, statSync } from "fs";
import { parse as parseSfc, type SFCTemplateBlock } from "@vue/compiler-sfc";
import { basename, dirname, extname, isAbsolute, join, relative, resolve, sep } from "path";

const J = (v: unknown) => JSON.stringify(v);
const posix = (p: string) => p.split(sep).join("/");

/** The `fs` compileScript resolves imported types through; `fileExists` is files only (`from "."` must land on ./index.ts). */
export const SFC_FS = {
  fileExists: (file: string) => {
    try {
      return statSync(file).isFile();
    } catch {
      return false;
    }
  },
  readFile: (file: string) => {
    try {
      return readFileSync(file, "utf-8");
    } catch {
      return undefined;
    }
  },
  realpath: (file: string) => {
    try {
      return realpathSync.native(file);
    } catch {
      return file;
    }
  },
};

/** Keys the builders own; an app may not redefine them. */
const RESERVED_DEFINES = new Set([
  "__DEV__",
  "process.env.NODE_ENV",
  "import.meta",
  "import.meta.url",
  "import.meta.hot",
  "import.meta.env",
  "import.meta.glob",
]);
const DEFINE_KEY = /^[A-Za-z_$][\w$]*(\.[A-Za-z_$][\w$]*)*$/;
const IDENT_PATH = DEFINE_KEY;

/** Is `code` a value Bun's `define` inserts as code? Anything but JSON or an identifier path becomes a string literal, so it is refused. */
function isDefineCode(code: string): boolean {
  if (IDENT_PATH.test(code)) return true;
  try {
    JSON.parse(code);
    return true;
  } catch {
    return false;
  }
}

export interface AppDefines {
  defines: Record<string, string>;
  errors: string[];
}

/** Vite's `define` from `vn.config.json` and the command line; `--define K=V` wins over the file. */
export function appDefines(configDefine: unknown, cli: string[], where = "vn.config.json"): AppDefines {
  const defines: Record<string, string> = {};
  const errors: string[] = [];
  const put = (key: string, code: string, from: string) => {
    if (!DEFINE_KEY.test(key)) {
      errors.push(`${from}: define key ${J(key)} is not an identifier or a dotted path`);
      return;
    }
    if (RESERVED_DEFINES.has(key)) {
      errors.push(`${from}: define ${J(key)} is set by the builder and cannot be overridden`);
      return;
    }
    if (!isDefineCode(code)) {
      errors.push(
        `${from}: define ${key} = ${J(code)} is neither JSON nor an identifier — Bun would insert it as a ` +
          `string. A string value must carry its own quotes (Vite's rule): ${J(J(code))}`,
      );
      return;
    }
    defines[key] = code;
  };
  if (configDefine !== undefined) {
    if (!configDefine || typeof configDefine !== "object" || Array.isArray(configDefine)) {
      errors.push(`${where}: "define" must be an object`);
    } else {
      for (const [k, v] of Object.entries(configDefine as Record<string, unknown>)) {
        if (k.startsWith("$")) continue;
        put(k, typeof v === "string" ? v : J(v), `${where} "define"`);
      }
    }
  }
  for (const arg of cli) {
    const i = arg.indexOf("=");
    if (i <= 0) {
      errors.push(`--define ${J(arg)}: expected KEY=VALUE`);
      continue;
    }
    put(arg.slice(0, i).trim(), arg.slice(i + 1), "--define");
  }
  return { defines, errors };
}

export function takeDefineArgs(argv: string[]): { defines: string[]; rest: string[] } {
  const defines: string[] = [];
  const rest: string[] = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]!;
    if (a === "--define") {
      if (i + 1 < argv.length) defines.push(argv[++i]!);
    } else if (a.startsWith("--define=")) defines.push(a.slice("--define=".length));
    else rest.push(a);
  }
  return { defines, rest };
}

export const ASSET_EXT =
  /\.(apng|bmp|png|jpe?g|jfif|pjpeg|pjp|gif|ico|webp|avif|cur|jxl|mp4|webm|ogg|mp3|wav|flac|aac|opus|mov|m4a|vtt|woff2?|eot|ttf|otf|webmanifest|pdf|txt)$/i;

/**
 * The payload key and URL of an imported file: root-relative inside the document root (one URL
 * works dev and packaged), else a content-addressed `_vn/fs/` key.
 */
export function assetKey(absPath: string, docRoot: string): { key: string; url: string } {
  const rel = relative(docRoot, absPath);
  let key: string;
  if (!rel || rel.startsWith("..") || isAbsolute(rel)) {
    const h = Bun.hash(posix(resolve(absPath)).toLowerCase()).toString(16).slice(0, 8);
    key = `_vn/fs/${h}/${basename(absPath)}`;
  } else {
    key = posix(rel);
  }
  const url = "/" + key.split("/").map(encodeURIComponent).join("/");
  return { key, url };
}

type Lit = string | number | boolean | null | Lit[] | { [k: string]: Lit };

class LitParser {
  i = 0;
  constructor(readonly s: string) {}
  ws() {
    for (;;) {
      while (this.i < this.s.length && /\s/.test(this.s[this.i]!)) this.i++;
      if (this.s.startsWith("//", this.i)) {
        const nl = this.s.indexOf("\n", this.i);
        this.i = nl < 0 ? this.s.length : nl + 1;
      } else if (this.s.startsWith("/*", this.i)) {
        const end = this.s.indexOf("*/", this.i + 2);
        this.i = end < 0 ? this.s.length : end + 2;
      } else return;
    }
  }
  fail(what: string): never {
    throw new Error(`${what} at offset ${this.i}`);
  }
  value(): Lit {
    this.ws();
    const c = this.s[this.i];
    if (c === "'" || c === '"' || c === "`") return this.str();
    if (c === "[") {
      this.i++;
      const out: Lit[] = [];
      for (;;) {
        this.ws();
        if (this.s[this.i] === "]") {
          this.i++;
          return out;
        }
        out.push(this.value());
        this.ws();
        if (this.s[this.i] === ",") this.i++;
        else if (this.s[this.i] !== "]") this.fail("expected , or ]");
      }
    }
    if (c === "{") {
      this.i++;
      const out: { [k: string]: Lit } = {};
      for (;;) {
        this.ws();
        if (this.s[this.i] === "}") {
          this.i++;
          return out;
        }
        let key: string;
        const q = this.s[this.i];
        if (q === "'" || q === '"') key = this.str();
        else {
          const m = /^[A-Za-z_$][\w$]*/.exec(this.s.slice(this.i));
          if (!m) this.fail("expected a property name");
          key = m[0];
          this.i += key.length;
        }
        this.ws();
        if (this.s[this.i] !== ":") this.fail(`expected : after ${key}`);
        this.i++;
        out[key] = this.value();
        this.ws();
        if (this.s[this.i] === ",") this.i++;
        else if (this.s[this.i] !== "}") this.fail("expected , or }");
      }
    }
    const m = /^(true|false|null|-?\d+(\.\d+)?)/.exec(this.s.slice(this.i));
    if (m) {
      this.i += m[0].length;
      return JSON.parse(m[0]);
    }
    this.fail("not a literal (import.meta.glob arguments must be literals — Vite's rule too)");
  }
  str(): string {
    const q = this.s[this.i++]!;
    let out = "";
    while (this.i < this.s.length && this.s[this.i] !== q) {
      const ch = this.s[this.i]!;
      if (q === "`" && ch === "$" && this.s[this.i + 1] === "{") this.fail("template interpolation in a glob argument");
      if (ch === "\\") {
        out += this.s[this.i + 1] ?? "";
        this.i += 2;
      } else {
        out += ch;
        this.i++;
      }
    }
    if (this.s[this.i] !== q) this.fail("unterminated string");
    this.i++;
    return out;
  }
}

function closingParen(s: string, open: number): number {
  let depth = 0;
  for (let i = open; i < s.length; i++) {
    const c = s[i]!;
    if (c === "'" || c === '"' || c === "`") {
      for (i++; i < s.length && s[i] !== c; i++) if (s[i] === "\\") i++;
    } else if (c === "/" && s[i + 1] === "/") {
      const nl = s.indexOf("\n", i);
      i = nl < 0 ? s.length : nl;
    } else if (c === "/" && s[i + 1] === "*") {
      const end = s.indexOf("*/", i + 2);
      i = end < 0 ? s.length : end + 1;
    } else if (c === "(" || c === "[" || c === "{") depth++;
    else if (c === ")" || c === "]" || c === "}") {
      depth--;
      if (depth === 0) return i + 1;
    }
  }
  return -1;
}

const GLOB_OPTIONS = new Set(["eager", "import", "query", "as", "exhaustive", "base"]);
const GLOB_CHAR = /[*?[\]{}!]/;

export interface GlobContext {
  file: string;
  docRoot: string;
  /** Dev form: import each match by importer-relative path (prod imports by absolute path). */
  relativeSpecs?: boolean;
}

function splitGlob(pattern: string): [string, string] {
  const segs = pattern.split("/");
  const at = segs.findIndex((s) => GLOB_CHAR.test(s));
  if (at < 0) return [segs.slice(0, -1).join("/") || ".", segs[segs.length - 1]!];
  return [segs.slice(0, at).join("/") || ".", segs.slice(at).join("/")];
}

function globBase(pattern: string, ctx: GlobContext): string {
  if (pattern.startsWith("./") || pattern.startsWith("../")) return dirname(ctx.file);
  if (pattern.startsWith("/")) return ctx.docRoot;
  throw new Error(
    `import.meta.glob(${J(pattern)}): only relative ("./", "../") and root ("/") patterns are supported — ` +
      `an alias pattern needs resolving first, which vue-native's builder does not do yet`,
  );
}

function globFiles(pattern: string, ctx: GlobContext, exhaustive: boolean): string[] {
  const base = globBase(pattern, ctx);
  const [dir, glob] = splitGlob(pattern.startsWith("/") ? "." + pattern : pattern);
  const cwd = resolve(base, dir);
  if (!existsSync(cwd)) return [];
  const out: string[] = [];
  const throughModules = exhaustive || pattern.includes("node_modules");
  for (const rel of new Bun.Glob(glob).scanSync({ cwd, onlyFiles: true, dot: exhaustive })) {
    const p = posix(rel);
    if (!throughModules && p.split("/").includes("node_modules")) continue;
    out.push(resolve(cwd, rel));
  }
  return out;
}

function queryString(q: Lit | undefined): string {
  if (q === undefined || q === null || q === "") return "";
  if (typeof q === "string") return q.startsWith("?") ? q : `?${q}`;
  if (typeof q === "object" && !Array.isArray(q)) {
    const parts = Object.entries(q).map(([k, v]) => (v === true || v === "" ? k : `${k}=${String(v)}`));
    return parts.length ? `?${parts.join("&")}` : "";
  }
  throw new Error(`import.meta.glob: "query" must be a string or an object`);
}

/**
 * Expands every `import.meta.glob(...)` in `src` as Vite does; `null` when there is nothing to
 * expand. `base` rebases the keys; `node_modules` is skipped unless named or `exhaustive`.
 */
export function expandGlobs(src: string, ctx: GlobContext): { code: string; count: number; files: number } | null {
  if (!src.includes("import.meta.glob")) return null;
  const re = /\bimport\.meta\.glob\b/g;
  const header: string[] = [];
  let out = "";
  let last = 0;
  let call = 0;
  let files = 0;
  const lineOf = (i: number) => src.slice(0, i).split("\n").length;
  for (let m = re.exec(src); m; m = re.exec(src)) {
    const start = m.index;
    let i = start + m[0].length;
    while (/\s/.test(src[i] ?? "")) i++;
    if (src[i] === "<") {
      let depth = 0;
      for (; i < src.length; i++) {
        if (src[i] === "<") depth++;
        else if (src[i] === ">" && --depth === 0) {
          i++;
          break;
        }
      }
      while (/\s/.test(src[i] ?? "")) i++;
    }
    if (src[i] !== "(") continue; // a mention, not a call (`typeof import.meta.glob`)
    const end = closingParen(src, i);
    const where = `${ctx.file}:${lineOf(start)}`;
    if (end < 0) throw new Error(`${where}: unterminated import.meta.glob(`);
    const p = new LitParser(src.slice(i + 1, end - 1));
    let patterns: Lit;
    let options: Lit = {};
    try {
      patterns = p.value();
      p.ws();
      if (p.s[p.i] === ",") {
        p.i++;
        p.ws();
        if (p.i < p.s.length) options = p.value();
      }
      p.ws();
      if (p.s[p.i] === ",") p.i++;
      p.ws();
      if (p.i < p.s.length) p.fail("unexpected trailing argument");
    } catch (e) {
      throw new Error(`${where}: import.meta.glob: ${(e as Error).message}`);
    }
    const list = typeof patterns === "string" ? [patterns] : patterns;
    if (!Array.isArray(list) || !list.every((x) => typeof x === "string") || !list.length) {
      throw new Error(`${where}: import.meta.glob: the first argument must be a string or an array of strings`);
    }
    if (!options || typeof options !== "object" || Array.isArray(options)) {
      throw new Error(`${where}: import.meta.glob: the options must be an object literal`);
    }
    const opts = options as Record<string, Lit>;
    for (const k of Object.keys(opts)) {
      if (!GLOB_OPTIONS.has(k)) throw new Error(`${where}: import.meta.glob: unknown option ${J(k)}`);
    }
    let importName = opts.import as string | undefined;
    let query = opts.query;
    if (opts.as !== undefined) {
      // Vite 4's `as`, deprecated in Vite 5: `as: "raw" | "url"` == query + default.
      if (opts.as !== "raw" && opts.as !== "url") {
        throw new Error(`${where}: import.meta.glob: "as" must be "raw" or "url"`);
      }
      query = `?${opts.as}`;
      importName ??= "default";
    }
    const qs = (() => {
      try {
        return queryString(query);
      } catch (e) {
        throw new Error(`${where}: ${(e as Error).message}`);
      }
    })();
    const eager = opts.eager === true;
    const exhaustive = opts.exhaustive === true;
    const positive = (list as string[]).filter((x) => !x.startsWith("!"));
    const negative = (list as string[]).filter((x) => x.startsWith("!")).map((x) => x.slice(1));
    const matched = new Set<string>();
    try {
      for (const pat of positive) for (const f of globFiles(pat, ctx, exhaustive)) matched.add(f);
      for (const pat of negative) for (const f of globFiles(pat, ctx, true)) matched.delete(f);
    } catch (e) {
      throw new Error(`${where}: ${(e as Error).message}`);
    }
    matched.delete(resolve(ctx.file));
    const rooted = positive[0]!.startsWith("/");
    const keyBase =
      typeof opts.base === "string"
        ? resolve(opts.base.startsWith("/") ? ctx.docRoot : dirname(ctx.file), opts.base.replace(/^\//, "./"))
        : rooted
          ? ctx.docRoot
          : dirname(ctx.file);
    const keyed = [...matched].map((f) => {
      const rel = posix(relative(keyBase, f));
      const key = rooted && opts.base === undefined ? `/${rel}` : rel.startsWith("../") ? rel : `./${rel}`;
      if (!ctx.relativeSpecs) return { key, spec: posix(f) + qs };
      const near = posix(relative(dirname(ctx.file), f));
      return { key, spec: (near.startsWith("../") ? near : `./${near}`) + qs };
    });
    keyed.sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
    const entries = keyed.map(({ key, spec }, n) => {
      if (eager) {
        const v = `__vn_glob_${call}_${n}`;
        header.push(
          importName && importName !== "*"
            ? `import { ${importName === "default" ? "default" : importName} as ${v} } from ${J(spec)};`
            : `import * as ${v} from ${J(spec)};`,
        );
        return `${J(key)}: ${v}`;
      }
      const load = `() => import(${J(spec)})`;
      return `${J(key)}: ${importName && importName !== "*" ? `${load}.then((m) => m[${J(importName)}])` : load}`;
    });
    files += keyed.length;
    out += src.slice(last, start) + `({ ${entries.join(", ")} })`;
    last = end;
    re.lastIndex = end;
    call++;
  }
  if (!call) return null;
  out += src.slice(last);
  // A shebang has to stay on line 1.
  const bang = out.startsWith("#!") ? out.indexOf("\n") + 1 : 0;
  const code = out.slice(0, bang) + header.join("\n") + (header.length ? "\n" : "") + out.slice(bang);
  return { code, count: call, files };
}

/** The advisory prefix runtime.ts's `warnCompat` uses — one grep finds both. */
const ADVISE = "[vue-native compat]";

function notSupportedError(what: string, why: string): string {
  const msg = `${what}: ${why}`;
  return (
    `(() => { const m = ${J(msg)}; console.warn(${J(ADVISE + " ")} + m); ` +
    `return typeof DOMException === "function" ? new DOMException(m, "NotSupportedError") ` +
    `: Object.assign(new Error(m), { name: "NotSupportedError" }); })()`
  );
}

const WORKERS_WHY =
  "vue-native has no Web Workers yet — run the work on the main thread for now";

export function workerStubModule(id: string, url = false): string {
  if (url) {
    return `console.warn(${J(`${ADVISE} ${id}?worker&url: ${WORKERS_WHY}`)});\nexport default ${J(`vn-unsupported:worker/${id}`)};\n`;
  }
  return (
    `export default class VnWorkerUnavailable {\n` +
    `  constructor() { throw ${notSupportedError(`new Worker(${id}?worker)`, WORKERS_WHY)}; }\n` +
    `}\n`
  );
}

/** `new Worker(new URL("./w.ts", import.meta.url))` becomes the same named-error class as `?worker`. */
export function stubWorkerUrls(src: string, file: string): { code: string; count: number } | null {
  if (!/\bnew\s+(?:Shared)?Worker\s*\(\s*new\s+URL\s*\(/.test(src)) return null;
  let count = 0;
  const code = src.replace(
    /\bnew\s+(Shared)?Worker\s*\(\s*(new\s+URL\s*\(\s*(['"`])([^'"`]+)\3\s*,\s*import\.meta\.url\s*\))/g,
    (_m, shared: string | undefined, urlExpr: string, _q: string, spec: string) => {
      count++;
      const what = `new ${shared ? "Shared" : ""}Worker(new URL(${J(spec)}, import.meta.url)) in ${file}`;
      return `new (class VnWorkerUnavailable { constructor() { throw ${notSupportedError(what, WORKERS_WHY)}; } })(${urlExpr}`;
    },
  );
  return count ? { code, count } : null;
}

const WASM_WHY =
  "WebAssembly is out of scope for vue-native; " +
  "the native replacement is a host binding";

export function wasmStubModule(id: string, query: Set<string>): string {
  const advise = `console.warn(${J(`${ADVISE} ${id}: not bundled — ${WASM_WHY}`)});\n`;
  if (query.has("url")) return advise + `export default ${J(`vn-unsupported:wasm/${id}`)};\n`;
  return advise + `export default function init() { return Promise.reject(${notSupportedError(id, WASM_WHY)}); }\n`;
}

export function svgDefaultOf(appPkg: Record<string, unknown> | null): "component" | "url" {
  if (!appPkg) return "url";
  for (const k of ["dependencies", "devDependencies"]) {
    const deps = appPkg[k];
    if (deps && typeof deps === "object" && "vite-svg-loader" in (deps as object)) return "component";
  }
  return "url";
}

export function svgAsSfc(svg: string): string {
  const body = svg
    .replace(/<\?xml[\s\S]*?\?>/g, "")
    .replace(/<!DOCTYPE[^>]*>/gi, "")
    .replace(/<!--[\s\S]*?-->/g, "")
    // vite-svg-loader's rewrite: a `<style>` inside the SVG must survive the template compiler.
    .replace(/<style/g, '<component is="style"')
    .replace(/<\/style>/g, "</component>")
    .trim();
  return `<template>${body}</template>`;
}

export interface ShapesOptions {
  sourceLoaded?: (file: string, source: string, transformed: string) => void;
  /** The document root: where `vn.config.json` / `index.html` live. URLs and `/` globs start here. */
  docRoot: string;
  wsRoot: string;
  svgDefault: "component" | "url";
  compileSfc: (
    absPath: string,
    source: string,
  ) => { contents: string; loader: "js" | "ts" } | Promise<{ contents: string; loader: "js" | "ts" }>;
  assets: Map<string, string>;
  notes: Map<string, number>;
  /** The dev builder: one module per file, imports left to its registry. */
  dev?: boolean;
  /** Called with each file a `?query` import builds into its importer, so a watcher can rebuild the importer. */
  inlined?: (absPath: string) => void;
}

/**
 * Resolves an import specifier as the bundler does (tsconfig `paths`, package `exports`, browser
 * field). build.ts calls it directly because its per-module builds leave imports external.
 */
export function resolveImport(spec: string, fromDir: string): string | null {
  try {
    return Bun.resolveSync(spec, fromDir);
  } catch {
    return null;
  }
}

export const importerDir = (args: { resolveDir?: string; importer?: string }, fallback: string) =>
  args.resolveDir || (args.importer ? dirname(args.importer) : fallback);

const NS = "vn-vite";
const KINDS = ["raw", "url", "inline", "worker", "sharedworker", "component", "init"];

function note(o: ShapesOptions, what: string) {
  o.notes.set(what, (o.notes.get(what) ?? 0) + 1);
}

export function loaderOf(path: string): "ts" | "tsx" | "js" | "jsx" {
  const ext = extname(path).toLowerCase();
  if (ext === ".tsx") return "tsx";
  if (ext === ".jsx") return "jsx";
  if (ext === ".ts" || ext === ".mts" || ext === ".cts") return "ts";
  return "js";
}

export function transformSource(
  code: string,
  file: string,
  o: Pick<ShapesOptions, "docRoot" | "wsRoot" | "notes" | "dev">,
): string | null {
  let changed = false;
  const add = (what: string, n: number) => o.notes.set(what, (o.notes.get(what) ?? 0) + n);
  const g = expandGlobs(code, { file, docRoot: o.docRoot, relativeSpecs: o.dev });
  if (g) {
    code = g.code;
    changed = true;
    add("import.meta.glob calls expanded", g.count);
    add("import.meta.glob files matched", g.files);
  }
  const w = stubWorkerUrls(code, posix(relative(o.wsRoot, file)));
  if (w) {
    code = w.code;
    changed = true;
    add("new Worker(new URL(…)) stubbed (no Workers yet)", w.count);
  }
  return changed ? code : null;
}

/** Bun plugin for the import shapes above. Register it BEFORE the SFC plugin. */
export function viteShapesPlugin(o: ShapesOptions) {
  const idOf = (abs: string) => posix(relative(o.wsRoot, abs));
  const urlModule = (abs: string) => {
    const { key, url } = assetKey(abs, o.docRoot);
    o.assets.set(key, realpathSync(abs));
    return `export default ${J(url)};\n`;
  };
  return {
    name: "vn-vite-shapes",
    setup(build: any) {
      // A `?query` specifier: the query stays in the path of our own namespace, since Bun keys modules
      // by path (`a.svg?raw` and `a.svg` would collapse) and pluginData does not reach onLoad in Bun 1.4.
      build.onResolve({ filter: /\?[\w&=.-]+$/ }, (args: any) => {
        const q = args.path.lastIndexOf("?");
        const bare = args.path.slice(0, q);
        const params = new Set(args.path.slice(q + 1).split("&").map((p: string) => p.split("=")[0]));
        if (![...params].some((p) => KINDS.includes(p as string))) return undefined;
        const real = resolveImport(bare, importerDir(args, o.docRoot));
        if (real === null) return undefined; // let Bun report the unresolvable import itself
        o.inlined?.(real);
        return { path: `${real}?${args.path.slice(q + 1)}`, namespace: NS };
      });
      build.onLoad({ filter: /.*/, namespace: NS }, async (args: any) => {
        const q = args.path.lastIndexOf("?");
        const file = args.path.slice(0, q);
        const params = new Set<string>(args.path.slice(q + 1).split("&").map((p: string) => p.split("=")[0]));
        const id = idOf(file);
        const isWasm = /\.wasm$/i.test(file);
        if (params.has("worker") || params.has("sharedworker")) {
          note(o, `?worker stubbed (no Workers yet)`);
          return { contents: workerStubModule(id, params.has("url")), loader: "js" };
        }
        if (isWasm) {
          note(o, `.wasm stubbed (WebAssembly out of scope)`);
          return { contents: wasmStubModule(id, params), loader: "js" };
        }
        if (params.has("raw")) {
          note(o, `?raw`);
          return { contents: `export default ${J(await Bun.file(file).text())};\n`, loader: "js" };
        }
        if (params.has("component") && /\.svg$/i.test(file)) {
          note(o, `.svg as a component`);
          return o.compileSfc(file + ".vue", svgAsSfc(await Bun.file(file).text()));
        }
        if (params.has("inline")) {
          note(o, `?inline`);
          if (/\.css$/i.test(file)) return { contents: `export default ${J(await Bun.file(file).text())};\n`, loader: "js" };
          const bytes = Buffer.from(await Bun.file(file).arrayBuffer());
          const mime = Bun.file(file).type.split(";")[0] || "application/octet-stream";
          return { contents: `export default ${J(`data:${mime};base64,${bytes.toString("base64")}`)};\n`, loader: "js" };
        }
        note(o, `?url (packed as an asset)`);
        return { contents: urlModule(file), loader: "js" };
      });
      build.onLoad({ filter: /\.svg$/i }, async (args: any) => {
        if (o.svgDefault === "component") {
          note(o, `.svg as a component`);
          return o.compileSfc(args.path + ".vue", svgAsSfc(await Bun.file(args.path).text()));
        }
        note(o, `asset URL (packed as an asset)`);
        return { contents: urlModule(args.path), loader: "js" };
      });
      // Any other asset: its URL (Bun's file loader would emit a hashed copy nothing packs).
      build.onLoad({ filter: ASSET_EXT }, (args: any) => {
        note(o, `asset URL (packed as an asset)`);
        return { contents: urlModule(args.path), loader: "js" };
      });
      build.onLoad({ filter: /\.wasm$/i }, (args: any) => {
        note(o, `.wasm stubbed (WebAssembly out of scope)`);
        return { contents: wasmStubModule(idOf(args.path), new Set()), loader: "js" };
      });
      build.onLoad({ filter: /\.[cm]?[jt]sx?$/ }, async (args: any) => {
        const src = await Bun.file(args.path).text();
        const code = src.includes("import.meta.glob") || src.includes("new URL") ? transformSource(src, args.path, o) : null;
        o.sourceLoaded?.(args.path, src, code ?? src);
        return code === null ? undefined : { contents: code, loader: loaderOf(args.path) };
      });
    },
  };
}

export const LEAF_IMPORT = new RegExp(`\\.(?:json|svg|wasm)$|${ASSET_EXT.source}`, "i");

export interface DevImportsOptions {
  srcDir: string;
  preseeded: (spec: string) => boolean;
  inlined?: (absPath: string) => void;
  /** Registry id of the npm module a bare import outside the source root loads; undefined leaves the specifier as written. */
  npmId?: (spec: string, absPath: string, fromDir: string) => string | undefined;
}

/**
 * The dev builder's resolution: each file is built with every import external, so this does what
 * prod's bundle does. A tsconfig `paths` alias into the source root becomes the importer-relative
 * path (one graph edge, one instance); data and asset files are built into the importer (register
 * `viteShapesPlugin` first); an npm import gets the id `npmId` gives it.
 */
export function devImportsPlugin(o: DevImportsOptions) {
  const inSrc = (abs: string) => {
    const rel = relative(o.srcDir, abs);
    return !!rel && !rel.startsWith("..") && !isAbsolute(rel);
  };
  return {
    name: "vn-dev-imports",
    setup(build: any) {
      build.onResolve({ filter: LEAF_IMPORT }, (args: any) => {
        if (args.kind === "entry-point-build") return undefined;
        const abs = resolveImport(args.path, importerDir(args, o.srcDir));
        if (abs === null || !isAbsolute(abs)) return undefined; // external: build.ts reports the miss
        o.inlined?.(abs);
        return { path: abs };
      });
      // Bare specifiers only: an absolute path is the entry point (Windows `C:\…` starts with a letter).
      // `node:` stays out of the filter: a plugin that sees Bun's own `node:module` import makes it a real require.
      build.onResolve({ filter: /^(?!node:)[^./\\]/ }, (args: any) => {
        if (args.kind === "entry-point-build" || isAbsolute(args.path) || o.preseeded(args.path)) return undefined;
        const from = importerDir(args, o.srcDir);
        const abs = resolveImport(args.path, from);
        if (abs === null || !isAbsolute(abs)) return undefined;
        if (!inSrc(abs)) {
          if (/\.css$/i.test(abs)) return undefined;
          const id = o.npmId?.(args.path, abs, from);
          return id === undefined ? undefined : { path: id, external: true };
        }
        const rel = posix(relative(from, abs));
        return { path: rel.startsWith("../") ? rel : `./${rel}`, external: true };
      });
    },
  };
}

export const GLOB_TRIPWIRE = "__vn_import_meta_glob_unexpanded";

// Bun builds each `import * as ns` / `import()` namespace as a plain `{}` without `__esModule` or
// toStringTag "Module", so Vue's defineAsyncComponent mounts the namespace itself. Mark them as modules.

const BUN_EXPORT_HELPER =
  /(\(\s*([\w$]+)\s*,\s*([\w$]+)\s*\)\s*=>\s*\{)(\s*for\s*\(\s*var\s+([\w$]+)\s+in\s+\3\s*\)\s*[\w$]+\(\s*\2\s*,\s*\5\s*,\s*\{\s*get\s*:\s*\3\[\5\]\s*,\s*enumerable\s*:\s*(?:!0|true)\s*,\s*configurable\s*:\s*(?:!0|true))/g;
const BUN_EXPORT_SIGNATURE =
  /get\s*:\s*([\w$]+)\[([\w$]+)\]\s*,\s*enumerable\s*:\s*(?:!0|true)\s*,\s*configurable\s*:\s*(?:!0|true)\s*,\s*set\s*:\s*[\w$]+\.bind\(\s*\1\s*,\s*\2\s*\)/;

/** Makes every namespace object a bundle builds look like a module namespace. Throws on an unrecognised `__export` helper shape, so a Bun upgrade fails the build. */
export function tagModuleNamespaces(bundle: string): { code: string; count: number } {
  let count = 0;
  const code = bundle.replace(BUN_EXPORT_HELPER, (_m, head: string, target: string, _all: string, loop: string) => {
    count++;
    return (
      `${head}Object.setPrototypeOf(${target},null),` +
      `Object.defineProperty(${target},Symbol.toStringTag,{value:"Module"});${loop}`
    );
  });
  if (count === 0 && BUN_EXPORT_SIGNATURE.test(bundle)) {
    throw new Error(
      "the bundle's __export helper is not in the shape vitecompat.ts tagModuleNamespaces knows: " +
        "namespace objects would lack Symbol.toStringTag \"Module\", and defineAsyncComponent(() => import(…)) " +
        "would render nothing — update BUN_EXPORT_HELPER for this Bun version",
    );
  }
  return { code, count };
}

const DYNAMIC_IMPORT = /(?<![\w$.])import\(\s*("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')\s*\)/g;
const DYNAMIC_IMPORT_ANY = /(?<![\w$.])import\(\s*(?!["'])/g;
const NODE_MODE_TO_ESM = /\b__toESM\((require\(\s*(?:"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')\s*\))\s*,\s*1\s*\)/g;

/**
 * The dev builder's per-module CJS, corrected to bundler ESM semantics: `__toESM(require(x), 1)`
 * makes `default` the whole `module.exports`, so the `1` is dropped; `import("./X.vue")` becomes a
 * promise of the exports through the registry `require`. Returns the body and the count of
 * `import(...)` with a non-literal specifier (these still reject at runtime).
 */
export function devEsmInterop(body: string): { code: string; unresolved: number } {
  let code = body.replace(NODE_MODE_TO_ESM, "__toESM($1)");
  let dynamic = false;
  code = code.replace(DYNAMIC_IMPORT, (_m, spec: string) => {
    dynamic = true;
    return `Promise.resolve().then(() => __vnImportNs(require(${spec})))`;
  });
  if (dynamic) {
    code +=
      "\nfunction __vnImportNs(m) {\n" +
      '  return m && (m.__esModule || m[Symbol.toStringTag] === "Module") ? m : ' +
      'Object.defineProperty(Object.assign(Object.create(null), m, { default: m }), Symbol.toStringTag, { value: "Module" });\n' +
      "}\n";
  }
  const unresolved = (code.match(DYNAMIC_IMPORT_ANY) ?? []).length;
  return { code, unresolved };
}

const IMPORT_RULE = /@import\s+(?:url\(\s*(['"]?)([^'")]+)\1\s*\)|(['"])([^'"]+)\3)\s*([^;]*);/g;

const CSS_URL = /\/\*[\s\S]*?\*\/|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|\burl\(\s*(['"]?)([^'")]*)\1\s*\)/gi;

/**
 * Rewrites a sheet's relative `url()`s as Vite does: resolved against the sheet's file and made
 * root-relative. Absolute, rooted, `data:`, `#fragment`, aliased URLs and `@import url(...)` are left alone.
 */
export function rebaseCssUrls(css: string, from: string, docRoot: string, assets?: Map<string, string>): string {
  if (!/url\(/i.test(css)) return css;
  const dir = dirname(from.replace(/\?.*$/, ""));
  return css.replace(CSS_URL, (whole, q: string, url: string, at: number, all: string) => {
    if (url === undefined) return whole;
    const u = url.trim();
    if (!u || u.includes("(") || u.startsWith("//") || /^([a-z][a-z0-9+.-]*:|[#~@]|%23)/i.test(u)) return whole;
    if (/@import\s*$/i.test(all.slice(Math.max(0, at - 16), at))) return whole;
    const [path, tail] = (() => {
      const i = u.search(/[?#]/);
      return i < 0 ? [u, ""] : [u.slice(0, i), u.slice(i)];
    })();
    let decoded: string;
    try { decoded = decodeURIComponent(path); } catch { return whole; }
    const abs = u.startsWith("/") ? resolve(docRoot, "." + decoded) : resolve(dir, decoded);
    if (assets && existsSync(abs) && statSync(abs).isFile()) {
      const { key, url: packedUrl } = assetKey(abs, docRoot);
      assets.set(key, abs);
      return `url(${q || '"'}${packedUrl}${tail}${q || '"'})`;
    }
    if (u.startsWith("/")) return whole;
    const rel = relative(resolve(docRoot), abs);
    const out =
      rel && !rel.startsWith("..") && !isAbsolute(rel)
        ? `/${rel.split(sep).join("/")}`
        : `file:///${abs.split(sep).join("/").replace(/^\/+/, "")}`;
    return `url(${q || '"'}${out}${tail}${q || '"'})`;
  });
}

/** Inlines every local `@import` in `css` recursively, as postcss-import does (vn-css would drop it as an unknown at-rule). */
export function inlineCssImports(
  css: string,
  from: string,
  docRoot: string,
  warn: (m: string) => void,
  seen: Set<string> = new Set([resolve(from)]),
  assets?: Map<string, string>,
): string {
  css = rebaseCssUrls(css, from, docRoot, assets);
  if (!css.includes("@import")) return css;
  return css.replace(IMPORT_RULE, (whole, _q1, urlSpec: string | undefined, _q2, strSpec: string | undefined, media: string) => {
    const spec = (urlSpec ?? strSpec ?? "").trim();
    if (/^(https?:)?\/\//i.test(spec) || spec.startsWith("data:")) return whole;
    let target: string | null = null;
    const clean = spec.replace(/^~/, "").replace(/[?#].*$/, "");
    try {
      if (clean.startsWith("/")) target = join(docRoot, clean);
      else if (clean.startsWith(".")) target = resolve(dirname(from), clean);
      else target = resolveImport(clean, dirname(from));
    } catch {
      target = null;
    }
    if (!target || !existsSync(target)) {
      warn(`${from}: @import ${J(spec)} does not resolve to a file — dropped`);
      return "";
    }
    const abs = resolve(target);
    if (seen.has(abs)) return ""; // a cycle, or a second import of the same sheet: postcss-import skips it too
    seen.add(abs);
    const inner = inlineCssImports(readFileSync(abs, "utf8"), abs, docRoot, warn, seen, assets);
    const m = media.trim();
    return m && !/^(layer|supports)\b/.test(m) ? `@media ${m} {\n${inner}\n}` : inner;
  });
}

export interface CssPiece {
  text: string;
  from: string;
}
export interface LiftedHtml {
  /** `<style>` and local `<link rel=stylesheet>` in `<head>`, document order, AHEAD of the bundle's CSS. */
  headCss: CssPiece[];
  /** The same in `<body>`: AFTER it. */
  bodyCss: CssPiece[];
  /** Inline classic `<script>`s, document order, run before the entry module. */
  scripts: { text: string; label: string }[];
  rootClasses: string[];
  rootAttrs: Record<string, string>;
  mountElement?: { tag: string; attrs: Record<string, string> };
  svgDefinitions?: string[];
  advisories: string[];
}

function attrsOf(tag: string): Record<string, string> {
  const out: Record<string, string> = {};
  const re = /([^\s=<>"'/]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g;
  const inner = tag.replace(/^<\s*[\w-]+/, "").replace(/\/?>$/, "");
  for (let m = re.exec(inner); m; m = re.exec(inner)) out[m[1]!.toLowerCase()] = m[2] ?? m[3] ?? m[4] ?? "";
  return out;
}

/** What a Vite app's `index.html` contributes; the rest is named in `advisories`. */
export function liftIndexHtml(htmlPath: string, docRoot: string): LiftedHtml {
  const raw = readFileSync(htmlPath, "utf8");
  const html = raw.replace(/<!--[\s\S]*?-->/g, (c) => " ".repeat(c.length));
  const out: LiftedHtml = { headCss: [], bodyCss: [], scripts: [], rootClasses: [], rootAttrs: {}, advisories: [] };
  const bodyAt = (() => {
    const m = /<body\b[^>]*>/i.exec(html);
    return m ? m.index : html.search(/<\/head>/i);
  })();
  const inBody = (i: number) => bodyAt >= 0 && i >= bodyAt;
  const label = (i: number) => `index.html:${raw.slice(0, i).split("\n").length}`;

  const htmlTag = /<html\b[^>]*>/i.exec(html);
  if (htmlTag) {
    const a = attrsOf(htmlTag[0]);
    if (a.class) out.rootClasses.push(...a.class.split(/\s+/).filter(Boolean));
  }
  const bodyTag = /<body\b[^>]*>/i.exec(html);
  if (bodyTag) {
    const a = attrsOf(bodyTag[0]);
    if (a.class) out.rootClasses.push(...a.class.split(/\s+/).filter(Boolean));
    const other = Object.keys(a).filter((k) => k !== "class");
    if (other.length) out.advisories.push(`<body ${other.join(" ")}>: only "class" is carried to the root`);
  }

  const re = /<style\b([^>]*)>([\s\S]*?)<\/style\s*>|<link\b[^>]*>|<script\b([^>]*)>([\s\S]*?)<\/script\s*>/gi;
  for (let m = re.exec(html); m; m = re.exec(html)) {
    const at = m.index;
    const sink = inBody(at) ? out.bodyCss : out.headCss;
    if (m[0].toLowerCase().startsWith("<style")) {
      const a = attrsOf(`<style${m[1]}>`);
      const text = raw.slice(at, at + m[0].length).replace(/^<style\b[^>]*>/i, "").replace(/<\/style\s*>$/i, "");
      sink.push({ text: a.media ? `@media ${a.media} {\n${text}\n}` : text, from: `${htmlPath}?style@${label(at)}` });
    } else if (m[0].toLowerCase().startsWith("<link")) {
      const a = attrsOf(m[0]);
      const rel = (a.rel ?? "").toLowerCase().split(/\s+/);
      if (!rel.includes("stylesheet")) continue; // icons, manifest, preconnect: nothing to lift
      const href = a.href ?? "";
      if (/^(https?:)?\/\//i.test(href)) {
        out.advisories.push(`${label(at)}: remote stylesheet ${href} is not loaded — vue-native has no network stylesheets`);
        continue;
      }
      const clean = href.replace(/[?#].*$/, "");
      const abs = clean.startsWith("/") ? join(docRoot, clean) : resolve(dirname(htmlPath), clean);
      if (!existsSync(abs)) {
        out.advisories.push(`${label(at)}: <link rel=stylesheet href=${J(href)}> names no file — skipped`);
        continue;
      }
      const text = readFileSync(abs, "utf8");
      sink.push({ text: a.media ? `@media ${a.media} {\n${text}\n}` : text, from: abs });
    } else {
      const a = attrsOf(`<script${m[3]}>`);
      const type = (a.type ?? "").toLowerCase();
      if (a.src !== undefined) {
        if (type !== "module") out.advisories.push(`${label(at)}: <script src=${J(a.src)}> is not run — only the entry module is`);
        continue; // the module entry: vue-native already knows its entry
      }
      const text = raw.slice(at, at + m[0].length).replace(/^<script\b[^>]*>/i, "").replace(/<\/script\s*>$/i, "");
      if (!text.trim()) continue;
      if (type === "" || type === "text/javascript" || type === "application/javascript") {
        out.scripts.push({ text, label: label(at) });
      } else if (type === "module") {
        out.advisories.push(`${label(at)}: an inline <script type="module"> is not run — move it into the entry module`);
      }
    }
  }

  const appTag = /<([\w-]+)\b[^>]*\bid\s*=\s*["']app["'][^>]*>/i.exec(html);
  if (appTag) out.mountElement = { tag: appTag[1]!.toLowerCase(), attrs: attrsOf(appTag[0]) };
  out.svgDefinitions = [...html.matchAll(/<svg\b[^>]*>[\s\S]*?<\/svg\s*>/gi)]
    .filter((match) => inBody(match.index ?? 0) && /<(?:defs|filter)\b/i.test(match[0]))
    .map((match) => raw.slice(match.index!, match.index! + match[0].length));
  if (bodyTag) {
    const body = html.slice(bodyTag.index + bodyTag[0].length).replace(re, "")
      .replace(/<svg\b[^>]*>[\s\S]*?<\/svg\s*>/gi, (svg) => /<(?:defs|filter)\b/i.test(svg) ? "" : svg);
    const tags = new Set<string>();
    for (const t of body.matchAll(/<([a-zA-Z][\w-]*)\b([^>]*)>/g)) {
      if (/\bid\s*=\s*["']app["']/i.test(t[2] ?? "")) continue;
      tags.add(t[1]!.toLowerCase());
    }
    if (tags.size) {
      const named = [...tags].slice(0, 6).map((t) => `<${t}>`);
      out.advisories.push(
        `<body> markup is not rendered — the window is the root (${named.join(", ")}${tags.size > 6 ? ", …" : ""})`,
      );
    }
  }
  return out;
}

type StaticSvgNode = NonNullable<SFCTemplateBlock["ast"]>["children"][number];

/** Static SVG definitions become real DOM nodes: the runtime has no innerHTML parser. v-pre makes Vue syntax literal. */
function staticSvgDom(svg: string): string {
  const { descriptor, errors } = parseSfc(`<template><div v-pre>${svg}</div></template>`, {
    templateParseOptions: { whitespace: "preserve", comments: true },
  });
  if (errors.length) {
    throw new Error(`index.html: SVG definitions did not parse: ${errors.map(String).join("; ")}`);
  }
  const wrapper = descriptor.template?.ast?.children.find((node) => node.type === 1);
  if (!wrapper || wrapper.type !== 1) throw new Error("index.html: SVG definitions have no root");
  const namespaces = ["http://www.w3.org/1999/xhtml", "http://www.w3.org/2000/svg", "http://www.w3.org/1998/Math/MathML"];
  const lines = ["{"];
  let next = 0;
  const append = (node: StaticSvgNode, parent: string): void => {
    if (node.type === 1) {
      const name = `svg${next++}`;
      lines.push(`const ${name} = document.createElementNS(${J(namespaces[node.ns])}, ${J(node.tag)});`);
      for (const prop of node.props) {
        if (prop.type !== 6) continue;
        const key = prop.name;
        const value = prop.value?.content ?? "";
        const namespace = key === "xmlns" || key.startsWith("xmlns:") ? "http://www.w3.org/2000/xmlns/"
          : key.startsWith("xlink:") ? "http://www.w3.org/1999/xlink"
          : key.startsWith("xml:") ? "http://www.w3.org/XML/1998/namespace" : null;
        lines.push(namespace
          ? `${name}.setAttributeNS(${J(namespace)}, ${J(key)}, ${J(value)});`
          : `${name}.setAttribute(${J(key)}, ${J(value)});`);
      }
      for (const child of node.children) append(child, name);
      lines.push(`${parent}.appendChild(${name});`);
    } else if (node.type === 2) {
      lines.push(`${parent}.appendChild(document.createTextNode(${J(node.content)}));`);
    }
  };
  for (const child of wrapper.children) append(child, "document.body");
  lines.push("}");
  return lines.join("\n");
}

export function htmlPreludeModule(lifted: LiftedHtml): string {
  const lines: string[] = [
    // The runtime first: it installs `document.documentElement`, which buffers classes until the window root exists.
    `import "vue-native";`,
  ];
  if (lifted.rootClasses.length) lines.push(`document.documentElement.classList.add(...${J(lifted.rootClasses)});`);
  for (const [k, v] of Object.entries(lifted.rootAttrs)) {
    lines.push(`document.documentElement.setAttribute(${J(k)}, ${J(v)});`);
  }
  if (lifted.mountElement) {
    const mount = lifted.mountElement;
    lines.push(`{ const mount = document.getElementById("app") || document.createElement(${J(mount.tag)});`);
    for (const [k, v] of Object.entries(mount.attrs)) lines.push(`mount.setAttribute(${J(k)}, ${J(v)});`);
    lines.push(`if (!mount.parentNode) document.body.appendChild(mount); }`);
  }
  for (const svg of lifted.svgDefinitions ?? []) lines.push(staticSvgDom(svg));
  for (const s of lifted.scripts) {
    // Global scope, as a classic <script> has (indirect eval); a throw is reported, the next script still runs.
    lines.push(
      `try { (0, eval)(${J(s.text)}); } catch (e) { console.error(${J(`[index.html] ${s.label}: `)} + ((e && e.stack) || e)); }`,
    );
  }
  return lines.join("\n") + "\n";
}

const major = (v: string | undefined) => {
  const m = /(\d+)/.exec(v ?? "");
  return m ? Number(m[1]) : NaN;
};

function versionAt(pkgJson: string): string | undefined {
  try {
    return JSON.parse(readFileSync(pkgJson, "utf8")).version;
  } catch {
    return undefined;
  }
}

function pkgJsonOf(entry: string, name: string): string | undefined {
  let dir = dirname(entry);
  for (;;) {
    const candidate = join(dir, "package.json");
    if (existsSync(candidate)) {
      try {
        if (JSON.parse(readFileSync(candidate, "utf8")).name === name) return candidate;
      } catch {}
    }
    const up = dirname(dir);
    if (up === dir) return undefined;
    dir = up;
  }
}

export interface FrameworkDep {
  path: string;
  from: "app" | "vue-native";
  version?: string;
  warning?: string;
}

/** Which copy of `name` (pinia / vue-router) the builders use: the app's own when declared and installed, else vue-native's. */
export function frameworkDep(
  name: string,
  appPkg: Record<string, unknown> | null,
  appResolve: (spec: string) => string,
  runtimeResolve: (spec: string) => string,
): FrameworkDep {
  const runtimePath = runtimeResolve(name);
  const runtimeVersion = versionAt(pkgJsonOf(runtimePath, name) ?? "");
  let declared: string | undefined;
  for (const k of ["dependencies", "devDependencies", "peerDependencies"]) {
    const deps = appPkg?.[k] as Record<string, string> | undefined;
    if (deps && typeof deps[name] === "string") {
      declared = deps[name];
      break;
    }
  }
  if (declared === undefined) return { path: runtimePath, from: "vue-native", version: runtimeVersion };
  let appPath: string | undefined;
  try {
    appPath = appResolve(name);
  } catch {
    appPath = undefined;
  }
  if (!appPath) {
    const warning =
      major(declared) !== major(runtimeVersion)
        ? `the app declares ${name} ${declared} but it is not installed — using vue-native's ${name} ${runtimeVersion} (a different major)`
        : undefined;
    return { path: runtimePath, from: "vue-native", version: runtimeVersion, warning };
  }
  const appVersion = versionAt(pkgJsonOf(appPath, name) ?? "");
  return { path: appPath, from: "app", version: appVersion };
}

export function readPackageJson(dir: string): Record<string, unknown> | null {
  try {
    const v = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    return v && typeof v === "object" ? v : null;
  } catch {
    return null;
  }
}
