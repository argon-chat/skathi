// Dev builder: two-tier output for Vue HMR.
// Usage: bun build.ts [app-entry.js] [dist-dir] [--watch]
//
//   dist/vendor.js           IIFE: @vue/runtime-core + src/runtime.ts + src/loader.ts.
//   dist/modules/<enc>.js    one classic script per app module; <enc> is the id with "/" -> "__".
//   dist/manifest.json       { rev, vendorRev, entry, appRoot, modules, deps }
//   dist/changes.json        { rev, vendorRev, changed: [{ id, kind }] }
//
// Module ids are workspace-relative POSIX paths. Change kind: script or <style> scoped/module/lang
// shape changed -> "reload"; template -> "rerender" (state survives); <style> text only ->
// "css-only"; non-.vue source -> "reload"; an npm module already loaded that now belongs to a
// rebuilt package group -> "restart".
// hoistStatic and inlineTemplate are forced off: the host has no insertStaticContent, and an inline
// render inside setup() could not be swapped without losing state.
// npm dependencies are one shared module per package entry (inlining per importer breaks
// module-scope identity such as reka-ui's Symbol() injection keys); vue, @skathi-native/runtime, vue-router
// and pinia stay external so libraries land on our runtime.

import { parse, compileScript, compileTemplate, compileStyle, registerTS } from "@vue/compiler-sfc";
import type { BindingMetadata, SFCBlock, SFCDescriptor } from "@vue/compiler-sfc";
import { createRequire } from "module";
import { resolve, dirname, isAbsolute, join, relative, sep } from "path";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  realpathSync,
  renameSync,
  statSync,
  writeFileSync,
  rmSync,
} from "fs";

import { TW_ID, declaresUtilities, findConfig, generate as generateTailwind, needsTailwindPass } from "./tailwind";
import { pruneVendorPrefixes } from "./css-prefixes";
import { LOADER_ENTRY, RUNTIME_ENTRY, RUNTIME_SRC_DIR, isPreseeded, runtimeRequire } from "./runtime-path";
import {
  GLOB_TRIPWIRE,
  LEAF_IMPORT,
  SFC_FS,
  appDefines,
  devEsmInterop,
  devImportsPlugin,
  frameworkDep,
  importerDir,
  inlineCssImports,
  readPackageJson,
  resolveImport,
  svgDefaultOf,
  tagModuleNamespaces,
  takeDefineArgs,
  transformSource,
  viteShapesPlugin,
} from "./vitecompat";
import { colorEnabled, paint } from "./cli/term";
import { SVELTE_SOURCE, compileSvelte } from "./svelte";

const C = paint(colorEnabled(process.stdout));
const styled = (rest: (text: string) => string) => (text: string) => {
  const m = /^(\[[^\]]*\]) ([\s\S]*)$/.exec(text);
  return m ? `${C.dim(m[1]!)} ${rest(m[2]!)}` : rest(text);
};
const say = (text: string) => console.log(styled((t) => t)(text));
const warn = (text: string) => console.warn(styled(C.yellow)(text));
const fail = (text: string, ...rest: unknown[]) => console.error(styled(C.red)(text), ...rest);

const { defines: cliDefines, rest: argv } = takeDefineArgs(process.argv.slice(2));
const positional = argv.filter((a) => !a.startsWith("--"));
const WATCH = argv.includes("--watch");

const PKG_DIR = import.meta.dir; // .../packages/cli
const WS_ROOT = resolve(PKG_DIR, "../.."); // the checkout root, when this package is in one

const ENTRY_ABS = resolve(positional[0] ?? join(WS_ROOT, "apps/hello/src/main.ts"));
const APP_SRC_DIR = dirname(ENTRY_ABS);
const DIST = resolve(positional[1] ?? join(PKG_DIR, "dist"));
const MODULES_DIR = join(DIST, "modules");

const SOURCE_EXT = /\.(vue|svelte|js|ts|mjs)$/;

// The window block of sk.config.json is copied verbatim into the manifest: the host reads it
// before the window opens, the only time creation-time options can be set without a flash.
const SK_CONFIG_DIR = APP_SRC_DIR.endsWith(`${sep}src`) ? dirname(APP_SRC_DIR) : APP_SRC_DIR;
const SK_CONFIG_PATH = join(SK_CONFIG_DIR, "sk.config.json");

function readSkConfig(): Record<string, unknown> | undefined {
  if (!existsSync(SK_CONFIG_PATH)) return undefined;
  try {
    const cfg = JSON.parse(readFileSync(SK_CONFIG_PATH, "utf8"));
    return cfg && typeof cfg === "object" ? cfg : undefined;
  } catch (e) {
    // A malformed config must not fail the build, but must not be silently ignored either.
    fail(`[build] ${SK_CONFIG_PATH}: ${e}`);
    return undefined;
  }
}

// Top-level "devtools": false goes inside the window block (the host reads it before the window
// exists); a window block that sets it itself wins.
function foldDevtoolsFlag(
  cfg: Record<string, unknown> | undefined,
  win: Record<string, unknown> | undefined,
): Record<string, unknown> | undefined {
  if (!cfg || typeof cfg.devtools !== "boolean") return win;
  if (win && typeof win.devtools === "boolean") return win;
  return { ...(win ?? {}), devtools: cfg.devtools };
}

function foldHttpCache(
  cfg: Record<string, unknown> | undefined,
  win: Record<string, unknown> | undefined,
): Record<string, unknown> | undefined {
  if (!cfg || cfg.httpCache === undefined) return win;
  if (win && win.httpCache !== undefined) return win;
  return { ...(win ?? {}), httpCache: cfg.httpCache };
}

function readWindowConfig(): Record<string, unknown> | undefined {
  const cfg = readSkConfig();
  const win = cfg && typeof cfg.window === "object" && cfg.window ? (cfg.window as Record<string, unknown>) : undefined;
  return foldHttpCache(cfg, foldDevtoolsFlag(cfg, win));
}

function readAppName(): string | undefined {
  const cfg = readSkConfig();
  const name = cfg?.name;
  return typeof name === "string" && name.trim() ? name.trim() : undefined;
}

// Launch keys the host reads before anything else; only well-typed keys are copied (`sk build`
// is where a wrong one is an error).
function readLaunchConfig(): Record<string, unknown> | undefined {
  const cfg = readSkConfig();
  if (!cfg) return undefined;
  const out: Record<string, unknown> = {};
  if (typeof cfg.id === "string" && cfg.id.trim()) out.id = cfg.id.trim();
  if (typeof cfg.singleInstance === "boolean") out.singleInstance = cfg.singleInstance;
  if (typeof cfg.raiseOnSecondInstance === "boolean") out.raiseOnSecondInstance = cfg.raiseOnSecondInstance;
  if (Array.isArray(cfg.protocols)) {
    out.protocols = cfg.protocols.filter((p): p is string => typeof p === "string" && p.trim() !== "");
  }
  for (const key of ["id", "singleInstance", "raiseOnSecondInstance", "protocols"]) {
    if (cfg[key] !== undefined && out[key] === undefined) {
      fail(`[build] ${SK_CONFIG_PATH}: "${key}" has the wrong type and was ignored`);
    }
  }
  return Object.keys(out).length ? out : undefined;
}

// Ids are relative to the checkout for its own apps, else to the app: an app elsewhere (or a package
// installed outside its tree) must not get ids that climb out of it. A file on another drive keeps its
// absolute path, minus the drive's colon, which a module file name cannot hold.
const toId = (abs: string) =>
  relative(ID_ROOT, abs)
    .split(sep)
    .join("/")
    .replace(/^([A-Za-z]):/, "$1");
const encId = (id: string) => id.split("/").join("__") + ".js";
const J = (v: unknown) => JSON.stringify(v);

function findAppRoot(startDir: string, stopDir: string): string {
  let dir = startDir;
  for (;;) {
    // The workspace root's package.json is the SDK's, not an app's.
    if (dir === stopDir) return startDir;
    if (existsSync(join(dir, "package.json")) || existsSync(join(dir, "node_modules"))) return dir;
    const parent = dirname(dir);
    if (parent === dir) return startDir;
    dir = parent;
  }
}
const appRootArg = argv.find((arg) => arg.startsWith("--app-root="))?.slice("--app-root=".length);
const APP_ROOT = appRootArg ? resolve(appRootArg) : findAppRoot(APP_SRC_DIR, WS_ROOT);
if (appRootArg && !existsSync(join(APP_ROOT, "package.json"))) {
  throw new Error(`--app-root must name an application with package.json: ${APP_ROOT}`);
}
const inside = (dir: string, root: string) => {
  const rel = relative(root, dir);
  return !rel.startsWith("..") && !isAbsolute(rel);
};
const ID_ROOT = inside(APP_ROOT, WS_ROOT) ? WS_ROOT : APP_ROOT;
const appRequire = createRequire(join(APP_ROOT, "package.json"));
const APP_PKG = APP_ROOT === PKG_DIR ? null : readPackageJson(APP_ROOT);
const FRAMEWORK = new Map(
  ["vue-router", "pinia"].map(
    (name) =>
      [
        name,
        frameworkDep(
          name,
          APP_PKG,
          (spec) => appRequire.resolve(spec),
          (spec) => runtimeRequire.resolve(spec),
        ),
      ] as const,
  ),
);

/** Resolves a bare specifier from the app root with browser/ESM conditions; createRequire cannot see ESM-only `exports` maps. */
function resolveFromApp(spec: string): string {
  try {
    return Bun.resolveSync(spec, APP_ROOT);
  } catch {
    return appRequire.resolve(spec);
  }
}

/**
 * Entry files to try for an npm package, best first: what a bundler picks for a browser build
 * (`require` before `import`: reka-ui's ESM entry breaks under Bun's CJS output), then Node's CJS
 * entry, then Bun's. A candidate whose build fails falls through.
 */
async function entryCandidates(spec: string, from = APP_ROOT): Promise<string[]> {
  const out: string[] = [];
  for (const kind of ["require", "import"] as const) {
    const hit = await bundlerResolve(spec, from, kind);
    if (hit && !out.includes(hit)) out.push(hit);
  }
  try {
    const req = from === APP_ROOT ? appRequire : createRequire(join(from, "package.json"));
    const cjs = req.resolve(spec);
    if (isAbsolute(cjs) && !out.includes(cjs)) out.push(cjs);
  } catch {}
  try {
    const esm = Bun.resolveSync(spec, from);
    if (isAbsolute(esm) && !out.includes(esm)) out.push(esm);
  } catch {}
  return out;
}

async function bundlerResolve(spec: string, from: string, kind: "require" | "import"): Promise<string | null> {
  const probe = join(from, "__sk_resolve_probe__.js"); // never written: onLoad answers for it
  let hit: string | null = null;
  await Bun.build({
    entrypoints: [probe],
    target: "browser",
    format: "cjs",
    plugins: [
      {
        name: "sk-resolve-probe",
        setup(b: Bun.PluginBuilder) {
          b.onResolve({ filter: /__sk_resolve_probe__\.js$/ }, () => ({ path: probe }));
          b.onLoad({ filter: /.*/ }, (args: Bun.OnLoadArgs) => {
            if (args.path === probe) {
              return { contents: kind === "require" ? `require(${J(spec)});` : `import ${J(spec)};`, loader: "js" };
            }
            hit ??= args.path;
            return { contents: "", loader: "js" };
          });
        },
      },
    ],
  }).catch(() => null);
  return hit && isAbsolute(hit) ? hit : null;
}

// compileScript needs `fs` (Bun is not detected as Node) and `registerTS` (cross-package type
// resolution loads typescript from the app's node_modules). SFC_FS answers fileExists for files
// only: compiler-sfc probes ./types before ./types.ts.

const TS_AVAILABLE = (() => {
  try {
    const ts = appRequire("typescript");
    registerTS(() => ts);
    return true;
  } catch {
    return false;
  }
})();

{
  const aliases = readSkConfig()?.aliases;
  if (aliases && typeof aliases === "object" && Object.keys(aliases).length) {
    warn(
      `[build] sk.config.json "aliases" (${Object.keys(aliases).join(", ")}) are applied by sk build only; this dev build resolves them from node_modules`,
    );
  }
}

// The app's defines: sk.config.json "define" plus `--define K=V`, with Vite semantics.
const APP_DEFINES = appDefines(readSkConfig()?.define, cliDefines, SK_CONFIG_PATH);
if (APP_DEFINES.errors.length) {
  for (const e of APP_DEFINES.errors) fail(`[build] ${e}`);
  process.exit(1);
}

const VITE_ENV_DEV: Record<string, unknown> = {
  MODE: "development",
  DEV: true,
  PROD: false,
  SSR: false,
  BASE_URL: "/",
  SK: true,
};
for (const [k, v] of Object.entries(APP_DEFINES.defines)) {
  if (!k.startsWith("import.meta.env.")) continue;
  try {
    VITE_ENV_DEV[k.slice("import.meta.env.".length)] = JSON.parse(v);
  } catch {}
}

const DEFINES: Record<string, string> = {
  __DEV__: "true",
  "process.env.NODE_ENV": '"development"',
  __FEATURE_SUSPENSE__: "true",
  __COMPAT__: "false",
  __BROWSER__: "false",
  __ESM_BUNDLER__: "true",
  __GLOBAL__: "false",
  __NODE_JS__: "false",
  __SSR__: "false",
  __TEST__: "false",
  __VERSION__: '"3.5"',
  // `import.meta` is a SyntaxError in a classic script, so substitute what Vite would inline.
  // Dotted forms first (Bun matches the longest key). Object values are JSON.stringify'd: Bun
  // parses a define value as JSON or an identifier and inserts anything else as a string.
  "import.meta.env.MODE": '"development"',
  "import.meta.env.DEV": "true",
  "import.meta.env.PROD": "false",
  "import.meta.env.SSR": "false",
  "import.meta.env.BASE_URL": '"/"',
  // `true` in every Skathi build, undefined under Vite: `if (!import.meta.env.SK)` is dropped
  // by Bun while parsing.
  "import.meta.env.SK": "true",
  "import.meta.env": JSON.stringify(VITE_ENV_DEV),
  // Undefined in the vendor bundle and npm modules (library code guards on it); an app module gets its own
  // (`MODULE_DEFINES`).
  "import.meta.hot": "undefined",
  "import.meta.url": '"sk://module"',
  // Leftover import.meta.glob calls are defined to a findable name so buildModule can fail clearly.
  "import.meta.glob": GLOB_TRIPWIRE,
  "import.meta": JSON.stringify({ env: VITE_ENV_DEV, url: "sk://module" }),
  ...APP_DEFINES.defines,
};

// An app module's `import.meta.hot` is the loader's per-module HMR context.
const MODULE_DEFINES: Record<string, string> = { ...DEFINES, "import.meta.hot": "module.hot" };

function writeAtomic(path: string, text: string) {
  const tmp = path + ".tmp";
  writeFileSync(tmp, text);
  renameSync(tmp, path);
}

function ensureDirs() {
  mkdirSync(MODULES_DIR, { recursive: true });
}

async function buildVendor() {
  const result = await Bun.build({
    entrypoints: [LOADER_ENTRY],
    format: "iife",
    target: "browser",
    define: DEFINES,
    plugins: [
      {
        // "vue" is our runtime. Exact-match filter only: bun 1.4 crashes on catch-all onResolve filters.
        name: "vendor-vue-alias",
        setup(b: Bun.PluginBuilder) {
          b.onResolve({ filter: /^vue$/ }, () => ({ path: RUNTIME_ENTRY }));
        },
      },
    ],
  });
  if (!result.success) {
    for (const log of result.logs) console.error(log);
    throw new Error("vendor build failed");
  }
  const text = await result.outputs[0]!.text();
  // The pre-seeded "vue" namespace must still carry the compiler helpers; if Bun tree-shakes the
  // `export *` re-export, templates die with "not a function".
  for (const sym of ["openBlock", "createElementBlock", "createElementVNode", "toDisplayString"]) {
    if (!text.includes(sym)) throw new Error(`vendor bundle is missing ${sym}`);
  }
  writeAtomic(join(DIST, "vendor.js"), text);
  // vendorRev is a CONTENT hash: restarting the watcher must not make a running host believe its
  // already-evaluated vendor.js is stale.
  vendorRev = Number(BigInt.asUintN(48, BigInt(Bun.hash(text))));
  return text.length;
}

const scopeIdOf = (id: string) => Bun.hash(id).toString(16).slice(0, 8);

let capturedCss: string | null = null;

const devAdvised = new Set<string>();
function devAdvise(m: string) {
  if (devAdvised.has(m)) return;
  devAdvised.add(m);
  warn(`${stamp()} advisory: ${m}`);
}

async function viteCss(text: string, from: string, label: string): Promise<string> {
  const css = inlineCssImports(text, from, SK_CONFIG_DIR, devAdvise);
  if (!TW_CONFIG || !needsTailwindPass(css)) return css;
  try {
    return await generateTailwind(TW_CONFIG, { text: css, from });
  } catch (e) {
    fail(`${stamp()} ${label}: tailwind pass failed — emitting the source verbatim`, e);
    return css;
  }
}

async function compileVue(
  absPath: string,
  source: string,
  captureCss = true,
): Promise<{ contents: string; loader: "js" | "ts" }> {
  const id = toId(absPath);
  const { descriptor, errors } = parse(source, { filename: absPath });
  if (errors.length) {
    throw new Error(`${id}: ${errors.map((e) => e.message).join("; ")}`);
  }
  const shortId = scopeIdOf(id);
  const scopeId = `data-v-${shortId}`;
  const hasScoped = descriptor.styles.some((s) => s.scoped);
  // compileScript leaves TS syntax in place, so the loader must follow the block's lang. RETURNED,
  // not stashed: the style pass awaits and compiles can interleave.
  const isTS = /^tsx?$/.test(descriptor.scriptSetup?.lang ?? descriptor.script?.lang ?? "");

  const chunks: string[] = [];
  let bindings: BindingMetadata | undefined;

  if (descriptor.scriptSetup || descriptor.script) {
    const compiled = compileScript(descriptor, {
      id: shortId,
      inlineTemplate: false, // REQUIRED for rerender-based HMR
      genDefaultAs: "_sfc_main",
      fs: SFC_FS, // imported-type resolution — see SFC_FS above
    });
    bindings = compiled.bindings;
    chunks.push(compiled.content);
  } else {
    chunks.push("const _sfc_main = {};");
  }

  if (descriptor.template) {
    const t = compileTemplate({
      source: descriptor.template.content,
      filename: absPath,
      id: shortId,
      scoped: hasScoped,
      slotted: descriptor.slotted,
      compilerOptions: {
        hoistStatic: false, // compileTemplate force-sets true; our spread wins (F7)
        ...(bindings && { bindingMetadata: bindings }),
        expressionPlugins: isTS ? ["typescript"] : [],
        // Skathi host elements missing from the HTML tag list: otherwise the SFC compiler warns
        // on every render. Keep in lockstep with prod.ts and crates/sk-tree/src/render.rs.
        isCustomElement: (tag: string) => tag === "lottie" || tag.startsWith("sk-"),
      },
    });
    if (t.errors.length) {
      throw new Error(`${id} <template>: ${t.errors.map(String).join("; ")}`);
    }
    chunks.push(t.code);
    chunks.push("_sfc_main.render = render;");
  }

  if (hasScoped) chunks.push(`_sfc_main.__scopeId = ${J(scopeId)};`);
  chunks.push(`_sfc_main.__file = ${J(id)};`);

  // Replaces plugin-vue's import.meta.hot epilogue; createRecord is idempotent, so re-evaluating a
  // module keeps the record that holds the live instances.
  chunks.push(`_sfc_main.__hmrId = ${J(id)};`);
  chunks.push(`if (typeof __VUE_HMR_RUNTIME__ !== "undefined") __VUE_HMR_RUNTIME__.createRecord(${J(id)}, _sfc_main);`);
  chunks.push("export default _sfc_main;");

  const cssChunks: string[] = [];
  for (const style of descriptor.styles) {
    let text = style.content;
    if (!style.lang || style.lang === "css" || style.lang === "postcss") {
      text = await viteCss(text, absPath, `${id} <style>`);
    } else {
      devAdvise(`${id}: <style lang="${style.lang}"> is not preprocessed by Skathi — it is compiled as plain CSS`);
    }
    if (style.module) {
      devAdvise(`${id}: <style module> — CSS modules are not supported yet; useCssModule() answers {}`);
    }
    // `isProd` left at its default (false) on both this and compileScript:
    // they hash a `v-bind()` variable's name from it and must agree.
    const out = compileStyle({
      source: text,
      filename: absPath,
      id: scopeId,
      scoped: style.scoped ?? false,
    });
    if (out.errors.length) throw new Error(`${id} <style>: ${out.errors.join("; ")}`);
    cssChunks.push(out.code);
  }
  if (captureCss) capturedCss = cssChunks.length ? pruneVendorPrefixes(cssChunks.join("\n")) : null;

  return { contents: chunks.join("\n"), loader: isTS ? "ts" : "js" };
}

const vuePlugin = {
  name: "vue-sfc",
  setup(build: Bun.PluginBuilder) {
    build.onLoad({ filter: /\.vue$/ }, async (args: Bun.OnLoadArgs) => {
      const source = await Bun.file(args.path).text();
      return compileVue(args.path, source);
    });
    // A package's Svelte components (the runtime's `<SkWindow>`, a component library), compiled as the app's are,
    // without HMR: a package module is rebuilt whole.
    build.onLoad({ filter: SVELTE_SOURCE }, async (args: Bun.OnLoadArgs) => {
      const id = toId(args.path);
      const out = await compileSvelte(APP_ROOT, args.path, await Bun.file(args.path).text(), {
        dev: false,
        filename: id,
      });
      for (const w of out.warnings) devAdvise(w);
      if (out.css) devAdvise(`${id}: a package component's <style> is applied by sk build, not by sk dev`);
      return { contents: out.js, loader: "js" as const };
    });
  },
};

// Vite import shapes (vitecompat.ts), shared with prod.ts. In dev, import.meta.glob imports by
// importer-relative path.

const SHAPE_NOTES = new Map<string, number>();
let inlinedNow: Set<string> | null = null;
const noteInlined = (abs: string) => void inlinedNow?.add(abs);

const DEV_ASSET_FILES = new Map<string, string>();
const shapesOptions = (dev: boolean) => ({
  docRoot: SK_CONFIG_DIR,
  wsRoot: ID_ROOT,
  svgDefault: svgDefaultOf(APP_PKG),
  compileSfc: (absPath: string, source: string) => compileVue(absPath, source, false),
  assets: DEV_ASSET_FILES,
  notes: SHAPE_NOTES,
  dev,
  inlined: dev ? noteInlined : undefined,
});
const DEV_SHAPES = shapesOptions(true);
const devShapesPlugin = viteShapesPlugin(DEV_SHAPES);
const npmShapesPlugin = viteShapesPlugin(shapesOptions(false));
const devImports = devImportsPlugin({
  srcDir: APP_SRC_DIR,
  preseeded: isPreseeded,
  inlined: noteInlined,
  npmId: (spec, abs, from) => npmIdOf(spec, abs, from),
});

const appModulePlugin = {
  name: "sk-dev-app-module",
  setup(build: Bun.PluginBuilder) {
    build.onLoad({ filter: /\.vue$/ }, async (args: Bun.OnLoadArgs) => {
      const out = await compileVue(args.path, await Bun.file(args.path).text());
      const code = transformSource(out.contents, args.path, DEV_SHAPES);
      return code === null ? out : { ...out, contents: code };
    });
    build.onLoad({ filter: SVELTE_SOURCE }, async (args: Bun.OnLoadArgs) => {
      const id = toId(args.path);
      const out = await compileSvelte(APP_ROOT, args.path, await Bun.file(args.path).text(), {
        dev: true,
        filename: id,
      });
      for (const w of out.warnings) devAdvise(w);
      capturedCss = out.css ? pruneVendorPrefixes(await viteCss(out.css, args.path, `${id} <style>`)) : null;
      return { contents: transformSource(out.js, args.path, DEV_SHAPES) ?? out.js, loader: "js" as const };
    });
  },
};

// Every module is built with external:"*", so each import survives as a literal require("<spec>")
// and deps are read from the emitted body. Pre-seeded bare specifiers and npm packages stay out
// of the graph.

const REQUIRE_RE = /\brequire\(\s*("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')\s*\)/g;

const CANDIDATE_EXTS = [
  "",
  ".js",
  ".ts",
  ".mjs",
  ".vue",
  ".svelte",
  ".svelte.ts",
  ".svelte.js",
  "/index.js",
  "/index.ts",
  "/index.vue",
  "/index.svelte",
];

function normalizeId(path: string): string {
  const out: string[] = [];
  for (const seg of path.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") {
      if (out.length && out[out.length - 1] !== "..") out.pop();
      else out.push("..");
      continue;
    }
    out.push(seg);
  }
  return out.join("/");
}

function unquote(raw: string): string | null {
  try {
    if (raw.charCodeAt(0) === 34 /* " */) return JSON.parse(raw);
    const inner = raw.slice(1, -1).replace(/\\'/g, "'").replace(/"/g, '\\"');
    return JSON.parse('"' + inner + '"');
  } catch {
    return null;
  }
}

function depsOf(id: string, body: string, ids: Set<string>, skip: Set<string>): string[] {
  const slash = id.lastIndexOf("/");
  const dir = slash < 0 ? "" : id.slice(0, slash);
  const found: string[] = [];
  REQUIRE_RE.lastIndex = 0;
  for (let m = REQUIRE_RE.exec(body); m; m = REQUIRE_RE.exec(body)) {
    const spec = unquote(m[1]!);
    if (spec === null || spec.charCodeAt(0) !== 46 /* . */) continue; // bare -> vendor
    if (skip.has(spec)) continue; // a stylesheet: its own synthetic module
    const base = normalizeId((dir ? dir + "/" : "") + spec);
    // TypeScript's ESM spelling: `./x.js` names `x.ts`.
    const hit = CANDIDATE_EXTS.map((ext) => base + ext)
      .concat(base.endsWith(".js") ? [base.slice(0, -3) + ".ts"] : [])
      .find((cand) => ids.has(cand));
    if (!hit) {
      warn(`${stamp()} ${id}: import ${J(spec)} resolves to no built module — ` + `require() will throw at runtime`);
      continue;
    }
    if (hit !== id && !found.includes(hit)) found.push(hit);
  }
  return found;
}

function bareOf(body: string, skip: Set<string>): string[] {
  const found: string[] = [];
  REQUIRE_RE.lastIndex = 0;
  for (let m = REQUIRE_RE.exec(body); m; m = REQUIRE_RE.exec(body)) {
    const spec = unquote(m[1]!);
    if (spec === null) continue;
    const c = spec.charCodeAt(0);
    if (c === 46 /* . */ || c === 47 /* / */) continue; // relative/absolute -> graph
    if ((isPreseeded(spec) && !FRAMEWORK.has(spec)) || found.includes(spec) || skip.has(spec)) continue;
    found.push(spec);
  }
  return found;
}

const moduleDeps = new Map<string, string[]>(); // id -> dep ids
const moduleBare = new Map<string, string[]>(); // app module id -> npm module ids it requires

// CSS entries are emitted as css-only synthetic modules (the Tailwind transport), not into a JS body.

const cssModules = new Map<string, string>();
const cssEmitted = new Map<string, string>();

function cssImportsOf(absPath: string, body: string, cssSpecs: Set<string>): string[] {
  const dir = dirname(absPath);
  const found: string[] = [];
  REQUIRE_RE.lastIndex = 0;
  for (let m = REQUIRE_RE.exec(body); m; m = REQUIRE_RE.exec(body)) {
    const spec = unquote(m[1]!);
    if (spec === null) continue;
    const relative_ = spec.charCodeAt(0) === 46; /* . */
    let abs: string;
    if (relative_) {
      if (!spec.endsWith(".css")) continue;
      abs = resolve(dir, spec);
    } else {
      if (isPreseeded(spec)) continue;
      try {
        abs = resolveFromApp(spec);
      } catch {
        continue;
      }
      if (!abs.endsWith(".css")) continue;
    }
    if (!existsSync(abs)) {
      warn(`${stamp()} ${toId(absPath)}: import ${J(spec)} names no file — skipped`);
      continue;
    }
    cssSpecs.add(spec);
    const id = relative_ ? toId(abs) : spec;
    cssModules.set(id, abs);
    if (!found.includes(id)) found.push(id);
  }
  return found;
}

const cssIds = () => Array.from(cssModules.keys());

/**
 * Writes one CSS-only module script; true when the sheet moved. With a Tailwind config the sheet
 * goes through Tailwind with itself as the entry, otherwise it is emitted byte-for-byte.
 */
async function buildCssModule(id: string): Promise<boolean> {
  const abs = cssModules.get(id)!;
  let css = inlineCssImports(readFileSync(abs, "utf-8"), abs, SK_CONFIG_DIR, devAdvise);
  if (TW_CONFIG && needsTailwindPass(css)) {
    const t0 = Date.now();
    try {
      css = await generateTailwind(TW_CONFIG, { text: css, from: abs });
      say(`${stamp()} css ${id}: ${(css.length / 1024).toFixed(1)} KB through tailwind in ${Date.now() - t0} ms`);
    } catch (e) {
      fail(`${stamp()} css ${id}: tailwind pass failed — emitting the source verbatim`, e);
    }
  } else {
    say(`${stamp()} css ${id}: ${(css.length / 1024).toFixed(1)} KB`);
  }
  css = pruneVendorPrefixes(css);
  moduleDeps.set(id, []);
  if (cssEmitted.get(id) === css) return false;
  cssEmitted.set(id, css);
  writeAtomic(
    join(MODULES_DIR, encId(id)),
    [
      `// Skathi module: ${id} (synthetic — CSS entry)`,
      `// source: ${toId(abs)}; do not edit`,
      `__SK__.setCss(${J(id)}, ${J(css)});`,
      `__SK__.setDeps && __SK__.setDeps(${J(id)}, []);`,
      // Empty factory: the import is a side effect; the constant body hash keeps a CSS edit inert.
      `__SK__.define(${J(id)}, function (require, module, exports) {`,
      `}, ${J(TW_HASH)});`,
    ].join("\n"),
  );
  return true;
}

async function syncCssModules(): Promise<string[]> {
  const changed: string[] = [];
  for (const id of cssIds()) {
    if (await buildCssModule(id)) changed.push(id);
  }
  return changed;
}

const appOwnsUtilities = () =>
  Array.from(cssEmitted.keys()).some((id) => {
    const abs = cssModules.get(id);
    return !!abs && declaresUtilities(readFileSync(abs, "utf-8"));
  });

const moduleInlined = new Map<string, Map<string, number>>();

const mtimeOf = (abs: string) => {
  try {
    return statSync(abs).mtimeMs;
  } catch {
    return -1;
  }
};

function inlinedMoved(abs: string): boolean {
  for (const [file, mtime] of moduleInlined.get(abs) ?? []) if (mtimeOf(file) !== mtime) return true;
  return false;
}

async function buildModule(absPath: string, ids: Set<string>) {
  const id = toId(absPath);
  capturedCss = null;
  const inlined = (inlinedNow = new Set());
  let result: Awaited<ReturnType<typeof Bun.build>>;
  try {
    result = await Bun.build({
      entrypoints: [absPath],
      format: "cjs",
      target: "node",
      external: ["*"],
      plugins: [devShapesPlugin, devImports, appModulePlugin],
      define: MODULE_DEFINES,
    });
  } finally {
    inlinedNow = null;
  }
  if (!result.success) {
    for (const log of result.logs) console.error(log);
    throw new Error(`module build failed: ${id}`);
  }
  moduleInlined.set(absPath, new Map(Array.from(inlined, (f) => [f, mtimeOf(f)])));
  // Bundler ESM semantics rather than Node's (vitecompat.ts devEsmInterop).
  const interop = devEsmInterop(tagModuleNamespaces(await result.outputs[0]!.text()).code);
  const body = interop.code;
  if (interop.unresolved) {
    warn(
      `${stamp()} ${id}: ${interop.unresolved} import(…) with a computed specifier — the dev loader resolves ` +
        `only a string literal, so that promise rejects (prod.ts bundles it)`,
    );
  }
  if (/\bcreateStaticVNode\b/.test(body)) {
    throw new Error(`${id}: emitted createStaticVNode — hoistStatic leaked back on (F7)`);
  }
  if (new RegExp(`\\b${GLOB_TRIPWIRE}\\b`).test(body)) {
    throw new Error(
      `${id}: an import.meta.glob the expansion did not see (not a call with literal arguments) — ` +
        `the dev builder would emit a call that throws at eval`,
    );
  }

  const cssSpecs = new Set<string>();
  const css = cssImportsOf(absPath, body, cssSpecs);
  const deps = depsOf(id, body, ids, cssSpecs).concat(css);
  moduleDeps.set(id, deps);
  moduleBare.set(id, bareOf(body, cssSpecs));

  const out: string[] = [`// Skathi module: ${id}`];
  if (capturedCss !== null) out.push(`__SK__.setCss(${J(id)}, ${J(capturedCss)});`);
  out.push(`__SK__.setDeps && __SK__.setDeps(${J(id)}, ${J(deps)});`);
  out.push(`__SK__.define(${J(id)}, function (require, module, exports) {`);
  out.push(body);
  out.push(`}, ${J(Bun.hash(body).toString(16))});`);
  writeAtomic(join(MODULES_DIR, encId(id)), out.join("\n"));
}

// npm modules: one per package ENTRY, keyed on the resolved directory (not the name), so every
// importer of a directory shares it and a nested copy at another version stays separate. The id
// is the specifier when the app resolves it to that directory, else `<spec>~<hash of the directory>`.
// Two or more entries of one package (`svelte`, `svelte/internal/client`) share module state through
// relative imports: they are bundled together into one group module, and each entry's module reads
// its namespace from it.

const npmBuilt = new Map<string, number>();
const npmFailed = new Set<string>();
const npmEntries = new Map<string, { spec: string; from: string; root: string | null }>();
const npmBare = new Map<string, string[]>();
/** Package root -> its group module's id and members. */
const npmGroups = new Map<string, { id: string; members: string[] }>();
/** npm id -> the group module it was last built as a member of, "" when built alone. */
const npmShape = new Map<string, string>();

function splitSpec(spec: string): [string, string] {
  const parts = spec.split("/");
  const n = spec.startsWith("@") ? 2 : 1;
  const rest = parts.slice(n).join("/");
  return [parts.slice(0, n).join("/"), rest ? `./${rest}` : "."];
}

const pkgNameAt = new Map<string, string | null>();
function packageRootOf(file: string, name: string): string | null {
  for (let dir = dirname(file); ;) {
    let found = pkgNameAt.get(dir);
    if (found === undefined) {
      found = null;
      try {
        const n = JSON.parse(readFileSync(join(dir, "package.json"), "utf-8")).name;
        if (typeof n === "string") found = n;
      } catch {}
      pkgNameAt.set(dir, found);
    }
    if (found === name) return dir;
    const up = dirname(dir);
    if (up === dir) return null;
    dir = up;
  }
}

function npmKey(spec: string, abs: string): string {
  const [name, sub] = splitSpec(spec);
  const root = packageRootOf(abs, name);
  return root ? `${toId(root)}|${sub}` : toId(abs);
}

const appKeys = new Map<string, string | null>();

function npmIdOf(spec: string, abs: string, from: string): string {
  const key = npmKey(spec, abs);
  let appKey = appKeys.get(spec);
  if (appKey === undefined) {
    let appAbs: string | null = null;
    try {
      appAbs = resolveFromApp(spec);
    } catch {}
    appKey = appAbs && isAbsolute(appAbs) ? npmKey(spec, appAbs) : null;
    appKeys.set(spec, appKey);
  }
  const id = key === appKey ? spec : `${spec}~${Bun.hash(key).toString(16).slice(0, 8)}`;
  if (!npmEntries.has(id)) {
    npmEntries.set(id, { spec, from: id === spec ? APP_ROOT : from, root: packageRootOf(abs, splitSpec(spec)[0]) });
  }
  return id;
}

/** A file of the package at `root` itself, not of one installed beneath it. */
function inPackage(abs: string, root: string): boolean {
  const rel = relative(root, abs);
  return !!rel && !rel.startsWith("..") && !isAbsolute(rel) && !rel.split(sep).includes("node_modules");
}

function npmSharePlugin(selfKey: string, selfRoot: string | null, found: Set<string>) {
  return {
    name: "sk-dev-npm-share",
    setup(build: Bun.PluginBuilder) {
      build.onResolve({ filter: /^(?!node:)[^./\\]/ }, (args: Bun.OnResolveArgs) => {
        if (args.kind === "entry-point-build" || isAbsolute(args.path)) return undefined;
        // Every importer lands on the one copy: the vendor bundle's, or the framework module built on demand.
        if (isPreseeded(args.path)) {
          if (FRAMEWORK.has(args.path)) found.add(args.path);
          return { path: args.path, external: true };
        }
        const from = importerDir(args, APP_ROOT);
        const abs = resolveImport(args.path, from);
        if (abs === null || !isAbsolute(abs) || /\.css$/i.test(abs) || LEAF_IMPORT.test(abs)) return undefined;
        // The package's own `#imports` and self-references are part of it, as its relative imports are.
        if (npmKey(args.path, abs) === selfKey || (selfRoot && inPackage(abs, selfRoot))) return undefined;
        const id = npmIdOf(args.path, abs, from);
        found.add(id);
        return { path: id, external: true };
      });
    },
  };
}

function noteUnresolved(id: string, spec: string, from: string): void {
  if (npmFailed.has(id)) return;
  npmFailed.add(id);
  const alias = !/^(?:@[a-z0-9][\w.-]*\/)?[a-z0-9][\w.-]*(?:\/|$)/i.test(spec);
  warn(
    `${stamp()} cannot resolve ${J(spec)} from ${toId(from)} — ` +
      `require(${J(id)}) will throw at runtime ` +
      (alias
        ? "(an import alias resolves through tsconfig.json compilerOptions.paths, as in sk build; " +
          "a vite.config resolve.alias is not read)"
        : "(is it installed?)"),
  );
}

const npmHash = new Map<string, string>();

/** Writes an npm module script; true when its body differs from the last one written under this id. */
function writeNpmModule(id: string, what: string, body: string): boolean {
  moduleDeps.set(id, []);
  // Constant unless the package changes, so a rebuild never invalidates module-scope singletons.
  const hash = Bun.hash(body).toString(16);
  writeAtomic(
    join(MODULES_DIR, encId(id)),
    [
      `// Skathi module: ${id} (${what})`,
      `__SK__.setDeps && __SK__.setDeps(${J(id)}, []);`,
      `__SK__.define(${J(id)}, function (require, module, exports) {`,
      body,
      `}, ${J(hash)});`,
    ].join("\n"),
  );
  npmBuilt.set(id, body.length);
  const changed = npmHash.get(id) !== hash;
  npmHash.set(id, hash);
  return changed;
}

async function buildNpmModule(id: string): Promise<boolean> {
  const { spec, from } = npmEntries.get(id) ?? { spec: id, from: APP_ROOT };
  // vue-router and pinia: the copy the app declares (else the runtime's), like the production bundle.
  const framework = FRAMEWORK.get(id);
  if (framework) {
    if (framework.warning) devAdvise(framework.warning);
    say(
      `${stamp()} ${id} ${framework.version ?? "unknown"}: the ${framework.from === "app" ? "app's" : "runtime's"} copy`,
    );
  }
  const candidates = framework ? [framework.path] : await entryCandidates(spec, from);
  if (!candidates.length) {
    noteUnresolved(id, spec, from);
    return false;
  }
  const selfKey = npmKey(spec, candidates[0]!);
  const selfRoot = packageRootOf(candidates[0]!, splitSpec(spec)[0]);
  const requires = new Set<string>();
  let result: Awaited<ReturnType<typeof Bun.build>> | null = null;
  let entry = candidates[0]!;
  for (const [i, candidate] of candidates.entries()) {
    entry = candidate;
    requires.clear();
    result = await Bun.build({
      entrypoints: [candidate],
      format: "cjs",
      target: "browser", // picks the browser/ESM conditions, not node builtins
      // Pre-seeded ids plus every other package entry stay external. A dependency can ship raw .vue
      // source: without the SFC plugin Bun treats it as an asset and the import silently renders
      // nothing.
      plugins: [npmShapesPlugin, npmSharePlugin(selfKey, selfRoot, requires), vuePlugin],
      define: DEFINES,
    }).catch(() => null);
    if (result?.success) break;
    if (i + 1 < candidates.length) {
      warn(`${stamp()} npm ${id}: ${toId(candidate)} did not build — retrying with ` + `${toId(candidates[i + 1]!)}`);
    }
  }
  if (!result?.success) {
    for (const log of result?.logs ?? []) console.error(log);
    throw new Error(`npm module build failed: ${id}`);
  }
  npmBare.set(id, Array.from(requires).sort());
  npmShape.set(id, "");
  writeNpmModule(
    id,
    `npm dependency, bundled from ${toId(entry)}`,
    tagModuleNamespaces(await result.outputs[0]!.text()).code,
  );
  return true;
}

const GROUP_ENTRY = "__sk_npm_group__.js";

/** Bundles the entries `members` of the package at `root` into one module: the members it built, and whether that module changed. */
async function buildNpmGroup(root: string, members: string[]): Promise<{ built: string[]; changed: boolean }> {
  const lists: string[][] = [];
  const built: string[] = [];
  for (const id of members) {
    const { spec, from } = npmEntries.get(id)!;
    const candidates = await entryCandidates(spec, from);
    if (!candidates.length) {
      noteUnresolved(id, spec, from);
      continue;
    }
    lists.push(candidates);
    built.push(id);
  }
  if (!built.length) return { built, changed: false };
  const gid = `${pkgNameAt.get(root) ?? toId(root)}~pkg~${Bun.hash(toId(root)).toString(16).slice(0, 8)}`;
  const probe = join(root, GROUP_ENTRY); // never written: the plugin answers for it
  const requires = new Set<string>();
  let result: Awaited<ReturnType<typeof Bun.build>> | null = null;
  const attempts = Math.max(...lists.map((l) => l.length));
  for (let i = 0; i < attempts; i++) {
    const source = lists.map((l, k) => `export * as e${k} from ${J(l[Math.min(i, l.length - 1)])};`).join("\n");
    requires.clear();
    result = await Bun.build({
      entrypoints: [probe],
      format: "cjs",
      target: "browser",
      plugins: [
        {
          name: "sk-npm-group-entry",
          setup(b: Bun.PluginBuilder) {
            b.onResolve({ filter: /__sk_npm_group__\.js$/ }, () => ({ path: probe }));
            b.onLoad({ filter: /__sk_npm_group__\.js$/ }, () => ({ contents: source, loader: "js" }));
          },
        },
        npmShapesPlugin,
        npmSharePlugin("", root, requires),
        vuePlugin,
      ],
      define: DEFINES,
    }).catch(() => null);
    if (result?.success) break;
  }
  if (!result?.success) {
    for (const log of result?.logs ?? []) console.error(log);
    throw new Error(`npm module build failed: ${built.join(", ")} (bundled together as ${gid})`);
  }
  npmBare.set(gid, Array.from(requires).sort());
  const changed = writeNpmModule(
    gid,
    `npm dependency, the entries ${built.join(", ")} of ${toId(root)}`,
    tagModuleNamespaces(await result.outputs[0]!.text()).code,
  );
  for (const [k, id] of built.entries()) {
    // An ES module's namespace is the module; a CommonJS entry's is its `module.exports`, as when built alone.
    writeNpmModule(
      id,
      `npm dependency, entry of ${gid}`,
      `var ns = require(${J(gid)}).e${k};\n` +
        'module.exports = ns && ns[Symbol.toStringTag] === "Module" ? ' +
        'Object.defineProperty(ns, "__esModule", { value: true }) : ns && ns.default;',
    );
    npmBare.set(id, [gid]);
    npmShape.set(id, gid);
  }
  npmGroups.set(root, { id: gid, members: built });
  return { built, changed };
}

const npmIds = () => Array.from(npmBuilt.keys()).sort();

function wantedNpm(): Set<string> {
  const wanted = new Set<string>();
  const queue: string[] = [];
  for (const list of moduleBare.values()) queue.push(...list);
  for (let id = queue.pop(); id !== undefined; id = queue.pop()) {
    if (wanted.has(id)) continue;
    wanted.add(id);
    queue.push(...(npmBare.get(id) ?? []));
  }
  return wanted;
}

/** `added`: npm modules new to the build; `regrouped`: ones already built that now belong to a new group module (a running app holds the old copy). */
async function syncNpmModules(): Promise<{ added: string[]; regrouped: string[] }> {
  const added: string[] = [];
  const regrouped: string[] = [];
  const tried = new Set<string>();
  for (let progress = true; progress;) {
    progress = false;
    const wanted = wantedNpm();
    const groupIds = new Set(Array.from(npmGroups.values(), (g) => g.id));
    const byRoot = new Map<string, string[]>();
    for (const id of wanted) {
      const root = groupIds.has(id) ? null : npmEntries.get(id)?.root;
      if (root) byRoot.set(root, [...(byRoot.get(root) ?? []), id]);
    }
    for (const [root, ids] of byRoot) {
      const known = npmGroups.get(root)?.members ?? [];
      // A member that went away leaves the group as it is; a new one rebuilds it.
      if (ids.length < 2 && !known.length) continue;
      if (ids.every((id) => known.includes(id) && npmBuilt.has(id))) continue;
      const members = Array.from(new Set([...known, ...ids])).sort();
      const key = members.join("\n");
      if (tried.has(key)) continue;
      tried.add(key);
      progress = true;
      const before = new Set(members.filter((id) => npmBuilt.has(id)));
      const shapeBefore = new Map(members.map((id) => [id, npmShape.get(id)]));
      const t0 = Date.now();
      const { built, changed } = await buildNpmGroup(root, members);
      for (const id of built) {
        if (!before.has(id)) added.push(id);
        else if (changed || shapeBefore.get(id) !== npmShape.get(id)) regrouped.push(id);
      }
      if (built.length) say(`${stamp()} npm ${built.join(", ")}: bundled together in ${Date.now() - t0} ms`);
    }
    const grouped = (id: string) => {
      const root = npmEntries.get(id)?.root;
      return !!root && npmGroups.has(root);
    };
    const todo = Array.from(wanted)
      .filter((id) => !npmBuilt.has(id) && !tried.has(id) && !groupIds.has(id) && !grouped(id))
      .sort();
    for (const id of todo) {
      tried.add(id);
      progress = true;
      const t0 = Date.now();
      if (!(await buildNpmModule(id))) continue;
      added.push(id);
      say(`${stamp()} npm ${id}: ${((npmBuilt.get(id) ?? 0) / 1024) | 0} KB in ${Date.now() - t0} ms`);
    }
  }

  const wanted = wantedNpm();
  for (const id of Array.from(npmBuilt.keys())) {
    if (wanted.has(id)) continue;
    npmBuilt.delete(id);
    npmBare.delete(id);
    npmShape.delete(id);
    moduleDeps.delete(id);
    try {
      rmSync(join(MODULES_DIR, encId(id)));
    } catch {}
  }
  for (const [root, group] of npmGroups) if (!npmBuilt.has(group.id)) npmGroups.delete(root);
  return { added: added.filter((id) => npmBuilt.has(id)), regrouped: regrouped.filter((id) => npmBuilt.has(id)) };
}

const TW_CONFIG = findConfig(appRootArg ? APP_ROOT : APP_SRC_DIR, ID_ROOT);
let twCss: string | null = null;
const TW_HASH = Bun.hash("").toString(16);

async function buildTailwind(): Promise<boolean> {
  if (!TW_CONFIG) return false;
  const t0 = Date.now();
  const css = pruneVendorPrefixes(await generateTailwind(TW_CONFIG));
  moduleDeps.set(TW_ID, []);
  if (css === twCss) return false;
  twCss = css;
  writeAtomic(
    join(MODULES_DIR, encId(TW_ID)),
    [
      `// Skathi module: ${TW_ID} (synthetic — Tailwind utilities, generated)`,
      `// source: ${toId(TW_CONFIG)}; do not edit`,
      `__SK__.setCss(${J(TW_ID)}, ${J(css)});`,
      `__SK__.setDeps && __SK__.setDeps(${J(TW_ID)}, []);`,
      `__SK__.define(${J(TW_ID)}, function (require, module, exports) {`,
      `}, ${J(TW_HASH)});`,
    ].join("\n"),
  );
  say(`${stamp()} tailwind: ${(css.length / 1024).toFixed(1)} KB of utilities in ${Date.now() - t0} ms`);
  return true;
}

type Blocks = { src: string; script: string; template: string; styleShape: string; styleText: string };
type Kind = "reload" | "rerender" | "css-only" | "restart";

const prevBlocks = new Map<string, Blocks>(); // abs path -> blocks

function blocksOf(absPath: string, src: string): Blocks {
  if (!absPath.endsWith(".vue")) {
    return { src, script: src, template: "", styleShape: "", styleText: "" };
  }
  let d: SFCDescriptor;
  try {
    d = parse(src, { filename: absPath }).descriptor;
  } catch {
    return { src, script: src, template: "", styleShape: "", styleText: "" };
  }
  const blockKey = (b: SFCBlock | null) => (b ? JSON.stringify([b.content, b.lang ?? "", b.attrs]) : "");
  return {
    src,
    script: blockKey(d.script) + " " + blockKey(d.scriptSetup),
    template: d.template ? d.template.content : "",
    styleShape: JSON.stringify(d.styles.map((s) => [!!s.scoped, !!s.module, s.lang ?? ""])),
    styleText: d.styles.map((s) => s.content).join(" "),
  };
}

function classify(prev: Blocks | undefined, next: Blocks): Kind | null {
  if (!prev) return "reload"; // new file
  if (prev.src === next.src) return null;
  if (prev.script !== next.script || prev.styleShape !== next.styleShape) return "reload";
  if (prev.template !== next.template) return "rerender";
  if (prev.styleText !== next.styleText) return "css-only";
  return null; // whitespace between blocks etc.
}

function scanSources(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    if (name === "node_modules" || name === "dist" || name.startsWith(".")) continue;
    const abs = join(dir, name);
    const st = statSync(abs);
    if (st.isDirectory()) scanSources(abs, out);
    else if (SOURCE_EXT.test(name)) out.push(abs);
  }
  return out.sort();
}

let rev = 0;
let vendorRev = 0; // content hash of vendor.js, set by buildVendor()

function writeManifest(files: string[]) {
  // Order matters: the host evals modules in order and cssOrder is first-setCss-wins. Tailwind
  // utilities, npm modules, app stylesheets, then component styles; an app stylesheet carrying
  // `@tailwind utilities` makes the synthetic module redundant.
  const wantTw = TW_CONFIG && !appOwnsUtilities();
  const ids = (wantTw ? [TW_ID] : []).concat(cssIds(), npmIds(), files.map(toId));
  const seen = new Set<string>();
  for (const id of ids) {
    const enc = encId(id);
    if (seen.has(enc)) throw new Error(`module filename collision for ${id} -> ${enc}`);
    seen.add(enc);
  }
  const deps: Record<string, string[]> = {};
  for (const id of ids) deps[id] = moduleDeps.get(id) ?? [];
  writeAtomic(
    join(DIST, "manifest.json"),
    JSON.stringify(
      {
        rev,
        vendorRev,
        entry: toId(ENTRY_ABS),
        appRoot: appRootArg ? APP_ROOT : SK_CONFIG_DIR,
        name: readAppName(),
        modules: ids,
        assetFiles: Object.fromEntries(
          [...DEV_ASSET_FILES]
            .filter(([key, path]) => key.startsWith("_vn/fs/") && existsSync(path))
            .map(([key, path]): [string, string] => [key, realpathSync(path)])
            .sort(([a], [b]) => a.localeCompare(b)),
        ),
        deps,
        window: readWindowConfig(),
        launch: readLaunchConfig(),
      },
      null,
      2,
    ) + "\n",
  );
}

function writeChanges(changed: { id: string; kind: Kind }[]) {
  // Written LAST, and it is the file the host polls: once its mtime moves, every module script
  // and the manifest are already on disk.
  writeAtomic(join(DIST, "changes.json"), JSON.stringify({ rev, vendorRev, changed }) + "\n");
}

const stamp = () => `[${new Date().toLocaleTimeString()}]`;

async function fullBuild(reason: string) {
  DEV_ASSET_FILES.clear();
  ensureDirs();
  const t0 = Date.now();
  const bytes = await buildVendor(); // also refreshes vendorRev
  const files = scanSources(APP_SRC_DIR);
  const ids = new Set(files.map(toId));
  prevBlocks.clear();
  moduleDeps.clear();
  moduleBare.clear();
  npmBuilt.clear(); // force a rewrite: the module files may not exist yet
  npmBare.clear();
  npmEntries.clear();
  npmGroups.clear();
  npmShape.clear();
  npmHash.clear();
  appKeys.clear();
  pkgNameAt.clear();
  cssModules.clear();
  cssEmitted.clear();
  moduleInlined.clear();
  SHAPE_NOTES.clear();
  twCss = null; // ditto
  await buildTailwind();
  for (const abs of files) {
    const src = await Bun.file(abs).text();
    await buildModule(abs, ids);
    prevBlocks.set(abs, blocksOf(abs, src));
  }
  await syncNpmModules(); // needs every module's `require`s to be known first
  await syncCssModules(); // ditto: discovered while scanning each module's body
  rev++;
  writeManifest(files);
  writeChanges([]);
  const npmBytes = Array.from(npmBuilt.values()).reduce((a, b) => a + b, 0);
  say(
    `${stamp()} full build (${reason}): vendor ${(bytes / 1024) | 0} KB + ${files.length} modules` +
      (npmBuilt.size ? ` + ${npmBuilt.size} npm (${(npmBytes / 1024) | 0} KB)` : "") +
      (TW_CONFIG ? ` + tailwind (${toId(TW_CONFIG)})` : "") +
      // Cross-package `defineProps<T>()` silently depends on this; say so once per build.
      (TS_AVAILABLE ? " [ts type resolution on]" : "") +
      ` in ${Date.now() - t0} ms`,
  );
  for (const [what, n] of SHAPE_NOTES) say(`${stamp()} ${what}: ${n}`);
  SHAPE_NOTES.clear(); // an incremental pass rebuilds one file: its counts would mislead
}

async function incrementalBuild() {
  ensureDirs();
  const files = scanSources(APP_SRC_DIR);
  const ids = new Set(files.map(toId));
  const changed: { id: string; kind: Kind }[] = [];
  let structural = false;

  for (const abs of files) {
    const src = await Bun.file(abs).text();
    const next = blocksOf(abs, src);
    const isNew = !prevBlocks.has(abs);
    const kind = classify(prevBlocks.get(abs), next) ?? (inlinedMoved(abs) ? "reload" : null);
    prevBlocks.set(abs, next);
    if (!kind) continue;
    if (isNew) structural = true;
    try {
      await buildModule(abs, ids);
    } catch (e) {
      fail(`${stamp()} ${String(e)}`);
      return; // leave dist untouched; the app keeps running the last good build
    }
    changed.push({ id: toId(abs), kind });
  }

  const alive = new Set(files);
  for (const abs of Array.from(prevBlocks.keys())) {
    if (alive.has(abs)) continue;
    prevBlocks.delete(abs);
    moduleDeps.delete(toId(abs));
    moduleBare.delete(toId(abs));
    moduleInlined.delete(abs);
    try {
      rmSync(join(MODULES_DIR, encId(toId(abs))));
    } catch {}
    structural = true;
  }

  const npm = await syncNpmModules();
  for (const id of npm.added) changed.push({ id, kind: "reload" });
  for (const id of npm.regrouped) changed.push({ id, kind: "restart" });
  if (npm.added.length || npm.regrouped.length) structural = true;

  // Always re-run Tailwind: any content file can add a utility, and a stylesheet that went
  // through Tailwind is re-emitted on the same signal.
  for (const id of await syncCssModules()) changed.push({ id, kind: "css-only" });

  if (!changed.length && !structural) return;
  rev++;
  writeManifest(files);
  writeChanges(changed);
  say(
    `${stamp()} ${C.green(`rev ${rev}`)}: ` +
      (changed.map((c) => `${c.kind} ${c.id}`).join(", ") || "module set changed"),
  );
}

await fullBuild("startup").catch((e) => {
  console.error(e);
  if (!WATCH) process.exit(1);
});

if (WATCH) {
  const { watch } = await import("fs");
  let timer: ReturnType<typeof setTimeout> | undefined;
  let vendorTouched = false;
  const schedule = (isRuntime: boolean) => (_evt: string, file: string | null) => {
    const name = String(file ?? "");
    if (name.endsWith(".tmp")) return;
    if (isRuntime) vendorTouched = true;
    clearTimeout(timer);
    timer = setTimeout(async () => {
      const full = vendorTouched;
      vendorTouched = false;
      try {
        if (full) await fullBuild("runtime source changed");
        else await incrementalBuild();
      } catch (e) {
        fail(`${stamp()} ${String(e)}`);
      }
    }, 60);
  };
  watch(APP_SRC_DIR, { recursive: true }, schedule(false));
  watch(RUNTIME_SRC_DIR, { recursive: true }, schedule(true));
  const watched = [APP_SRC_DIR, RUNTIME_SRC_DIR];
  if (TW_CONFIG) {
    // The config lives outside the source tree: one non-recursive watch on its directory,
    // filtered to the config file.
    const configDir = dirname(TW_CONFIG);
    const configName = relative(configDir, TW_CONFIG);
    watch(configDir, { recursive: false }, (evt, file) => {
      if (String(file ?? "") !== configName) return;
      schedule(false)(evt, file);
    });
    watched.push(TW_CONFIG);
  }
  console.log(`${C.green("watching")} ${C.dim(watched.join(", "))}`);
  // SK_OWNER_PID: exit once the process that started the watcher is gone, however it ended (the
  // host does the same, crates/sk-app/src/owner.rs).
  const owner = Number(process.env.SK_OWNER_PID);
  if (Number.isInteger(owner) && owner > 0) {
    setInterval(() => {
      try {
        process.kill(owner, 0);
      } catch (e) {
        if ((e as NodeJS.ErrnoException).code === "ESRCH") process.exit(0);
      }
    }, 1000);
  }
}
