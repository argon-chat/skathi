// vue-native dev builder, v2: two-tier output for REAL Vue HMR.
//
// Usage: bun build.ts [app-entry.js] [dist-dir] [--watch]
//
//   dist/vendor.js            IIFE — @vue/runtime-core + src/runtime.ts +
//                             src/loader.ts. Installs globalThis.__VN__.
//                             Rebuilt only when packages/vue-native/src/* change.
//   dist/modules/<enc>.js     one CLASSIC SCRIPT per app source file:
//                               __VN__.setCss("<id>", "...")      (if it has <style>)
//                               __VN__.setDeps("<id>", ["<id>", ...])
//                               __VN__.define("<id>", function (require, module, exports) { ... },
//                                             "<bodyHash>")
//                             <enc> = id with "/" -> "__".
//   dist/manifest.json        { rev, vendorRev, entry, modules: ["<id>", ...],
//                               deps: { "<id>": ["<imported id>", ...] } }
//   dist/changes.json         { rev, vendorRev, changed: [{ id, kind }] }
//
// Module ids are workspace-relative POSIX paths ("apps/hello/src/App.vue").
//
// The module GRAPH is what makes a shared plain .js file hot-reloadable: the
// loader walks importers up to the nearest .vue components and reloads those
// through Vue's HMR record instead of restarting the app. It rides along in the
// per-module script (`setDeps`) because the loader never reads manifest.json —
// only the Rust host does. `manifest.deps` mirrors it for tooling (prod builds,
// debugging) and is additively ignored by the host's reader.
//
// `bodyHash` (3rd arg of define) hashes the FACTORY BODY only — not the CSS —
// so a <style>-only edit redefines the module without invalidating its require
// cache entry, leaving the live exports (and the component def Vue's HMR record
// already owns) in place.
//
// TAILWIND rides the same transport as a SYNTHETIC CSS-ONLY MODULE: when the
// app has a tailwind.config.* (see ./tailwind.ts), the generated utilities
// sheet is written as dist/modules/.vn__tailwind.css.js — a real module script
// the host evals like any other, that calls setCss/setDeps/define and whose
// factory is empty. Nothing new in the protocol; the id (".vn/tailwind.css")
// sorts first, so utilities land ahead of every component <style> in the
// aggregate. A template edit that introduces a new class re-runs Tailwind and
// emits a "css-only" change entry — the running app restyles with NO component
// churn (no rerender, no reload, no state loss).
//
// npm DEPENDENCIES ride the same transport as SHARED SYNTHETIC MODULES, one
// per bare specifier the app imports (`reka-ui`, `clsx`, `@vueuse/core`, …).
// Each is a full Bun bundle of that package — its own transitive npm graph
// inlined — emitted as `dist/modules/<spec>.js` under the bare specifier as its
// module id, which is exactly what `require("reka-ui")` resolves to in the
// loader (bare ids are registry keys, used verbatim). `vue`, `vue-native`,
// `vue-router` and `pinia` stay EXTERNAL inside those bundles, so a library's
// own `import { ref } from "vue"` lands on our runtime, not a second copy.
//
// ONE SHARED COPY, NOT ONE PER IMPORTER. Inlining a package into every module
// that imports it is simpler, but it is also wrong for any library that keeps
// module-scope identity — and the headless-primitive libraries this exists for
// are exactly that. reka-ui's `createContext` mints `Symbol()` injection keys
// at module scope:
//
//     const injectionKey: InjectionKey<...> = Symbol(symbolDescription)
//
// With one copy per importer, `AccordionRoot` (Accordion.vue) and
// `AccordionItem` (AccordionItem.vue) each get their OWN symbol, so the inject
// misses and every multi-file primitive throws at setup. Same class of bug for
// singleton registries and `instanceof` checks. A bundler in the browser
// dedupes for the same reason; so do we.
//
// `kind` is derived by diffing each .vue file's previous descriptor blocks:
//   script/scriptSetup changed, or a <style> block's scoped/module/lang shape
//                                changed          -> "reload"
//   template changed                              -> "rerender"   (state survives)
//   only <style> text changed                     -> "css-only"
//   non-.vue source changed                       -> "reload"
//
// Non-negotiables (research report F7 / e.3):
// - hoistStatic is FORCED OFF — our host has no insertStaticContent.
// - inlineTemplate is FORCED OFF — with an inline render the template lives
//   inside setup(), so __VUE_HMR_RUNTIME__.rerender cannot swap it without
//   re-running setup (which is exactly the state loss we are removing).
//   A separate `render` on _sfc_main is what makes rerender state-preserving.
// - the scope id hashes the module id ONLY, never the source, so [data-v-x]
//   stays stable across edits.

import { parse, compileScript, compileTemplate, compileStyle, registerTS } from "@vue/compiler-sfc";
import type { SFCDescriptor } from "@vue/compiler-sfc";
import { createRequire } from "module";
import { resolve, dirname, join, relative, sep } from "path";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
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
import {
  GLOB_TRIPWIRE,
  SFC_FS,
  appDefines,
  devEsmInterop,
  devImportsPlugin,
  inlineCssImports,
  readPackageJson,
  svgDefaultOf,
  tagModuleNamespaces,
  takeDefineArgs,
  transformSource,
  viteShapesPlugin,
} from "./vitecompat";

// ---- paths ----------------------------------------------------------------

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

// ---- vn.config.json -------------------------------------------------------
// `apps/<app>/vn.config.json`'s `window` block is copied VERBATIM into the
// manifest. The Rust host reads it BEFORE `cx.open_window`, which is the only
// moment creation-time gpui options (min size, resizable, titlebar, backdrop)
// can be set without a visible flash (docs/WINDOWING.md 4.1).
//
// It sits next to `src/`, i.e. where a Vite app keeps `index.html` - the same
// directory a relative `<img src>` resolves against.
const VN_CONFIG_DIR = APP_SRC_DIR.endsWith(`${sep}src`) ? dirname(APP_SRC_DIR) : APP_SRC_DIR;
const VN_CONFIG_PATH = join(VN_CONFIG_DIR, "vn.config.json");

function readVnConfig(): Record<string, unknown> | undefined {
  if (!existsSync(VN_CONFIG_PATH)) return undefined;
  try {
    const cfg = JSON.parse(readFileSync(VN_CONFIG_PATH, "utf8"));
    return cfg && typeof cfg === "object" ? cfg : undefined;
  } catch (e) {
    // A malformed config must not take the whole build down, but it must also
    // never be silently ignored - the window would just look wrong.
    console.error(`[build] ${VN_CONFIG_PATH}: ${e}`);
    return undefined;
  }
}

// The top-level `"devtools": false` (docs/DEVTOOLS.md) rides along inside the
// window block, because that blob is the only JSON the host reads before the
// window exists - and since m22 the flag decides whether a devtools listener is
// bound at all, which has to be answered before a port is opened rather than by
// closing one a moment later.
//
// It was DOCUMENTED as top-level from the day it was added and never actually
// copied, so only the undocumented `window.devtools` spelling worked. Both work
// now; a `window` block that declares its own wins, because the more specific
// place is the one somebody edited last.
function foldDevtoolsFlag(
  cfg: Record<string, unknown> | undefined,
  win: Record<string, unknown> | undefined,
): Record<string, unknown> | undefined {
  if (!cfg || typeof cfg.devtools !== "boolean") return win;
  if (win && typeof win.devtools === "boolean") return win;
  return { ...(win ?? {}), devtools: cfg.devtools };
}

function readWindowConfig(): Record<string, unknown> | undefined {
  const cfg = readVnConfig();
  const win =
    cfg && typeof cfg.window === "object" && cfg.window
      ? (cfg.window as Record<string, unknown>)
      : undefined;
  return foldDevtoolsFlag(cfg, win);
}

// The app's own name, copied into the manifest for the Rust host.
//
// It names the app's PLACE ON DISK (%LOCALAPPDATA%\<name>\storage - see
// crates/vn-storage and docs/STORAGE.md), which is why it is derived here and
// not from the window title: a title is prose that changes, a storage folder is
// an identity that must not. A dev host runs many apps out of one vn-app.exe,
// so the executable cannot be the name either.
//
// Optional. Without it the host falls back to the app directory of the entry
// module ("apps/hello/src/main.ts" -> "hello"), which is right for every app in
// this repo; declaring `"name"` matters when the shipped app is called
// something other than its folder.
function readAppName(): string | undefined {
  const cfg = readVnConfig();
  const name = cfg?.name;
  return typeof name === "string" && name.trim() ? name.trim() : undefined;
}

// The launch keys (docs/LAUNCH.md), gathered from the TOP LEVEL of
// vn.config.json into one `launch` object for the host: `id`,
// `singleInstance`, `raiseOnSecondInstance`, `protocols`. The host reads it
// before anything else exists — the single-instance check is the first thing
// `main()` does — so it rides in the manifest like the window block does.
// Only well-typed keys are copied; `vn build` (cli/config.ts) is where a
// wrong one is an error, and the host re-validates every scheme anyway.
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
      console.error(`[build] ${VN_CONFIG_PATH}: "${key}" has the wrong type and was ignored (docs/LAUNCH.md)`);
    }
  }
  return Object.keys(out).length ? out : undefined;
}

// TEMPORARY (m26): `"argonSecure": true` turns on the host's ArgonSecure cookie
// producer (crates/vn-app/src/temp_argon_secure.rs) for the Argon client. It is
// copied only when it is exactly `true`; anything else is omitted, and the host
// reads a missing key as off. Goes away with the producer.
function readArgonSecure(): true | undefined {
  const cfg = readVnConfig();
  if (cfg?.argonSecure === undefined) return undefined;
  if (typeof cfg.argonSecure !== "boolean") {
    console.error(`[build] ${VN_CONFIG_PATH}: "argonSecure" must be true or false and was ignored`);
    return undefined;
  }
  return cfg.argonSecure ? true : undefined;
}

const toId = (abs: string) => relative(WS_ROOT, abs).split(sep).join("/");
const encId = (id: string) => id.split("/").join("__") + ".js";
const J = (v: unknown) => JSON.stringify(v);

// ---- the app's own package root -------------------------------------------
// An app may carry its own package.json + node_modules (apps/shadcn does;
// apps/hello does not). Bare specifiers, `typescript` and everything Bun
// resolves for the app are resolved from HERE, never from the builder's own
// node_modules — otherwise every app would have to declare its dependencies in
// packages/vue-native/package.json, which is backwards.
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

/**
 * Resolves a bare specifier from the APP root with BROWSER/ESM conditions —
 * what a bundler would pick. `createRequire().resolve` walks Node's CJS
 * conditions and cannot see an `exports` entry that only declares `import`,
 * which is every modern ESM-only workspace package and every `"./styles":
 * "./styles/index.css"` asset entry.
 */
function resolveFromApp(spec: string): string {
  try {
    return Bun.resolveSync(spec, APP_ROOT);
  } catch {
    return appRequire.resolve(spec);
  }
}

/**
 * The entry files to TRY for an npm package, best first.
 *
 * There is no single right resolver here, and the two ways of being wrong are
 * both real and both silent-ish:
 *
 * * Node's CJS conditions pick a package's **node** build when it ships one —
 *   `axios` resolves to `dist/node/axios.cjs`, which `require("http2")` and
 *   fails the bundle outright — and cannot see an ESM-only `exports` map at
 *   all, which is every `@argon/*`-shaped workspace package.
 * * The ESM entry is not always safe either: Bun's `format: "cjs"` output for a
 *   package with a CIRCULAR ESM graph hoists a binding it then reads before
 *   assignment. `reka-ui` bundles clean from `dist/index.cjs` and throws
 *   `ReferenceError: import_Primitive is not defined` from `dist/index.js`.
 *
 * So: prefer the CJS entry, which is the one whose *output* is known-good, and
 * fall back to the ESM/browser one when the CJS build actually FAILS. The
 * failure is a Bun.build error, not a guess — an unresolvable Node builtin is
 * exactly how the axios case announces itself.
 */
function entryCandidates(spec: string): string[] {
  const out: string[] = [];
  try {
    out.push(appRequire.resolve(spec));
  } catch {
    /* ESM-only, or no `main` — Bun's resolver is the answer */
  }
  try {
    const esm = Bun.resolveSync(spec, APP_ROOT);
    if (!out.includes(esm)) out.push(esm);
  } catch {
    /* nothing more to try */
  }
  return out;
}

// ---- <script lang="ts"> across files ---------------------------------------
// `defineProps<TabsRootProps>()` where the type is IMPORTED (the dominant style
// in real component libraries — every shadcn-vue wrapper does it) makes
// compileScript read and parse other files to build the runtime props object.
// Two things have to be handed to it or it throws instead:
//
//   fs         Bun is not detected as Node by compiler-sfc's environment probe,
//              so without this every such component fails with
//              "No fs option provided to `compileScript` in non-Node environment".
//   registerTS resolving a type across a *package* boundary ("reka-ui") goes
//              through TypeScript's own module resolution; compiler-sfc treats
//              `typescript` as an optional peer and errors with
//              "TypeScript is required as a peer dep for vue …" when absent.
//              We load it from the APP's node_modules, so an app that needs
//              cross-package type resolution declares `typescript` itself and
//              apps that don't stay dependency-free.
//
// SFC_FS lives in vitecompat.ts, shared with prod.ts: its `fileExists` answers
// for FILES only, because compiler-sfc probes `./types` before `./types.ts`
// and a directory of that name used to win (client-gap/B-build-ui.md row 2).

const TS_AVAILABLE = (() => {
  try {
    const ts = appRequire("typescript");
    registerTS(() => ts);
    return true;
  } catch {
    return false;
  }
})();

// ---- bare specifiers -------------------------------------------------------
// Provided by vendor.js and pre-seeded in the loader's registry: they must stay
// external everywhere (a second copy of Vue would be a second reactivity
// system), and they are never emitted as modules.
//
// "livekit-client" is not a convenience entry. Without it, an app author who
// runs `bun add livekit-client` for the TypeScript typings — a LIKELY thing to
// do — gets the real browser SDK silently bundled into dist/modules/ by the
// npm path below, and the failure is `WebSocket is not defined` deep inside
// minified code. This entry is what makes that substitution impossible.
const PRESEEDED = ["vue", "vue-native", "vue-router", "pinia", "livekit-client"];
const isPreseeded = (spec: string) => PRESEEDED.includes(spec);

// The app's own defines: vn.config.json `"define"` plus `--define K=V`, with
// Vite's semantics (vitecompat.ts `appDefines`). Read once at startup — a
// watcher picks up a changed `define` on restart, as `vite` does.
const APP_DEFINES = appDefines(readVnConfig()?.define, cliDefines, VN_CONFIG_PATH);
if (APP_DEFINES.errors.length) {
  for (const e of APP_DEFINES.errors) console.error(`[build] ${e}`);
  process.exit(1);
}

/** What Vite's dev server inlines for `import.meta.env` — plus the app's own `import.meta.env.*` defines. */
const VITE_ENV_DEV: Record<string, unknown> = { MODE: "development", DEV: true, PROD: false, SSR: false, BASE_URL: "/" };
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
  // `import.meta` in a CLASSIC SCRIPT is a hard SyntaxError, and the module
  // scripts this builder emits are classic scripts. It is not an exotic thing
  // for an app to write either: `import.meta.env.MODE` is how every Vite app
  // asks what build it is, and one occurrence kills the whole module at eval —
  // which on a real app was five modules, entry included.
  //
  // Substituting the values Vite would have inlined is exactly what Vite does,
  // so this is parity rather than a shim. The dotted forms come first (Bun
  // matches the longest key), and the bare `import.meta` catches destructuring
  // (`const { MODE } = import.meta.env`) and anything else.
  //
  // The two OBJECT values are JSON.stringify'd, and that is load-bearing: Bun
  // parses a define value as JSON (or an identifier), and anything else — the
  // `({MODE:"development",…})` JS literal this used to be — is inserted as a
  // STRING. So `const { MODE } = import.meta.env` destructured a string and got
  // `undefined`, in dev AND in prod (m25 packaging, found by the prod test).
  // JSON has no `undefined`, so `hot` is simply absent from the bare object,
  // which reads back as `undefined` all the same.
  "import.meta.env.MODE": '"development"',
  "import.meta.env.DEV": "true",
  "import.meta.env.PROD": "false",
  "import.meta.env.SSR": "false",
  "import.meta.env.BASE_URL": '"/"',
  "import.meta.env": JSON.stringify(VITE_ENV_DEV),
  // A dev server's HMR client hook. Vue SFCs compiled by `@vue/compiler-sfc`
  // do not emit it (our HMR rides `__VUE_HMR_RUNTIME__`), but library code
  // guards on it, and `undefined` is the "not a dev server" answer.
  "import.meta.hot": "undefined",
  "import.meta.url": '"vn://module"',
  // `import.meta.glob(…)` calls are expanded before this applies
  // (the shapes plugin and `appModulePlugin`, vitecompat.ts `transformSource`). Anything left is defined
  // to a findable name, so buildModule can fail with a sentence instead of
  // the module dying at eval with `{env,url}.glob is not a function`.
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

// ---- tier 1: vendor.js ----------------------------------------------------

async function buildVendor() {
  const result = await Bun.build({
    entrypoints: [LOADER_ENTRY],
    format: "iife",
    target: "browser",
    define: DEFINES,
    plugins: [
      {
        // vue-router (and anything else vendored later) imports from "vue";
        // in the vendor bundle that must be OUR runtime module. Exact-match
        // filter only — bun 1.4 crashes on catch-all onResolve filters.
        name: "vendor-vue-alias",
        setup(b: any) {
          b.onResolve({ filter: /^vue$/ }, () => ({ path: join(RUNTIME_SRC_DIR, "runtime.ts") }));
        },
      },
    ],
  });
  if (!result.success) {
    for (const log of result.logs) console.error(log);
    throw new Error("vendor build failed");
  }
  const text = await result.outputs[0].text();
  // Sanity: the pre-seeded "vue" namespace must still carry the compiler
  // helpers that generated render functions call at runtime — if Bun ever
  // tree-shakes the `export *` re-export, templates die with "not a function".
  for (const sym of ["openBlock", "createElementBlock", "createElementVNode", "toDisplayString"]) {
    if (!text.includes(sym)) throw new Error(`vendor bundle is missing ${sym}`);
  }
  writeAtomic(join(DIST, "vendor.js"), text);
  // vendorRev is a CONTENT hash, not a counter: restarting the watcher must
  // not make a running host believe its already-evaluated vendor.js is current.
  vendorRev = Number(BigInt.asUintN(48, BigInt(Bun.hash(text) as any)));
  return text.length;
}

// ---- tier 2: one classic script per app module ----------------------------

/** Stable per-file scope id — must NOT depend on file contents (see header). */
const scopeIdOf = (id: string) => Bun.hash(id).toString(16).slice(0, 8);

/** Filled by the .vue onLoad hook; each Bun.build below handles exactly one file. */
let capturedCss: string | null = null;

/** Build-log lines said once per watcher run (an unsupported `<style lang>`, …). */
const devAdvised = new Set<string>();
function devAdvise(m: string) {
  if (devAdvised.has(m)) return;
  devAdvised.add(m);
  console.warn(`${stamp()} advisory: ${m}`);
}

/**
 * A stylesheet through what Vite's CSS pipeline does for every sheet: local
 * `@import`s inlined (postcss-import), then the app's Tailwind when the text
 * uses it (`@apply`, `@layer`, `theme()`, `@tailwind`). Same steps as prod.ts
 * `viteCss`; a failed Tailwind pass logs and keeps the inlined text, as the
 * CSS-entry path below always has, so one bad `@apply` does not stop a watcher.
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

/**
 * `captureCss` false: an SFC compiled INSIDE another module's build (an `.svg`
 * as a component) must not replace that module's captured `<style>`.
 */
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
  // compileScript strips the type-only bits it understands but leaves the rest
  // of the TS syntax in place (interfaces, `satisfies`, annotations on plain
  // locals) — the bundler is expected to transpile. Loading that chunk as "js"
  // is a syntax error, so the loader has to follow the block's `lang`.
  // RETURNED, not stashed: the style pass below awaits, and an npm bundle can
  // carry several SFCs, so two compiles can interleave.
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
        // vue-native HOST ELEMENTS the HTML tag list does not contain. Without
        // this the SFC compiler classifies an unknown lowercase tag as a
        // COMPONENT, so `<lottie>` would compile to resolveComponent("lottie")
        // and warn "Failed to resolve component" on every render. This is the
        // same escape hatch a browser app uses
        // (`app.config.compilerOptions.isCustomElement`), applied at build time
        // because our templates are precompiled. Keep in lockstep with
        // prod.ts and with the tag tests in crates/vn-tree/src/render.rs.
        isCustomElement: (tag: string) => tag === "lottie",
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

  // Our replacement for plugin-vue's `import.meta.hot` epilogue. createRecord
  // is idempotent (returns false when the record exists), so re-evaluating a
  // module never clobbers the record that holds the live instances.
  chunks.push(`_sfc_main.__hmrId = ${J(id)};`);
  chunks.push(
    `if (typeof __VUE_HMR_RUNTIME__ !== "undefined") __VUE_HMR_RUNTIME__.createRecord(${J(id)}, _sfc_main);`,
  );
  chunks.push("export default _sfc_main;");

  const cssChunks: string[] = [];
  for (const style of descriptor.styles) {
    let text = style.content;
    // Vite's order: the CSS pipeline first, the scoped/v-bind transform after,
    // so an `@import`ed sheet is scoped too and `@apply` lands inside the
    // scoped rule.
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
  if (captureCss) capturedCss = cssChunks.length ? cssChunks.join("\n") : null;

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

// ---- Vite's import shapes (vitecompat.ts) ----------------------------------
// The plugin prod.ts bundles with, so `?raw`, `?url`, assets, `.svg`,
// `?worker`, `new Worker(new URL(…))`, `.wasm` and `import.meta.glob` mean the
// same thing in both builders. What the plugin answers is built INTO the
// importer, as in prod's bundle. In dev, `import.meta.glob` imports by
// importer-relative path (what the module registry resolves; lazy entries are
// `() => import(…)`, which `devEsmInterop` routes through require), and the
// asset URLs are not packed: the dev host serves `/<key>` from the app root.

const APP_PKG = APP_ROOT === PKG_DIR ? null : readPackageJson(APP_ROOT);
/** What the shapes plugin did during a full build: printed after it, as prod.ts prints its own. */
const SHAPE_NOTES = new Map<string, number>();
/** The files the module being built inlined (`?raw`, `.json`, `.svg`, …), for `buildModule`. */
let inlinedNow: Set<string> | null = null;
const noteInlined = (abs: string) => void inlinedNow?.add(abs);

const shapesOptions = (dev: boolean) => ({
  docRoot: VN_CONFIG_DIR,
  wsRoot: WS_ROOT,
  svgDefault: svgDefaultOf(APP_PKG),
  compileSfc: (absPath: string, source: string) => compileVue(absPath, source, false),
  assets: new Map<string, string>(),
  notes: SHAPE_NOTES,
  dev,
  inlined: dev ? noteInlined : undefined,
});
const DEV_SHAPES = shapesOptions(true);
const devShapesPlugin = viteShapesPlugin(DEV_SHAPES);
/** An npm package is bundled whole, so the plugin works on it exactly as in prod. */
const npmShapesPlugin = viteShapesPlugin(shapesOptions(false));
/** `@/…` aliases and data/asset files (vitecompat.ts `devImportsPlugin`). */
const devImports = devImportsPlugin({ srcDir: APP_SRC_DIR, preseeded: isPreseeded, inlined: noteInlined });

/**
 * An APP module (`buildModule`): the SFC compile above, then the rewrites prod
 * applies to a component's compiled `<script>` (`transformSource`). A plain
 * source gets the same through the shapes plugin.
 */
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

// ---- module dependency graph ----------------------------------------------
// We build every module with external:"*", so each import survives the CJS
// transform as a literal `require("<spec>")` — the specifiers are recoverable
// straight from the emitted body, no second parse of the source needed. A
// tsconfig `paths` alias is already rewritten to its relative path there
// (vitecompat.ts `devImportsPlugin`), so it is an edge like any other.
//
// Bare specifiers are split two ways: PRESEEDED ones ("vue", "vue-native",
// "vue-router", "pinia") live in vendor.js and are deliberately kept OUT of the
// graph; every other one is an npm package we emit as its own shared module
// (see the header) and is likewise out of the graph — it never changes, so it
// has nothing to contribute to HMR's importer walk.

const REQUIRE_RE = /\brequire\(\s*("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')\s*\)/g;

/** Same candidate list as loader.ts `resolveId` — keep the two in lockstep. */
const CANDIDATE_EXTS = ["", ".js", ".ts", ".mjs", ".vue", "/index.js", "/index.ts", "/index.vue"];

function normalizeId(path: string): string {
  const out: string[] = [];
  for (const seg of path.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") {
      out.pop();
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

/** Resolved, deduped, in-graph ids that `id`'s emitted body requires. */
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
      // Not a divergence we can paper over: the loader will throw the same way
      // a browser would 404 — say so now, while there is a file to point at.
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

/** Non-preseeded bare specifiers `body` requires — i.e. the npm packages. */
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
const moduleBare = new Map<string, string[]>(); // id -> npm specifiers it requires

// ---- CSS entries: `import "./index.css"` from a JS/TS module ---------------
// A Vite app's global stylesheet is imported from its entry module, not linked
// from the HTML — and for a shadcn app that stylesheet is where the ENTIRE
// palette lives (`@layer base { :root { --background: … } .dark { … } }`). Left
// unhandled the `.css` file was emitted verbatim into a JS module body and died
// at eval with `SyntaxError: Invalid character: '@'`.
//
// The transport is the one Tailwind already rides: a CSS-ONLY SYNTHETIC MODULE
// — a real module script that calls setCss/setDeps/define with an empty factory.
// Nothing new in the protocol.

/** module id -> absolute path, for every stylesheet the app imports. */
const cssModules = new Map<string, string>();
/** id -> last emitted CSS, the change detector for watch mode. */
const cssEmitted = new Map<string, string>();

/**
 * Registers the stylesheets `body` imports and returns their ids.
 *
 * The id follows the loader's own resolution: a RELATIVE import is a
 * workspace-relative path (what `resolveId` builds), a BARE one is the
 * specifier verbatim (what the registry is keyed by). `cssSpecs` collects the
 * raw specifiers so `depsOf`/`bareOf` skip them — a stylesheet is neither a
 * graph edge nor an npm package.
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

/** Emitted CSS module ids, in first-import order — see `writeManifest`. */
const cssIds = () => Array.from(cssModules.keys());

/**
 * Writes one CSS-only module script. Returns true when the sheet moved.
 *
 * When the app has a Tailwind config the stylesheet goes through Tailwind's own
 * PostCSS pass with ITSELF as the entry — which is exactly what the app's Vite
 * build does, and the only way `@tailwind`, `@layer` and `@apply` can mean
 * anything. Without a config (or without any of those directives) it is emitted
 * byte-for-byte.
 */
async function buildCssModule(id: string): Promise<boolean> {
  const abs = cssModules.get(id)!;
  // Local `@import`s inlined first, as postcss-import does ahead of Tailwind.
  // The watcher rebuilds this sheet when IT changes; an edit to an imported
  // file shows on the next change here or on restart.
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

/** Rebuilds every registered stylesheet; returns the ids whose CSS moved. */
async function syncCssModules(): Promise<string[]> {
  const changed: string[] = [];
  for (const id of cssIds()) {
    if (await buildCssModule(id)) changed.push(id);
  }
  return changed;
}

/**
 * True when one of the app's own stylesheets already carries `@tailwind
 * utilities` — then the synthetic utilities module would be a duplicate.
 */
const appOwnsUtilities = () =>
  Array.from(cssEmitted.keys()).some((id) => {
    const abs = cssModules.get(id);
    return !!abs && declaresUtilities(readFileSync(abs, "utf-8"));
  });

// ---- inlined files ---------------------------------------------------------
// A `?raw` text, a `.json` or an `.svg` component is built INTO its importer,
// so an edit to it changes no module source. `incrementalBuild` rebuilds the
// importer when one of these moves, as Vite's HMR updates the importer.

/** module abs path -> the files its build inlined -> their mtime then. */
const moduleInlined = new Map<string, Map<string, number>>();

const mtimeOf = (abs: string) => {
  try {
    return statSync(abs).mtimeMs;
  } catch {
    return -1;
  }
};

/** True when a file `abs`'s build inlined has changed since. */
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
      // Transform, don't bundle: imports stay as require() calls, except what
      // the plugins resolve (an alias, rewritten) or build in (data, assets,
      // `?query` imports). The shapes plugin goes first: see devImportsPlugin.
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
  // A bundler's ESM semantics, not Node's: `import App from "./App.vue"` in a
  // "type": "module" package, and `import()` (vitecompat.ts `devEsmInterop`).
  // A namespace an inlined `import()` gives is Module-tagged, as in prod.
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
        `the dev builder would emit a call that throws at eval (docs/PACKAGING.md §3.1)`,
    );
  }

  // Stylesheets first: their specifiers must be known before `depsOf`/`bareOf`
  // classify the rest, or a `.css` import looks like a missing module (relative)
  // or an npm package (bare).
  const cssSpecs = new Set<string>();
  const css = cssImportsOf(absPath, body, cssSpecs);
  const deps = depsOf(id, body, ids, cssSpecs).concat(css);
  moduleDeps.set(id, deps);
  moduleBare.set(id, bareOf(body, cssSpecs));

  const out: string[] = [`// vue-native module: ${id}`];
  if (capturedCss !== null) out.push(`__VN__.setCss(${J(id)}, ${J(capturedCss)});`);
  // Guarded: a module script is also readable by an older vendor.js (a dist
  // that was only half-rebuilt) — a missing setDeps must degrade to "no graph",
  // not to a TypeError that kills the whole script.
  out.push(`__VN__.setDeps && __VN__.setDeps(${J(id)}, ${J(deps)});`);
  out.push(`__VN__.define(${J(id)}, function (require, module, exports) {`);
  out.push(body);
  out.push(`}, ${J(Bun.hash(body).toString(16))});`);
  writeAtomic(join(MODULES_DIR, encId(id)), out.join("\n"));
}

// ---- npm dependencies: one shared module per bare specifier ----------------
// Absent any npm import (apps/hello), `npmBuilt` stays empty and every line
// below is inert — the emitted manifest and module set are byte-identical to a
// pre-npm build.

/** specifier -> emitted body size, for the log and for change detection. */
const npmBuilt = new Map<string, number>();
/** Specifiers we already complained about, so watch mode says it once. */
const npmFailed = new Set<string>();

/** Bundles one npm package into a classic module script. False = unresolvable. */
async function buildNpmModule(spec: string): Promise<boolean> {
  // Resolved from the APP root, not ours: the app owns its dependencies.
  const candidates = entryCandidates(spec);
  if (!candidates.length) {
    if (!npmFailed.has(spec)) {
      npmFailed.add(spec);
      // `@/x`, `~/x`, `#x`: no npm package is named like that, so it is an
      // alias neither builder knows. Both read tsconfig `paths` only.
      const alias = !/^(?:@[a-z0-9][\w.-]*\/)?[a-z0-9][\w.-]*(?:\/|$)/i.test(spec);
      console.warn(
        `${stamp()} cannot resolve ${J(spec)} from ${toId(APP_ROOT)} — ` +
          `require(${J(spec)}) will throw at runtime ` +
          (alias
            ? "(an import alias resolves through tsconfig.json compilerOptions.paths, as in vn build; " +
              "a vite.config resolve.alias is not read — docs/PACKAGING.md §3.1)"
            : "(is it installed?)"),
      );
    }
    return false;
  }
  let result: Awaited<ReturnType<typeof Bun.build>> | null = null;
  let entry = candidates[0];
  for (const [i, candidate] of candidates.entries()) {
    entry = candidate;
    result = await Bun.build({
      entrypoints: [candidate],
      format: "cjs" as any,
      target: "browser", // picks the browser/ESM conditions, not node builtins
      // The ONLY externals: everything else (the package's own transitive npm
      // graph) is inlined, because nothing outside this bundle can require it.
      external: PRESEEDED,
      // A dependency can ship RAW `.vue` SOURCE — a vendored component library
      // (`@argon/ui`: 208 SFCs) is the normal case, not an exotic one. Without
      // the SFC plugin here Bun treats the file as an asset and the import
      // becomes a FILE-PATH STRING: `var Alert_default = "./Alert-7e55.vue"`.
      // That is a silent corruption — no error, the component renders as
      // nothing — which is the worst failure mode a compat-first builder can
      // have. Vite's import shapes (`.svg`, `?raw`, assets, …) go first, as in
      // prod.ts, where they apply to every package in the bundle.
      plugins: [npmShapesPlugin, vuePlugin],
      define: DEFINES,
    }).catch(() => null);
    if (result?.success) break;
    if (i + 1 < candidates.length) {
      console.warn(
        `${stamp()} npm ${spec}: ${toId(candidate)} did not build — retrying with ` +
          `${toId(candidates[i + 1])}`,
      );
    }
  }
  if (!result?.success) {
    for (const log of result?.logs ?? []) console.error(log);
    throw new Error(`npm module build failed: ${spec}`);
  }
  const body = tagModuleNamespaces(await result.outputs[0].text()).code;
  moduleDeps.set(spec, []);
  moduleBare.set(spec, []);
  writeAtomic(
    join(MODULES_DIR, encId(spec)),
    [
      `// vue-native module: ${spec} (npm dependency, bundled from ${toId(entry)})`,
      `__VN__.setDeps && __VN__.setDeps(${J(spec)}, []);`,
      `__VN__.define(${J(spec)}, function (require, module, exports) {`,
      body,
      // Constant across rebuilds unless the package itself changes, so a
      // rebuild never invalidates the live exports (and with them every
      // module-scope Symbol / singleton the package handed out).
      `}, ${J(Bun.hash(body).toString(16))});`,
    ].join("\n"),
  );
  npmBuilt.set(spec, body.length);
  return true;
}

/** Emitted npm module ids, sorted — manifest order. */
const npmIds = () => Array.from(npmBuilt.keys()).sort();

/**
 * Reconciles the emitted npm modules with what the app currently imports.
 * Returns the specifiers ADDED by this pass (they need a change entry so the
 * running host evaluates their scripts before hmrApply re-requires importers).
 */
async function syncNpmModules(): Promise<string[]> {
  const wanted = new Set<string>();
  for (const list of moduleBare.values()) for (const spec of list) wanted.add(spec);

  for (const spec of Array.from(npmBuilt.keys())) {
    if (wanted.has(spec)) continue;
    npmBuilt.delete(spec);
    moduleDeps.delete(spec);
    moduleBare.delete(spec);
    try {
      rmSync(join(MODULES_DIR, encId(spec)));
    } catch {}
  }

  const added: string[] = [];
  for (const spec of Array.from(wanted).sort()) {
    if (npmBuilt.has(spec)) continue;
    const t0 = Date.now();
    if (!(await buildNpmModule(spec))) continue;
    added.push(spec);
    console.log(
      `${stamp()} npm ${spec}: ${((npmBuilt.get(spec) ?? 0) / 1024) | 0} KB in ${Date.now() - t0} ms`,
    );
  }
  return added;
}

// ---- tailwind: the synthetic css-only module -------------------------------
// Absent a tailwind.config.*, EVERY line below is inert and the emitted
// manifest/module set is byte-identical to a pre-Tailwind build.

const TW_CONFIG = findConfig(APP_SRC_DIR, WS_ROOT);
/** Last generated utilities sheet — the change detector for watch mode. */
let twCss: string | null = null;
/** Constant: the factory body never changes, so define() never invalidates. */
const TW_HASH = Bun.hash("").toString(16);

/** Runs Tailwind and rewrites the module script. True when the CSS moved. */
async function buildTailwind(): Promise<boolean> {
  if (!TW_CONFIG) return false;
  const t0 = Date.now();
  const css = await generateTailwind(TW_CONFIG);
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

// ---- change classification ------------------------------------------------

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

// ---- source discovery -----------------------------------------------------

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

// ---- manifest / changes ---------------------------------------------------

let rev = 0;
let vendorRev = 0; // content hash of vendor.js, set by buildVendor()

function writeManifest(files: string[]) {
  // The synthetic Tailwind module goes FIRST: the host evals `modules` in
  // order, the loader's cssOrder is first-setCss-wins, so this is what puts
  // the utilities layer ahead of every component <style>. (It also sorts
  // first — "." < any letter — which keeps prod.ts's sorted aggregation in
  // agreement without a second rule.)
  // npm modules go between the utilities sheet and the app's own files: they
  // must be REGISTERED before the entry runs (the host evals `modules` in
  // order, then boots), and they contribute no CSS, so their position relative
  // to the Tailwind layer is irrelevant.
  // The app's own stylesheets come straight after the utilities layer and
  // before every component `<style>` — the position `import "./index.css"` at
  // the top of an entry module has in a browser build. When one of them
  // declares `@tailwind utilities` itself (the normal Vite+Tailwind shape) the
  // synthetic utilities module is redundant and is left out entirely.
  const wantTw = TW_CONFIG && !appOwnsUtilities();
  const ids = (wantTw ? [TW_ID] : []).concat(cssIds(), npmIds(), files.map(toId));
  const seen = new Set<string>();
  for (const id of ids) {
    const enc = encId(id);
    if (seen.has(enc)) throw new Error(`module filename collision for ${id} -> ${enc}`);
    seen.add(enc);
  }
  // `deps` is additive: the host's reader only looks at vendorRev/entry/modules.
  // The loader gets the same edges from each module script's setDeps() call —
  // this copy is for tooling (prod bundling, graph debugging).
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
        deps,
        window: readWindowConfig(),
        launch: readLaunchConfig(),
        argonSecure: readArgonSecure(),
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

// ---- build passes ---------------------------------------------------------

const stamp = () => `[${new Date().toLocaleTimeString()}]`;

async function fullBuild(reason: string) {
  ensureDirs();
  const t0 = Date.now();
  const bytes = await buildVendor(); // also refreshes vendorRev
  const files = scanSources(APP_SRC_DIR);
  const ids = new Set(files.map(toId));
  prevBlocks.clear();
  moduleDeps.clear();
  moduleBare.clear();
  npmBuilt.clear(); // force a rewrite: the module files may not exist yet
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

  // A newly-imported package needs its script EVALUATED before hmrApply
  // re-requires the importer, so it rides in `changed` like any other module.
  // hmrApply sees a non-.vue id nothing has required yet and skips it, which is
  // exactly right: registering the factory was the whole job.
  for (const spec of await syncNpmModules()) {
    changed.push({ id: spec, kind: "reload" });
    structural = true;
  }

  // ALWAYS re-run Tailwind: a class list changes with the template (a
  // "rerender" edit), with a `:class` binding (a "reload" edit), and even with
  // a plain .js file that builds class strings — there is no cheaper signal
  // than "some content file moved", which is exactly what got us here.
  // A new utility therefore arrives as its OWN css-only entry, next to (not
  // instead of) whatever the edited component needs.
  if (await buildTailwind()) changed.push({ id: TW_ID, kind: "css-only" });
  // A stylesheet that went through the Tailwind pass changes with the templates
  // too (its `@apply` rules and its utilities layer are JIT-generated), so it is
  // re-emitted on the same signal.
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

// ---- entry ----------------------------------------------------------------

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
}
