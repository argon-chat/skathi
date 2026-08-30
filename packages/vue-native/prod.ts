// vue-native PRODUCTION builder: one self-contained IIFE, no loader, no HMR.
//
// Usage: bun prod.ts [app-entry.js] [out-file]
//
//   dist/app.prod.js   IIFE — @vue/runtime-core + src/runtime.ts + every app
//                      module + every SFC <style>, minified, __DEV__ = false.
//                      The Rust host `include_str!`s this file (vn-app with
//                      --features prod) so the shipped exe needs no dist/.
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
// DEBT: compileVue()/the style pipeline below is a deliberate ~100-line copy of
// build.ts. build.ts is owned by the dev-loop track this round, so importing
// from it (or refactoring a shared module out of it) is off limits; the two
// must be merged into a shared `sfc.ts` next round.

import { parse, compileScript, compileTemplate, compileStyle, registerTS } from "@vue/compiler-sfc";
import { createRequire } from "module";
import { resolve, dirname, join, relative, sep } from "path";
import { existsSync, mkdirSync, readFileSync, realpathSync, renameSync, writeFileSync } from "fs";

import {
  TW_ID,
  declaresUtilities,
  findConfig,
  generate as generateTailwind,
  needsTailwindPass,
} from "./tailwind";

// ---- paths ----------------------------------------------------------------

const argv = process.argv.slice(2);
const positional = argv.filter((a) => !a.startsWith("--"));

const PKG_DIR = import.meta.dir; // .../packages/vue-native
const WS_ROOT = resolve(PKG_DIR, "../.."); // repo root — module ids are relative to this
const RUNTIME_ENTRY = join(PKG_DIR, "src", "runtime.ts");

const ENTRY_ABS = resolve(positional[0] ?? join(WS_ROOT, "apps/hello/src/main.ts"));
const APP_SRC_DIR = dirname(ENTRY_ABS);
const OUT_FILE = resolve(positional[1] ?? join(PKG_DIR, "dist", "app.prod.js"));

// `apps/<app>/vn.config.json`'s `window` block, emitted next to the bundle as
// `dist/window.json` and `include_str!`d by crates/vn-app/src/prod.rs. The
// host reads it BEFORE `cx.open_window`, so it cannot ride inside the bundle
// (which evaluates long after the window exists) - see docs/WINDOWING.md 4.1.
// Always written, `{}` when the app declares none: a missing file would be a
// Rust compile error, which is the right failure mode for the bundle but not
// for an optional config.
//
// It sits next to `src/`, which is NOT the same thing as the app's package
// root: `APP_ROOT` below walks up to the nearest `package.json`/`node_modules`
// for module resolution and can land a level higher. Same name, same rule and
// same reason as `build.ts`'s `VN_CONFIG_DIR`.
const VN_CONFIG_DIR = APP_SRC_DIR.endsWith(`${sep}src`) ? dirname(APP_SRC_DIR) : APP_SRC_DIR;
const WINDOW_JSON = join(dirname(OUT_FILE), "window.json");

function windowConfig(): string {
  const path = join(VN_CONFIG_DIR, "vn.config.json");
  if (!existsSync(path)) return "{}";
  try {
    const cfg = JSON.parse(readFileSync(path, "utf8"));
    return JSON.stringify(cfg?.window ?? {}, null, 2);
  } catch (e) {
    console.error(`[prod] ${path}: ${e}`);
    return "{}";
  }
}

/// Same discovery rule as the dev builder — no config, no Tailwind, and the
/// emitted bundle is byte-identical to a pre-Tailwind one.
const TW_CONFIG = findConfig(APP_SRC_DIR, WS_ROOT);

// ---- app package root + TypeScript-aware SFC compilation -------------------
// Mirrors build.ts exactly; see the long comment there for why `fs` and
// `registerTS` are both required to compile a component that writes
// `defineProps<SomeImportedType>()`. npm DEPENDENCIES need nothing special
// here: prod is a single Bun bundle, so a bare specifier resolves and inlines
// like any other import (only "vue"/"vue-native" are re-pointed, below).
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

const SFC_FS = {
  fileExists: (file: string) => existsSync(file),
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

try {
  const ts = appRequire("typescript");
  registerTS(() => ts);
} catch {
  // No typescript in the app: fine unless it actually imports types across a
  // package boundary, and compileScript's own error says so precisely.
}

const toId = (abs: string) => relative(WS_ROOT, abs).split(sep).join("/");
const J = (v: unknown) => JSON.stringify(v);

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
};

function writeAtomic(path: string, text: string) {
  const tmp = path + ".tmp";
  writeFileSync(tmp, text);
  renameSync(tmp, path);
}

// ---- SFC pipeline (see DEBT note in the header) ----------------------------

/** Stable per-file scope id — identical formula to build.ts, so dev and prod
 *  produce byte-identical `data-v-*` ids for the same file. */
const scopeIdOf = (id: string) => Bun.hash(id).toString(16).slice(0, 8);

/** id -> compiled css, filled by the .vue onLoad hook below. */
const cssById = new Map<string, string>();
/** The Bun loader the emitted chunk needs ("ts" for <script lang="ts">). */
let capturedLoader: "js" | "ts" = "js";

function compileVue(absPath: string, source: string): string {
  const id = toId(absPath);
  const { descriptor, errors } = parse(source, { filename: absPath });
  if (errors.length) {
    throw new Error(`${id}: ${errors.map((e) => e.message).join("; ")}`);
  }
  const shortId = scopeIdOf(id);
  const scopeId = `data-v-${shortId}`;
  const hasScoped = descriptor.styles.some((s) => s.scoped);
  // Same rule as build.ts: compileScript leaves TS syntax in its output, so the
  // chunk must be handed to Bun with the matching loader.
  const isTS = /^tsx?$/.test(descriptor.scriptSetup?.lang ?? descriptor.script?.lang ?? "");
  capturedLoader = isTS ? "ts" : "js";

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
          isCustomElement: (tag: string) => tag === "lottie",
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
          isCustomElement: (tag: string) => tag === "lottie",
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
  const code = chunks.join("\n");
  if (/\bcreateStaticVNode\b/.test(code)) {
    throw new Error(`${id}: emitted createStaticVNode — hoistStatic leaked back on (F7)`);
  }

  const cssChunks: string[] = [];
  for (const style of descriptor.styles) {
    const out = compileStyle({
      source: style.content,
      filename: absPath,
      id: scopeId,
      scoped: style.scoped,
      trim: true,
    });
    if (out.errors.length) throw new Error(`${id} <style>: ${out.errors.join("; ")}`);
    cssChunks.push(out.code);
  }
  if (cssChunks.length) cssById.set(id, cssChunks.join("\n"));

  return code;
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

const vuePlugin = {
  name: "vue-sfc-prod",
  setup(build: any) {
    // `import "./index.css"` — the shape every Vite app's entry has, and the
    // file a shadcn-shaped app keeps its whole palette in. Bun would treat a
    // .css file as an asset; it becomes an empty module whose CONTENT went into
    // the stylesheet instead. Same rule as build.ts: when the app has a
    // Tailwind config the sheet goes through Tailwind's own PostCSS pass with
    // ITSELF as the entry, which is what makes its `@tailwind`, `@layer` and
    // `@apply` mean anything.
    build.onLoad({ filter: /\.css$/ }, async (args: any) => {
      const id = toId(args.path);
      if (!cssById.has(id)) {
        const source = await Bun.file(args.path).text();
        if (declaresUtilities(source)) appOwnsUtilities = true;
        let css = source;
        if (TW_CONFIG && needsTailwindPass(source)) {
          css = await generateTailwind(TW_CONFIG, { text: source, from: args.path });
        }
        cssById.set(id, css);
        cssEntryIds.push(id);
      }
      return { contents: "", loader: "js" };
    });
    // Without the dev loader's pre-seeded registry, "vue" (compiled render
    // helpers) and "vue-native" (app code) have to resolve somewhere: both are
    // our runtime module, exactly as the loader aliases them at runtime.
    build.onResolve({ filter: /^(vue|vue-native)$/ }, () => ({ path: RUNTIME_ENTRY }));
    // The other two pre-seeded ids ride vendor.js in dev, which means they are
    // resolved from THIS package's node_modules, not the app's. Prod has no
    // vendor bundle, so the same resolution has to be spelled out or an app
    // that uses them (apps/hello uses both) fails to build.
    build.onResolve({ filter: /^(vue-router|pinia)$/ }, (args: any) => ({
      path: runtimeRequire.resolve(args.path),
    }));
    build.onLoad({ filter: /\.vue$/ }, async (args: any) => {
      const source = await Bun.file(args.path).text();
      const contents = compileVue(args.path, source);
      return { contents, loader: capturedLoader };
    });
  },
};

// ---- build ----------------------------------------------------------------

async function build() {
  const t0 = Date.now();
  let tailwindMs = 0;
  mkdirSync(dirname(OUT_FILE), { recursive: true });
  cssById.clear();
  cssEntryIds.length = 0;
  appOwnsUtilities = false;

  const result = await Bun.build({
    entrypoints: [ENTRY_ABS],
    format: "iife",
    target: "browser",
    define: DEFINES,
    plugins: [vuePlugin],
    minify: { whitespace: true, identifiers: true, syntax: true },
    sourcemap: "none",
  });
  if (!result.success) {
    for (const log of result.logs) console.error(log);
    throw new Error("prod build failed");
  }
  const body = await result.outputs[0].text();

  // (The F7 createStaticVNode tripwire now runs per-SFC inside compileVue —
  // see the comment there for why the finished bundle is the wrong place.)
  // The whole point of the prod define block: no dev branches survive.
  if (/\b__DEV__\b/.test(body)) {
    throw new Error("__DEV__ survived minification — the define block did not apply");
  }

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
  if (TW_CONFIG && !appOwnsUtilities) {
    const t = Date.now();
    cssById.set(TW_ID, await generateTailwind(TW_CONFIG));
    tailwindMs = Date.now() - t;
  }
  const css = Array.from(cssById.keys())
    .sort()
    .map((id) => `(globalThis.__VN_CSS__ ??= []).push(${J(cssById.get(id))});`)
    .join("\n");

  const out = [
    `// vue-native production bundle — entry ${toId(ENTRY_ABS)}`,
    `// generated by packages/vue-native/prod.ts; do not edit`,
    css,
    body,
  ].join("\n");
  writeAtomic(OUT_FILE, out);
  writeAtomic(WINDOW_JSON, windowConfig() + "\n");

  const sfcs = cssById.size - cssEntryIds.length - (cssById.has(TW_ID) ? 1 : 0);
  console.log(
    `prod build: ${(out.length / 1024) | 0} KB (${sfcs} styled SFC${sfcs === 1 ? "" : "s"}` +
      (cssEntryIds.length ? ` + ${cssEntryIds.length} css entr${cssEntryIds.length === 1 ? "y" : "ies"}` : "") +
      (cssById.has(TW_ID)
        ? ` + ${((cssById.get(TW_ID)?.length ?? 0) / 1024).toFixed(1)} KB tailwind in ${tailwindMs} ms`
        : "") +
      `) -> ${relative(WS_ROOT, OUT_FILE).split(sep).join("/")} in ${Date.now() - t0} ms`,
  );
}

await build().catch((e) => {
  console.error(e);
  process.exit(1);
});
