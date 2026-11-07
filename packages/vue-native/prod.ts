// vue-native PRODUCTION builder: one self-contained IIFE, no loader, no HMR.
//
// Usage: bun prod.ts [app-entry.js] [out-file] [--config <path>] [--app-root <path>] [--define=KEY=VALUE ...]
//
//   dist/app.prod.js   IIFE — @vue/runtime-core + src/runtime.ts + every app
//                      module + every SFC <style>, minified, __DEV__ = false.
//                      `vn build` (cli/build-app.ts) packs it into the app's
//                      VNPK payload and stamps that into a prebuilt host, so
//                      the shipped exe needs no dist/ and no Rust toolchain
//                      (docs/PACKAGING.md). crates/vn-app/build.rs also runs
//                      this file, for the embedded devtools inspector.
//   dist/window.json   the `window` block of vn.config.json (see below)
//   dist/assets.json   [{ key, path }] — every file an import referred to by
//                      URL (`?url`, `import x from "./a.png"`), which `vn build`
//                      packs into the payload under that key (vitecompat.ts)
//
// Differences vs the dev builder (build.ts), and why:
//
//   dev (build.ts)                    prod (this file)
//   ------------------------------    ---------------------------------------
//   vendor.js + one script per file   ONE iife (nothing to hot-swap)
//   __VN__ registry / require()       Bun bundles the graph; "vue" and
//                                     "vue-native" are ALIASED to src/runtime.ts
//   inlineTemplate: false (rerender   inlineTemplate: true — no HMR means the
//     needs a separate _sfc_main.     render fn may live inside setup(), which
//     render to swap)                 is faster and smaller
//   __VN__.setCss(id, css)            (globalThis.__VN_CSS__ ??= []).push(css)
//                                     emitted ahead of the bundle; runtime.ts
//                                     createApp installs the join on mount
//   __DEV__ true, HMR epilogue        __DEV__ false, no createRecord/__hmrId
//
// UNCHANGED and non-negotiable (research report F7 / e.3):
// - hoistStatic is FORCED OFF in prod too — the host has no
//   insertStaticContent, so createStaticVNode would render nothing. The
//   emitted bundle is asserted free of it below.
// - the scope id hashes the module id only, so it matches the dev builder's
//   (`Bun.hash(id)`), which keeps dev/prod CSS identical.
//
// VITE PARITY (vitecompat.ts): the import shapes a Vite app writes (`?raw`,
// `?url`, assets, `.svg` components, `import.meta.glob`, `?worker`/`.wasm`
// stubs), the app's `define`, SFC `<style>` through the app's Tailwind with
// `@import` inlined, `index.html`'s styles and classic scripts, and the app's
// own pinia/vue-router. research/reports/m26/client-gap.md §3.1 is the list.
//
// DEBT: compileVue()/the style pipeline below is a deliberate ~100-line copy of
// build.ts. build.ts is owned by the dev-loop track this round, so importing
// from it (or refactoring a shared module out of it) is off limits; the two
// must be merged into a shared `sfc.ts` next round.

import { parse, compileScript, compileTemplate, compileStyle, registerTS } from "@vue/compiler-sfc";
import { createRequire } from "module";
import { resolve, dirname, join, relative, sep } from "path";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "fs";

import { asciiOnly, checkAscii, nonAsciiCount } from "./ascii";
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

// ---- paths ----------------------------------------------------------------

const { defines: cliDefines, rest: argv } = takeDefineArgs(process.argv.slice(2));
let configOverride: string | undefined;
let appRootOverride: string | undefined;
const positional: string[] = [];
for (let i = 0; i < argv.length; i++) {
  const arg = argv[i]!;
  const flag = arg.split("=", 1)[0];
  if (flag === "--config" || flag === "--app-root") {
    const value = arg.includes("=") ? arg.slice(arg.indexOf("=") + 1) : argv[++i];
    if (!value?.trim() || value.startsWith("--")) throw new Error(`${flag} needs a path`);
    if (flag === "--config") configOverride = resolve(value);
    else appRootOverride = resolve(value);
  } else if (!arg.startsWith("--")) positional.push(arg);
}

const PKG_DIR = import.meta.dir; // .../packages/vue-native
const WS_ROOT = resolve(PKG_DIR, "../.."); // repo root — module ids are relative to this
const RUNTIME_ENTRY = join(PKG_DIR, "src", "runtime.ts");
/** vue-native's LiveKit, the module `livekit-client` means here (docs/VOICE.md). */
const LIVEKIT_ENTRY = join(PKG_DIR, "src", "livekit.ts");

const ENTRY_ABS = resolve(positional[0] ?? join(WS_ROOT, "apps/hello/src/main.ts"));
const APP_SRC_DIR = dirname(ENTRY_ABS);
const OUT_FILE = resolve(positional[1] ?? join(PKG_DIR, "dist", "app.prod.js"));

// `apps/<app>/vn.config.json`'s `window` block, emitted next to the bundle as
// `window.json`: `vn build` packs it as the payload's `vn/window.json`, and
// the inspector build `include_str!`s its own. The host reads it BEFORE
// `cx.open_window`, so it cannot ride inside the bundle (which evaluates long
// after the window exists) - see docs/WINDOWING.md 4.1. Always written, `{}`
// when the app declares none.
//
// It sits next to `src/`, which is NOT the same thing as the app's package
// root: `APP_ROOT` below walks up to the nearest `package.json`/`node_modules`
// for module resolution and can land a level higher. Same name, same rule and
// same reason as `build.ts`'s `VN_CONFIG_DIR`.
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
  // The top-level `"devtools": false` (docs/DEVTOOLS.md) rides along inside
  // the window block: that blob is the only JSON the host reads before the
  // window exists, and since m22 the flag decides whether a devtools listener
  // is bound at all. Same fold, same precedence, as build.ts - a `window`
  // block that declares its own wins.
  if (typeof cfg.devtools === "boolean" && typeof win.devtools !== "boolean") {
    win.devtools = cfg.devtools;
  }
  // `"httpCache": { "maxMB": N }` (docs/STORAGE.md §12): the host installs the
  // cache before any JS runs, so it rides here too, with the same precedence.
  if (cfg.httpCache !== undefined && win.httpCache === undefined) {
    win.httpCache = cfg.httpCache;
  }
  return JSON.stringify(win, null, 2);
}

/// Same discovery rule as the dev builder — no config, no Tailwind, and the
/// emitted bundle is byte-identical to a pre-Tailwind one.
const TW_CONFIG = findConfig(APP_SRC_DIR, WS_ROOT);

// ---- app package root + TypeScript-aware SFC compilation -------------------
// Mirrors build.ts exactly; see the long comment there for why `fs` and
// `registerTS` are both required to compile a component that writes
// `defineProps<SomeImportedType>()`. npm DEPENDENCIES need nothing special
// here: prod is a single Bun bundle, so a bare specifier resolves and inlines
// like any other import (only "vue"/"vue-native"/"livekit-client" are
// re-pointed, and pinia/vue-router chosen, below).
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

/** Build-log lines, said once each, printed after the build. */
const advisories = new Set<string>();
const advise = (m: string) => advisories.add(m);

// ---- defines --------------------------------------------------------------

const APP_DEFINES = appDefines(VN_CONFIG.define, cliDefines, VN_CONFIG_PATH);
if (APP_DEFINES.errors.length) {
  for (const e of APP_DEFINES.errors) console.error(`[prod] ${e}`);
  process.exit(1);
}

/**
 * What `vite build` inlines for `import.meta.env`, plus `VN: true` (vue-native's own, see build.ts)
 * and the app's own `import.meta.env.*` defines.
 */
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

// Vue's esm-bundler build reads these as bare identifiers. Same block as
// build.ts with the two production flips; `__DEV__: false` is what strips
// hmr.ts, customFormatter.ts and every dev warning through the
// `/*@__PURE__*/` + `__DEV__` guards.
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
  // `import.meta` is a SyntaxError in a classic script, and this bundle is one
  // (an IIFE the host evaluates) — Bun leaves `import.meta` VERBATIM in an
  // iife output, so a single `import.meta.env.MODE`, which is how every Vite
  // app asks what build it is, used to work in dev (build.ts defines it) and
  // kill the whole prod bundle at eval. Same block as build.ts, production
  // values: this is the substitution Vite itself performs, so it is parity
  // rather than a shim. Dotted forms first (Bun matches the longest key); the
  // bare `import.meta` catches destructuring. The tripwire in `build()` fails
  // the build if anything still survives.
  //
  // The object values MUST be JSON: Bun parses a define value as JSON (or an
  // identifier) and inserts anything else as a STRING — see the same block in
  // build.ts for how that made `const { MODE } = import.meta.env` undefined.
  "import.meta.env.MODE": '"production"',
  "import.meta.env.DEV": "false",
  "import.meta.env.PROD": "true",
  "import.meta.env.SSR": "false",
  "import.meta.env.BASE_URL": '"/"',
  // vue-native's own (build.ts): `if (!import.meta.env.VN) import("…")` keeps
  // browser-only code, and what it imports, out of the bundle.
  "import.meta.env.VN": "true",
  "import.meta.env": JSON.stringify(VITE_ENV_PROD),
  "import.meta.hot": "undefined",
  "import.meta.url": '"vn://app.prod.js"',
  // An `import.meta.glob` the load-time expansion did not reach would
  // otherwise become `{env,url}.glob(...)` — valid syntax, so the classic-script
  // check below passes, and a TypeError at eval. Defined to a name instead, so
  // a survivor is FINDABLE in the output and fails the build (vitecompat.ts).
  "import.meta.glob": GLOB_TRIPWIRE,
  "import.meta": JSON.stringify({ env: VITE_ENV_PROD, url: "vn://app.prod.js" }),
  // The app's own (vn.config.json "define", `--define`), Vite's semantics.
  ...APP_DEFINES.defines,
};

function writeAtomic(path: string, text: string) {
  const tmp = path + ".tmp";
  writeFileSync(tmp, text);
  renameSync(tmp, path);
}

// ---- index.html -----------------------------------------------------------
// A Vite app keeps its global CSS and its theme script in `index.html` next to
// `src/`. Nothing reads that file in a window that has no document, so it is
// lifted: styles into the stylesheet in document order, classic scripts into a
// prelude module that runs before the entry, the mount element's attributes
// onto the root (vitecompat.ts `liftIndexHtml`).
const INDEX_HTML = join(VN_CONFIG_DIR, "index.html");
const LIFTED: LiftedHtml | null = existsSync(INDEX_HTML) ? liftIndexHtml(INDEX_HTML, VN_CONFIG_DIR) : null;
const HTML_PRELUDE =
  LIFTED && (LIFTED.scripts.length || LIFTED.rootClasses.length || Object.keys(LIFTED.rootAttrs).length)
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

// ---- SFC pipeline (see DEBT note in the header) ----------------------------

/** Stable per-file scope id — identical formula to build.ts, so dev and prod
 *  produce byte-identical `data-v-*` ids for the same file. */
const scopeIdOf = (id: string) => Bun.hash(id).toString(16).slice(0, 8);

/**
 * Template comments are DROPPED in a production build, as Vite's are: its
 * build runs with NODE_ENV=production, which loads compiler-core's prod build,
 * whose `comments` default is false. This process loads the dev compiler, so
 * a comment was kept, and a component whose template starts with one
 * (`<!-- why --><keep-alive>…`) compiled to a FRAGMENT root. The dev runtime
 * sees through that (`filterSingleRoot`, `DEV_ROOT_FRAGMENT`); the prod runtime
 * this bundle runs does not, so the component lost its single root: no
 * fallthrough attrs and none of its parent's scope id. Argon's UserBar
 * `.user-avatar[data-v-…] { margin-right: 10px }` never matched the
 * ArgonAvatar it styles.
 */
const PROD_TEMPLATE_COMMENTS = false;

/** id -> compiled css, filled by the .vue onLoad hook below. */
const cssById = new Map<string, string>();

/**
 * A stylesheet's text through what Vite's CSS pipeline does for every sheet:
 * local `@import`s inlined (postcss-import), then the app's Tailwind when the
 * text uses it (`@apply`, `@layer`, `theme()`, `@tailwind`) — 228 `@apply`s in
 * the Argon client's SFCs reached vn-css verbatim before this and were dropped
 * there without a word.
 */
async function viteCss(text: string, from: string, label: string): Promise<string> {
  let css = inlineCssImports(text, from, VN_CONFIG_DIR, advise);
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
  // The template AST is parsed HERE and reused by compileTemplate, so this is
  // where comments are dropped; `compilerOptions.comments` below only covers
  // a template compiled from source.
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
  // Same rule as build.ts: compileScript leaves TS syntax in its output, so the
  // chunk must be handed to Bun with the matching loader. RETURNED rather than
  // stashed in a module variable: the style pass below awaits, so two
  // components compile interleaved.
  const isTS = /^tsx?$/.test(descriptor.scriptSetup?.lang ?? descriptor.script?.lang ?? "");

  const chunks: string[] = [];

  if (descriptor.scriptSetup || descriptor.script) {
    const compiled = compileScript(descriptor, {
      id: shortId,
      isProd: true,
      fs: SFC_FS,
      // Prod has no HMR, so the render fn may be inlined into setup() —
      // smaller output and one less property lookup per render.
      inlineTemplate: true,
      genDefaultAs: "_sfc_main",
      templateOptions: {
        compilerOptions: {
          // compileTemplate hard-defaults this to true and then spreads
          // compilerOptions over it, so ours wins (F7).
          hoistStatic: false,
          // vue-native host elements the HTML tag list does not contain —
          // without this `<lottie>` compiles to resolveComponent("lottie").
          // Lockstep with build.ts, where the rationale is written out.
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

  // F7 tripwire, applied to the COMPILED SFC rather than to the finished
  // bundle: `createStaticVNode` needs `insertStaticContent`, which the host
  // does not implement, so a hoistStatic leak must fail the build. It has to be
  // checked here — the bundle also *contains* runtime-core's own definition of
  // the function (any namespace re-export, e.g. through vue-router or pinia,
  // keeps it from being tree-shaken), so scanning the output is a false
  // positive waiting to happen. The compiled template is the only place the
  // leak can originate.
  let code = chunks.join("\n");
  if (/\bcreateStaticVNode\b/.test(code)) {
    throw new Error(`${id}: emitted createStaticVNode — hoistStatic leaked back on (F7)`);
  }
  // `import.meta.glob` / worker URLs written inside a component's <script>.
  code = transformSource(code, absPath, SHAPES) ?? code;

  const cssChunks: string[] = [];
  for (const style of descriptor.styles) {
    let text = style.content;
    // Vite's order: the CSS pipeline (postcss-import, Tailwind) first, the
    // scoped/v-bind transform after — so an `@import`ed sheet is scoped too
    // and `@apply`'s expansion lands inside the scoped rule.
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
      // MUST match compileScript's `isProd`: both hash a `v-bind()` variable's
      // name from it, and a mismatch makes useCssVars write `--x` while the
      // rule reads `--y`.
      isProd: true,
    });
    if (out.errors.length) throw new Error(`${id} <style>: ${out.errors.join("; ")}`);
    cssChunks.push(out.code);
  }
  if (cssChunks.length) cssById.set(id, cssChunks.join("\n"));

  return { contents: code, loader: isTS ? "ts" : "js" };
}

/**
 * Stylesheets the app imported from JS (`import "./index.css"`), in first-seen
 * order — the prod half of build.ts's CSS-entry support.
 *
 * They are keyed by module id into `cssById` like a component's `<style>`, so
 * the one sorted aggregation below orders them, and both builders agree on the
 * cascade. `.vn/` sorts first (utilities), then everything else by id.
 */
const cssEntryIds: string[] = [];
/** True when one of them declares `@tailwind utilities` itself. */
let appOwnsUtilities = false;

// ---- the import shapes, and the framework packages ---------------------------

/** payload key -> absolute path: every asset an import referred to by URL. */
const ASSETS = new Map<string, string>();
const SHAPE_NOTES = new Map<string, number>();
const SHAPES = {
  docRoot: VN_CONFIG_DIR,
  wsRoot: WS_ROOT,
  svgDefault: svgDefaultOf(APP_PKG),
  compileSfc: (absPath: string, source: string) => compileVue(absPath, source),
  assets: ASSETS,
  notes: SHAPE_NOTES,
};

// pinia / vue-router: the app's own copy when it declares one (an app on
// pinia 3 gets pinia 3, as in its Vite build), vue-native's otherwise.
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

/** Does the app have the browser SDK installed — the case where the substitution is worth a line? */
const APP_HAS_LIVEKIT = (() => {
  if (!APP_PKG) return false;
  try {
    appRequire.resolve("livekit-client");
    return true;
  } catch {
    return false;
  }
})();

const vuePlugin = {
  name: "vue-sfc-prod",
  setup(build: any) {
    // `import "./index.css"` — the shape every Vite app's entry has, and the
    // file a shadcn-shaped app keeps its whole palette in. Bun would treat a
    // .css file as an asset; it becomes an empty module whose CONTENT went into
    // the stylesheet instead. Same rule as build.ts: when the app has a
    // Tailwind config the sheet goes through Tailwind's own PostCSS pass with
    // ITSELF as the entry, which is what makes its `@tailwind`, `@layer` and
    // `@apply` mean anything — after its local `@import`s are inlined, as
    // postcss-import does in Vite.
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
    // Without the dev loader's pre-seeded registry, "vue" (compiled render
    // helpers) and "vue-native" (app code) have to resolve somewhere: both are
    // our runtime module, exactly as the loader aliases them at runtime.
    build.onResolve({ filter: /^(vue|vue-native)$/ }, () => ({ path: RUNTIME_ENTRY }));
    // "livekit-client" is vue-native's LiveKit, as in dev (build.ts PRESEEDED,
    // where the reason is written out): an app with the npm SDK installed —
    // for its typings, or because it also ships a browser build — must not get
    // the browser SDK silently bundled into its exe.
    build.onResolve({ filter: /^livekit-client$/ }, () => {
      if (APP_HAS_LIVEKIT) {
        advise("livekit-client → vue-native's LiveKit (src/livekit.ts); the app's npm SDK is not bundled (docs/VOICE.md)");
      }
      return { path: LIVEKIT_ENTRY };
    });
    // The other two pre-seeded ids ride vendor.js in dev, which means they are
    // resolved from THIS package's node_modules there. Prod resolves them once,
    // up front (FRAMEWORK above), so every importer — app code, a workspace
    // package, a library — lands on ONE copy.
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

// ---- ASCII ----------------------------------------------------------------

/**
 * The bundle with every non-ASCII character escaped (ascii.ts), so the host's
 * JavaScriptCore keeps its source 8-bit: one character outside ASCII makes the
 * whole source UTF-16, twice the memory for the life of the process. The
 * Argon client's translations made that 21 MB instead of 12
 * (research/reports/m26/perf-mem-3.md §3).
 *
 * The pass never fails the build. When it cannot lex the bundle, or Bun's
 * parser reads the result differently, the bundle is written as it was and an
 * advisory says why. `VN_BUNDLE_UTF8=1` skips the pass.
 */
function asciiBundle(code: string): string {
  const before = nonAsciiCount(code);
  if (!before || process.env.VN_BUNDLE_UTF8 === "1") return code;
  // UTF-16 takes one byte more per character than 8-bit.
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

// ---- build ----------------------------------------------------------------

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
  // Namespace objects that say they are (`import()` → defineAsyncComponent):
  // vitecompat.ts `tagModuleNamespaces`.
  const body = tagModuleNamespaces(await result.outputs[0].text()).code;

  // (The F7 createStaticVNode tripwire now runs per-SFC inside compileVue —
  // see the comment there for why the finished bundle is the wrong place.)
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
  // …and the bundle must PARSE as the classic script the host evaluates it as.
  // A surviving `import.meta` is the case this exists for — a SyntaxError that
  // takes the WHOLE bundle down at eval — and it has to fail here, at build
  // time, rather than on a user's machine as a blank window. Parsing (bun's
  // parser is JavaScriptCore's, the host's own engine) rather than grepping,
  // because a library's string literal that merely MENTIONS `import.meta`
  // would be a false positive. `new Function` compiles without running.
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

  // Styles run BEFORE the bundle: runtime.js createApp joins __VN_CSS__ into
  // the stylesheet on mount, so the first paint is already styled. Sorted by
  // module id so the cascade order matches the dev loader's (which aggregates
  // in sorted-file-scan order).
  //
  // The Tailwind utilities sheet is keyed under the SAME synthetic id the dev
  // builder uses (".vn/tailwind.css"), whose leading "." sorts it first — so
  // dev and prod put the utilities layer in exactly the same place: ahead of
  // every component <style>. See tailwind.ts for why that ordering is right.
  //
  // …unless one of the app's OWN stylesheets already declares `@tailwind
  // utilities`, in which case it carries them and a second copy would be pure
  // duplication. build.ts makes the same call for the same reason.
  //
  // index.html's sheets bracket all of it the way a browser orders them: the
  // ones in <head> come before the bundle's CSS (which Vite injects as a
  // <link> at the end of <head>), the ones in <body> after it.
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
