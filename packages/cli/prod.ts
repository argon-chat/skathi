// Production builder: one self-contained IIFE, no loader, no HMR.
// Usage: bun prod.ts [app-entry.js] [out-file] [--config <path>] [--app-root <path>] [--define=KEY=VALUE ...]
//        [--graph <path>]: write the bundle's module graph as JSON (for `sk check`) instead of the bundle
//
//   dist/app.prod.js   IIFE: runtime + every app module + every SFC <style>, minified, __DEV__ = false.
//                      `sk build` (cli/build-app.ts) packs it into the SKPK payload.
//   dist/window.json   the `window` block of sk.config.json
//   dist/assets.json   [{ key, path }]: every file imported by URL, packed into the payload under key
//   dist/styles.json   { files, tailwind }: the files whose CSS the bundle includes, for sk build's CSS warnings
//
// Unlike build.ts: "vue" and "@skathi-native/runtime" alias to src/runtime.ts (src/dom.ts without vue),
// inlineTemplate is true (no HMR), styles go to (globalThis.__SK_CSS__ ??= []) ahead of the bundle, no HMR epilogue.
//
// DEBT: compileVue() and the style pipeline are a ~100-line copy of build.ts; merge into a shared sfc.ts.

import { createRequire } from "module";
import { resolve, dirname, join, relative, sep } from "path";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "fs";

import { asciiOnly, checkAscii, nonAsciiCount } from "./ascii";
import { preserveLiteralText } from "./ascii-literals";
import { AotInputs, aotEngineIdentity } from "./aot-inputs";
import type { BuildStyles, ModuleGraph } from "./check/graph";
import { pruneVendorPrefixes } from "./css-prefixes";
import { SVELTE_SOURCE, compileSvelte, SvelteRendererError } from "./svelte";
import { nativeSvelteRenderer, viteConfigFile } from "./cli/config";
import { protectHapticPatterns } from "./haptic-patterns";
import {
  DOM_ENTRY,
  FRAMEWORK_LIBS,
  RUNTIME_DIR,
  RUNTIME_ENTRY,
  RUNTIME_SRC_DIR,
  RUNTIME_VERSION,
  NATIVE_MODULES_ENTRY,
  STD_MODULES,
  VUE_MISSING,
  appVue,
  isPreseeded,
  packageDir,
  sfcCompiler,
  stdModuleFile,
  type SfcCompiler,
} from "./runtime-path";
import { TW_ID, declaresUtilities, findConfig, generate as generateTailwind, needsTailwindPass } from "./tailwind";
import {
  GLOB_TRIPWIRE,
  TEMPLATE_ASSET_URLS,
  SFC_FS,
  appDefines,
  frameworkDep,
  htmlPreludeModule,
  inlineCssImports,
  liftIndexHtml,
  readPackageJson,
  svgDefaultOf,
  tagModuleNamespaces,
  takeDefineArgs,
  templateJsonUrls,
  transformSource,
  viteShapesPlugin,
  type CssPiece,
  type LiftedHtml,
} from "./vitecompat";

const { defines: cliDefines, rest: argv } = takeDefineArgs(process.argv.slice(2));
let configOverride: string | undefined;
let appRootOverride: string | undefined;
let aotInputsDir: string | undefined;
let graphOut: string | undefined;
let minify = true;
const positional: string[] = [];
for (let i = 0; i < argv.length; i++) {
  const arg = argv[i]!;
  const flag = arg.split("=", 1)[0];
  if (arg === "--no-minify") minify = false;
  else if (flag === "--config" || flag === "--app-root" || flag === "--aot-inputs" || flag === "--graph") {
    const value = arg.includes("=") ? arg.slice(arg.indexOf("=") + 1) : argv[++i];
    if (!value?.trim() || value.startsWith("--")) throw new Error(`${flag} needs a path`);
    if (flag === "--config") configOverride = resolve(value);
    else if (flag === "--aot-inputs") aotInputsDir = resolve(value);
    else if (flag === "--graph") graphOut = resolve(value);
    else appRootOverride = resolve(value);
  } else if (!arg.startsWith("--")) positional.push(arg);
}

const PKG_DIR = import.meta.dir; // .../packages/cli
const WS_ROOT = resolve(PKG_DIR, "../.."); // repo root — module ids are relative to this

const ENTRY_ABS = resolve(positional[0] ?? join(WS_ROOT, "apps/hello/src/main.ts"));
const APP_SRC_DIR = dirname(ENTRY_ABS);
const OUT_FILE = resolve(positional[1] ?? join(PKG_DIR, "dist", "app.prod.js"));

// The `window` block of sk.config.json, emitted as `window.json`: the host reads it before opening
// the window, so it cannot ride inside the bundle. Read from next to `src/`, not APP_ROOT.
const SK_CONFIG_DIR = appRootOverride ?? (APP_SRC_DIR.endsWith(`${sep}src`) ? dirname(APP_SRC_DIR) : APP_SRC_DIR);
const SK_CONFIG_PATH = configOverride ?? join(SK_CONFIG_DIR, "sk.config.json");
const WINDOW_JSON = join(dirname(OUT_FILE), "window.json");
const ASSETS_JSON = join(dirname(OUT_FILE), "assets.json");
const STYLES_JSON = join(dirname(OUT_FILE), "styles.json");

function readSkConfig(): Record<string, unknown> {
  const path = SK_CONFIG_PATH;
  if (!existsSync(path)) {
    if (configOverride) throw new Error(`${path}: no such config file`);
    return {};
  }
  try {
    const cfg = JSON.parse(readFileSync(path, "utf8"));
    if (cfg && typeof cfg === "object" && !Array.isArray(cfg)) return cfg;
    if (configOverride) throw new Error("not a JSON object");
    return {};
  } catch (e) {
    if (configOverride) throw new Error(`${path}: ${e}`);
    console.error(`[prod] ${path}: ${e}`);
    return {};
  }
}
const SK_CONFIG = readSkConfig();
const NATIVE_SVELTE = nativeSvelteRenderer(SK_CONFIG.svelte);

function windowConfig(): string {
  const cfg = SK_CONFIG;
  const win = { ...((cfg.window as Record<string, unknown> | undefined) ?? {}) };
  // Top-level "devtools": false folds into the window block; a window block that sets it wins.
  if (typeof cfg.devtools === "boolean" && typeof win.devtools !== "boolean") {
    win.devtools = cfg.devtools;
  }
  // "httpCache" folds in the same way.
  if (cfg.httpCache !== undefined && win.httpCache === undefined) {
    win.httpCache = cfg.httpCache;
  }
  if (cfg.colorPicker !== undefined && win.colorPicker === undefined) win.colorPicker = cfg.colorPicker;
  return JSON.stringify(win, null, 2);
}

const TW_CONFIG = findConfig(APP_SRC_DIR, WS_ROOT);

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
const APP_ROOT = findAppRoot(APP_SRC_DIR, WS_ROOT);
const appRequire = createRequire(join(APP_ROOT, "package.json"));
const cliRequire = createRequire(join(PKG_DIR, "package.json"));
const APP_PKG = APP_ROOT === PKG_DIR ? null : readPackageJson(APP_ROOT);
const APP_VUE = appVue(APP_ROOT);
/** What "@skathi-native/runtime" is for this app: with Vue's renderer only when the app has `vue`. */
const APP_RUNTIME = APP_VUE ? RUNTIME_ENTRY : DOM_ENTRY;
/** Bare modules the build maps to a file of its own choosing, for every importer. */
const buildAliases = (): Record<string, string> => ({
  ...(APP_VUE ? { vue: RUNTIME_ENTRY, "@vue/runtime-core": APP_VUE.runtimeCore } : {}),
  "@skathi-native/runtime": APP_RUNTIME,
  ...Object.fromEntries(STD_MODULES.map((name) => [`skathi:${name}`, stdModuleFile(`skathi:${name}`)])),
  ...Object.fromEntries(APP_ALIASES),
  ...Object.fromEntries([...FRAMEWORK].map(([name, dependency]) => [name, dependency.path])),
});

let compiler: SfcCompiler | null = null;
/** The app's own `vue/compiler-sfc`, loaded with the first .vue file. */
function vueCompiler(): SfcCompiler {
  if (compiler) return compiler;
  compiler = sfcCompiler(APP_ROOT);
  try {
    const ts = appRequire("typescript") as typeof import("typescript");
    compiler.registerTS(() => ts);
  } catch {}
  return compiler;
}

const toId = (abs: string) => relative(WS_ROOT, abs).split(sep).join("/");
const J = (v: unknown) => JSON.stringify(v);

const advisories = new Set<string>();
const advise = (m: string) => advisories.add(m);

const APP_DEFINES = appDefines(SK_CONFIG.define, cliDefines, SK_CONFIG_PATH);
if (APP_DEFINES.errors.length) {
  for (const e of APP_DEFINES.errors) console.error(`[prod] ${e}`);
  process.exit(1);
}

const VITE_ENV_PROD: Record<string, unknown> = {
  MODE: "production",
  DEV: false,
  PROD: true,
  SSR: false,
  BASE_URL: "/",
  SK: true,
};
for (const [k, v] of Object.entries(APP_DEFINES.defines)) {
  if (!k.startsWith("import.meta.env.")) continue;
  try {
    VITE_ENV_PROD[k.slice("import.meta.env.".length)] = JSON.parse(v);
  } catch {}
}

// As Vite's production build: a package's `production` export (esm-env's DEV is `false`, not a NODE_ENV probe).
const PROD_CONDITIONS = NATIVE_SVELTE ? ["production", "svelte-custom-renderer"] : ["production"];

const DEFINES: Record<string, string> = {
  // The runtime installs these APIs in the same bundle as the app. A bare reference in a
  // native image would capture the host's pre-install value at entry; look it up when used.
  ResizeObserver: "globalThis.ResizeObserver",
  MutationObserver: "globalThis.MutationObserver",
  IntersectionObserver: "globalThis.IntersectionObserver",
  getComputedStyle: "globalThis.getComputedStyle",
  __DEV__: "false",
  __SK_SVELTE_NATIVE__: String(NATIVE_SVELTE),
  "process.env.NODE_ENV": '"production"',
  __VUE_PROD_DEVTOOLS__: "false",
  __FEATURE_SUSPENSE__: "true",
  __COMPAT__: "false",
  __BROWSER__: "false",
  __ESM_BUNDLER__: "true",
  __GLOBAL__: "false",
  __NODE_JS__: "false",
  __SSR__: "false",
  __TEST__: "false",
  __VERSION__: '"3.5"',
  // `import.meta` is a SyntaxError in a classic script and Bun leaves it verbatim in iife output,
  // so substitute the production values Vite would inline; the tripwire in `build()` catches survivors.
  "import.meta.env.MODE": '"production"',
  "import.meta.env.DEV": "false",
  "import.meta.env.PROD": "true",
  "import.meta.env.SSR": "false",
  "import.meta.env.BASE_URL": '"/"',
  "import.meta.env.SK": "true",
  "import.meta.env": JSON.stringify(VITE_ENV_PROD),
  "import.meta.hot": "undefined",
  "import.meta.url": '"sk://app.prod.js"',
  // An unexpanded `import.meta.glob` would become `{env,url}.glob(...)` and fail at eval; define it
  // to a findable name so it fails the build.
  "import.meta.glob": GLOB_TRIPWIRE,
  "import.meta": JSON.stringify({ env: VITE_ENV_PROD, url: "sk://app.prod.js" }),
  ...APP_DEFINES.defines,
};

function writeAtomic(path: string, text: string) {
  const tmp = path + ".tmp";
  writeFileSync(tmp, text);
  renameSync(tmp, path);
}

const INDEX_HTML = join(SK_CONFIG_DIR, "index.html");
const LIFTED: LiftedHtml | null = existsSync(INDEX_HTML) ? liftIndexHtml(INDEX_HTML, SK_CONFIG_DIR) : null;
const HTML_PRELUDE =
  LIFTED &&
  (LIFTED.scripts.length ||
    LIFTED.rootClasses.length ||
    Object.keys(LIFTED.rootAttrs).length ||
    LIFTED.svgDefinitions?.length)
    ? htmlPreludeModule(LIFTED, APP_ROOT)
    : null;
if (LIFTED) {
  for (const a of LIFTED.advisories) advise(`index.html: ${a}`);
  for (const s of LIFTED.scripts) {
    try {
      new Function(s.text);
    } catch (e) {
      advise(`${s.label}: the inline <script> does not parse (${(e as Error).message}) — it will throw at startup`);
    }
  }
}

const scopeIdOf = (id: string) => Bun.hash(id).toString(16).slice(0, 8);

/**
 * Template comments are dropped in production, as in Vite. The dev compiler loaded here keeps
 * them, which compiled a comment-first template to a fragment root and lost fallthrough attrs.
 */
const PROD_TEMPLATE_COMMENTS = false;

const cssById = new Map<string, string>();
/** Every .css file whose CSS the build includes, @imports too, and index.html when it has a `<style>`. */
const STYLE_FILES = new Set<string>();
/** The .vue and .svelte files whose `<style>` the build includes. */
const STYLED_COMPONENTS = new Set<string>();

function noteStyles(seen: Set<string>) {
  for (const file of seen) if (/\.css$/i.test(file)) STYLE_FILES.add(file);
}

/** An index.html `<style>` piece: noted as the page itself. */
function inlinePage(piece: CssPiece): boolean {
  if (!piece.from.startsWith(`${INDEX_HTML}?`)) return false;
  STYLE_FILES.add(INDEX_HTML);
  return true;
}

async function viteCss(text: string, from: string, label: string): Promise<string> {
  const seen = new Set([resolve(from)]);
  let css = inlineCssImports(text, from, SK_CONFIG_DIR, advise, seen, ASSETS);
  noteStyles(seen);
  if (TW_CONFIG && needsTailwindPass(css)) {
    try {
      css = await generateTailwind(TW_CONFIG, { text: css, from: from.replace(/\?.*$/, "") });
    } catch (e) {
      throw new Error(`${label}: tailwind: ${(e as Error).message}`);
    }
  }
  return css;
}

async function compileVue(absPath: string, source: string): Promise<{ contents: string; loader: "js" | "ts" }> {
  const id = toId(absPath);
  const { parse, compileScript, compileTemplate, compileStyle } = vueCompiler();
  const { descriptor, errors } = parse(source, {
    filename: absPath,
    templateParseOptions: { comments: PROD_TEMPLATE_COMMENTS },
  });
  if (errors.length) {
    throw new Error(`${id}: ${errors.map((e) => e.message).join("; ")}`);
  }
  const shortId = scopeIdOf(id);
  const scopeId = `data-v-${shortId}`;
  const hasScoped = descriptor.styles.some((s) => s.scoped);
  // compileScript leaves TS syntax: hand the chunk to Bun with the matching loader. RETURNED, not
  // stashed, because the style pass awaits and compiles interleave.
  const isTS = /^tsx?$/.test(descriptor.scriptSetup?.lang ?? descriptor.script?.lang ?? "");

  const chunks: string[] = [];

  if (descriptor.scriptSetup || descriptor.script) {
    const compiled = compileScript(descriptor, {
      id: shortId,
      isProd: true,
      fs: SFC_FS,
      inlineTemplate: true,
      genDefaultAs: "_sfc_main",
      templateOptions: {
        transformAssetUrls: TEMPLATE_ASSET_URLS,
        compilerOptions: {
          // compileTemplate defaults this to true then spreads compilerOptions over it, so ours wins.
          hoistStatic: false,
          isCustomElement: (tag: string) => tag === "lottie" || tag.startsWith("sk-"),
          comments: PROD_TEMPLATE_COMMENTS,
        },
      },
    });
    chunks.push(compiled.content);
  } else {
    chunks.push("const _sfc_main = {};");
    if (descriptor.template) {
      const t = compileTemplate({
        source: descriptor.template.content,
        filename: absPath,
        id: shortId,
        isProd: true,
        scoped: hasScoped,
        slotted: descriptor.slotted,
        transformAssetUrls: TEMPLATE_ASSET_URLS,
        compilerOptions: {
          hoistStatic: false,
          expressionPlugins: isTS ? ["typescript"] : [],
          isCustomElement: (tag: string) => tag === "lottie" || tag.startsWith("sk-"),
          comments: PROD_TEMPLATE_COMMENTS,
        },
      });
      if (t.errors.length) {
        throw new Error(`${id} <template>: ${t.errors.map(String).join("; ")}`);
      }
      chunks.push(t.code);
      chunks.push("_sfc_main.render = render;");
    }
  }

  // Scoped styles are matched in Rust by the data-v bucket the renderer stamps via setScopeId.
  if (hasScoped) chunks.push(`_sfc_main.__scopeId = ${J(scopeId)};`);
  chunks.push("export default _sfc_main;");

  // hoistStatic tripwire: createStaticVNode needs insertStaticContent, which the host lacks.
  // Checked on the SFC, not the bundle, which holds runtime-core's own definition of it.
  let code = chunks.join("\n");
  if (/\bcreateStaticVNode\b/.test(code)) {
    throw new Error(`${id}: emitted createStaticVNode — hoistStatic leaked back on (F7)`);
  }
  code = templateJsonUrls(code);
  code = transformSource(code, absPath, SHAPES) ?? code;

  STYLED_COMPONENTS.add(absPath);
  const cssChunks: string[] = [];
  for (const style of descriptor.styles) {
    let text = style.content;
    if (!style.lang || style.lang === "css" || style.lang === "postcss") {
      text = await viteCss(text, absPath, `${id} <style>`);
    } else {
      advise(`${id}: <style lang="${style.lang}"> is not preprocessed by Skathi — it is compiled as plain CSS`);
    }
    if (style.module) {
      advise(`${id}: <style module> — CSS modules are not supported yet; useCssModule() answers {}`);
    }
    const patterns = protectHapticPatterns(text);
    const out = compileStyle({
      source: patterns.css,
      filename: absPath,
      id: scopeId,
      scoped: style.scoped ?? false,
      trim: true,
      // MUST match compileScript's `isProd`: both hash a v-bind() variable's name from it.
      isProd: true,
    });
    if (out.errors.length) throw new Error(`${id} <style>: ${out.errors.join("; ")}`);
    cssChunks.push(patterns.restore(out.code));
  }
  if (cssChunks.length) cssById.set(id, cssChunks.join("\n"));

  AOT_INPUTS?.vue(
    absPath,
    source,
    code,
    isTS ? "ts" : "js",
    [descriptor.script?.content, descriptor.scriptSetup?.content].filter(Boolean).join("\n"),
  );
  return { contents: code, loader: isTS ? "ts" : "js" };
}

const cssEntryIds: string[] = [];
let appOwnsUtilities = false;

const ASSETS = new Map<string, string>();

const FRAMEWORK = new Map(
  FRAMEWORK_LIBS.flatMap((name) => {
    const dep = packageDir(name, APP_ROOT) && frameworkDep(name, (s) => appRequire.resolve(s));
    return dep ? [[name, dep] as const] : [];
  }),
);

/**
 * sk.config.json `"aliases"`: a bare module name and the app file standing in for it, relative to
 * the app root: the JS half of a native extension. It wins over the npm package everywhere.
 */
function appAliases(raw: unknown): Map<string, string> {
  const out = new Map<string, string>();
  if (raw === undefined) return out;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    throw new Error(`${SK_CONFIG_PATH}: "aliases" must be an object of { "module": "file" }`);
  }
  for (const [name, target] of Object.entries(raw as Record<string, unknown>)) {
    if (!/^(@[\w.-]+\/)?[\w.-]+(\/[\w.-]+)*$/.test(name) || name.startsWith(".")) {
      throw new Error(`${SK_CONFIG_PATH}: alias "${name}" is not a bare module name`);
    }
    if (isPreseeded(name)) {
      throw new Error(`${SK_CONFIG_PATH}: "${name}" is the engine's own module and cannot be aliased`);
    }
    if (typeof target !== "string" || !target.trim()) {
      throw new Error(`${SK_CONFIG_PATH}: alias "${name}" needs a file path`);
    }
    const file = resolve(SK_CONFIG_DIR, target.trim());
    if (!existsSync(file)) throw new Error(`${SK_CONFIG_PATH}: alias "${name}" → ${file}: no such file`);
    out.set(name, file);
  }
  return out;
}
const APP_ALIASES = appAliases(SK_CONFIG.aliases);

const AOT_INPUTS = aotInputsDir
  ? new AotInputs({
      out: aotInputsDir,
      appRoot: SK_CONFIG_DIR,
      entry: ENTRY_ABS,
      typescript: cliRequire("typescript"),
      aliases: buildAliases(),
      sourceRoots: {
        sdk: RUNTIME_DIR,
        ...(APP_VUE ? { "sdk-modules": APP_VUE.modulesDir } : {}),
        "sdk-tooling": dirname(dirname(cliRequire.resolve("typescript/package.json"))),
      },
      declarations: [...new Bun.Glob("*.d.ts").scanSync({ cwd: RUNTIME_SRC_DIR, absolute: true })],
    })
  : null;

/** `--graph`: every .js/.ts/.vue/.svelte file the build loaded, by absolute path. */
const GRAPH_LOADED = new Set<string>();
const RENDERER_ERRORS: ModuleGraph["errors"] = [];
const SHAPE_NOTES = new Map<string, number>();
const SHAPES = {
  sourceLoaded:
    AOT_INPUTS || graphOut
      ? (file: string, source: string, transformed: string) => {
          GRAPH_LOADED.add(file);
          AOT_INPUTS?.source(file, source, transformed);
        }
      : undefined,
  docRoot: SK_CONFIG_DIR,
  wsRoot: WS_ROOT,
  svgDefault: svgDefaultOf(APP_PKG),
  compileSfc: (absPath: string, source: string) => compileVue(absPath, source),
  assets: ASSETS,
  notes: SHAPE_NOTES,
};

const vuePlugin = {
  name: "vue-sfc-prod",
  setup(build: Bun.PluginBuilder) {
    // `import "./index.css"`: Bun would treat the .css as an asset; it becomes an empty module and
    // its content goes into the stylesheet.
    build.onLoad({ filter: /\.css$/ }, async (args: Bun.OnLoadArgs) => {
      const id = toId(args.path);
      if (!cssById.has(id)) {
        const source = await Bun.file(args.path).text();
        if (declaresUtilities(source)) appOwnsUtilities = true;
        cssById.set(id, await viteCss(source, args.path, id));
        cssEntryIds.push(id);
      }
      return { contents: "", loader: "js" };
    });
    build.onResolve({ filter: /^@skathi-native\/runtime$/ }, () => ({ path: APP_RUNTIME }));
    build.onResolve({ filter: /^vue$/ }, () => {
      if (!APP_VUE) throw new Error(VUE_MISSING);
      return { path: RUNTIME_ENTRY };
    });
    if (APP_VUE && !APP_ALIASES.has("@vue/runtime-core")) {
      const core = APP_VUE.runtimeCore;
      build.onResolve({ filter: /^@vue\/runtime-core$/ }, () => ({ path: core }));
    }
    // Provided by the embedding program at run time: the bundle asks the runtime for it by name.
    build.onResolve({ filter: /^native:/ }, (args: Bun.OnResolveArgs) => ({ path: args.path, namespace: "sk-native" }));
    build.onLoad({ filter: /.*/, namespace: "sk-native" }, (args: Bun.OnLoadArgs) => ({
      contents: `module.exports = require(${J(NATIVE_MODULES_ENTRY.split(sep).join("/"))}).nativeModule(${J(args.path.slice("native:".length))});`,
      loader: "js",
    }));
    build.onResolve({ filter: /^skathi:/ }, (args: Bun.OnResolveArgs) => ({ path: stdModuleFile(args.path) }));
    if (APP_ALIASES.size) {
      const names = [...APP_ALIASES.keys()].map((name) => name.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&"));
      build.onResolve({ filter: new RegExp(`^(${names.join("|")})$`) }, (args: Bun.OnResolveArgs) => ({
        path: APP_ALIASES.get(args.path)!,
      }));
    }
    build.onResolve({ filter: /^(vue-router|pinia)$/ }, (args: Bun.OnResolveArgs) => {
      const dep = FRAMEWORK.get(args.path);
      return dep ? { path: dep.path } : undefined;
    });
    build.onLoad({ filter: /\.vue$/ }, async (args: Bun.OnLoadArgs) => {
      const source = await Bun.file(args.path).text();
      if (graphOut) GRAPH_LOADED.add(args.path);
      return compileVue(args.path, source);
    });
    build.onLoad({ filter: SVELTE_SOURCE }, async (args: Bun.OnLoadArgs) => {
      const id = toId(args.path);
      if (graphOut) GRAPH_LOADED.add(args.path);
      const source = await Bun.file(args.path).text();
      const out = await compileSvelte(APP_ROOT, args.path, source, {
        dev: false,
        filename: id,
        native: NATIVE_SVELTE,
      }).catch((e: unknown) => {
        if (!graphOut || !(e instanceof SvelteRendererError)) throw e;
        RENDERER_ERRORS.push({
          code: e.code,
          message: e.detail,
          file: e.file,
          line: e.line,
          column: e.column,
          specifier: null,
        });
        return null;
      });
      if (!out) return { contents: "export default function() {}", loader: "js" };
      AOT_INPUTS?.svelte(args.path, source, out.js);
      for (const w of out.warnings) advise(w);
      STYLED_COMPONENTS.add(args.path);
      if (out.css) cssById.set(id, await viteCss(out.css, args.path, `${id} <style>`));
      return { contents: transformSource(out.js, args.path, SHAPES) ?? out.js, loader: "js" as const };
    });
  },
};

const ENTRY_ID = "sk-entry:main";
const entryPlugin = {
  name: "sk-html-entry",
  setup(build: Bun.PluginBuilder) {
    build.onResolve({ filter: /^sk-(entry|html):/ }, (args: Bun.OnResolveArgs) => ({
      path: args.path,
      namespace: "sk-html",
    }));
    build.onLoad({ filter: /.*/, namespace: "sk-html" }, (args: Bun.OnLoadArgs) => {
      if (args.path === ENTRY_ID) {
        return {
          contents: `import "sk-html:prelude";\nimport ${J(ENTRY_ABS.split(sep).join("/"))};\n`,
          loader: "js",
        };
      }
      return { contents: HTML_PRELUDE ?? "", loader: "js" };
    });
  },
};

/**
 * JavaScriptCore uses UTF-16 for the whole source if even one character needs it.
 */
function asciiBundle(code: string): string {
  const before = nonAsciiCount(code);
  if (!before || process.env.SK_BUNDLE_UTF8 === "1") return code;
  const prepared = preserveLiteralText(code);
  const r = asciiOnly(prepared);
  const problem =
    r.failed ?? (nonAsciiCount(r.code) ? "some characters could not be escaped" : checkAscii(prepared, r.code));
  if (problem) {
    throw new Error(
      `Could not emit the bundle as ASCII (${problem}). Set SK_BUNDLE_UTF8=1 to build with the original source encoding.`,
    );
  }
  return r.code;
}

async function writeGraph(out: string) {
  const result = await Bun.build({
    entrypoints: [HTML_PRELUDE ? ENTRY_ID : ENTRY_ABS],
    format: "iife",
    target: "browser",
    define: DEFINES,
    plugins: [viteShapesPlugin(SHAPES), vuePlugin, entryPlugin],
    conditions: PROD_CONDITIONS,
    minify: minify ? { whitespace: true, identifiers: true, syntax: true } : false,
    sourcemap: "external",
    metafile: true,
    throw: false,
  });
  // Metafile keys and sourcemap sources are relative, to a base that varies between runs
  // ("node_modules/…", "../../C:/…"): match them against the absolute paths the plugins loaded.
  const byName = new Map<string, { key: string; path: string }[]>();
  for (const path of GRAPH_LOADED) {
    const key = resolve(path).split(sep).join("/").toLowerCase();
    const name = key.slice(key.lastIndexOf("/") + 1);
    byName.set(name, [...(byName.get(name) ?? []), { key, path: resolve(path) }]);
  }
  const loadedPaths = (key: string) => {
    const tail = `/${key.replace(/^(?:\.\.\/)+/, "").replace(/^[A-Za-z]:\//, "")}`.toLowerCase();
    return (byName.get(tail.slice(tail.lastIndexOf("/") + 1)) ?? []).filter((l) => l.key.endsWith(tail));
  };
  const modules = new Set<string>();
  for (const output of Object.values(result.metafile?.outputs ?? {})) {
    for (const [key, input] of Object.entries(output.inputs)) {
      if (input.bytesInOutput <= 0) continue;
      for (const loaded of loadedPaths(key)) modules.add(loaded.path);
    }
  }
  let bundle: ModuleGraph["bundle"];
  const code = result.outputs.find((o) => o.kind === "entry-point");
  const map = result.outputs.find((o) => o.kind === "sourcemap");
  if (result.success && code && map) {
    const raw = JSON.parse(await map.text()) as { sources: string[]; sourcesContent?: string[]; mappings: string };
    const sources = raw.sources.map((key, i) => {
      const [loaded, ...others] = loadedPaths(key);
      if (!loaded || others.length) return null;
      // Positions are the file's own only when the bundler read it as is (not a compiled .vue or .svelte).
      const text = readFileSync(loaded.path, "utf8").replace(/^\uFEFF/, "");
      return raw.sourcesContent?.[i] === text ? loaded.path : null;
    });
    bundle = { code: await code.text(), sources, mappings: raw.mappings };
  }
  for (const piece of LIFTED ? [...LIFTED.headCss, ...LIFTED.bodyCss] : []) {
    if (inlinePage(piece)) continue;
    const seen = new Set([resolve(piece.from)]);
    inlineCssImports(piece.text, piece.from, SK_CONFIG_DIR, () => {}, seen);
    noteStyles(seen);
  }
  const graph: ModuleGraph = {
    schema: "skathi.module-graph",
    version: 1,
    entry: ENTRY_ABS,
    appRoot: SK_CONFIG_DIR,
    aliases: buildAliases(),
    declarations: [...new Bun.Glob("*.d.ts").scanSync({ cwd: RUNTIME_SRC_DIR, absolute: true })],
    modules: [...modules].sort(),
    styles: [...STYLE_FILES].sort(),
    tailwind: TW_CONFIG !== null,
    ...(bundle ? { bundle } : {}),
    errors: [
      ...RENDERER_ERRORS,
      ...result.logs
        .filter((log) => log.level === "error")
        .map((log) => ({
          message: log.message,
          file: log.position?.file ? resolve(log.position.file) : null,
          line: log.position && log.position.line > 0 ? log.position.line : null,
          column: log.position && log.position.column >= 0 ? log.position.column : null,
          specifier: log.name === "ResolveMessage" ? log.specifier : null,
        })),
    ],
  };
  writeAtomic(out, JSON.stringify(graph) + "\n");
}

async function build() {
  const t0 = Date.now();
  let tailwindMs = 0;
  mkdirSync(dirname(OUT_FILE), { recursive: true });
  cssById.clear();
  cssEntryIds.length = 0;
  appOwnsUtilities = false;

  const result = await Bun.build({
    entrypoints: [HTML_PRELUDE ? ENTRY_ID : ENTRY_ABS],
    format: "iife",
    target: "browser",
    define: DEFINES,
    plugins: [viteShapesPlugin(SHAPES), vuePlugin, entryPlugin],
    conditions: PROD_CONDITIONS,
    minify: minify ? { whitespace: true, identifiers: true, syntax: true } : false,
    sourcemap: "none",
  });
  if (!result.success) {
    for (const log of result.logs) console.error(log);
    throw new Error("prod build failed");
  }
  const body = tagModuleNamespaces(await result.outputs[0]!.text()).code;

  if (/\b__DEV__\b/.test(body)) {
    throw new Error("__DEV__ survived minification — the define block did not apply");
  }
  const glob = new RegExp(`\\b${GLOB_TRIPWIRE}\\b`).exec(body);
  if (glob) {
    throw new Error(
      `an import.meta.glob(...) survived the build and would throw at startup: …${body.slice(Math.max(0, glob.index - 80), glob.index + 80)}…`,
    );
  }
  // The bundle must also PARSE as a classic script (a surviving `import.meta` is a SyntaxError at
  // eval): parse rather than grep, since a string literal may mention it.
  try {
    new Function(body);
  } catch (e) {
    if (!(e instanceof SyntaxError)) throw e;
    const meta = /\bimport\s*\.\s*meta\b/.exec(body);
    const near = meta ? `: …${body.slice(Math.max(0, meta.index - 60), meta.index + 60)}…` : "";
    throw new Error(
      `the bundle is not a valid classic script (${e.message})${near}` +
        (meta ? " — prod.ts defines import.meta.env/url/hot; anything else needs a define of its own" : ""),
    );
  }

  const htmlCss = async (pieces: CssPiece[]) => {
    const out: string[] = [];
    for (const p of pieces) {
      inlinePage(p);
      if (declaresUtilities(p.text)) appOwnsUtilities = true;
      out.push(await viteCss(p.text, p.from, toId(p.from)));
    }
    return out;
  };
  const headCss = LIFTED ? await htmlCss(LIFTED.headCss) : [];
  const bodyCss = LIFTED ? await htmlCss(LIFTED.bodyCss) : [];

  // Styles run before the bundle (first paint is styled), sorted by module id to match the dev
  // loader's cascade; the Tailwind sheet (".sk/tailwind.css") sorts first unless an app stylesheet
  // declares `@tailwind utilities`. index.html's sheets bracket it: <head> before, <body> after.
  if (TW_CONFIG && !appOwnsUtilities) {
    const t = Date.now();
    cssById.set(TW_ID, await generateTailwind(TW_CONFIG));
    tailwindMs = Date.now() - t;
  }
  const sheets = [
    ...headCss,
    ...Array.from(cssById.keys())
      .sort()
      .map((id) => cssById.get(id)!),
    ...bodyCss,
  ].map(pruneVendorPrefixes);
  const css = sheets.map((text) => `(globalThis.__SK_CSS__ ??= []).push(${J(text)});`).join("\n");

  const out = asciiBundle(
    [
      `// Skathi production bundle - entry ${toId(ENTRY_ABS)}`,
      `// generated by packages/cli/prod.ts; do not edit`,
      css,
      body,
    ].join("\n"),
  );
  writeAtomic(OUT_FILE, out);
  writeAtomic(WINDOW_JSON, windowConfig() + "\n");
  const assets = Array.from(ASSETS.entries())
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([key, path]) => ({ key, path }));
  writeAtomic(ASSETS_JSON, JSON.stringify(assets, null, 2) + "\n");
  const styles: BuildStyles = {
    files: [...STYLED_COMPONENTS, ...STYLE_FILES].sort(),
    tailwind: TW_CONFIG !== null,
  };
  writeAtomic(STYLES_JSON, JSON.stringify(styles, null, 2) + "\n");
  writeAotInputs(out, DEFINES);

  for (const [what, n] of SHAPE_NOTES) console.log(`[prod] ${what}: ${n}`);
  if (LIFTED) {
    console.log(
      `[prod] index.html: ${LIFTED.headCss.length + LIFTED.bodyCss.length} stylesheet(s), ` +
        `${LIFTED.scripts.length} inline script(s) lifted`,
    );
  }
  for (const a of advisories) console.log(`[prod] advisory: ${a}`);

  const sfcs = cssById.size - cssEntryIds.length - (cssById.has(TW_ID) ? 1 : 0);
  console.log(
    `prod build: ${(out.length / 1024) | 0} KB (${sfcs} styled SFC${sfcs === 1 ? "" : "s"}` +
      (cssEntryIds.length ? ` + ${cssEntryIds.length} css entr${cssEntryIds.length === 1 ? "y" : "ies"}` : "") +
      (cssById.has(TW_ID)
        ? ` + ${((cssById.get(TW_ID)?.length ?? 0) / 1024).toFixed(1)} KB tailwind in ${tailwindMs} ms`
        : "") +
      (assets.length ? ` + ${assets.length} asset${assets.length === 1 ? "" : "s"}` : "") +
      `) -> ${relative(WS_ROOT, OUT_FILE).split(sep).join("/")} in ${Date.now() - t0} ms`,
  );
}

function writeAotInputs(script: string, defines: Record<string, string>): void {
  if (!AOT_INPUTS) return;
  const pinsPath = join(WS_ROOT, "vendor", "redjs", "redjs", "pins.txt");
  const engine = aotEngineIdentity(existsSync(pinsPath) ? readFileSync(pinsPath, "utf8") : null);
  const manifest = AOT_INPUTS.write({
    script,
    scriptUrl: "sk://app.prod.js",
    defines,
    engine,
    sdkVersion: RUNTIME_VERSION,
  });
  console.log(`[prod] typed AOT inputs: ${manifest}`);
}

async function run(): Promise<void> {
  const configFile = viteConfigFile(SK_CONFIG_DIR);
  if (!configFile) return graphOut ? writeGraph(graphOut) : build();

  // This process is dedicated to one app. Config callbacks and plugins often resolve paths against cwd.
  process.chdir(SK_CONFIG_DIR);
  process.env.SKATHI_BUILD = "1";
  process.env.NODE_ENV = "production";
  const { buildWithVite } = await import("./vite/backend");
  await buildWithVite({
    appRoot: SK_CONFIG_DIR,
    entry: ENTRY_ABS,
    outFile: OUT_FILE,
    configFile,
    skConfig: SK_CONFIG,
    aliases: buildAliases(),
    defines: DEFINES,
    windowJson: windowConfig(),
    minify,
    graphOut,
    aotInputs: AOT_INPUTS ?? undefined,
    finishScript: asciiBundle,
    finishAot: writeAotInputs,
  });
}

await run().catch((e) => {
  console.error(e);
  process.exit(1);
});
