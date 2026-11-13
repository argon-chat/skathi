// Production builder: one self-contained IIFE, no loader, no HMR.
// Usage: bun prod.ts [app-entry.js] [out-file] [--config <path>] [--app-root <path>] [--define=KEY=VALUE ...]
//
//   dist/app.prod.js   IIFE: @vue/runtime-core + src/runtime.ts + every app module + every SFC <style>,
//                      minified, __DEV__ = false. `vn build` (cli/build-app.ts) packs it into the VNPK
//                      payload; crates/vn-app/build.rs also runs this file for the devtools inspector.
//   dist/window.json   the `window` block of vn.config.json
//   dist/assets.json   [{ key, path }]: every file imported by URL, packed into the payload under key
//
// Differences from the dev builder (build.ts):
// - one IIFE; "vue" and "vue-native" are aliased to src/runtime.ts instead of a __VN__ registry;
// - inlineTemplate is true (no HMR, so the render fn may live inside setup());
// - styles are pushed to (globalThis.__VN_CSS__ ??= []) ahead of the bundle instead of setCss;
// - __DEV__ false, no HMR epilogue.
// Unchanged: hoistStatic is forced off (the host has no insertStaticContent), and the scope id
// hashes the module id only, so dev and prod CSS match.
//
// DEBT: compileVue() and the style pipeline are a ~100-line copy of build.ts; merge into a shared sfc.ts.

import { parse, compileScript, compileTemplate, compileStyle, registerTS } from "@vue/compiler-sfc";
import { createRequire } from "module";
import { resolve, dirname, join, relative, sep } from "path";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "fs";

import { asciiOnly, checkAscii, nonAsciiCount } from "./ascii";
import { AotInputs, aotEngineIdentity } from "./aot-inputs";
import { pruneVendorPrefixes } from "./css-prefixes";
import {
  TW_ID,
  declaresUtilities,
  findConfig,
  generate as generateTailwind,
  needsTailwindPass,
} from "./tailwind";
import {
  GLOB_TRIPWIRE,
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
  transformSource,
  viteShapesPlugin,
  type CssPiece,
  type LiftedHtml,
} from "./vitecompat";

const { defines: cliDefines, rest: argv } = takeDefineArgs(process.argv.slice(2));
let configOverride: string | undefined;
let appRootOverride: string | undefined;
let aotInputsDir: string | undefined;
const positional: string[] = [];
for (let i = 0; i < argv.length; i++) {
  const arg = argv[i]!;
  const flag = arg.split("=", 1)[0];
  if (flag === "--config" || flag === "--app-root" || flag === "--aot-inputs") {
    const value = arg.includes("=") ? arg.slice(arg.indexOf("=") + 1) : argv[++i];
    if (!value?.trim() || value.startsWith("--")) throw new Error(`${flag} needs a path`);
    if (flag === "--config") configOverride = resolve(value);
    else if (flag === "--aot-inputs") aotInputsDir = resolve(value);
    else appRootOverride = resolve(value);
  } else if (!arg.startsWith("--")) positional.push(arg);
}

const PKG_DIR = import.meta.dir; // .../packages/vue-native
const WS_ROOT = resolve(PKG_DIR, "../.."); // repo root — module ids are relative to this
const RUNTIME_ENTRY = join(PKG_DIR, "src", "runtime.ts");

const ENTRY_ABS = resolve(positional[0] ?? join(WS_ROOT, "apps/hello/src/main.ts"));
const APP_SRC_DIR = dirname(ENTRY_ABS);
const OUT_FILE = resolve(positional[1] ?? join(PKG_DIR, "dist", "app.prod.js"));

// The `window` block of vn.config.json, emitted as `window.json` (`vn build` packs it as
// vn/window.json). The host reads it before opening the window, so it cannot ride inside the
// bundle. Always written, `{}` when absent. Read from next to `src/`, not APP_ROOT (which can land
// a level higher).
const VN_CONFIG_DIR = appRootOverride ?? (APP_SRC_DIR.endsWith(`${sep}src`) ? dirname(APP_SRC_DIR) : APP_SRC_DIR);
const VN_CONFIG_PATH = configOverride ?? join(VN_CONFIG_DIR, "vn.config.json");
const WINDOW_JSON = join(dirname(OUT_FILE), "window.json");
const ASSETS_JSON = join(dirname(OUT_FILE), "assets.json");

function readVnConfig(): Record<string, unknown> {
  const path = VN_CONFIG_PATH;
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
const VN_CONFIG = readVnConfig();

function windowConfig(): string {
  const cfg = VN_CONFIG;
  const win = { ...((cfg.window as Record<string, unknown> | undefined) ?? {}) };
  // Top-level "devtools": false folds into the window block, as in build.ts; a window block that sets it wins.
  if (typeof cfg.devtools === "boolean" && typeof win.devtools !== "boolean") {
    win.devtools = cfg.devtools;
  }
  // "httpCache" folds in the same way: the host installs the cache before any JS runs.
  if (cfg.httpCache !== undefined && win.httpCache === undefined) {
    win.httpCache = cfg.httpCache;
  }
  return JSON.stringify(win, null, 2);
}

/// Same discovery rule as the dev builder.
const TW_CONFIG = findConfig(APP_SRC_DIR, WS_ROOT);

// App package root and TS-aware SFC compilation: as in build.ts. npm dependencies need nothing
// special: prod is one Bun bundle, so bare specifiers inline like any import.
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
const APP_ROOT = findAppRoot(APP_SRC_DIR, WS_ROOT);
const appRequire = createRequire(join(APP_ROOT, "package.json"));
/** Resolves the runtime's OWN vendored deps (vue-router, pinia). */
const runtimeRequire = createRequire(join(PKG_DIR, "package.json"));
const APP_PKG = APP_ROOT === PKG_DIR ? null : readPackageJson(APP_ROOT);

try {
  const ts = appRequire("typescript");
  registerTS(() => ts);
} catch {
  // No typescript in the app: fine unless it actually imports types across a
  // package boundary, and compileScript's own error says so precisely.
}

const toId = (abs: string) => relative(WS_ROOT, abs).split(sep).join("/");
const J = (v: unknown) => JSON.stringify(v);

const advisories = new Set<string>();
const advise = (m: string) => advisories.add(m);

const APP_DEFINES = appDefines(VN_CONFIG.define, cliDefines, VN_CONFIG_PATH);
if (APP_DEFINES.errors.length) {
  for (const e of APP_DEFINES.errors) console.error(`[prod] ${e}`);
  process.exit(1);
}

/** What `vite build` inlines for `import.meta.env`, plus `VN: true` and the app's own defines. */
const VITE_ENV_PROD: Record<string, unknown> = {
  MODE: "production",
  DEV: false,
  PROD: true,
  SSR: false,
  BASE_URL: "/",
  VN: true,
};
for (const [k, v] of Object.entries(APP_DEFINES.defines)) {
  if (!k.startsWith("import.meta.env.")) continue;
  try {
    VITE_ENV_PROD[k.slice("import.meta.env.".length)] = JSON.parse(v);
  } catch {
    /* an identifier value: it reads right through the dotted define, not the object */
  }
}

// Vue's esm-bundler build reads these as bare identifiers. `__DEV__: false` strips hmr.ts,
// customFormatter.ts and the dev warnings.
const DEFINES: Record<string, string> = {
  __DEV__: "false",
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
  // `import.meta` is a SyntaxError in a classic script and Bun leaves it verbatim in iife output, so
  // substitute the production values Vite would inline (same block as build.ts; values must be JSON).
  // The tripwire in `build()` fails the build if anything survives.
  "import.meta.env.MODE": '"production"',
  "import.meta.env.DEV": "false",
  "import.meta.env.PROD": "true",
  "import.meta.env.SSR": "false",
  "import.meta.env.BASE_URL": '"/"',
  // vue-native's own: `if (!import.meta.env.VN) import("…")` keeps browser-only code out of the bundle.
  "import.meta.env.VN": "true",
  "import.meta.env": JSON.stringify(VITE_ENV_PROD),
  "import.meta.hot": "undefined",
  "import.meta.url": '"vn://app.prod.js"',
  // An unexpanded `import.meta.glob` would become `{env,url}.glob(...)`, valid syntax that fails at
  // eval; defined to a findable name so it fails the build.
  "import.meta.glob": GLOB_TRIPWIRE,
  "import.meta": JSON.stringify({ env: VITE_ENV_PROD, url: "vn://app.prod.js" }),
  ...APP_DEFINES.defines,
};

function writeAtomic(path: string, text: string) {
  const tmp = path + ".tmp";
  writeFileSync(tmp, text);
  renameSync(tmp, path);
}

// index.html: styles go into the stylesheet in document order, classic scripts into a prelude
// module that runs before the entry, mount attributes onto the root (vitecompat.ts `liftIndexHtml`).
const INDEX_HTML = join(VN_CONFIG_DIR, "index.html");
const LIFTED: LiftedHtml | null = existsSync(INDEX_HTML) ? liftIndexHtml(INDEX_HTML, VN_CONFIG_DIR) : null;
const HTML_PRELUDE =
  LIFTED && (LIFTED.scripts.length || LIFTED.rootClasses.length || Object.keys(LIFTED.rootAttrs).length || LIFTED.svgDefinitions?.length)
    ? htmlPreludeModule(LIFTED)
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

/** Stable per-file scope id: the same formula as build.ts, so dev and prod emit the same `data-v-*`. */
const scopeIdOf = (id: string) => Bun.hash(id).toString(16).slice(0, 8);

/**
 * Template comments are dropped in a production build, as in Vite. This process loads the dev
 * compiler, which keeps them, so a template starting with a comment compiled to a fragment root; the
 * prod runtime does not see through that and the component lost fallthrough attrs and its parent's
 * scope id.
 */
const PROD_TEMPLATE_COMMENTS = false;

/** id -> compiled css, filled by the .vue onLoad hook below. */
const cssById = new Map<string, string>();

/** A stylesheet's text through Vite's CSS pipeline: local @imports inlined, then the app's Tailwind when the text uses it. */
async function viteCss(text: string, from: string, label: string): Promise<string> {
  let css = inlineCssImports(text, from, VN_CONFIG_DIR, advise, undefined, ASSETS);
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
  // The template AST is parsed here and reused by compileTemplate, so comments are dropped here;
  // `compilerOptions.comments` below covers a template compiled from source.
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
  // As in build.ts: compileScript leaves TS syntax, so hand the chunk to Bun with the matching
  // loader. RETURNED, not stashed: the style pass awaits and compiles interleave.
  const isTS = /^tsx?$/.test(descriptor.scriptSetup?.lang ?? descriptor.script?.lang ?? "");

  const chunks: string[] = [];

  if (descriptor.scriptSetup || descriptor.script) {
    const compiled = compileScript(descriptor, {
      id: shortId,
      isProd: true,
      fs: SFC_FS,
      // No HMR in prod, so the render fn may be inlined into setup().
      inlineTemplate: true,
      genDefaultAs: "_sfc_main",
      templateOptions: {
        compilerOptions: {
          // compileTemplate defaults this to true then spreads compilerOptions over it, so ours wins.
          hoistStatic: false,
          // vue-native host elements missing from the HTML tag list (lockstep with build.ts).
          isCustomElement: (tag: string) => tag === "lottie" || tag.startsWith("vn-"),
          comments: PROD_TEMPLATE_COMMENTS,
        },
      },
    });
    chunks.push(compiled.content);
  } else {
    // No <script>: nothing for compileScript to inline into, so the template
    // becomes a standalone render function as in the dev builder.
    chunks.push("const _sfc_main = {};");
    if (descriptor.template) {
      const t = compileTemplate({
        source: descriptor.template.content,
        filename: absPath,
        id: shortId,
        isProd: true,
        scoped: hasScoped,
        slotted: descriptor.slotted,
        compilerOptions: {
          hoistStatic: false,
          expressionPlugins: isTS ? ["typescript"] : [],
          isCustomElement: (tag: string) => tag === "lottie" || tag.startsWith("vn-"),
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

  // Scoped styles are matched in Rust by the data-v bucket the renderer stamps
  // via setScopeId, which reads exactly this property.
  if (hasScoped) chunks.push(`_sfc_main.__scopeId = ${J(scopeId)};`);
  chunks.push("export default _sfc_main;");

  // hoistStatic tripwire on the compiled SFC: createStaticVNode needs insertStaticContent, which the
  // host lacks. Checked here, not on the bundle, which contains runtime-core's own definition of it.
  let code = chunks.join("\n");
  if (/\bcreateStaticVNode\b/.test(code)) {
    throw new Error(`${id}: emitted createStaticVNode — hoistStatic leaked back on (F7)`);
  }
  // `import.meta.glob` / worker URLs written inside a component's <script>.
  code = transformSource(code, absPath, SHAPES) ?? code;

  const cssChunks: string[] = [];
  for (const style of descriptor.styles) {
    let text = style.content;
    // Vite's order: CSS pipeline first, so an @imported sheet is scoped too.
    if (!style.lang || style.lang === "css" || style.lang === "postcss") {
      text = await viteCss(text, absPath, `${id} <style>`);
    } else {
      advise(`${id}: <style lang="${style.lang}"> is not preprocessed by vue-native — it is compiled as plain CSS`);
    }
    if (style.module) {
      advise(`${id}: <style module> — CSS modules are not supported yet; useCssModule() answers {}`);
    }
    const out = compileStyle({
      source: text,
      filename: absPath,
      id: scopeId,
      scoped: style.scoped,
      trim: true,
      // MUST match compileScript's `isProd`: both hash a v-bind() variable's name from it.
      isProd: true,
    });
    if (out.errors.length) throw new Error(`${id} <style>: ${out.errors.join("; ")}`);
    cssChunks.push(out.code);
  }
  if (cssChunks.length) cssById.set(id, cssChunks.join("\n"));

  AOT_INPUTS?.vue(absPath, source, code, isTS ? "ts" : "js", [descriptor.script?.content, descriptor.scriptSetup?.content].filter(Boolean).join("\n"));
  return { contents: code, loader: isTS ? "ts" : "js" };
}

/** Stylesheets imported from JS (`import "./index.css"`), in first-seen order. Keyed by module id into `cssById` so the one sorted aggregation orders them; `.vn/` sorts first. */
const cssEntryIds: string[] = [];
/** True when one of them declares `@tailwind utilities` itself. */
let appOwnsUtilities = false;

/** payload key -> absolute path: every asset an import referred to by URL. */
const ASSETS = new Map<string, string>();

const FRAMEWORK = new Map(
  ["vue-router", "pinia"].map((name) => {
    const dep = frameworkDep(
      name,
      APP_PKG,
      (s) => appRequire.resolve(s),
      (s) => runtimeRequire.resolve(s),
    );
    if (dep.warning) advise(dep.warning);
    return [name, dep] as const;
  }),
);

/**
 * vn.config.json `"aliases"`: a bare module name and the app file standing in for it, relative to
 * the app root (`{ "some-sdk": "native/some-sdk.ts" }`): the JS half of a native extension. Every
 * importer lands on that file, never the npm package.
 */
function appAliases(raw: unknown): Map<string, string> {
  const out = new Map<string, string>();
  if (raw === undefined) return out;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    throw new Error(`${VN_CONFIG_PATH}: "aliases" must be an object of { "module": "file" }`);
  }
  for (const [name, target] of Object.entries(raw as Record<string, unknown>)) {
    if (!/^(@[\w.-]+\/)?[\w.-]+(\/[\w.-]+)*$/.test(name) || name.startsWith(".")) {
      throw new Error(`${VN_CONFIG_PATH}: alias "${name}" is not a bare module name`);
    }
    if (name === "vue" || name === "vue-native" || name === "vue-router" || name === "pinia") {
      throw new Error(`${VN_CONFIG_PATH}: "${name}" is the engine's own module and cannot be aliased`);
    }
    if (typeof target !== "string" || !target.trim()) {
      throw new Error(`${VN_CONFIG_PATH}: alias "${name}" needs a file path`);
    }
    const file = resolve(VN_CONFIG_DIR, target.trim());
    if (!existsSync(file)) throw new Error(`${VN_CONFIG_PATH}: alias "${name}" → ${file}: no such file`);
    out.set(name, file);
  }
  return out;
}
const APP_ALIASES = appAliases(VN_CONFIG.aliases);

const AOT_INPUTS = aotInputsDir ? new AotInputs({
  out: aotInputsDir,
  appRoot: VN_CONFIG_DIR,
  entry: ENTRY_ABS,
  typescript: runtimeRequire("typescript"),
  aliases: { vue: RUNTIME_ENTRY, "vue-native": RUNTIME_ENTRY, ...Object.fromEntries(APP_ALIASES), ...Object.fromEntries([...FRAMEWORK].map(([name, dependency]) => [name, dependency.path])) },
  sourceRoots: { sdk: PKG_DIR },
  declarations: [...new Bun.Glob("*.d.ts").scanSync({ cwd: join(PKG_DIR, "src"), absolute: true })],
}) : null;

const SHAPE_NOTES = new Map<string, number>();
const SHAPES = {
  sourceLoaded: AOT_INPUTS ? (file: string, source: string, transformed: string) => AOT_INPUTS.source(file, source, transformed) : undefined,
  docRoot: VN_CONFIG_DIR,
  wsRoot: WS_ROOT,
  svgDefault: svgDefaultOf(APP_PKG),
  compileSfc: (absPath: string, source: string) => compileVue(absPath, source),
  assets: ASSETS,
  notes: SHAPE_NOTES,
};

const vuePlugin = {
  name: "vue-sfc-prod",
  setup(build: any) {
    // `import "./index.css"`: Bun would treat the .css as an asset; it becomes an empty module and its
    // content goes into the stylesheet, through Tailwind when configured (as in build.ts).
    build.onLoad({ filter: /\.css$/ }, async (args: any) => {
      const id = toId(args.path);
      if (!cssById.has(id)) {
        const source = await Bun.file(args.path).text();
        if (declaresUtilities(source)) appOwnsUtilities = true;
        cssById.set(id, await viteCss(source, args.path, id));
        cssEntryIds.push(id);
      }
      return { contents: "", loader: "js" };
    });
    // Without the dev loader's pre-seeded registry, "vue" and "vue-native" both resolve to our runtime.
    build.onResolve({ filter: /^(vue|vue-native)$/ }, () => ({ path: RUNTIME_ENTRY }));
    // The app's own "aliases" (appAliases): its stand-in file wins over the npm package everywhere.
    if (APP_ALIASES.size) {
      const names = [...APP_ALIASES.keys()].map((name) => name.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&"));
      build.onResolve({ filter: new RegExp(`^(${names.join("|")})$`) }, (args: any) => ({
        path: APP_ALIASES.get(args.path)!,
      }));
    }
    // vue-router and pinia are resolved once up front (FRAMEWORK), so every importer lands on one copy.
    build.onResolve({ filter: /^(vue-router|pinia)$/ }, (args: any) => ({
      path: FRAMEWORK.get(args.path)!.path,
    }));
    build.onLoad({ filter: /\.vue$/ }, async (args: any) => {
      const source = await Bun.file(args.path).text();
      return compileVue(args.path, source);
    });
  },
};

/** `index.html`'s scripts and root attributes run ahead of the entry: a virtual entry imports both. */
const ENTRY_ID = "vn-entry:main";
const entryPlugin = {
  name: "vn-html-entry",
  setup(build: any) {
    build.onResolve({ filter: /^vn-(entry|html):/ }, (args: any) => ({ path: args.path, namespace: "vn-html" }));
    build.onLoad({ filter: /.*/, namespace: "vn-html" }, (args: any) => {
      if (args.path === ENTRY_ID) {
        return {
          contents: `import "vn-html:prelude";\nimport ${J(ENTRY_ABS.split(sep).join("/"))};\n`,
          loader: "js",
        };
      }
      return { contents: HTML_PRELUDE ?? "", loader: "js" };
    });
  },
};

/**
 * The bundle with every non-ASCII character escaped (ascii.ts): one non-ASCII character makes
 * JavaScriptCore keep the whole source as UTF-16, doubling its memory. Never fails the build: if the
 * bundle cannot be lexed or the result parses differently, it is written as is with an advisory.
 * `VN_BUNDLE_UTF8=1` skips the pass.
 */
function asciiBundle(code: string): string {
  const before = nonAsciiCount(code);
  if (!before || process.env.VN_BUNDLE_UTF8 === "1") return code;
  const extraMB = (chars: number) => (chars / 1048576).toFixed(1);
  const r = asciiOnly(code);
  const problem = r.failed ?? checkAscii(code, r.code);
  if (problem) {
    advise(
      `the bundle stays UTF-8 with ${before} non-ASCII characters, so the host holds its source as UTF-16 ` +
        `(~${extraMB(code.length)} MB more): the ASCII pass ${r.failed ? "could not lex it" : "was refused"} - ${problem}`,
    );
    return code;
  }
  if (r.kept) {
    advise(
      `${r.kept} non-ASCII characters in tagged templates are kept as written (a tag can read \`raw\`), so the ` +
        `host still holds the bundle's source as UTF-16 (~${extraMB(r.code.length)} MB more)`,
    );
  }
  return r.code;
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
    // The import shapes FIRST, so their onLoad hooks (`.svg`, assets) win.
    plugins: [viteShapesPlugin(SHAPES), vuePlugin, entryPlugin],
    minify: { whitespace: true, identifiers: true, syntax: true },
    sourcemap: "none",
  });
  if (!result.success) {
    for (const log of result.logs) console.error(log);
    throw new Error("prod build failed");
  }
  const body = tagModuleNamespaces(await result.outputs[0].text()).code;

  // The whole point of the prod define block: no dev branches survive.
  if (/\b__DEV__\b/.test(body)) {
    throw new Error("__DEV__ survived minification — the define block did not apply");
  }
  // An `import.meta.glob` the expansion did not see (a non-literal argument
  // is an error already, so this is a shape vitecompat.ts does not know).
  const glob = new RegExp(`\\b${GLOB_TRIPWIRE}\\b`).exec(body);
  if (glob) {
    throw new Error(
      `an import.meta.glob(...) survived the build and would throw at startup: …${body.slice(Math.max(0, glob.index - 80), glob.index + 80)}…`,
    );
  }
  // The bundle must also PARSE as a classic script: a surviving `import.meta` is a SyntaxError that
  // takes the whole bundle down at eval, so fail at build time. Parse rather than grep (a string
  // literal may mention it); `new Function` compiles without running.
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

  // index.html's own sheets, through the same pipeline as a CSS entry.
  const htmlCss = async (pieces: CssPiece[]) => {
    const out: string[] = [];
    for (const p of pieces) {
      if (declaresUtilities(p.text)) appOwnsUtilities = true;
      out.push(await viteCss(p.text, p.from, toId(p.from)));
    }
    return out;
  };
  const headCss = LIFTED ? await htmlCss(LIFTED.headCss) : [];
  const bodyCss = LIFTED ? await htmlCss(LIFTED.bodyCss) : [];

  // Styles run before the bundle (createApp joins __VN_CSS__ on mount, so first paint is styled),
  // sorted by module id to match the dev loader's cascade. The Tailwind sheet is keyed
  // ".vn/tailwind.css", which sorts first, as in dev; it is skipped when an app stylesheet already
  // declares `@tailwind utilities`. index.html's sheets bracket it: <head> ones before the bundle's
  // CSS, <body> ones after.
  if (TW_CONFIG && !appOwnsUtilities) {
    const t = Date.now();
    cssById.set(TW_ID, await generateTailwind(TW_CONFIG));
    tailwindMs = Date.now() - t;
  }
  // The vendor-prefixed declarations vn-css would drop are dropped here, so
  // they are not shipped either (css-prefixes.ts).
  const sheets = [
    ...headCss,
    ...Array.from(cssById.keys())
      .sort()
      .map((id) => cssById.get(id)!),
    ...bodyCss,
  ].map(pruneVendorPrefixes);
  const css = sheets.map((text) => `(globalThis.__VN_CSS__ ??= []).push(${J(text)});`).join("\n");

  const out = asciiBundle(
    [
      `// vue-native production bundle - entry ${toId(ENTRY_ABS)}`,
      `// generated by packages/vue-native/prod.ts; do not edit`,
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
  if (AOT_INPUTS) {
    const pinsPath = join(WS_ROOT, "vendor", "redjs", "pins.txt");
    const engine = aotEngineIdentity(existsSync(pinsPath) ? readFileSync(pinsPath, "utf8") : null);
    const manifest = AOT_INPUTS.write({ script: out, scriptUrl: "vn://app.prod.js", defines: DEFINES, engine, sdkVersion: runtimeRequire("./package.json").version });
    console.log(`[prod] typed AOT inputs: ${manifest} (source archive; native AOT compilation not enabled)`);
  }

  for (const [what, n] of SHAPE_NOTES) console.log(`[prod] ${what}: ${n}`);
  if (LIFTED) {
    console.log(
      `[prod] index.html: ${LIFTED.headCss.length + LIFTED.bodyCss.length} stylesheet(s), ` +
        `${LIFTED.scripts.length} inline script(s) lifted`,
    );
  }
  for (const [name, dep] of FRAMEWORK) {
    if (dep.from === "app") console.log(`[prod] ${name} ${dep.version}: the app's own copy`);
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

await build().catch((e) => {
  console.error(e);
  process.exit(1);
});
