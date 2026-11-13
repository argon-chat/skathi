// Dev builder: two-tier output for Vue HMR.
// Usage: bun build.ts [app-entry.js] [dist-dir] [--watch]
//
//   dist/vendor.js           IIFE: @vue/runtime-core + src/runtime.ts + src/loader.ts.
//   dist/modules/<enc>.js    one classic script per app module (setCss / setDeps / define);
//                            <enc> is the id with "/" -> "__".
//   dist/manifest.json       { rev, vendorRev, entry, modules, deps }
//   dist/changes.json        { rev, vendorRev, changed: [{ id, kind }] }
//
// Module ids are workspace-relative POSIX paths ("apps/hello/src/App.vue").
//
// - The module graph (setDeps) lets a shared plain .js file hot-reload: the loader walks importers
//   up to the nearest .vue components. The loader never reads manifest.json, only the host does.
// - bodyHash hashes the factory body only, so a <style>-only edit keeps the live exports.
// - Tailwind rides as a synthetic css-only module (.vn/tailwind.css, sorts first); a new class
//   emits a "css-only" change with no component churn.
// - npm dependencies are one shared synthetic module per package entry, with other packages left
//   external. Inlining per importer breaks libraries with module-scope identity (reka-ui's
//   Symbol() injection keys), so inject would miss. vue, vue-native, vue-router and pinia stay
//   external so libraries land on our runtime.
// - change kind: script or <style> scoped/module/lang shape changed -> "reload"; template ->
//   "rerender" (state survives); <style> text only -> "css-only"; non-.vue source -> "reload".
//
// Forced compiler settings:
// - hoistStatic off: the host has no insertStaticContent.
// - inlineTemplate off: an inline render lives inside setup(), so rerender could not swap it
//   without re-running setup and losing state.
// - the scope id hashes the module id only, so [data-v-x] is stable across edits.

import { parse, compileScript, compileTemplate, compileStyle, registerTS } from "@vue/compiler-sfc";
import type { SFCDescriptor } from "@vue/compiler-sfc";
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

import {
  TW_ID,
  declaresUtilities,
  findConfig,
  generate as generateTailwind,
  needsTailwindPass,
} from "./tailwind";
import { pruneVendorPrefixes } from "./css-prefixes";
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

// `--define K=V` takes two argv slots; pulled out first so its value is not
// mistaken for the entry path.
const { defines: cliDefines, rest: argv } = takeDefineArgs(process.argv.slice(2));
const positional = argv.filter((a) => !a.startsWith("--"));
const WATCH = argv.includes("--watch");

const PKG_DIR = import.meta.dir; // .../packages/vue-native
const WS_ROOT = resolve(PKG_DIR, "../.."); // repo root — module ids are relative to this
const RUNTIME_SRC_DIR = join(PKG_DIR, "src");
const LOADER_ENTRY = join(RUNTIME_SRC_DIR, "loader.ts");

const ENTRY_ABS = resolve(positional[0] ?? join(WS_ROOT, "apps/hello/src/main.ts"));
const APP_SRC_DIR = dirname(ENTRY_ABS);
const DIST = resolve(positional[1] ?? join(PKG_DIR, "dist"));
const MODULES_DIR = join(DIST, "modules");

const SOURCE_EXT = /\.(vue|js|ts|mjs)$/;

// The window block of vn.config.json is copied verbatim into the manifest: the host reads it
// before opening the window, the only time creation-time window options can be set without a flash.
const VN_CONFIG_DIR = APP_SRC_DIR.endsWith(`${sep}src`) ? dirname(APP_SRC_DIR) : APP_SRC_DIR;
const VN_CONFIG_PATH = join(VN_CONFIG_DIR, "vn.config.json");

function readVnConfig(): Record<string, unknown> | undefined {
  if (!existsSync(VN_CONFIG_PATH)) return undefined;
  try {
    const cfg = JSON.parse(readFileSync(VN_CONFIG_PATH, "utf8"));
    return cfg && typeof cfg === "object" ? cfg : undefined;
  } catch (e) {
    // A malformed config must not fail the build, but must not be silently ignored either.
    console.error(`[build] ${VN_CONFIG_PATH}: ${e}`);
    return undefined;
  }
}

// Top-level "devtools": false goes inside the window block, the only JSON the host reads before
// the window exists (it decides whether a devtools listener is bound at all). A window block
// that sets it itself wins.
function foldDevtoolsFlag(
  cfg: Record<string, unknown> | undefined,
  win: Record<string, unknown> | undefined,
): Record<string, unknown> | undefined {
  if (!cfg || typeof cfg.devtools !== "boolean") return win;
  if (win && typeof win.devtools === "boolean") return win;
  return { ...(win ?? {}), devtools: cfg.devtools };
}

// Top-level "httpCache" goes in the same block: the host installs the cache before any JS runs.
// Same precedence as the devtools flag.
function foldHttpCache(
  cfg: Record<string, unknown> | undefined,
  win: Record<string, unknown> | undefined,
): Record<string, unknown> | undefined {
  if (!cfg || cfg.httpCache === undefined) return win;
  if (win && win.httpCache !== undefined) return win;
  return { ...(win ?? {}), httpCache: cfg.httpCache };
}

function readWindowConfig(): Record<string, unknown> | undefined {
  const cfg = readVnConfig();
  const win =
    cfg && typeof cfg.window === "object" && cfg.window
      ? (cfg.window as Record<string, unknown>)
      : undefined;
  return foldHttpCache(cfg, foldDevtoolsFlag(cfg, win));
}

// The app name is its storage folder identity (%LOCALAPPDATA%\<name>\storage), so it is not the
// window title. Optional: defaults to the entry module's app directory.
function readAppName(): string | undefined {
  const cfg = readVnConfig();
  const name = cfg?.name;
  return typeof name === "string" && name.trim() ? name.trim() : undefined;
}

// Launch keys (id, singleInstance, raiseOnSecondInstance, protocols) from the top level of
// vn.config.json, gathered for the host, which reads them before anything else. Only well-typed
// keys are copied; `vn build` is where a wrong one is an error.
function readLaunchConfig(): Record<string, unknown> | undefined {
  const cfg = readVnConfig();
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
      console.error(`[build] ${VN_CONFIG_PATH}: "${key}" has the wrong type and was ignored`);
    }
  }
  return Object.keys(out).length ? out : undefined;
}

const toId = (abs: string) => relative(WS_ROOT, abs).split(sep).join("/");
const encId = (id: string) => id.split("/").join("__") + ".js";
const J = (v: unknown) => JSON.stringify(v);

// The app's own package root: bare specifiers and `typescript` resolve from here, never from the
// builder's node_modules.
function findAppRoot(startDir: string, stopDir: string): string {
  let dir = startDir;
  for (;;) {
    if (existsSync(join(dir, "package.json")) || existsSync(join(dir, "node_modules"))) return dir;
    if (dir === stopDir) return startDir;
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
const appRequire = createRequire(join(APP_ROOT, "package.json"));
const runtimeRequire = createRequire(join(PKG_DIR, "package.json"));
const APP_PKG = APP_ROOT === PKG_DIR ? null : readPackageJson(APP_ROOT);
const FRAMEWORK = new Map(
  ["vue-router", "pinia"].map((name) => [name, frameworkDep(
    name, APP_PKG, (spec) => appRequire.resolve(spec), (spec) => runtimeRequire.resolve(spec),
  )] as const),
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
 * (`require` before `import`: reka-ui's ESM entry breaks under Bun's CJS output; node builds of
 * axios and consola fail at runtime), then Node's CJS entry, then Bun's. A candidate whose build
 * fails falls through to the next.
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
    // A Node builtin (`crypto`) resolves to its own name: nothing to bundle.
    if (isAbsolute(cjs) && !out.includes(cjs)) out.push(cjs);
  } catch {
    /* ESM-only, or no `main` — Bun's resolver is the answer */
  }
  try {
    const esm = Bun.resolveSync(spec, from);
    if (isAbsolute(esm) && !out.includes(esm)) out.push(esm);
  } catch {
    /* nothing more to try */
  }
  return out;
}

/** The file Bun's bundler resolves `spec` to from `from` for a browser build, as a require or an import. */
async function bundlerResolve(spec: string, from: string, kind: "require" | "import"): Promise<string | null> {
  const probe = join(from, "__vn_resolve_probe__.js"); // never written: onLoad answers for it
  let hit: string | null = null;
  await Bun.build({
    entrypoints: [probe],
    target: "browser",
    format: "cjs" as any,
    plugins: [
      {
        name: "vn-resolve-probe",
        setup(b: any) {
          b.onResolve({ filter: /__vn_resolve_probe__\.js$/ }, () => ({ path: probe }));
          b.onLoad({ filter: /.*/ }, (args: any) => {
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
// resolution loads typescript from the app's node_modules, so apps that do not need it stay
// dependency-free). SFC_FS (vitecompat.ts) answers fileExists for files only: compiler-sfc
// probes ./types before ./types.ts.

const TS_AVAILABLE = (() => {
  try {
    const ts = appRequire("typescript");
    registerTS(() => ts);
    return true;
  } catch {
    return false;
  }
})();

// Provided by vendor.js and pre-seeded in the loader's registry: always external (a second Vue
// would be a second reactivity system), never emitted as modules.
const PRESEEDED = ["vue", "vue-native", "vue-router", "pinia"];
const isPreseeded = (spec: string) => PRESEEDED.includes(spec);

// vn.config.json "aliases" apply to `vn build` only (prod.ts appAliases).
{
  const aliases = readVnConfig()?.aliases;
  if (aliases && typeof aliases === "object" && Object.keys(aliases).length) {
    console.warn(`[build] vn.config.json "aliases" (${Object.keys(aliases).join(", ")}) are applied by vn build only; this dev build resolves them from node_modules`);
  }
}

// The app's defines: vn.config.json "define" plus `--define K=V`, with Vite semantics. Read once
// at startup.
const APP_DEFINES = appDefines(readVnConfig()?.define, cliDefines, VN_CONFIG_PATH);
if (APP_DEFINES.errors.length) {
  for (const e of APP_DEFINES.errors) console.error(`[build] ${e}`);
  process.exit(1);
}

/** What the Vite dev server inlines for `import.meta.env`, plus `VN: true` and the app's own defines. */
const VITE_ENV_DEV: Record<string, unknown> = {
  MODE: "development",
  DEV: true,
  PROD: false,
  SSR: false,
  BASE_URL: "/",
  VN: true,
};
for (const [k, v] of Object.entries(APP_DEFINES.defines)) {
  if (!k.startsWith("import.meta.env.")) continue;
  try {
    VITE_ENV_DEV[k.slice("import.meta.env.".length)] = JSON.parse(v);
  } catch {
    /* an identifier value: it reads right through the dotted define, not the object */
  }
}

// Vue's esm-bundler build reads these as bare identifiers.
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
  // Dotted forms come first (Bun matches the longest key); bare `import.meta` catches
  // destructuring. The object values are JSON.stringify'd: Bun parses a define value as JSON or an
  // identifier and inserts anything else as a string.
  "import.meta.env.MODE": '"development"',
  "import.meta.env.DEV": "true",
  "import.meta.env.PROD": "false",
  "import.meta.env.SSR": "false",
  "import.meta.env.BASE_URL": '"/"',
  // `true` in every vue-native build, undefined under Vite: apps gate browser-only code with
  // `if (!import.meta.env.VN)` and Bun drops the branch while parsing.
  "import.meta.env.VN": "true",
  "import.meta.env": JSON.stringify(VITE_ENV_DEV),
  // Library code guards on it; undefined means "not a dev server" (our HMR rides __VUE_HMR_RUNTIME__).
  "import.meta.hot": "undefined",
  "import.meta.url": '"vn://module"',
  // import.meta.glob calls are expanded earlier; any left is defined to a findable name so
  // buildModule can fail with a sentence.
  "import.meta.glob": GLOB_TRIPWIRE,
  "import.meta": JSON.stringify({ env: VITE_ENV_DEV, url: "vn://module" }),
  ...APP_DEFINES.defines,
};

function writeAtomic(path: string, text: string) {
  const tmp = path + ".tmp";
  writeFileSync(tmp, text);
  renameSync(tmp, path);
}

function ensureDirs() {
  mkdirSync(MODULES_DIR, { recursive: true });
}

async function buildVendor() {
  for (const [name, dep] of FRAMEWORK) {
    if (dep.warning) devAdvise(dep.warning);
    console.log(`${stamp()} ${name} ${dep.version ?? "unknown"}: the ${dep.from === "app" ? "app's" : "runtime's"} copy`);
  }
  const result = await Bun.build({
    entrypoints: [LOADER_ENTRY],
    format: "iife",
    target: "browser",
    define: DEFINES,
    plugins: [
      {
        // vue-router and others import from "vue": that must be our runtime. Exact-match filter only:
        // bun 1.4 crashes on catch-all onResolve filters.
        name: "vendor-vue-alias",
        setup(b: any) {
          b.onResolve({ filter: /^vue$/ }, () => ({ path: join(RUNTIME_SRC_DIR, "runtime.ts") }));
          // The app owns these packages in both dev and prod; mixing majors
          // breaks store/router identity before an application module runs.
          b.onResolve({ filter: /^(vue-router|pinia)$/ }, (args: any) => ({
            path: FRAMEWORK.get(args.path)!.path,
          }));
        },
      },
    ],
  });
  if (!result.success) {
    for (const log of result.logs) console.error(log);
    throw new Error("vendor build failed");
  }
  const text = await result.outputs[0].text();
  // The pre-seeded "vue" namespace must still carry the compiler helpers; if Bun tree-shakes the
  // `export *` re-export, templates die with "not a function".
  for (const sym of ["openBlock", "createElementBlock", "createElementVNode", "toDisplayString"]) {
    if (!text.includes(sym)) throw new Error(`vendor bundle is missing ${sym}`);
  }
  writeAtomic(join(DIST, "vendor.js"), text);
  // vendorRev is a CONTENT hash, not a counter: restarting the watcher must
  // not make a running host believe its already-evaluated vendor.js is current.
  vendorRev = Number(BigInt.asUintN(48, BigInt(Bun.hash(text) as any)));
  return text.length;
}

/** Stable per-file scope id: must not depend on file contents. */
const scopeIdOf = (id: string) => Bun.hash(id).toString(16).slice(0, 8);

/** Filled by the .vue onLoad hook; each Bun.build below handles exactly one file. */
let capturedCss: string | null = null;

const devAdvised = new Set<string>();
function devAdvise(m: string) {
  if (devAdvised.has(m)) return;
  devAdvised.add(m);
  console.warn(`${stamp()} advisory: ${m}`);
}

/**
 * A stylesheet through Vite's CSS pipeline: local @imports inlined, then the app's Tailwind when
 * the text uses it. A failed Tailwind pass logs and keeps the inlined text.
 */
async function viteCss(text: string, from: string, label: string): Promise<string> {
  const css = inlineCssImports(text, from, VN_CONFIG_DIR, devAdvise);
  if (!TW_CONFIG || !needsTailwindPass(css)) return css;
  try {
    return await generateTailwind(TW_CONFIG, { text: css, from });
  } catch (e) {
    console.error(`${stamp()} ${label}: tailwind pass failed — emitting the source verbatim`, e);
    return css;
  }
}

/** `captureCss` false: an SFC compiled inside another module's build must not replace its captured style. */
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
  let bindings: any;

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
        bindingMetadata: bindings,
        // `{{ x as string }}` / `v-bind="p as any"` parse only with this on.
        expressionPlugins: isTS ? ["typescript"] : [],
        // vue-native host elements missing from the HTML tag list: otherwise the SFC compiler treats
        // them as components and warns on every render. Keep in lockstep with prod.ts and
        // crates/vn-tree/src/render.rs.
        isCustomElement: (tag: string) => tag === "lottie" || tag.startsWith("vn-"),
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
  chunks.push(
    `if (typeof __VUE_HMR_RUNTIME__ !== "undefined") __VUE_HMR_RUNTIME__.createRecord(${J(id)}, _sfc_main);`,
  );
  chunks.push("export default _sfc_main;");

  const cssChunks: string[] = [];
  for (const style of descriptor.styles) {
    let text = style.content;
    // Vite's order: CSS pipeline first, so an @imported sheet is scoped too.
    if (!style.lang || style.lang === "css" || style.lang === "postcss") {
      text = await viteCss(text, absPath, `${id} <style>`);
    } else {
      devAdvise(`${id}: <style lang="${style.lang}"> is not preprocessed by vue-native — it is compiled as plain CSS`);
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
      scoped: style.scoped,
    });
    if (out.errors.length) throw new Error(`${id} <style>: ${out.errors.join("; ")}`);
    cssChunks.push(out.code);
  }
  if (captureCss) capturedCss = cssChunks.length ? pruneVendorPrefixes(cssChunks.join("\n")) : null;

  return { contents: chunks.join("\n"), loader: isTS ? "ts" : "js" };
}

const vuePlugin = {
  name: "vue-sfc",
  setup(build: any) {
    build.onLoad({ filter: /\.vue$/ }, async (args: any) => {
      const source = await Bun.file(args.path).text();
      return compileVue(args.path, source);
    });
  },
};

// Vite import shapes (vitecompat.ts): the plugin prod.ts bundles with, so ?raw, ?url, assets,
// .svg, ?worker, .wasm and import.meta.glob mean the same in both builders. In dev,
// import.meta.glob imports by importer-relative path.

const SHAPE_NOTES = new Map<string, number>();
let inlinedNow: Set<string> | null = null;
const noteInlined = (abs: string) => void inlinedNow?.add(abs);

const DEV_ASSET_FILES = new Map<string, string>();
const shapesOptions = (dev: boolean) => ({
  docRoot: VN_CONFIG_DIR,
  wsRoot: WS_ROOT,
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

/** An app module: the SFC compile, then the rewrites prod applies to a component script (`transformSource`). */
const appModulePlugin = {
  name: "vn-dev-app-module",
  setup(build: any) {
    build.onLoad({ filter: /\.vue$/ }, async (args: any) => {
      const out = await compileVue(args.path, await Bun.file(args.path).text());
      const code = transformSource(out.contents, args.path, DEV_SHAPES);
      return code === null ? out : { ...out, contents: code };
    });
  },
};

// Every module is built with external:"*", so each import survives as a literal require("<spec>")
// and deps are read from the emitted body. Pre-seeded bare specifiers and npm packages stay out
// of the graph: they never change, so they add nothing to the HMR importer walk.

const REQUIRE_RE = /\brequire\(\s*("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')\s*\)/g;

/** Same candidate list as loader.ts `resolveId` — keep the two in lockstep. */
const CANDIDATE_EXTS = ["", ".js", ".ts", ".mjs", ".vue", "/index.js", "/index.ts", "/index.vue"];

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
    const spec = unquote(m[1]);
    if (spec === null || spec.charCodeAt(0) !== 46 /* . */) continue; // bare -> vendor
    if (skip.has(spec)) continue; // a stylesheet: its own synthetic module
    const base = normalizeId((dir ? dir + "/" : "") + spec);
    const hit = CANDIDATE_EXTS.map((ext) => base + ext).find((cand) => ids.has(cand));
    if (!hit) {
      // The loader would throw the same way; say so now, while there is a file to point at.
      console.warn(
        `${stamp()} ${id}: import ${J(spec)} resolves to no built module — ` +
          `require() will throw at runtime`,
      );
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
    const spec = unquote(m[1]);
    if (spec === null) continue;
    const c = spec.charCodeAt(0);
    if (c === 46 /* . */ || c === 47 /* / */) continue; // relative/absolute -> graph
    if (isPreseeded(spec) || found.includes(spec) || skip.has(spec)) continue;
    found.push(spec);
  }
  return found;
}

const moduleDeps = new Map<string, string[]>(); // id -> dep ids
const moduleBare = new Map<string, string[]>(); // app module id -> npm module ids it requires

// CSS entries: a Vite app imports its global stylesheet from the entry module. Emitted as a
// css-only synthetic module (the Tailwind transport), not verbatim into a JS body.

const cssModules = new Map<string, string>();
const cssEmitted = new Map<string, string>();

/**
 * Registers the stylesheets `body` imports and returns their ids. A relative import is a
 * workspace-relative path, a bare one is the specifier verbatim (as the loader resolves them).
 */
function cssImportsOf(absPath: string, body: string, cssSpecs: Set<string>): string[] {
  const dir = dirname(absPath);
  const found: string[] = [];
  REQUIRE_RE.lastIndex = 0;
  for (let m = REQUIRE_RE.exec(body); m; m = REQUIRE_RE.exec(body)) {
    const spec = unquote(m[1]);
    if (spec === null) continue;
    const relative_ = spec.charCodeAt(0) === 46 /* . */;
    // A bare specifier can name a stylesheet through its `exports` map
    // (`@argon/assets/styles`), so the extension test is on the RESOLVED path.
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
      console.warn(`${stamp()} ${toId(absPath)}: import ${J(spec)} names no file — skipped`);
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
 * Writes one CSS-only module script. Returns true when the sheet moved. With a Tailwind config the
 * sheet goes through Tailwind with itself as the entry, as in the app's Vite build; otherwise it
 * is emitted byte-for-byte.
 */
async function buildCssModule(id: string): Promise<boolean> {
  const abs = cssModules.get(id)!;
  // Local @imports inlined first, as postcss-import does ahead of Tailwind.
  let css = inlineCssImports(readFileSync(abs, "utf-8"), abs, VN_CONFIG_DIR, devAdvise);
  if (TW_CONFIG && needsTailwindPass(css)) {
    const t0 = Date.now();
    try {
      css = await generateTailwind(TW_CONFIG, { text: css, from: abs });
      console.log(
        `${stamp()} css ${id}: ${(css.length / 1024).toFixed(1)} KB through tailwind in ${Date.now() - t0} ms`,
      );
    } catch (e) {
      console.error(`${stamp()} css ${id}: tailwind pass failed — emitting the source verbatim`, e);
    }
  } else {
    console.log(`${stamp()} css ${id}: ${(css.length / 1024).toFixed(1)} KB`);
  }
  // Without the prefixed declarations vn-css would drop (css-prefixes.ts).
  css = pruneVendorPrefixes(css);
  moduleDeps.set(id, []);
  if (cssEmitted.get(id) === css) return false;
  cssEmitted.set(id, css);
  writeAtomic(
    join(MODULES_DIR, encId(id)),
    [
      `// vue-native module: ${id} (synthetic — CSS entry)`,
      `// source: ${toId(abs)}; do not edit`,
      `__VN__.setCss(${J(id)}, ${J(css)});`,
      `__VN__.setDeps && __VN__.setDeps(${J(id)}, []);`,
      // Empty factory: `import "./index.css"` is a side effect, and the
      // constant body hash keeps a CSS-only edit from invalidating anything.
      `__VN__.define(${J(id)}, function (require, module, exports) {`,
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

/** True when an app stylesheet already carries `@tailwind utilities`: the synthetic module would duplicate it. */
const appOwnsUtilities = () =>
  Array.from(cssEmitted.keys()).some((id) => {
    const abs = cssModules.get(id);
    return !!abs && declaresUtilities(readFileSync(abs, "utf-8"));
  });

// ?raw, .json and .svg components are built into their importer; incrementalBuild rebuilds it
// when one moves.

/** module abs path -> the files its build inlined -> their mtime then. */
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

/** ESM -> CJS for one file, everything else external, wrapped in __VN__.define. */
async function buildModule(absPath: string, ids: Set<string>) {
  const id = toId(absPath);
  capturedCss = null;
  const inlined = (inlinedNow = new Set());
  let result: Awaited<ReturnType<typeof Bun.build>>;
  try {
    result = await Bun.build({
      entrypoints: [absPath],
      format: "cjs" as any,
      target: "node",
      // Transform rather than bundle: imports stay require() calls except what the plugins resolve
      // or inline.
      external: ["*"],
      plugins: [devShapesPlugin, devImports, appModulePlugin],
      define: DEFINES,
    });
  } finally {
    inlinedNow = null;
  }
  if (!result.success) {
    for (const log of result.logs) console.error(log);
    throw new Error(`module build failed: ${id}`);
  }
  moduleInlined.set(absPath, new Map(Array.from(inlined, (f) => [f, mtimeOf(f)])));
  // Bundler ESM semantics rather than Node's (vitecompat.ts devEsmInterop); an inlined import()
  // namespace is Module-tagged, as in prod.
  const interop = devEsmInterop(tagModuleNamespaces(await result.outputs[0].text()).code);
  const body = interop.code;
  if (interop.unresolved) {
    console.warn(
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

  // Stylesheets first: their specifiers must be known before depsOf/bareOf classify the rest.
  const cssSpecs = new Set<string>();
  const css = cssImportsOf(absPath, body, cssSpecs);
  const deps = depsOf(id, body, ids, cssSpecs).concat(css);
  moduleDeps.set(id, deps);
  moduleBare.set(id, bareOf(body, cssSpecs));

  const out: string[] = [`// vue-native module: ${id}`];
  if (capturedCss !== null) out.push(`__VN__.setCss(${J(id)}, ${J(capturedCss)});`);
  // Guarded: an older vendor.js may read this script; a missing setDeps must degrade to no graph.
  out.push(`__VN__.setDeps && __VN__.setDeps(${J(id)}, ${J(deps)});`);
  out.push(`__VN__.define(${J(id)}, function (require, module, exports) {`);
  out.push(body);
  out.push(`}, ${J(Bun.hash(body).toString(16))});`);
  writeAtomic(join(MODULES_DIR, encId(id)), out.join("\n"));
}

// npm modules: one per package ENTRY, keyed on the resolved directory (not the name), so every
// importer of the same directory shares it and a nested copy at another version stays separate.
// A package's own npm imports are external (npmSharePlugin); inlined copies broke module-scope
// registries and injection keys.
// The id is the specifier when the app resolves it to that directory, else `<spec>~<hash of the
// directory>`; both are stable across rebuilds.

/** module id -> emitted body size, for the log and for change detection. */
const npmBuilt = new Map<string, number>();
/** Ids we already complained about, so watch mode says it once. */
const npmFailed = new Set<string>();
const npmEntries = new Map<string, { spec: string; from: string }>();
const npmBare = new Map<string, string[]>();

/** `@scope/pkg/sub` -> ["@scope/pkg", "./sub"]; `pkg` -> ["pkg", "."]. */
function splitSpec(spec: string): [string, string] {
  const parts = spec.split("/");
  const n = spec.startsWith("@") ? 2 : 1;
  const rest = parts.slice(n).join("/");
  return [parts.slice(0, n).join("/"), rest ? `./${rest}` : "."];
}

const pkgNameAt = new Map<string, string | null>();
/** The directory of the nearest package.json named `name` above `file` (a nested `{"type":"module"}` one does not count). */
function packageRootOf(file: string, name: string): string | null {
  for (let dir = dirname(file); ; ) {
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

/** What makes two imports one module: the package directory and the subpath, or the file when no package claims it. */
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
  if (!npmEntries.has(id)) npmEntries.set(id, { spec, from: id === spec ? APP_ROOT : from });
  return id;
}

/** Inside an npm bundle another package's import is external under its id (`npmIdOf`) and recorded in `found`. */
function npmSharePlugin(selfKey: string, found: Set<string>) {
  return {
    name: "vn-dev-npm-share",
    setup(build: any) {
      // `node:` stays out of the filter (see vitecompat.ts devImportsPlugin).
      build.onResolve({ filter: /^(?!node:)[^./\\]/ }, (args: any) => {
        if (args.kind === "entry-point-build" || isAbsolute(args.path) || isPreseeded(args.path)) return undefined;
        const from = importerDir(args, APP_ROOT);
        const abs = resolveImport(args.path, from);
        if (abs === null || !isAbsolute(abs) || /\.css$/i.test(abs) || LEAF_IMPORT.test(abs)) return undefined;
        if (npmKey(args.path, abs) === selfKey) return undefined;
        const id = npmIdOf(args.path, abs, from);
        found.add(id);
        return { path: id, external: true };
      });
    },
  };
}

/** Bundles one npm package entry into a classic module script. False = unresolvable. */
async function buildNpmModule(id: string): Promise<boolean> {
  // Resolved from the APP root, not ours: the app owns its dependencies. An
  // entry only another package reaches is resolved from that package.
  const { spec, from } = npmEntries.get(id) ?? { spec: id, from: APP_ROOT };
  const candidates = await entryCandidates(spec, from);
  if (!candidates.length) {
    if (!npmFailed.has(id)) {
      npmFailed.add(id);
      // `@/x`, `~/x`, `#x`: no npm package is named like that, so it is an
      // alias neither builder knows. Both read tsconfig `paths` only.
      const alias = !/^(?:@[a-z0-9][\w.-]*\/)?[a-z0-9][\w.-]*(?:\/|$)/i.test(spec);
      console.warn(
        `${stamp()} cannot resolve ${J(spec)} from ${toId(from)} — ` +
          `require(${J(id)}) will throw at runtime ` +
          (alias
            ? "(an import alias resolves through tsconfig.json compilerOptions.paths, as in vn build; " +
              "a vite.config resolve.alias is not read)"
            : "(is it installed?)"),
      );
    }
    return false;
  }
  const selfKey = npmKey(spec, candidates[0]);
  const requires = new Set<string>();
  let result: Awaited<ReturnType<typeof Bun.build>> | null = null;
  let entry = candidates[0];
  for (const [i, candidate] of candidates.entries()) {
    entry = candidate;
    requires.clear();
    result = await Bun.build({
      entrypoints: [candidate],
      format: "cjs" as any,
      target: "browser", // picks the browser/ESM conditions, not node builtins
      // Pre-seeded ids plus every other package entry stay external; what is left is this package's own
      // code. A dependency can ship raw .vue source: without the SFC plugin Bun treats it as an asset
      // and the import becomes a file-path string, silently rendering nothing. Vite import shapes go
      // first, as in prod.ts.
      plugins: [npmShapesPlugin, npmSharePlugin(selfKey, requires), vuePlugin],
      define: DEFINES,
    }).catch(() => null);
    if (result?.success) break;
    if (i + 1 < candidates.length) {
      console.warn(
        `${stamp()} npm ${id}: ${toId(candidate)} did not build — retrying with ` +
          `${toId(candidates[i + 1])}`,
      );
    }
  }
  if (!result?.success) {
    for (const log of result?.logs ?? []) console.error(log);
    throw new Error(`npm module build failed: ${id}`);
  }
  const body = tagModuleNamespaces(await result.outputs[0].text()).code;
  moduleDeps.set(id, []);
  // What the plugin made external, not a scan of the body: a package's own
  // guarded `require("crypto")` is not a module to build.
  npmBare.set(id, Array.from(requires).sort());
  writeAtomic(
    join(MODULES_DIR, encId(id)),
    [
      `// vue-native module: ${id} (npm dependency, bundled from ${toId(entry)})`,
      `__VN__.setDeps && __VN__.setDeps(${J(id)}, []);`,
      `__VN__.define(${J(id)}, function (require, module, exports) {`,
      body,
      // Constant unless the package changes, so a rebuild never invalidates live exports (module-scope
      // Symbols and singletons).
      `}, ${J(Bun.hash(body).toString(16))});`,
    ].join("\n"),
  );
  npmBuilt.set(id, body.length);
  return true;
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

/** Reconciles emitted npm modules with current imports. Returns the ids ADDED (they need a change entry so the host evaluates them before hmrApply). */
async function syncNpmModules(): Promise<string[]> {
  const added: string[] = [];
  const tried = new Set<string>();
  // A package's build finds the packages it requires: build until none is new.
  for (;;) {
    const todo = Array.from(wantedNpm())
      .filter((id) => !npmBuilt.has(id) && !tried.has(id))
      .sort();
    if (!todo.length) break;
    for (const id of todo) {
      tried.add(id);
      const t0 = Date.now();
      if (!(await buildNpmModule(id))) continue;
      added.push(id);
      console.log(`${stamp()} npm ${id}: ${((npmBuilt.get(id) ?? 0) / 1024) | 0} KB in ${Date.now() - t0} ms`);
    }
  }

  const wanted = wantedNpm();
  for (const id of Array.from(npmBuilt.keys())) {
    if (wanted.has(id)) continue;
    npmBuilt.delete(id);
    npmBare.delete(id);
    moduleDeps.delete(id);
    try {
      rmSync(join(MODULES_DIR, encId(id)));
    } catch {}
  }
  return added.filter((id) => npmBuilt.has(id));
}

const TW_CONFIG = findConfig(appRootArg ? APP_ROOT : APP_SRC_DIR, WS_ROOT);
let twCss: string | null = null;
/** Constant: the factory body never changes, so define() never invalidates. */
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
      `// vue-native module: ${TW_ID} (synthetic — Tailwind utilities, generated)`,
      `// source: ${toId(TW_CONFIG)}; do not edit`,
      `__VN__.setCss(${J(TW_ID)}, ${J(css)});`,
      `__VN__.setDeps && __VN__.setDeps(${J(TW_ID)}, []);`,
      `__VN__.define(${J(TW_ID)}, function (require, module, exports) {`,
      `}, ${J(TW_HASH)});`,
    ].join("\n"),
  );
  console.log(
    `${stamp()} tailwind: ${(css.length / 1024).toFixed(1)} KB of utilities in ${Date.now() - t0} ms`,
  );
  return true;
}

type Blocks = { src: string; script: string; template: string; styleShape: string; styleText: string };
type Kind = "reload" | "rerender" | "css-only";

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
  const blockKey = (b: any) => (b ? JSON.stringify([b.content, b.lang ?? "", b.attrs ?? {}]) : "");
  return {
    src,
    script: blockKey(d.script) + " " + blockKey(d.scriptSetup),
    template: d.template ? d.template.content : "",
    // scoped/module/lang changes alter codegen or the scope id -> full reload
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
  // Order matters: the host evals modules in order and the loader's cssOrder is first-setCss-wins.
  // Tailwind utilities first, then npm modules (registered before the entry runs), then the app's
  // own stylesheets, then component styles. An app stylesheet carrying `@tailwind utilities` makes
  // the synthetic module redundant, so it is left out.
  const wantTw = TW_CONFIG && !appOwnsUtilities();
  const ids = (wantTw ? [TW_ID] : []).concat(cssIds(), npmIds(), files.map(toId));
  const seen = new Set<string>();
  for (const id of ids) {
    const enc = encId(id);
    if (seen.has(enc)) throw new Error(`module filename collision for ${id} -> ${enc}`);
    seen.add(enc);
  }
  // `deps` is additive: the host reads only vendorRev/entry/modules; the loader gets the edges
  // from setDeps().
  const deps: Record<string, string[]> = {};
  for (const id of ids) deps[id] = moduleDeps.get(id) ?? [];
  writeAtomic(
    join(DIST, "manifest.json"),
    JSON.stringify(
      {
        rev,
        vendorRev,
        entry: toId(ENTRY_ABS),
        name: readAppName(),
        modules: ids,
        assetFiles: Object.fromEntries([...DEV_ASSET_FILES].filter(([key, path]) => key.startsWith("_vn/fs/") && existsSync(path)).map(([key, path]) => [key, realpathSync(path)]).sort(([a], [b]) => a.localeCompare(b))),
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
  // Written LAST, and it is the file the host polls: by the time its mtime
  // moves, every module script and the manifest are already on disk.
  writeAtomic(
    join(DIST, "changes.json"),
    JSON.stringify({ rev, vendorRev, changed }) + "\n",
  );
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
  // Empty change list: if vendorRev actually moved the host cold-restarts,
  // otherwise this is a no-op ping (e.g. a runtime edit that did not alter the
  // emitted vendor bundle).
  writeChanges([]);
  const npmBytes = Array.from(npmBuilt.values()).reduce((a, b) => a + b, 0);
  console.log(
    `${stamp()} full build (${reason}): vendor ${(bytes / 1024) | 0} KB + ${files.length} modules` +
      (npmBuilt.size ? ` + ${npmBuilt.size} npm (${(npmBytes / 1024) | 0} KB)` : "") +
      (TW_CONFIG ? ` + tailwind (${toId(TW_CONFIG)})` : "") +
      // Cross-package `defineProps<T>()` silently depends on this; say so once
      // per build instead of leaving a confusing compile error as the only clue.
      (TS_AVAILABLE ? " [ts type resolution on]" : "") +
      ` in ${Date.now() - t0} ms`,
  );
  for (const [what, n] of SHAPE_NOTES) console.log(`${stamp()} ${what}: ${n}`);
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
    // An unchanged source whose inlined `.json` / `?raw` / `.svg` moved is rebuilt.
    const kind = classify(prevBlocks.get(abs), next) ?? (inlinedMoved(abs) ? "reload" : null);
    prevBlocks.set(abs, next);
    if (!kind) continue;
    if (isNew) structural = true;
    try {
      await buildModule(abs, ids);
    } catch (e) {
      console.error(`${stamp()} ${String(e)}`);
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

  // A new package script must be evaluated before hmrApply re-requires the importer, so it rides
  // in `changed`.
  for (const spec of await syncNpmModules()) {
    changed.push({ id: spec, kind: "reload" });
    structural = true;
  }

  // Always re-run Tailwind: the class list can change with any content file, and a new utility
  // arrives as its own css-only entry. A stylesheet that went through Tailwind is re-emitted on the
  // same signal.
  for (const id of await syncCssModules()) changed.push({ id, kind: "css-only" });

  if (!changed.length && !structural) return;
  rev++;
  writeManifest(files);
  writeChanges(changed);
  console.log(
    `${stamp()} rev ${rev}: ` +
      (changed.map((c) => `${c.kind} ${c.id}`).join(", ") || "module set changed"),
  );
}

await fullBuild("startup").catch((e) => {
  console.error(e);
  if (!WATCH) process.exit(1);
});

if (WATCH) {
  const { watch } = await import("fs");
  let timer: any = null;
  let vendorTouched = false;
  const schedule = (isRuntime: boolean) => (_evt: any, file: any) => {
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
        console.error(`${stamp()} ${String(e)}`);
      }
    }, 60);
  };
  watch(APP_SRC_DIR, { recursive: true }, schedule(false));
  watch(RUNTIME_SRC_DIR, { recursive: true }, schedule(true));
  const watched = [APP_SRC_DIR, RUNTIME_SRC_DIR];
  if (TW_CONFIG) {
    // The config lives OUTSIDE the source tree (apps/hello/, not apps/hello/src/),
    // so editing the theme would otherwise never reach the running app. One
    // non-recursive watch on its directory, filtered to the config file itself.
    const configDir = dirname(TW_CONFIG);
    const configName = relative(configDir, TW_CONFIG);
    watch(configDir, { recursive: false }, (evt, file) => {
      if (String(file ?? "") !== configName) return;
      schedule(false)(evt, file);
    });
    watched.push(TW_CONFIG);
  }
  console.log(`watching ${watched.join(", ")} ...`);
  // VN_OWNER_PID: exit once the process that started the watcher is gone, however it ended (the
  // host does the same, crates/vn-app/src/owner.rs).
  const owner = Number(process.env.VN_OWNER_PID);
  if (Number.isInteger(owner) && owner > 0) {
    setInterval(() => {
      try {
        process.kill(owner, 0);
      } catch (e: any) {
        if (e?.code === "ESRCH") process.exit(0);
      }
    }, 1000);
  }
}
