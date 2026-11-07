// Vite parity for vue-native's builders — the parts of a Vite app's build that
// live in Vite's own plugins rather than in the app, shared by prod.ts and
// build.ts so the two cannot drift on what an import means.
//
// A Vite app does not only import `.ts` and `.vue`. It writes
//
//   import svg from "./x.svg?raw"            the file's TEXT
//   import url from "./a.png"                the file's URL (any asset)
//   import url from "./w.js?url"             ditto, for a file that is not one
//   import Icon from "./icon.svg"            a COMPONENT (vite-svg-loader)
//   import W from "./w.ts?worker"            a Worker constructor
//   new Worker(new URL("./w.ts", import.meta.url))
//   import.meta.glob("./kinds/*.ts", { eager: true, import: "default" })
//
// and keeps global CSS and a theme script in `index.html`, static files in
// `public/`, and build constants in `define`. Every one of those is either a
// build error or — worse — a SILENT wrong answer under a bundler that does not
// know them (a `.svg` import became a path string; `import.meta.glob` built
// and then threw at eval). This module is what each of them means here:
//
//   ?raw            the text, exactly as Vite
//   ?url / assets   the URL of the file, which `vn build` packs into the
//                   payload under that key (`assets.json`, docs/PACKAGING.md)
//   .svg            a component when the app uses vite-svg-loader (as Vite
//                   with that plugin), a URL otherwise (as plain Vite)
//   import.meta.glob  expanded at load time: eager / lazy / `import` / `query`
//   ?worker, new Worker(new URL(...))
//                   a constructor that throws a named NotSupportedError, with
//                   an advisory — vue-native has no Workers yet
//                   (research/reports/m25/workers-design.md)
//   .wasm           a stub with an advisory: WebAssembly is out of scope by
//                   owner decision (research/reports/m26/client-gap.md)
//
// Nothing here is a vue-native invention: each shape answers what Vite answers,
// or fails with a name instead of silently.

import { existsSync, readFileSync, realpathSync, statSync } from "fs";
import { basename, dirname, extname, isAbsolute, join, relative, resolve, sep } from "path";

const J = (v: unknown) => JSON.stringify(v);
const posix = (p: string) => p.split(sep).join("/");

// ---------------------------------------------------------------------------
// SFC type resolution
// ---------------------------------------------------------------------------

/**
 * The `fs` compileScript resolves imported types through.
 *
 * `fileExists` is FILES ONLY. `existsSync` is also true for a directory, so
 * `import type { ToastProps } from "."` (a directory import that should land on
 * `./index.ts`) resolved to the directory itself and `defineProps<ToastProps>()`
 * failed with "Unresolvable type reference" (B-build-ui §1.1 #1a). TypeScript's
 * own `ts.sys.fileExists` — what plugin-vue ends up using — answers false for a
 * directory.
 */
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

// ---------------------------------------------------------------------------
// app defines: vn.config.json "define" + `--define K=V`
// ---------------------------------------------------------------------------

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

/**
 * Is `code` a value Bun's `define` inserts as CODE? Bun parses a define value
 * as JSON or as an identifier path and inserts anything else as a STRING
 * literal (see build.ts's DEFINES comment) — so `1 + 1` would silently become
 * `"1 + 1"`. Refusing it is the compat-honest answer.
 */
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

/**
 * Vite's `define`, from `vn.config.json` and the command line.
 *
 * Semantics are Vite's: a STRING value is an expression (`"\"v1\""` is the
 * string `v1`, `"true"` is the boolean), so a `vite.config.ts` block carries
 * over unchanged once its `JSON.stringify(...)` calls are evaluated. A JSON
 * file can also give a non-string value directly (`{ "version": "1" }`), which
 * is serialized — the same thing `JSON.stringify` in a vite config produces.
 * `--define K=V` (repeatable) wins over the file; V is an expression.
 */
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

/** `--define K=V` and `--define=K=V` out of an argv; returns the rest. */
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

// ---------------------------------------------------------------------------
// asset URLs
// ---------------------------------------------------------------------------

/**
 * The asset types Vite answers a default import of with a URL (its
 * KNOWN_ASSET_TYPES, minus `svg`, which is decided per app, and `wasm`, which
 * is out of scope here).
 */
export const ASSET_EXT =
  /\.(apng|bmp|png|jpe?g|jfif|pjpeg|pjp|gif|ico|webp|avif|cur|jxl|mp4|webm|ogg|mp3|wav|flac|aac|opus|mov|m4a|vtt|woff2?|eot|ttf|otf|webmanifest|pdf|txt)$/i;

/**
 * The payload key and URL of a file an import refers to.
 *
 * Inside the document root (where `vn.config.json` / `index.html` live) the key
 * is the root-relative path — what Vite's dev server serves the file under, and
 * what vue-native's dev host already resolves a URL to on disk, so the same URL
 * works in a dev host and in the packaged exe. A file outside it (a hoisted
 * `node_modules`, another drive) gets a content-addressed key under `_vn/fs/`,
 * which only the payload carries.
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

// ---------------------------------------------------------------------------
// import.meta.glob
// ---------------------------------------------------------------------------

/** A literal, parsed from source: what `import.meta.glob`'s arguments may be. */
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

/** Index just past the `)` that closes the `(` at `open`, skipping strings and comments. */
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
  /** The importing file. */
  file: string;
  /** Where a `/`-rooted pattern starts: the app's document root. */
  docRoot: string;
  /**
   * The dev builder's form: each matched file is imported by a path relative
   * to the importer (`./kinds/a.ts`), the shape its module registry resolves.
   * Prod bundles, and imports by absolute path.
   */
  relativeSpecs?: boolean;
}

/** Splits `../dir/sub/*.ts` into the static directory (`../dir/sub`) and the glob (`*.ts`). */
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
 * Expands every `import.meta.glob(...)` in `src` into what Vite generates: an
 * object literal keyed by path, eager entries as hoisted static imports, lazy
 * ones as `() => import(...)`. Returns `null` when there is nothing to expand.
 *
 * Keys follow Vite: a relative pattern keeps the importer-relative spelling
 * (`./kinds/a.ts`, `../effects/b.ts`), a root pattern the root-relative one
 * (`/src/x.vue`); `base` rebases them. The importer never matches itself, and
 * `node_modules` is skipped unless the pattern names it or `exhaustive` is set.
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
    // `import.meta.glob<T>(…)` — a TS type argument; balanced angle brackets.
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
      // A `query` entry is a `?raw` / `?url` import, which the dev builder's
      // plugins build into the importer as prod's do (viteShapesPlugin).
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
  // Static imports may come first in a module whatever precedes them; a
  // shebang is the one thing that has to stay on line 1.
  const bang = out.startsWith("#!") ? out.indexOf("\n") + 1 : 0;
  const code = out.slice(0, bang) + header.join("\n") + (header.length ? "\n" : "") + out.slice(bang);
  return { code, count: call, files };
}

// ---------------------------------------------------------------------------
// Workers and WebAssembly: named failures, not silent ones
// ---------------------------------------------------------------------------

/** The advisory prefix runtime.ts's `warnCompat` uses — one grep finds both. */
const ADVISE = "[vue-native compat]";

/**
 * A JS EXPRESSION that logs one advisory and evaluates to a `NotSupportedError`
 * naming `what` — a DOMException where the engine has one (redjs does), an
 * Error with that name otherwise.
 */
function notSupportedError(what: string, why: string): string {
  const msg = `${what}: ${why}`;
  return (
    `(() => { const m = ${J(msg)}; console.warn(${J(ADVISE + " ")} + m); ` +
    `return typeof DOMException === "function" ? new DOMException(m, "NotSupportedError") ` +
    `: Object.assign(new Error(m), { name: "NotSupportedError" }); })()`
  );
}

const WORKERS_WHY =
  "vue-native has no Web Workers yet (research/reports/m25/workers-design.md) — run the work on the main thread for now";

/** The module a `?worker` / `?sharedworker` import becomes. */
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

/**
 * `new Worker(new URL("./w.ts", import.meta.url), …)` — Vite's other worker
 * idiom, which it bundles as a worker entry. Rewritten to construct a class
 * that throws the same named error as `?worker`, so the failure names the
 * file instead of being `Worker is not defined` from deep in a call stack.
 */
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
  "WebAssembly is out of scope for vue-native (owner decision, research/reports/m26/client-gap.md); " +
  "the native replacement is a host binding";

/** The module a `.wasm` import becomes: a URL that no fetch can satisfy, or an `init` that rejects. */
export function wasmStubModule(id: string, query: Set<string>): string {
  const advise = `console.warn(${J(`${ADVISE} ${id}: not bundled — ${WASM_WHY}`)});\n`;
  if (query.has("url")) return advise + `export default ${J(`vn-unsupported:wasm/${id}`)};\n`;
  return advise + `export default function init() { return Promise.reject(${notSupportedError(id, WASM_WHY)}); }\n`;
}

// ---------------------------------------------------------------------------
// SVG as a component (vite-svg-loader)
// ---------------------------------------------------------------------------

/** Does the app use vite-svg-loader, i.e. is a bare `.svg` import a component? */
export function svgDefaultOf(appPkg: Record<string, unknown> | null): "component" | "url" {
  if (!appPkg) return "url";
  for (const k of ["dependencies", "devDependencies"]) {
    const deps = appPkg[k];
    if (deps && typeof deps === "object" && "vite-svg-loader" in (deps as object)) return "component";
  }
  return "url";
}

/** The SFC source an `.svg` becomes: its markup as a template, minus what a template cannot hold. */
export function svgAsSfc(svg: string): string {
  const body = svg
    .replace(/<\?xml[\s\S]*?\?>/g, "")
    .replace(/<!DOCTYPE[^>]*>/gi, "")
    .replace(/<!--[\s\S]*?-->/g, "")
    // vite-svg-loader's own rewrite: a `<style>` inside the SVG must survive
    // the template compiler, which would otherwise hoist it out.
    .replace(/<style/g, '<component is="style"')
    .replace(/<\/style>/g, "</component>")
    .trim();
  return `<template>${body}</template>`;
}

// ---------------------------------------------------------------------------
// the Bun plugin
// ---------------------------------------------------------------------------

export interface ShapesOptions {
  /** The document root: where `vn.config.json` / `index.html` live. URLs and `/` globs start here. */
  docRoot: string;
  /** Workspace root — module ids in messages are relative to it. */
  wsRoot: string;
  /** What a bare `.svg` import is (see `svgDefaultOf`). */
  svgDefault: "component" | "url";
  /** Compiles an SFC source to a module (the builder's own `compileVue`). */
  compileSfc: (
    absPath: string,
    source: string,
  ) => { contents: string; loader: "js" | "ts" } | Promise<{ contents: string; loader: "js" | "ts" }>;
  /** Filled: payload key -> absolute path of every asset an import referenced. */
  assets: Map<string, string>;
  /** Filled: one line per thing this plugin did that a build log should say. */
  notes: Map<string, number>;
  /**
   * The dev builder (build.ts), which builds one module per file and leaves the
   * imports to its module registry: `import.meta.glob` imports by
   * importer-relative path, and an asset URL the dev host cannot serve is
   * named in the notes.
   */
  dev?: boolean;
  /** Called with each file a `?query` import builds into its importer, so a watcher can rebuild the importer when it changes. */
  inlined?: (absPath: string) => void;
}

/**
 * Resolves an import specifier as the bundler does. This is Bun's resolver,
 * which reads the `compilerOptions.paths` of the importer's nearest
 * tsconfig.json (`"@/*": ["./src/*"]`), plus package `exports` and the browser
 * field. prod.ts's bundle resolves every import this way inside Bun.build.
 * build.ts calls it directly (`devImportsPlugin`), because an import its
 * per-module builds leave external is never resolved by the bundler.
 *
 * A vite.config `resolve.alias` is not read by either builder (docs/PACKAGING.md
 * §3.1).
 */
export function resolveImport(spec: string, fromDir: string): string | null {
  try {
    return Bun.resolveSync(spec, fromDir);
  } catch {
    return null;
  }
}

/** The directory an `onResolve` call resolves from. */
export const importerDir = (args: { resolveDir?: string; importer?: string }, fallback: string) =>
  args.resolveDir || (args.importer ? dirname(args.importer) : fallback);

const NS = "vn-vite";
const KINDS = ["raw", "url", "inline", "worker", "sharedworker", "component", "init"];

function note(o: ShapesOptions, what: string) {
  o.notes.set(what, (o.notes.get(what) ?? 0) + 1);
}

/** Loader for a source file by extension. */
export function loaderOf(path: string): "ts" | "tsx" | "js" | "jsx" {
  const ext = extname(path).toLowerCase();
  if (ext === ".tsx") return "tsx";
  if (ext === ".jsx") return "jsx";
  if (ext === ".ts" || ext === ".mts" || ext === ".cts") return "ts";
  return "js";
}

/**
 * The source-level rewrites: `import.meta.glob` and `new Worker(new URL(...))`.
 * Exported so the SFC path can run it over a component's compiled script.
 */
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

/** Bun plugin: the import shapes listed in this file's header. Register it BEFORE the SFC plugin. */
export function viteShapesPlugin(o: ShapesOptions) {
  const idOf = (abs: string) => posix(relative(o.wsRoot, abs));
  const urlModule = (abs: string) => {
    const { key, url } = assetKey(abs, o.docRoot);
    o.assets.set(key, abs);
    if (o.dev && key.startsWith("_vn/fs/")) {
      // The dev host reads `<app root>/<key>` from disk; a URL cannot climb
      // above the app root (docs/PACKAGING.md §4).
      note(o, `asset outside the app root, which the dev host does not serve (404 in dev; packed by vn build): ${idOf(abs)}`);
    }
    return `export default ${J(url)};\n`;
  };
  return {
    name: "vn-vite-shapes",
    setup(build: any) {
      // `x?raw`, `x?url`, `x?worker`, … — any specifier with a query naming a
      // kind. The query stays IN the path of a namespace of our own: Bun keys
      // modules by path, so `a.svg?raw` and `a.svg` would otherwise collapse
      // into one module, and `pluginData` does not reach onLoad in Bun 1.4.
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
      // A bare `.svg`: a component under vite-svg-loader, a URL otherwise.
      build.onLoad({ filter: /\.svg$/i }, async (args: any) => {
        if (o.svgDefault === "component") {
          note(o, `.svg as a component`);
          return o.compileSfc(args.path + ".vue", svgAsSfc(await Bun.file(args.path).text()));
        }
        note(o, `asset URL (packed as an asset)`);
        return { contents: urlModule(args.path), loader: "js" };
      });
      // Any other asset: its URL, as Vite answers. Bun's own `file` loader
      // would emit a hashed copy NEXT TO the bundle and return its relative
      // path, which nothing packs and nothing resolves.
      build.onLoad({ filter: ASSET_EXT }, (args: any) => {
        note(o, `asset URL (packed as an asset)`);
        return { contents: urlModule(args.path), loader: "js" };
      });
      build.onLoad({ filter: /\.wasm$/i }, (args: any) => {
        note(o, `.wasm stubbed (WebAssembly out of scope)`);
        return { contents: wasmStubModule(idOf(args.path), new Set()), loader: "js" };
      });
      // `import.meta.glob` and worker URLs in plain sources. Returning nothing
      // hands the file back to Bun's own loader untouched.
      build.onLoad({ filter: /\.[cm]?[jt]sx?$/ }, async (args: any) => {
        const src = await Bun.file(args.path).text();
        if (!src.includes("import.meta.glob") && !src.includes("new URL")) return undefined;
        const code = transformSource(src, args.path, o);
        return code === null ? undefined : { contents: code, loader: loaderOf(args.path) };
      });
    },
  };
}

/** A specifier naming a file an import builds INTO its importer: data, `.svg`, `.wasm` and Vite's asset types. */
export const LEAF_IMPORT = new RegExp(`\\.(?:json|svg|wasm)$|${ASSET_EXT.source}`, "i");

export interface DevImportsOptions {
  /** The directory whose files the dev builder emits as modules (the app's source root). */
  srcDir: string;
  /** Bare specifiers vendor.js provides (build.ts PRESEEDED): left as written. */
  preseeded: (spec: string) => boolean;
  /** Called with each data or asset file an import builds into its importer. */
  inlined?: (absPath: string) => void;
  /**
   * The registry id of the npm module a bare import outside the source root
   * loads (build.ts `npmIdOf`): one per package entry, keyed on where it
   * resolves. Undefined leaves the specifier as written.
   */
  npmId?: (spec: string, absPath: string, fromDir: string) => string | undefined;
}

/**
 * The dev builder's resolution (build.ts `buildModule`). Each app file is built
 * on its own with every import external, so the bundler never resolves an
 * import, and the module registry is keyed by relative path. Two things need
 * the resolver prod's bundle uses (`resolveImport`):
 *
 * - An import that resolves into the app's source root under another name, a
 *   tsconfig `paths` alias (`@/store/x`), becomes the importer-relative path of
 *   that file. It is then a graph edge and loads the one module instance, as in
 *   prod and Vite. Left bare, it was built as an npm package: a second copy of
 *   the module, or a failed build.
 * - Data and asset files (`.json`, `.svg`, Vite's asset types, `.wasm`) are
 *   built into the importer, as prod's bundle builds them, through Bun's JSON
 *   loader and `viteShapesPlugin`'s loaders. Register that plugin first: its
 *   `?query` resolution must see a query import before this one does.
 * - An npm import gets the id `npmId` gives it, so every importer of one
 *   package entry loads one module.
 *
 * Anything else is left to the build's `external`.
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
      // Bare specifiers only. An absolute path is the entry point (Windows'
      // `C:\…` starts with a letter too) and must never be marked external.
      // `node:` stays out of the FILTER: a plugin that so much as sees Bun's
      // own `node:module` import makes it a real `require("node:module")`.
      build.onResolve({ filter: /^(?!node:)[^./\\]/ }, (args: any) => {
        if (args.kind === "entry-point-build" || isAbsolute(args.path) || o.preseeded(args.path)) return undefined;
        const from = importerDir(args, o.srcDir);
        const abs = resolveImport(args.path, from);
        if (abs === null || !isAbsolute(abs)) return undefined;
        if (!inSrc(abs)) {
          // A package. A stylesheet stays as written: build.ts keys it by specifier.
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

/** Identifier an unexpanded `import.meta.glob` is defined to, so a survivor is findable in the output. */
export const GLOB_TRIPWIRE = "__vn_import_meta_glob_unexpanded";

// ---------------------------------------------------------------------------
// Module namespace objects (m26 client sweep #2)
// ---------------------------------------------------------------------------
//
// A browser's (and Vite's) `import * as ns` and `await import("./X.vue")`
// give a MODULE NAMESPACE OBJECT: null prototype, `Symbol.toStringTag` =
// "Module", one getter per export. Vue's `defineAsyncComponent` unwraps
// `.default` only from something that says it is one:
//
//   if (comp && (comp.__esModule || comp[Symbol.toStringTag] === "Module")) comp = comp.default
//
// Bun's bundle builds each namespace as a plain `{}` that its `__export`
// helper fills with getters, with neither mark. So every
// `defineAsyncComponent(() => import(…))` mounted the namespace object itself,
// which renders nothing: each pane of the Argon client's Settings was blank.

/** Bun's `__export(target, all)`, minified or not: `(o,t)=>{for(var e in t)p(o,e,{get:t[e],enumerable:!0,configurable:!0,…`. */
const BUN_EXPORT_HELPER =
  /(\(\s*([\w$]+)\s*,\s*([\w$]+)\s*\)\s*=>\s*\{)(\s*for\s*\(\s*var\s+([\w$]+)\s+in\s+\3\s*\)\s*[\w$]+\(\s*\2\s*,\s*\5\s*,\s*\{\s*get\s*:\s*\3\[\5\]\s*,\s*enumerable\s*:\s*(?:!0|true)\s*,\s*configurable\s*:\s*(?:!0|true))/g;
/** The descriptor only that helper writes (`set: __exportSetter.bind(all, name)`): the tripwire for a new shape. */
const BUN_EXPORT_SIGNATURE =
  /get\s*:\s*([\w$]+)\[([\w$]+)\]\s*,\s*enumerable\s*:\s*(?:!0|true)\s*,\s*configurable\s*:\s*(?:!0|true)\s*,\s*set\s*:\s*[\w$]+\.bind\(\s*\1\s*,\s*\2\s*\)/;

/**
 * Makes every namespace object a bundle builds look like a module namespace:
 * Bun's `__export` helper now starts by giving its target a null prototype and
 * `Symbol.toStringTag` "Module". Throws when the bundle has the helper in a
 * shape this does not recognise, so a Bun upgrade fails the build instead of
 * blanking every lazy component again.
 */
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

/** `import("<literal>")` in a module body, but not `x.import(…)`. */
const DYNAMIC_IMPORT = /(?<![\w$.])import\(\s*("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')\s*\)/g;
/** An `import(` whose argument is not a string literal. */
const DYNAMIC_IMPORT_ANY = /(?<![\w$.])import\(\s*(?!["'])/g;
/** `__toESM(require("…"), 1)`: Bun's Node-mode interop. */
const NODE_MODE_TO_ESM = /\b__toESM\((require\(\s*(?:"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')\s*\))\s*,\s*1\s*\)/g;

/**
 * The dev builder's per-module CJS (build.ts `buildModule`), corrected to a
 * bundler's ESM semantics, which are what a Vite app is written against:
 *
 * - In a `"type": "module"` package Bun emits `__toESM(require(x), 1)`. The `1`
 *   is Node's rule for importing CommonJS: `default` is the whole
 *   `module.exports`, even when that object says `__esModule`. Every module
 *   here is ESM compiled to CJS, so `import App from "./App.vue"` got
 *   `{ __esModule, default }` instead of the component, and the app failed to
 *   mount. Without the `1`, `default` is the module's own default export, as in
 *   Vite.
 * - `import("./X.vue")` is left as a native dynamic import, which the host's
 *   classic scripts cannot run. It becomes a promise of the module's exports,
 *   loaded through the same registry `require` uses. A CJS module's exports are
 *   wrapped as Vite wraps them, with `default` = `module.exports`.
 *
 * Returns the rewritten body and the specifier-less `import(…)` count, which
 * the caller reports (those still reject at runtime).
 */
export function devEsmInterop(body: string): { code: string; unresolved: number } {
  let code = body.replace(NODE_MODE_TO_ESM, "__toESM($1)");
  let dynamic = false;
  code = code.replace(DYNAMIC_IMPORT, (_m, spec: string) => {
    dynamic = true;
    return `Promise.resolve().then(() => __vnImportNs(require(${spec})))`;
  });
  // A function declaration, appended: hoisted, and never ahead of a directive.
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

// ---------------------------------------------------------------------------
// CSS: `@import` inlining (postcss-import, the part Vite always applies)
// ---------------------------------------------------------------------------

/** `@import "x";`, `@import 'x' screen;`, `@import url(x);`, `@import url("x") print;` */
const IMPORT_RULE = /@import\s+(?:url\(\s*(['"]?)([^'")]+)\1\s*\)|(['"])([^'"]+)\3)\s*([^;]*);/g;

/** `url(x)`, `url('x')`, `url("x")` — group 2 is the URL. */
const CSS_URL = /\burl\(\s*(['"]?)([^'")]*)\1\s*\)/gi;

/**
 * A sheet's relative `url()`s, rewritten to say what they mean once the sheet
 * is one piece of the app's stylesheet, as Vite rewrites them: resolved
 * against the sheet's own file and made root-relative (`/packages/assets/fonts/
 * Lato/Lato-Bold.ttf`), which the host resolves against the app root as it
 * does an `<img src>`. Outside the root, a `file:` URL. Absolute, rooted,
 * `data:`, `#fragment` and aliased (`~`, `@`) URLs are left alone, and so are
 * `@import url(…)`s, which inlining resolves.
 *
 * Without this, `@font-face { src: url('./Lato/Lato-Bold.ttf') }` in
 * `packages/assets/fonts/fonts.css` named a file next to `index.html`.
 */
export function rebaseCssUrls(css: string, from: string, docRoot: string): string {
  if (!/url\(/i.test(css)) return css;
  const dir = dirname(from.replace(/\?.*$/, ""));
  return css.replace(CSS_URL, (whole, q: string, url: string, at: number, all: string) => {
    const u = url.trim();
    if (!u || u.includes("(") || /^([a-z][a-z0-9+.-]*:|[/#~@]|%23)/i.test(u)) return whole;
    // `@import url(x)` is inlining's to resolve (a package path included).
    if (/@import\s*$/i.test(all.slice(Math.max(0, at - 16), at))) return whole;
    const [path, tail] = (() => {
      const i = u.search(/[?#]/);
      return i < 0 ? [u, ""] : [u.slice(0, i), u.slice(i)];
    })();
    const abs = resolve(dir, path);
    const rel = relative(resolve(docRoot), abs);
    const out =
      rel && !rel.startsWith("..") && !isAbsolute(rel)
        ? `/${rel.split(sep).join("/")}`
        : `file:///${abs.split(sep).join("/").replace(/^\/+/, "")}`;
    return `url(${q || '"'}${out}${tail}${q || '"'})`;
  });
}

/**
 * Inlines every LOCAL `@import` in `css` (relative, root-relative or a package
 * path), recursively, the way postcss-import does inside Vite's CSS pipeline —
 * so an `@import "@vue-flow/core/dist/style.css"` in a component's `<style>`
 * reaches the stylesheet instead of being dropped by vn-css as an at-rule it
 * does not know. A media list wraps the imported rules in `@media`. Remote
 * imports are left in place (vn-css drops them with its own advisory).
 *
 * Every sheet's relative `url()`s are rebased on the way ([`rebaseCssUrls`]),
 * each against its own file.
 */
export function inlineCssImports(
  css: string,
  from: string,
  docRoot: string,
  warn: (m: string) => void,
  seen: Set<string> = new Set([resolve(from)]),
): string {
  css = rebaseCssUrls(css, from, docRoot);
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
    const inner = inlineCssImports(readFileSync(abs, "utf8"), abs, docRoot, warn, seen);
    const m = media.trim();
    return m && !/^(layer|supports)\b/.test(m) ? `@media ${m} {\n${inner}\n}` : inner;
  });
}

// ---------------------------------------------------------------------------
// index.html
// ---------------------------------------------------------------------------

export interface CssPiece {
  text: string;
  from: string;
}
export interface LiftedHtml {
  /** `<style>` and local `<link rel=stylesheet>` in `<head>`, document order: AHEAD of the bundle's CSS. */
  headCss: CssPiece[];
  /** …and in `<body>`: AFTER it, where Vite's injected `<link>` (end of head) leaves them. */
  bodyCss: CssPiece[];
  /** Inline classic `<script>`s, document order: they run before the entry module, as in a browser. */
  scripts: { text: string; label: string }[];
  /** Attributes for the document root (`<html class>` and `<body class>`). */
  rootClasses: string[];
  rootAttrs: Record<string, string>;
  /** The app container stays a child of body, outside body-level portals. */
  mountElement?: { tag: string; attrs: Record<string, string> };
  advisories: string[];
}

function attrsOf(tag: string): Record<string, string> {
  const out: Record<string, string> = {};
  const re = /([^\s=<>"'/]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g;
  const inner = tag.replace(/^<\s*[\w-]+/, "").replace(/\/?>$/, "");
  for (let m = re.exec(inner); m; m = re.exec(inner)) out[m[1]!.toLowerCase()] = m[2] ?? m[3] ?? m[4] ?? "";
  return out;
}

/**
 * What a Vite app's `index.html` contributes, lifted out of a document the
 * window does not have: inline `<style>`, local `<link rel=stylesheet>`, inline
 * classic `<script>` (the theme script every app has), the root's classes and
 * the mount element's attributes. Everything else is named in `advisories`.
 */
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
      // json / importmap / templates: data, not code — nothing to run.
    }
  }

  // Preserve the app container: libraries transform it while their portals stay under body.
  const appTag = /<([\w-]+)\b[^>]*\bid\s*=\s*["']app["'][^>]*>/i.exec(html);
  if (appTag) out.mountElement = { tag: appTag[1]!.toLowerCase(), attrs: attrsOf(appTag[0]) };
  // Body markup that is neither the mount element nor a script/style: named,
  // because it is simply not there (a hidden SVG `<filter>` set, a splash).
  if (bodyTag) {
    const body = html.slice(bodyTag.index + bodyTag[0].length).replace(re, "");
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

/** The module the lifted scripts and root attributes run in, ahead of the entry. */
export function htmlPreludeModule(lifted: LiftedHtml): string {
  const lines: string[] = [
    // The runtime first: it is what installs `document.documentElement` (the
    // pre-mount root that buffers classes until the window root exists).
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
  for (const s of lifted.scripts) {
    // Global scope, as a classic <script> has (indirect eval); a throw is
    // reported and the next script still runs — a browser's rule.
    lines.push(
      `try { (0, eval)(${J(s.text)}); } catch (e) { console.error(${J(`[index.html] ${s.label}: `)} + ((e && e.stack) || e)); }`,
    );
  }
  return lines.join("\n") + "\n";
}

// ---------------------------------------------------------------------------
// the app's own framework packages (pinia, vue-router)
// ---------------------------------------------------------------------------

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
  // `…/node_modules/<name>/…` — walk up from the resolved entry to the package root.
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

/**
 * Which copy of `name` (pinia / vue-router) a prod bundle uses.
 *
 * The app's OWN when it declares the package and it is installed — an app that
 * asks for pinia 3 gets pinia 3, exactly as its Vite build does. Otherwise
 * vue-native's (what the dev builder's vendor.js carries). A major version that
 * differs between the copy used and vue-native's own is said out loud, because
 * that is a dev/prod divergence or a silent swap either way.
 */
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
  const warning =
    major(appVersion) !== major(runtimeVersion)
      ? `using the app's own ${name} ${appVersion}; vue-native's dev builder (vendor.js) carries ${name} ` +
        `${runtimeVersion} — dev and prod run different majors`
      : undefined;
  return { path: appPath, from: "app", version: appVersion, warning };
}

/** The app's package.json, or null. */
export function readPackageJson(dir: string): Record<string, unknown> | null {
  try {
    const v = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    return v && typeof v === "object" ? v : null;
  } catch {
    return null;
  }
}
